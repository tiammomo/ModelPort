#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

models=()
all_models=0
run_non_stream=1
run_stream=1
timeout_secs=120

usage() {
  cat <<'USAGE'
Usage: scripts/provider-matrix.sh [options]

Runs real compatibility checks through the local ModelPort gateway.
Secrets are read from .env but never printed.

Options:
  --model MODEL          Test one model. Can be repeated.
  --models A,B,C         Test a comma-separated model list.
  --all                  Test every model returned by /v1/models. Requires jq.
  --non-stream-only      Only test non-streaming /v1/messages.
  --stream-only          Only test streaming /v1/messages.
  --timeout SECONDS      Per-request timeout. Default: 120.
  -h, --help             Show this help.

Default model: ANTHROPIC_MODEL, then MIMO_MODEL, then mimo-v2.5-pro.
USAGE
}

add_csv_models() {
  local raw="$1"
  local item

  IFS=',' read -r -a parts <<< "$raw"
  for item in "${parts[@]}"; do
    item="$(printf '%s' "$item" | xargs)"
    if [[ -n "$item" ]]; then
      models+=("$item")
    fi
  done
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --model)
      if [[ -z "${2:-}" ]]; then
        die "--model requires a value"
      fi
      models+=("$2")
      shift 2
      ;;
    --models)
      if [[ -z "${2:-}" ]]; then
        die "--models requires a comma-separated value"
      fi
      add_csv_models "$2"
      shift 2
      ;;
    --all)
      all_models=1
      shift
      ;;
    --non-stream-only)
      run_stream=0
      shift
      ;;
    --stream-only)
      run_non_stream=0
      shift
      ;;
    --timeout)
      timeout_secs="${2:-}"
      if [[ -z "$timeout_secs" || ! "$timeout_secs" =~ ^[0-9]+$ || "$timeout_secs" -lt 1 ]]; then
        die "--timeout requires a positive integer"
      fi
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

if [[ "$run_non_stream" == "0" && "$run_stream" == "0" ]]; then
  die "at least one of non-stream or stream checks must be enabled"
fi

load_env

if ! command -v curl >/dev/null 2>&1; then
  die "curl is required"
fi

tmp_files=()
cleanup() {
  if [[ "${#tmp_files[@]}" -gt 0 ]]; then
    rm -f "${tmp_files[@]}"
  fi
}
trap cleanup EXIT

fetch_all_models() {
  local body_file

  if ! command -v jq >/dev/null 2>&1; then
    die "--all requires jq. Install jq or pass explicit --model values."
  fi

  body_file="$(mktemp)"
  tmp_files+=("$body_file")

  curl_local -fsS -m 10 \
    -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
    "$(base_url)/v1/models" > "$body_file"

  mapfile -t models < <(jq -r '.data[].id' "$body_file" | awk 'NF && !seen[$0]++')
}

request_payload() {
  local model="$1"
  local stream="$2"

  printf '{"model":"%s","max_tokens":64,"stream":%s,"messages":[{"role":"user","content":"只回复 OK。"}]}' "$model" "$stream"
}

check_non_stream() {
  local model="$1"
  local body_file
  local status

  body_file="$(mktemp)"
  tmp_files+=("$body_file")

  status="$(
    curl_local -sS -m "$timeout_secs" \
      -o "$body_file" \
      -w '%{http_code}' \
      -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
      -H 'Content-Type: application/json' \
      "$(base_url)/v1/messages" \
      -d "$(request_payload "$model" false)" || true
  )"

  if [[ ! "$status" =~ ^[0-9]+$ ]]; then
    printf 'FAIL: curl failed'
    return 1
  fi

  if [[ "$status" -lt 200 || "$status" -ge 300 ]]; then
    printf 'FAIL: HTTP %s' "$status"
    return 1
  fi

  if grep -Eq '"type"[[:space:]]*:[[:space:]]*"message"' "$body_file"; then
    printf 'PASS'
    return 0
  fi

  printf 'FAIL: no message body'
  return 1
}

check_stream() {
  local model="$1"
  local body_file
  local status

  body_file="$(mktemp)"
  tmp_files+=("$body_file")

  status="$(
    curl_local -N -sS -m "$timeout_secs" \
      -o "$body_file" \
      -w '%{http_code}' \
      -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
      -H 'Content-Type: application/json' \
      "$(base_url)/v1/messages" \
      -d "$(request_payload "$model" true)" || true
  )"

  if [[ ! "$status" =~ ^[0-9]+$ ]]; then
    printf 'FAIL: curl failed'
    return 1
  fi

  if [[ "$status" -lt 200 || "$status" -ge 300 ]]; then
    printf 'FAIL: HTTP %s' "$status"
    return 1
  fi

  if grep -Eq '^event:[[:space:]]*error' "$body_file"; then
    printf 'FAIL: event error'
    return 1
  fi

  if grep -q 'message_stop' "$body_file"; then
    printf 'PASS'
    return 0
  fi

  printf 'FAIL: no message_stop'
  return 1
}

if [[ "$all_models" == "1" ]]; then
  fetch_all_models
fi

if [[ "${#models[@]}" -eq 0 ]]; then
  models+=("${ANTHROPIC_MODEL:-${MIMO_MODEL:-mimo-v2.5-pro}}")
fi

if ! health_ok; then
  die "ModelPort is not healthy at $(base_url). Run scripts/start.sh first."
fi

log "checking provider compatibility through $(base_url)"
printf '| Model | Non-stream | Stream |\n'
printf '| --- | --- | --- |\n'

failures=0
for model in "${models[@]}"; do
  non_stream_result='SKIP'
  stream_result='SKIP'

  if [[ "$run_non_stream" == "1" ]]; then
    if non_stream_result="$(check_non_stream "$model")"; then
      :
    else
      failures=$((failures + 1))
    fi
  fi

  if [[ "$run_stream" == "1" ]]; then
    if stream_result="$(check_stream "$model")"; then
      :
    else
      failures=$((failures + 1))
    fi
  fi

  printf '| `%s` | %s | %s |\n' "$model" "$non_stream_result" "$stream_result"
done

if [[ "$failures" -gt 0 ]]; then
  printf '\nModelPort provider matrix failed: %d failed check(s).\n' "$failures" >&2
  exit 1
fi

printf '\nModelPort provider matrix passed for %d model(s).\n' "${#models[@]}"
