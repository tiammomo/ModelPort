#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BACKUP_DIR="${MODELPORT_BACKUP_DIR:-$ROOT_DIR/backups}"
RETENTION_DAYS="${MODELPORT_BACKUP_RETENTION_DAYS:-14}"
POSTGRES_IMAGE="${MODELPORT_BACKUP_POSTGRES_IMAGE:-postgres:16-alpine}"
STAGING_DIR=""
DRILL_CONTAINER=""

usage() {
  cat <<'USAGE'
Usage:
  scripts/backup-compose.sh create
  scripts/backup-compose.sh verify ARCHIVE
  scripts/backup-compose.sh drill ARCHIVE

Environment:
  MODELPORT_BACKUP_DIR             Destination directory (default: ./backups)
  MODELPORT_BACKUP_RETENTION_DAYS  Delete completed archives older than this (default: 14)
  MODELPORT_COMPOSE_ENV_FILE       Compose environment file to include (default: ./.env)
  MODELPORT_BACKUP_POSTGRES_IMAGE  Ephemeral restore-drill image (default: postgres:16-alpine)

Archives contain a complete PostgreSQL dump and plaintext runtime configuration,
including the Compose environment file. Treat every archive as credential material.
USAGE
}

die() {
  printf '[modelport-backup] ERROR: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  if [[ -n "$DRILL_CONTAINER" && "$DRILL_CONTAINER" == modelport-restore-drill-* ]]; then
    docker rm -f "$DRILL_CONTAINER" >/dev/null 2>&1 || true
  fi
  if [[ -n "$STAGING_DIR" && -d "$STAGING_DIR" ]]; then
    rm -rf -- "$STAGING_DIR"
  fi
}
trap cleanup EXIT

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required"
}

validate_settings() {
  [[ "$RETENTION_DAYS" =~ ^[0-9]+$ ]] || die "retention days must be an integer"
  (( RETENTION_DAYS >= 1 && RETENTION_DAYS <= 3650 )) \
    || die "retention days must be in [1, 3650]"
}

compose_env_path() {
  local configured="${MODELPORT_COMPOSE_ENV_FILE:-.env}"
  if [[ "$configured" == /* ]]; then
    printf '%s\n' "$configured"
  else
    printf '%s/%s\n' "$ROOT_DIR" "$configured"
  fi
}

prepare_staging() {
  local parent="$1"
  mkdir -p "$parent"
  STAGING_DIR="$(mktemp -d "$parent/.modelport-backup.XXXXXX")"
  chmod 700 "$STAGING_DIR"
}

validate_archive_members() {
  local archive="$1"
  local member
  while IFS= read -r member; do
    case "$member" in
      /*|../*|*/../*|*/..)
        die "archive contains an unsafe path: $member"
        ;;
    esac
  done < <(tar -tzf "$archive")
}

extract_archive() {
  local archive="$1"
  [[ -f "$archive" ]] || die "archive not found: $archive"
  validate_archive_members "$archive"
  prepare_staging "${TMPDIR:-/tmp}"
  tar -xzf "$archive" -C "$STAGING_DIR"
  [[ -f "$STAGING_DIR/SHA256SUMS" ]] || die "archive is missing SHA256SUMS"
  (
    cd "$STAGING_DIR"
    sha256sum -c SHA256SUMS >/dev/null
  )
  [[ -s "$STAGING_DIR/postgres.dump" ]] || die "archive PostgreSQL dump is empty"
  [[ -s "$STAGING_DIR/environment.env" ]] || die "archive environment file is empty"
  [[ -s "$STAGING_DIR/config.toml" ]] || die "archive config.toml is empty"
}

verify_dump_catalog() {
  docker compose -f "$ROOT_DIR/docker-compose.yml" exec -T postgres \
    pg_restore --list < "$STAGING_DIR/postgres.dump" >/dev/null
}

create_backup() {
  local env_file timestamp final_archive temporary_archive image_id revision source_state
  env_file="$(compose_env_path)"
  [[ -f "$env_file" ]] || die "Compose environment file not found: $env_file"
  [[ -f "$ROOT_DIR/config.toml" ]] || die "config.toml not found: $ROOT_DIR/config.toml"
  docker compose -f "$ROOT_DIR/docker-compose.yml" ps --status running --services postgres \
    | grep -qx postgres || die "Compose PostgreSQL service is not running"

  mkdir -p "$BACKUP_DIR"
  chmod 700 "$BACKUP_DIR"
  prepare_staging "$BACKUP_DIR"
  cp -- "$env_file" "$STAGING_DIR/environment.env"
  cp -- "$ROOT_DIR/config.toml" "$STAGING_DIR/config.toml"
  chmod 600 "$STAGING_DIR/environment.env" "$STAGING_DIR/config.toml"

  docker compose -f "$ROOT_DIR/docker-compose.yml" exec -T postgres sh -c \
    'exec pg_dump --format=custom --no-owner --no-privileges --username="$POSTGRES_USER" --dbname="$POSTGRES_DB"' \
    > "$STAGING_DIR/postgres.dump"
  chmod 600 "$STAGING_DIR/postgres.dump"
  verify_dump_catalog

  image_id="$(docker compose -f "$ROOT_DIR/docker-compose.yml" images -q modelport)"
  revision="$(docker image inspect "$image_id" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' 2>/dev/null || true)"
  source_state="$(docker image inspect "$image_id" --format '{{index .Config.Labels "io.modelport.source-state"}}' 2>/dev/null || true)"
  timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
  python3 - "$STAGING_DIR/manifest.json" "$timestamp" "$(git -C "$ROOT_DIR" rev-parse HEAD)" "$image_id" "$revision" "$source_state" <<'PY'
import json
import os
import sys
from pathlib import Path

path, generated_at, git_commit, image_id, revision, source_state = sys.argv[1:]
manifest = {
    "schemaVersion": 1,
    "service": "model-port",
    "generatedAt": generated_at,
    "containsSecrets": True,
    "scope": ["postgresql", "compose-environment", "provider-configuration"],
    "source": {
        "gitCommit": git_commit,
        "imageId": image_id,
        "imageRevision": revision,
        "imageSourceState": source_state,
    },
}
output = Path(path)
output.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
os.chmod(output, 0o600)
PY
  (
    cd "$STAGING_DIR"
    sha256sum config.toml environment.env manifest.json postgres.dump > SHA256SUMS
    chmod 600 SHA256SUMS
  )

  final_archive="$BACKUP_DIR/modelport-$timestamp.tar.gz"
  temporary_archive="$BACKUP_DIR/.modelport-$timestamp.tar.gz.tmp"
  tar -czf "$temporary_archive" -C "$STAGING_DIR" \
    SHA256SUMS config.toml environment.env manifest.json postgres.dump
  chmod 600 "$temporary_archive"
  mv -- "$temporary_archive" "$final_archive"
  find "$BACKUP_DIR" -maxdepth 1 -type f -name 'modelport-*.tar.gz' \
    -mtime "+$RETENTION_DAYS" -delete
  printf '%s\n' "$final_archive"
}

verify_backup() {
  local archive="$1"
  extract_archive "$archive"
  verify_dump_catalog
  python3 -m json.tool "$STAGING_DIR/manifest.json" >/dev/null
  printf '[modelport-backup] verified %s\n' "$archive"
}

drill_backup() {
  local archive="$1" namespace_count
  extract_archive "$archive"
  verify_dump_catalog
  DRILL_CONTAINER="modelport-restore-drill-$$-$RANDOM"
  docker run --detach --rm --name "$DRILL_CONTAINER" \
    -e POSTGRES_PASSWORD=local-restore-drill-only \
    -e POSTGRES_USER=modelport \
    -e POSTGRES_DB=modelport \
    "$POSTGRES_IMAGE" >/dev/null
  for _ in $(seq 1 60); do
    if docker exec "$DRILL_CONTAINER" pg_isready -U modelport -d modelport >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
  docker exec "$DRILL_CONTAINER" pg_isready -U modelport -d modelport >/dev/null \
    || die "ephemeral PostgreSQL did not become ready"
  docker exec -i "$DRILL_CONTAINER" pg_restore --exit-on-error --no-owner \
    --no-privileges -U modelport -d modelport < "$STAGING_DIR/postgres.dump"
  namespace_count="$(docker exec "$DRILL_CONTAINER" psql -U modelport -d modelport -Atc \
    "select count(*) from modelport_state where namespace in ('auth', 'control')")"
  [[ "$namespace_count" == "2" ]] \
    || die "restored database is missing auth/control namespaces"
  printf '[modelport-backup] isolated restore drill passed for %s\n' "$archive"
}

main() {
  umask 077
  require_command docker
  require_command git
  require_command python3
  require_command sha256sum
  require_command tar
  validate_settings
  case "${1:-}" in
    create)
      [[ $# -eq 1 ]] || die "create accepts no positional arguments"
      create_backup
      ;;
    verify)
      [[ $# -eq 2 ]] || die "verify requires one archive path"
      verify_backup "$2"
      ;;
    drill)
      [[ $# -eq 2 ]] || die "drill requires one archive path"
      drill_backup "$2"
      ;;
    -h|--help|help)
      usage
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
}

main "$@"
