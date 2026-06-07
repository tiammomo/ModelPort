#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${ZIG_BIN:-}" ]]; then
  zig_bin="$ZIG_BIN"
elif command -v zig >/dev/null 2>&1; then
  zig_bin="$(command -v zig)"
else
  zig_bin="/home/tiammomo/.local/share/dev-tools/zig/0.16.0/zig"
fi
args=("c++" "-target" "x86_64-linux-gnu")

for arg in "$@"; do
  case "$arg" in
    --target=x86_64-unknown-linux-gnu)
      ;;
    *)
      args+=("$arg")
      ;;
  esac
done

exec "$zig_bin" "${args[@]}"
