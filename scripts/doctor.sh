#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"
COMPOSE_FILE="${MODELPORT_COMPOSE_FILE:-$ROOT_DIR/docker-compose.yml}"

mode="runtime"
upstream=0
case "${1:-}" in
  "")
    ;;
  --setup)
    mode="setup"
    ;;
  --development)
    mode="development"
    ;;
  --upstream)
    upstream=1
    ;;
  -h|--help)
    cat <<'USAGE'
Usage:
  scripts/doctor.sh --setup
  scripts/doctor.sh --development
  scripts/doctor.sh [--upstream]

Modes:
  --setup        Check Linux, Docker Compose, local files, and required values
                 before the first container pull/build or start. Set
                 MODELPORT_COMPOSE_FILE to select the manifest. Does not start
                 services.
  --development  Check the pinned Rust/Node toolchain and Linux C compiler.
                 Does not require local configuration or running services.
  (no option)    Check configuration plus the running local gateway.
  --upstream     Run the runtime checks and a paid/provider-backed message call.

No mode prints secret values.
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

check_command() {
  local name="$1"
  local hint="$2"
  local executable

  executable="$(command -v "$name" 2>/dev/null || true)"
  if [[ -z "$executable" ]]; then
    fail "$name is required; $hint"
  elif [[ "$executable" =~ ^/mnt/[a-zA-Z]/ ]]; then
    fail "$name resolves to a Windows-mounted executable ($executable); install or activate the Linux version"
  else
    ok "$name is available ($executable)"
  fi
}

check_linux_platform() {
  local kernel
  local architecture
  kernel="$(uname -s 2>/dev/null || true)"
  architecture="$(uname -m 2>/dev/null || true)"

  if [[ "$kernel" == "Linux" ]]; then
    if grep -qi microsoft /proc/sys/kernel/osrelease 2>/dev/null; then
      ok "Linux environment detected (WSL2, architecture=$architecture)"
    else
      ok "Linux environment detected (architecture=$architecture)"
    fi
  else
    fail "ModelPort development and maintained scripts require Linux; detected ${kernel:-unknown}"
  fi

  case "$architecture" in
    x86_64)
      ;;
    aarch64)
      if [[ "$mode" == "setup" ]]; then
        fail "Linux arm64 release images are experimental; the Tier 1 setup path requires x86_64"
      else
        warn "Linux arm64 source builds are experimental and outside the Tier 1 release matrix"
      fi
      ;;
    *)
      if [[ "$mode" == "setup" ]]; then
        fail "architecture '$architecture' is outside the Tier 1 Linux x86_64 setup matrix"
      else
        warn "architecture '$architecture' is not part of the maintained CI matrix"
      fi
      ;;
  esac
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

  MODELPORT_BIND="${MODELPORT_BIND:-127.0.0.1:38082}"
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

  if git -C "$ROOT_DIR" check-ignore -q config.toml; then
    ok "config.toml is ignored by git"
  else
    fail "config.toml is not ignored by git"
  fi
}

check_setup_scripts() {
  local script

  for script in build-container compose-up database-preflight production-preflight smoke-test doctor; do
    if [[ -x "$SCRIPT_DIR/$script.sh" ]]; then
      ok "scripts/$script.sh is executable"
    else
      fail "scripts/$script.sh is not executable"
    fi
  done
}

check_binary_and_scripts() {
  local script

  for script in start stop restart status smoke-test acceptance provider-matrix config-validate build-release check doctor backup-compose compose-up database-preflight production-preflight; do
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

config_has_provider() {
  local provider="$1"
  [[ -f "$ROOT_DIR/config.toml" ]] &&
    grep -Eq "^[[:space:]]*\\[providers\\.${provider}\\][[:space:]]*$" \
      "$ROOT_DIR/config.toml"
}

check_provider_env() {
  check_required_value MODELPORT_BIND
  check_required_secret MODELPORT_AUTH_TOKEN

  if config_has_provider deepseek; then
    if is_placeholder_key; then
      fail "$(upstream_key_name) is missing or placeholder"
    else
      ok "$(upstream_key_name) is set"
    fi

    if [[ -n "${DEEPSEEK_ANTHROPIC_BASE_URL:-}" ]]; then
      ok "DEEPSEEK_ANTHROPIC_BASE_URL=$DEEPSEEK_ANTHROPIC_BASE_URL"
    else
      ok "DEEPSEEK_ANTHROPIC_BASE_URL defaults to https://api.deepseek.com/anthropic"
    fi
  else
    ok "DeepSeek credential is not required by config.toml"
  fi

  if config_has_provider local_qwen; then
    if [[ -n "${QWEN_LOCAL_BASE_URL:-}" ]]; then
      ok "QWEN_LOCAL_BASE_URL=$QWEN_LOCAL_BASE_URL"
    else
      ok "QWEN_LOCAL_BASE_URL uses the config.toml container-network default"
    fi
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

  if config_has_provider local_qwen &&
    [[ "${ANTHROPIC_MODEL:-}" == qwen3.5-* ]]; then
    ok "ANTHROPIC_MODEL selects a local Qwen logical model"
  elif config_has_provider deepseek &&
    [[ "${ANTHROPIC_MODEL:-}" == "${DEEPSEEK_MODEL:-deepseek-v4-flash}" ]]; then
    ok "ANTHROPIC_MODEL matches DeepSeek model"
  else
    warn "ANTHROPIC_MODEL is '${ANTHROPIC_MODEL:-unset}'; verify it is an alias in config.toml"
  fi
}

check_compose_values() {
  check_provider_env
  check_required_value MODELPORT_ADMIN_USERNAME
  check_required_secret MODELPORT_ADMIN_PASSWORD
  if [[ -z "${MODELPORT_DATABASE_URL:-}" ]]; then
    check_required_secret MODELPORT_POSTGRES_PASSWORD
  fi
}

check_compose_setup() {
  check_command git "install it with the Linux distribution package manager"
  check_command curl "install it with the Linux distribution package manager"
  check_command docker "install Docker Engine or Docker Desktop with WSL integration"

  if ! command -v docker >/dev/null 2>&1; then
    return
  fi
  if docker info >/dev/null 2>&1; then
    ok "Docker daemon is reachable"
  else
    fail "Docker daemon is not reachable; start Docker and verify current-user access"
    return
  fi
  if docker compose version >/dev/null 2>&1; then
    ok "Docker Compose v2 is available"
  else
    fail "Docker Compose v2 is required; 'docker compose version' failed"
    return
  fi
  if [[ -f "$ROOT_DIR/config.toml" ]]; then
    ok "local config exists: $ROOT_DIR/config.toml"
  else
    fail "missing $ROOT_DIR/config.toml; copy config.example.toml before setup validation"
    return
  fi

  local body_file
  if [[ ! -f "$COMPOSE_FILE" ]]; then
    fail "Compose manifest does not exist: $COMPOSE_FILE"
    return
  fi
  body_file="$(mktemp)"
  temp_files+=("$body_file")
  if (
    cd "$ROOT_DIR"
    docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" config --quiet
  ) >"$body_file" 2>&1; then
    ok "Docker Compose manifest renders successfully ($COMPOSE_FILE)"
  else
    fail "Docker Compose manifest validation failed"
    sed -n '1,40p' "$body_file" >&2 || true
  fi
}

check_development_tools() {
  local expected_rust
  local rust_version
  local node_major
  local compiler
  local probe_source
  local probe_object

  check_command git "install git"
  check_command cargo "install Rust through rustup"
  check_command rustc "install Rust through rustup"
  check_command rustfmt "run 'rustup component add rustfmt'"
  check_command clippy-driver "run 'rustup component add clippy'"
  check_command node "install Node.js 24"
  check_command npm "install npm with Node.js 24"
  check_command curl "install curl"

  if command -v rustc >/dev/null 2>&1; then
    expected_rust="$(
      sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' \
        "$ROOT_DIR/rust-toolchain.toml" | head -n 1
    )"
    rust_version="$(rustc --version | awk '{print $2}')"
    if [[ -n "$expected_rust" && "$rust_version" == "$expected_rust" ]]; then
      ok "Rust matches rust-toolchain.toml ($rust_version)"
    else
      fail "Rust version is ${rust_version:-unknown}; expected ${expected_rust:-the pinned toolchain}"
    fi
  fi

  if command -v node >/dev/null 2>&1; then
    node_major="$(node -p 'process.versions.node.split(".")[0]')"
    if [[ "$node_major" == "24" ]]; then
      ok "Node.js major version matches CI (24)"
    else
      fail "Node.js major version is ${node_major:-unknown}; expected 24"
    fi
  fi

  setup_cc_fallback
  compiler="${CC_x86_64_unknown_linux_gnu:-${CC:-}}"
  if [[ -z "$compiler" ]]; then
    if command -v cc >/dev/null 2>&1; then
      compiler="$(command -v cc)"
    elif command -v gcc >/dev/null 2>&1; then
      compiler="$(command -v gcc)"
    elif command -v clang >/dev/null 2>&1; then
      compiler="$(command -v clang)"
    fi
  fi
  if [[ -z "$compiler" ]]; then
    fail "a Linux C compiler is required; install build-essential/clang or Zig"
  else
    probe_source="$(mktemp)"
    probe_object="$(mktemp)"
    temp_files+=("$probe_source" "$probe_object")
    printf 'int main(void) { return 0; }\n' >"$probe_source"
    if "$compiler" -x c -c "$probe_source" -o "$probe_object" >/dev/null 2>&1; then
      ok "Linux C compiler can compile a probe"
    else
      fail "the selected Linux C compiler could not compile a probe: $compiler"
    fi
  fi

  if [[ -d "$ROOT_DIR/dashboard/node_modules" ]] &&
    npm --prefix "$ROOT_DIR/dashboard" ls --depth=0 --silent >/dev/null 2>&1; then
    ok "dashboard dependencies match package-lock.json"
  else
    warn "dashboard dependencies are missing or stale; run 'npm --prefix dashboard ci'"
  fi
}

check_static_config() {
  local body_file
  local modelport_container
  local -a validate_command
  body_file="$(mktemp)"
  temp_files+=("$body_file")

  modelport_container="$(
    docker compose -f "$COMPOSE_FILE" ps -q modelport 2>/dev/null || true
  )"
  if [[ -n "$modelport_container" ]]; then
    validate_command=(
      docker compose -f "$COMPOSE_FILE"
      exec -T modelport model-port config validate
    )
  else
    validate_command=("$SCRIPT_DIR/config-validate.sh")
  fi

  if "${validate_command[@]}" > "$body_file" 2>&1; then
    ok "static config validation passed"
  else
    fail "static config validation failed"
    sed -n '1,80p' "$body_file" >&2 || true
  fi
}

check_gateway() {
  if ! command -v curl >/dev/null 2>&1; then
    fail "curl is required for runtime checks"
    return
  fi

  if health_ok; then
    ok "liveness endpoint is reachable: $(base_url)/livez"
  else
    fail "liveness endpoint is not reachable: $(base_url)/livez"
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
      "$(base_url)/readyz" || true
  )"

  if [[ "$status" == "200" ]]; then
    ok "authenticated /readyz returned HTTP 200"
  else
    fail "authenticated /readyz returned HTTP ${status:-unknown}"
    sed -n '1,20p' "$body_file" >&2 || true
  fi

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

  status="$(
    curl_local -sS -m 5 \
      -o "$body_file" \
      -w '%{http_code}' \
      -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
      "$(base_url)/metrics" || true
  )"

  if [[ "$status" == "200" ]] && grep -q '^modelport_uptime_seconds ' "$body_file"; then
    ok "authenticated /metrics returned Prometheus text"
  else
    fail "authenticated /metrics returned HTTP ${status:-unknown} or invalid body"
    sed -n '1,20p' "$body_file" >&2 || true
  fi
}

check_database_alignment() {
  local body_file

  if ! command -v docker >/dev/null 2>&1; then
    warn "docker is unavailable; local Compose database alignment check skipped"
    return
  fi
  if ! docker compose -f "$COMPOSE_FILE" config --services 2>/dev/null \
    | grep -Fxq postgres; then
    warn "selected Compose profile uses an external database; local volume alignment check skipped"
    return
  fi
  if [[ -z "$(docker compose -f "$COMPOSE_FILE" ps -q postgres 2>/dev/null || true)" ]]; then
    warn "selected bundled Compose PostgreSQL is not running; alignment check skipped"
    return
  fi

  body_file="$(mktemp)"
  temp_files+=("$body_file")
  if MODELPORT_COMPOSE_FILE="$COMPOSE_FILE" \
    "$SCRIPT_DIR/database-preflight.sh" >"$body_file" 2>&1; then
    ok "running PostgreSQL image, volume, migrations, and durable state are aligned"
  else
    fail "running PostgreSQL does not match the declared Compose database"
    sed -n '1,40p' "$body_file" >&2 || true
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

  if grep -Fq '"deepseek-v4-flash"' "$settings_file"; then
    ok "VS Code settings references deepseek-v4-flash"
  else
    warn "VS Code settings does not reference deepseek-v4-flash"
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
  local selected_model
  selected_model="$(default_upstream_model)"

  if [[ "$upstream" != "1" ]]; then
    if [[ "$selected_model" == qwen3.5-* ]]; then
      ok "local model is selected; use --upstream only when a real local generation is intended"
    elif is_placeholder_key; then
      warn "upstream test skipped because $(upstream_key_name) is missing or placeholder"
    else
      ok "upstream key is present; run scripts/doctor.sh --upstream for a real model call"
    fi
    return
  fi

  if [[ "$selected_model" != qwen3.5-* ]] && is_placeholder_key; then
    fail "cannot run upstream test because $(upstream_key_name) is missing or placeholder"
    return
  fi

  local body_file
  local model
  local status
  body_file="$(mktemp)"
  temp_files+=("$body_file")
  model="$selected_model"

  status="$(
    curl_local -sS -m 60 \
      -o "$body_file" \
      -w '%{http_code}' \
      -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
      -H 'Content-Type: application/json' \
      "$(base_url)/v1/messages" \
      -d "$(printf '{"model":"%s","max_tokens":128,"messages":[{"role":"user","content":"用一句话回复：ModelPort doctor OK。"}]}' "$model")" || true
  )"

  if [[ "$status" =~ ^[0-9]+$ && "$status" -ge 200 && "$status" -lt 300 ]]; then
    ok "real upstream /v1/messages returned HTTP $status"
  else
    fail "real upstream /v1/messages returned HTTP ${status:-unknown}"
    sed -n '1,40p' "$body_file" >&2 || true
  fi
}

check_linux_platform

case "$mode" in
  setup)
    load_doctor_env
    check_env_is_ignored
    check_setup_scripts
    check_compose_values
    check_compose_setup
    ;;
  development)
    check_development_tools
    ;;
  runtime)
    load_doctor_env
    check_env_is_ignored
    check_binary_and_scripts
    check_provider_env
    check_static_config
    check_database_alignment
    check_gateway
    check_vscode_settings
    check_upstream_message
    ;;
esac

if [[ "$failures" -gt 0 ]]; then
  printf '\nModelPort doctor (%s) failed: %d failure(s), %d warning(s).\n' \
    "$mode" "$failures" "$warnings" >&2
  exit 1
fi

printf '\nModelPort doctor (%s) passed: %d warning(s).\n' "$mode" "$warnings"
