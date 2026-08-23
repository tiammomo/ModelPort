#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
STACK_DIR="${LOCAL_INFERENCE_STACK_DIR:-}"
MODELPORT_CONFIG_PATH=""
RELEASE=0
JSON=0

usage() {
  cat <<'USAGE'
Usage:
  scripts/local-inference-check.sh [options]

Options:
  --stack-dir <path>  Deprecated external compatibility mode. May also be
                      supplied through LOCAL_INFERENCE_STACK_DIR.
  --config <path>     ModelPort config for deprecated compatibility mode.
  --release           Add release checks in deprecated compatibility mode.
  --json              Emit machine-readable JSON.
  -h, --help          Show this help.

Without --stack-dir, this command validates ModelPort's repository-owned Qwen
Runtime Adapter fixture. It does not connect to a runtime, download a model,
or change GPU state. Prefer scripts/runtime-adapter-check.sh for new adapters.
USAGE
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --stack-dir)
      [[ "$#" -ge 2 ]] || { printf '%s\n' '--stack-dir requires a path' >&2; exit 2; }
      STACK_DIR="$2"
      shift 2
      ;;
    --config)
      [[ "$#" -ge 2 ]] || { printf '%s\n' '--config requires a path' >&2; exit 2; }
      MODELPORT_CONFIG_PATH="$2"
      shift 2
      ;;
    --release)
      RELEASE=1
      shift
      ;;
    --json)
      JSON=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'unknown option: %s\n\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$STACK_DIR" ]]; then
  if [[ -n "$MODELPORT_CONFIG_PATH" || "$RELEASE" -eq 1 ]]; then
    printf '%s\n' '--config and --release require deprecated --stack-dir compatibility mode' >&2
    exit 2
  fi
  arguments=()
  if [[ "$JSON" -eq 1 ]]; then
    arguments+=(--json)
  fi
  exec "$SCRIPT_DIR/runtime-adapter-check.sh" "${arguments[@]}"
fi

printf '%s\n' \
  'warning: --stack-dir compatibility mode is deprecated; migrate to a v1alpha1 Runtime Adapter capability document.' >&2

if [[ "$(uname -s)" != "Linux" ]]; then
  printf '%s\n' 'local inference integration checks require Linux or WSL2.' >&2
  exit 2
fi
if ! command -v python3 >/dev/null 2>&1; then
  printf '%s\n' 'python3 3.11 or newer is required.' >&2
  exit 2
fi

STACK_DIR="$(cd "$STACK_DIR" 2>/dev/null && pwd)" || {
  printf 'local-inference-stack directory not found: %s\n' "$STACK_DIR" >&2
  exit 2
}
CHECKER="$STACK_DIR/scripts/compatibility-check.py"
CONTRACT="$STACK_DIR/contracts/local-qwen-provider-v1.json"
if [[ ! -f "$CHECKER" || ! -f "$CONTRACT" ]]; then
  printf 'not a compatible legacy local-inference-stack checkout: %s\n' "$STACK_DIR" >&2
  exit 2
fi

arguments=(
  "$CHECKER"
  --modelport-project "$ROOT_DIR"
  --contract "$CONTRACT"
)
if [[ -n "$MODELPORT_CONFIG_PATH" ]]; then
  arguments+=(--modelport-config "$MODELPORT_CONFIG_PATH")
fi
if [[ "$RELEASE" -eq 1 ]]; then
  arguments+=(--release)
fi
if [[ "$JSON" -eq 1 ]]; then
  arguments+=(--json)
fi

exec python3 "${arguments[@]}"
