#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

upstream=0
case "${1:-}" in
  "")
    ;;
  --upstream)
    upstream=1
    ;;
  -h|--help)
    cat <<'USAGE'
Usage: scripts/doctor.sh [--upstream]

Checks local ModelPort configuration without printing secrets.
Use --upstream to also verify a real /v1/messages call through the configured Mimo upstream.
USAGE
    exit 0
    ;;
  *)
    die "unknown argument: $1"
    ;;
esac

failures=0
warnings=0
temp_files=()

cleanup() {
  if [[ "${#temp_files[@]}" -gt 0 ]]; then
    rm -f "${temp_files[@]}"
  fi
}
trap cleanup EXIT

ok() {
  printf '[ok] %s\n' "$*"
}

warn() {
  warnings=$((warnings + 1))
  printf '[warn] %s\n' "$*" >&2
}

fail() {
  failures=$((failures + 1))
  printf '[fail] %s\n' "$*" >&2
}

is_placeholder_value() {
  local value="${1:-}"
  [[ -z "$value" || "$value" == replace-with-* || "$value" == *placeholder* ]]
}

check_required_secret() {
  local name="$1"
  local value="${!name:-}"

  if is_placeholder_value "$value"; then
    fail "$name is missing or placeholder"
  else
    ok "$name is set"
  fi
}

check_required_value() {
  local name="$1"
  local value="${!name:-}"

  if is_placeholder_value "$value"; then
    fail "$name is missing or placeholder"
  else
    ok "$name=$value"
  fi
}

load_doctor_env() {
  if [[ ! -f "$ENV_FILE" ]]; then
    fail "missing env file: $ENV_FILE"
  else
    set -a
    # shellcheck disable=SC1090
    source "$ENV_FILE"
    set +a
    ok "loaded env file: $ENV_FILE"
  fi

  MODELPORT_BIND="${MODELPORT_BIND:-127.0.0.1:17878}"
  MODELPORT_AUTH_TOKEN="${MODELPORT_AUTH_TOKEN:-${ANTHROPIC_AUTH_TOKEN:-}}"
  export MODELPORT_BIND MODELPORT_AUTH_TOKEN
}

check_env_is_ignored() {
  if ! git -C "$ROOT_DIR" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    warn "not inside a git worktree; skipped .env ignore check"
    return
  fi

  if [[ "$ENV_FILE" == "$ROOT_DIR/.env" ]]; then
    if git -C "$ROOT_DIR" check-ignore -q .env; then
      ok ".env is ignored by git"
    else
      fail ".env is not ignored by git"
    fi
  else
    warn "custom MODELPORT_ENV_FILE is used; verify it is not committed: $ENV_FILE"
  fi
}

check_binary_and_scripts() {
  local script

  for script in start stop restart status smoke-test build-release check doctor; do
    if [[ -x "$SCRIPT_DIR/$script.sh" ]]; then
      ok "scripts/$script.sh is executable"
    else
      fail "scripts/$script.sh is not executable"
    fi
  done

  if [[ -x "$RELEASE_BIN" ]]; then
    ok "release binary exists: $RELEASE_BIN"
  else
    warn "release binary does not exist yet; run scripts/build-release.sh"
  fi
}

check_mimo_env() {
  check_required_value MODELPORT_BIND
  check_required_secret MODELPORT_AUTH_TOKEN
  check_required_secret MIMO_OPENAI_API_KEY
  check_required_value MIMO_MODEL

  if [[ -n "${MIMO_OPENAI_BASE_URL:-}" ]]; then
    ok "MIMO_OPENAI_BASE_URL=$MIMO_OPENAI_BASE_URL"
  elif [[ -n "${BASE_URL:-}" ]]; then
    ok "BASE_URL=$BASE_URL"
  else
    fail "BASE_URL or MIMO_OPENAI_BASE_URL is required for Mimo"
  fi

  if [[ "${ANTHROPIC_AUTH_TOKEN:-}" == "$MODELPORT_AUTH_TOKEN" ]]; then
    ok "ANTHROPIC_AUTH_TOKEN matches MODELPORT_AUTH_TOKEN"
  else
    fail "ANTHROPIC_AUTH_TOKEN must match MODELPORT_AUTH_TOKEN"
  fi

  if [[ "${ANTHROPIC_BASE_URL:-}" == "$(base_url)" ]]; then
    ok "ANTHROPIC_BASE_URL points to ModelPort"
  else
    warn "ANTHROPIC_BASE_URL is '${ANTHROPIC_BASE_URL:-unset}', expected '$(base_url)' for local VS Code"
  fi

  if [[ "${ANTHROPIC_MODEL:-}" == "${MIMO_MODEL:-mimo-v2.5-pro}" ]]; then
    ok "ANTHROPIC_MODEL matches Mimo model"
  else
    warn "ANTHROPIC_MODEL is '${ANTHROPIC_MODEL:-unset}', MIMO_MODEL is '${MIMO_MODEL:-unset}'"
  fi
}

check_gateway() {
  if ! command -v curl >/dev/null 2>&1; then
    fail "curl is required for runtime checks"
    return
  fi

  if health_ok; then
    ok "health endpoint is reachable: $(base_url)/health"
  else
    fail "health endpoint is not reachable: $(base_url)/health"
    return
  fi

  local body_file
  local status
  body_file="$(mktemp)"
  temp_files+=("$body_file")

  status="$(
    curl_local -sS -m 5 \
      -o "$body_file" \
      -w '%{http_code}' \
      -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
      "$(base_url)/v1/models" || true
  )"

  if [[ "$status" == "200" ]]; then
    ok "authenticated /v1/models returned HTTP 200"
  else
    fail "authenticated /v1/models returned HTTP ${status:-unknown}"
    sed -n '1,20p' "$body_file" >&2 || true
  fi
}

check_vscode_settings_text() {
  local settings_file="$1"

  if grep -Fq '"claudeCode.environmentVariables"' "$settings_file"; then
    ok "VS Code settings contains claudeCode.environmentVariables: $settings_file"
  else
    warn "VS Code settings does not contain claudeCode.environmentVariables: $settings_file"
  fi

  if grep -Fq '"ANTHROPIC_BASE_URL"' "$settings_file" && grep -Fq "$(base_url)" "$settings_file"; then
    ok "VS Code settings points ANTHROPIC_BASE_URL to ModelPort"
  else
    warn "VS Code settings may not point ANTHROPIC_BASE_URL to $(base_url)"
  fi

  if grep -Fq '"mimo-v2.5-pro"' "$settings_file"; then
    ok "VS Code settings references mimo-v2.5-pro"
  else
    warn "VS Code settings does not reference mimo-v2.5-pro"
  fi
}

check_vscode_settings() {
  local settings_files=()
  local file
  local found=0

  settings_files+=("$HOME/.config/Code/User/settings.json")
  settings_files+=("$HOME/.config/Code - Insiders/User/settings.json")
  settings_files+=("/mnt/c/Users/pearf/AppData/Roaming/Code/User/settings.json")

  for file in "${settings_files[@]}"; do
    if [[ -f "$file" ]]; then
      found=1
      check_vscode_settings_text "$file"
    fi
  done

  if [[ "$found" == "0" ]]; then
    warn "VS Code settings.json was not found in the common Linux/WSL paths"
  fi
}

check_upstream_message() {
  if [[ "$upstream" != "1" ]]; then
    if is_placeholder_key; then
      warn "upstream test skipped because MIMO_OPENAI_API_KEY is missing or placeholder"
    else
      ok "upstream key is present; run scripts/doctor.sh --upstream for a real model call"
    fi
    return
  fi

  if is_placeholder_key; then
    fail "cannot run upstream test because MIMO_OPENAI_API_KEY is missing or placeholder"
    return
  fi

  local body_file
  local status
  body_file="$(mktemp)"
  temp_files+=("$body_file")

  status="$(
    curl_local -sS -m 60 \
      -o "$body_file" \
      -w '%{http_code}' \
      -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
      -H 'Content-Type: application/json' \
      "$(base_url)/v1/messages" \
      -d '{
        "model": "mimo-v2.5-pro",
        "max_tokens": 128,
        "messages": [
          {
            "role": "user",
            "content": "用一句话回复：ModelPort doctor OK。"
          }
        ]
      }' || true
  )"

  if [[ "$status" =~ ^[0-9]+$ && "$status" -ge 200 && "$status" -lt 300 ]]; then
    ok "real upstream /v1/messages returned HTTP $status"
  else
    fail "real upstream /v1/messages returned HTTP ${status:-unknown}"
    sed -n '1,40p' "$body_file" >&2 || true
  fi
}

load_doctor_env
check_env_is_ignored
check_binary_and_scripts
check_mimo_env
check_gateway
check_vscode_settings
check_upstream_message

if [[ "$failures" -gt 0 ]]; then
  printf '\nModelPort doctor failed: %d failure(s), %d warning(s).\n' "$failures" "$warnings" >&2
  exit 1
fi

printf '\nModelPort doctor passed: %d warning(s).\n' "$warnings"
