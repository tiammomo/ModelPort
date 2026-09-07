#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"
cd "$ROOT_DIR"

setup_cc_fallback
log "building release binary"
cargo build --release --locked --bin model-port
log "built $RELEASE_BIN"
