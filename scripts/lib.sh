#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${MODELPORT_ENV_FILE:-$ROOT_DIR/.env}"
RUNTIME_DIR="${MODELPORT_RUNTIME_DIR:-$ROOT_DIR/.modelport}"
PID_FILE="${MODELPORT_PID_FILE:-$RUNTIME_DIR/model-port.pid}"
LOG_FILE="${MODELPORT_LOG_FILE:-$RUNTIME_DIR/model-port.log}"
RELEASE_BIN="$ROOT_DIR/target/release/model-port"
DEBUG_BIN="$ROOT_DIR/target/debug/model-port"

log() {
  printf '[modelport] %s\n' "$*"
}

die() {
  printf '[modelport] ERROR: %s\n' "$*" >&2
  exit 1
}

load_env() {
  if [[ ! -f "$ENV_FILE" ]]; then
    die "missing env file: $ENV_FILE. Copy .env.example to .env and fill real keys."
  fi

  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a

  MODELPORT_BIND="${MODELPORT_BIND:-127.0.0.1:17878}"
  MODELPORT_AUTH_TOKEN="${MODELPORT_AUTH_TOKEN:-${ANTHROPIC_AUTH_TOKEN:-}}"
  export MODELPORT_BIND MODELPORT_AUTH_TOKEN MODELPORT_ENV_FILE="$ENV_FILE"
}

require_runtime_dir() {
  mkdir -p "$RUNTIME_DIR"
}

base_url() {
  printf 'http://%s' "$MODELPORT_BIND"
}

curl_local() {
  curl --noproxy '*' "$@"
}

health_ok() {
  curl_local -fsS -m 3 "$(base_url)/livez" >/dev/null 2>&1
}

pid_running() {
  local pid="${1:-}"
  [[ -n "$pid" ]] && kill -0 "$pid" >/dev/null 2>&1
}

pid_from_file() {
  [[ -f "$PID_FILE" ]] && tr -d '[:space:]' < "$PID_FILE"
}

project_pids() {
  ps -eo pid=,comm=,args= | awk -v root="$ROOT_DIR" '
    $2 == "model-port" && (index($0, root "/target/debug/model-port") || index($0, root "/target/release/model-port") || index($0, "./target/debug/model-port") || index($0, "./target/release/model-port")) {
      print $1
    }
  '
}

listen_pids() {
  local port="${MODELPORT_BIND##*:}"
  ss -ltnp 2>/dev/null | awk -v port=":$port" '
    index($4, port) && match($0, /pid=[0-9]+/) {
      print substr($0, RSTART + 4, RLENGTH - 4)
    }
  '
}

setup_cc_fallback() {
  if [[ -x /usr/bin/gcc ]]; then
    return
  fi

  if [[ -z "${CC_x86_64_unknown_linux_gnu:-}" && -x "$ROOT_DIR/tools/zig-cc-wrapper.sh" ]]; then
    export CC_x86_64_unknown_linux_gnu="$ROOT_DIR/tools/zig-cc-wrapper.sh"
  fi

  if [[ -z "${CXX_x86_64_unknown_linux_gnu:-}" && -x "$ROOT_DIR/tools/zig-cxx-wrapper.sh" ]]; then
    export CXX_x86_64_unknown_linux_gnu="$ROOT_DIR/tools/zig-cxx-wrapper.sh"
  fi
}

wait_for_health() {
  local attempts="${1:-30}"
  local delay="${2:-1}"

  for _ in $(seq 1 "$attempts"); do
    if health_ok; then
      return 0
    fi
    sleep "$delay"
  done

  return 1
}

auth_header_args() {
  if [[ -z "${MODELPORT_AUTH_TOKEN:-}" ]]; then
    die "MODELPORT_AUTH_TOKEN or ANTHROPIC_AUTH_TOKEN is required"
  fi

  printf '%s\n' "-H" "x-api-key: $MODELPORT_AUTH_TOKEN"
}

default_upstream_model() {
  printf '%s' "${ANTHROPIC_MODEL:-${DEEPSEEK_MODEL:-deepseek-v4-flash}}"
}

is_placeholder_value() {
  local value="${1:-}"
  [[ -z "$value" || "$value" == replace-with-* || "$value" == *placeholder* ]]
}

upstream_key_name() {
  if ! is_placeholder_value "${DEEPSEEK_ANTHROPIC_AUTH_TOKEN:-}"; then
    printf '%s' "DEEPSEEK_ANTHROPIC_AUTH_TOKEN"
  elif [[ -n "${DEEPSEEK_API_KEY:-}" ]]; then
    printf '%s' "DEEPSEEK_API_KEY"
  else
    printf '%s' "DEEPSEEK_ANTHROPIC_AUTH_TOKEN"
  fi
}

upstream_key_value() {
  if ! is_placeholder_value "${DEEPSEEK_ANTHROPIC_AUTH_TOKEN:-}"; then
    printf '%s' "$DEEPSEEK_ANTHROPIC_AUTH_TOKEN"
  else
    printf '%s' "${DEEPSEEK_API_KEY:-}"
  fi
}

is_placeholder_key() {
  local value
  value="$(upstream_key_value)"
  is_placeholder_value "$value"
}
