#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

load_env

release_is_fresh() {
  [[ -x "$RELEASE_BIN" ]] || return 1
  ! find "$ROOT_DIR/src" "$ROOT_DIR/Cargo.toml" "$ROOT_DIR/Cargo.lock" -newer "$RELEASE_BIN" -print -quit | grep -q .
}

if [[ "${MODELPORT_FORCE_BUILD:-0}" != "1" ]] && release_is_fresh; then
  "$RELEASE_BIN" config validate
  exit 0
fi

setup_cc_fallback
cargo run -- config validate
