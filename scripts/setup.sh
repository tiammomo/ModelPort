#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  cat <<'USAGE'
Usage: scripts/setup.sh [DIRECTORY]

Create minimal .env and config.toml files for a local Compose installation.
Existing files are preserved. Generated secrets are stored only in .env (0600).
Set the Provider key in .env before running scripts/doctor.sh --setup.
USAGE
  exit 0
fi
[[ $# -le 1 && "${1:-}" != -* ]] || { printf 'Use scripts/setup.sh --help\n' >&2; exit 1; }
target_dir="${1:-$ROOT_DIR}"
mkdir -p "$target_dir"
umask 077
set -o noclobber

random_secret() {
  od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
}

if [[ ! -e "$target_dir/.env" ]]; then
  cat > "$target_dir/.env" <<EOF
# Generated local Compose configuration. Advanced options: docs/CONFIGURATION.md
MODELPORT_AUTH_TOKEN=$(random_secret)
MODELPORT_ADMIN_USERNAME=admin
MODELPORT_ADMIN_PASSWORD=Mp_$(random_secret)
MODELPORT_POSTGRES_PASSWORD=$(random_secret)
MODELPORT_BIND=127.0.0.1:38082
MODELPORT_DEFAULT_PROVIDER=deepseek
DEEPSEEK_ANTHROPIC_AUTH_TOKEN=replace-with-provider-key
ANTHROPIC_AUTH_TOKEN=\${MODELPORT_AUTH_TOKEN}
ANTHROPIC_MODEL=deepseek-v4-flash
EOF
  printf '[modelport] Created %s/.env with unique local credentials.\n' "$target_dir"
else
  printf '[modelport] Preserved existing %s/.env\n' "$target_dir"
fi
if [[ ! -e "$target_dir/config.toml" ]]; then
  cat "$ROOT_DIR/config.example.toml" > "$target_dir/config.toml"
  # This file contains only configuration and secret references. The gateway
  # runs as an unprivileged container user; only .env requires owner-only read.
  chmod 0644 "$target_dir/config.toml"
  printf '[modelport] Created %s/config.toml\n' "$target_dir"
else
  printf '[modelport] Preserved existing %s/config.toml\n' "$target_dir"
fi
printf '[modelport] Set the Provider key in .env, then run scripts/doctor.sh --setup.\n'
