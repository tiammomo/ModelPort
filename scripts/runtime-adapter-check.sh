#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

DOCUMENT_PATH="$ROOT_DIR/fixtures/runtime-adapters/qwen-llama-cpp-capabilities-v1alpha1.json"
JSON=0

usage() {
  cat <<'USAGE'
Usage:
  scripts/runtime-adapter-check.sh [options]

Options:
  --document <path>  Capability document to validate. Relative paths resolve
                     from the ModelPort checkout.
  --json             Emit a machine-readable validation result.
  -h, --help         Show this help.

This command reads one local JSON file. It does not connect to an adapter,
start a runtime, download a model, or change GPU state.
USAGE
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --document)
      [[ "$#" -ge 2 ]] || { printf '%s\n' '--document requires a path' >&2; exit 2; }
      DOCUMENT_PATH="$2"
      shift 2
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

if [[ "$DOCUMENT_PATH" != /* ]]; then
  DOCUMENT_PATH="$ROOT_DIR/$DOCUMENT_PATH"
fi
[[ -f "$DOCUMENT_PATH" ]] || die "Runtime Adapter capability document not found: $DOCUMENT_PATH"

arguments=(runtime-adapter validate "$DOCUMENT_PATH")
if [[ "$JSON" -eq 1 ]]; then
  arguments+=(--json)
fi

if release_is_fresh; then
  exec "$RELEASE_BIN" "${arguments[@]}"
fi

setup_cc_fallback
exec cargo run --locked --quiet --bin model-port -- "${arguments[@]}"
