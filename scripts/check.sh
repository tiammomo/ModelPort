#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

setup_cc_fallback
log "checking rustfmt"
cargo fmt --all -- --check

log "running tests"
cargo test --locked --all-targets

log "running clippy"
cargo clippy --locked --all-targets --all-features -- -D warnings

log "all checks passed"
