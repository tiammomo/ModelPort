#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${ZIG_BIN:-}" ]]; then
  if ! zig_bin="$(command -v -- "$ZIG_BIN" 2>/dev/null)"; then
    printf 'modelport: ZIG_BIN does not point to an executable Zig compiler: %s\n' "$ZIG_BIN" >&2
    exit 127
  fi
elif zig_bin="$(command -v zig 2>/dev/null)"; then
  :
elif [[ -x "${HOME:-}/.local/share/dev-tools/zig/current/zig" ]]; then
  zig_bin="${HOME}/.local/share/dev-tools/zig/current/zig"
else
  printf 'modelport: Zig compiler not found; install zig or set ZIG_BIN to its executable path\n' >&2
  exit 127
fi
args=("cc" "-target" "x86_64-linux-gnu")

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
