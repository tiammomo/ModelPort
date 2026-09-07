#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

die() {
  printf '[modelport-capacity] ERROR: %s\n' "$*" >&2
  exit 1
}

main() {
  [[ $# -eq 0 ]] || die "this command accepts no arguments"
  [[ "$(uname -s)" == "Linux" ]] || die "the capacity baseline must run in Linux or WSL2"
  command -v cargo >/dev/null 2>&1 || die "cargo is required"

  cd "$ROOT_DIR"
  cargo test --locked governance::tests:: -- --nocapture
  printf '%s\n' \
    '[modelport-capacity] admission policy unit invariants passed (not a runtime load test):' \
    '  per-user local execution=1, queued=2' \
    '  global interactive queue=16' \
    '  local_first/balanced overflow threshold=5s' \
    '  local_strict timeout=60s with HTTP 429 Retry-After' \
    '  batch queue=independent low priority' \
    '  runtime concurrency/recovery: scripts/acceptance.sh --isolated'
}

main "$@"
