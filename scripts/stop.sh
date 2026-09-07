#!/usr/bin/env bash
set -euo pipefail

# Compatibility entry point; implementation lives in dev.sh.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/dev.sh" stop "$@"
