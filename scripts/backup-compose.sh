#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="${MODELPORT_COMPOSE_FILE:-$ROOT_DIR/docker-compose.yml}"
BACKUP_DIR="${MODELPORT_BACKUP_DIR:-$ROOT_DIR/backups}"
RETENTION_DAYS="${MODELPORT_BACKUP_RETENTION_DAYS:-14}"
POSTGRES_IMAGE="${MODELPORT_BACKUP_POSTGRES_IMAGE:-postgres:18.4-alpine}"
STAGING_DIR=""
DRILL_CONTAINER=""

usage() {
  cat <<'USAGE'
Usage:
  scripts/backup-compose.sh create
  scripts/backup-compose.sh verify ARCHIVE
  scripts/backup-compose.sh drill ARCHIVE
  scripts/backup-compose.sh upgrade-drill ARCHIVE

Environment:
  MODELPORT_COMPOSE_FILE          Deployment manifest (default: ./docker-compose.yml)
  MODELPORT_BACKUP_DIR             Destination directory (default: ./backups)
  MODELPORT_BACKUP_RETENTION_DAYS  Delete completed archives older than this (default: 14)
  MODELPORT_BACKUP_POSTGRES_IMAGE  Ephemeral restore/upgrade image (default: postgres:18.4-alpine)

New archives contain a PostgreSQL dump plus secret-free deployment provenance.
Runtime .env and config.toml files are deliberately excluded; recover them from
Git and the production secret manager. Legacy schema-v1 archives may contain
plaintext credentials and remain readable only for migration and recovery.
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

prepare_staging() {
  local parent="$1"
  mkdir -p "$parent"
  STAGING_DIR="$(mktemp -d "$parent/.modelport-backup.XXXXXX")"
  chmod 700 "$STAGING_DIR"
}

validate_archive_members() {
  local archive="$1"
  python3 - "$archive" <<'PY'
import pathlib
import sys
import tarfile

archive = pathlib.Path(sys.argv[1])
allowed = {
    "SHA256SUMS",
    "manifest.json",
    "postgres.dump",
    # Legacy schema-v1 members. New archives never contain these files.
    "config.toml",
    "environment.env",
}
required = {"SHA256SUMS", "manifest.json", "postgres.dump"}
with tarfile.open(archive, "r:gz") as handle:
    members = handle.getmembers()
    names = {member.name for member in members}
    if len(names) != len(members):
        raise SystemExit("archive contains duplicate member names")
    for member in members:
        path = pathlib.PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts:
            raise SystemExit(f"archive contains an unsafe path: {member.name}")
        if member.name not in allowed:
            raise SystemExit(f"archive contains an unexpected member: {member.name}")
        if not member.isfile():
            raise SystemExit(f"archive member is not a regular file: {member.name}")
    missing = sorted(required - names)
    if missing:
        raise SystemExit(f"archive is missing required members: {', '.join(missing)}")
PY
}

validate_checksum_manifest() {
  python3 - "$STAGING_DIR" <<'PY'
import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
schema = manifest.get("schemaVersion")
expected = {
    1: {"config.toml", "environment.env", "manifest.json", "postgres.dump"},
    2: {"manifest.json", "postgres.dump"},
}.get(schema)
if expected is None:
    raise SystemExit(f"unsupported backup schemaVersion: {schema!r}")

entries = []
for line in (root / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
    match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9._-]+)", line)
    if match is None:
        raise SystemExit("SHA256SUMS contains an invalid entry")
    entries.append(match.group(2))
if len(entries) != len(set(entries)):
    raise SystemExit("SHA256SUMS contains duplicate entries")
if set(entries) != expected:
    raise SystemExit(
        "SHA256SUMS member set does not match backup schema: "
        f"actual={sorted(entries)} expected={sorted(expected)}"
    )
PY
}

validate_manifest() {
  python3 - "$STAGING_DIR" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
schema = manifest.get("schemaVersion")
contains_secrets = manifest.get("containsSecrets")
if schema == 1:
    if contains_secrets is not True:
        raise SystemExit("legacy schema-v1 backup must declare containsSecrets=true")
    for name in ("config.toml", "environment.env"):
        if not (root / name).is_file():
            raise SystemExit(f"legacy schema-v1 backup is missing {name}")
    print(
        "[modelport-backup] WARNING: legacy schema-v1 archive contains plaintext "
        "runtime configuration and must be treated as credential material",
        file=sys.stderr,
    )
elif schema == 2:
    if contains_secrets is not False:
        raise SystemExit("schema-v2 backup must declare containsSecrets=false")
    for name in ("config.toml", "environment.env"):
        if (root / name).exists():
            raise SystemExit(f"schema-v2 backup must not contain {name}")
    if manifest.get("configurationRecovery") != "git-and-secret-manager":
        raise SystemExit("schema-v2 backup is missing its configuration recovery contract")
else:
    raise SystemExit(f"unsupported backup schemaVersion: {schema!r}")
PY
}

extract_archive() {
  local archive="$1"
  [[ -f "$archive" ]] || die "archive not found: $archive"
  validate_archive_members "$archive"
  prepare_staging "${TMPDIR:-/tmp}"
  tar -xzf "$archive" -C "$STAGING_DIR"
  [[ -f "$STAGING_DIR/SHA256SUMS" ]] || die "archive is missing SHA256SUMS"
  validate_checksum_manifest
  (
    cd "$STAGING_DIR"
    sha256sum -c SHA256SUMS >/dev/null
  )
  [[ -s "$STAGING_DIR/postgres.dump" ]] || die "archive PostgreSQL dump is empty"
  [[ -s "$STAGING_DIR/manifest.json" ]] || die "archive manifest is empty"
  validate_manifest
}

verify_dump_catalog() {
  docker compose -f "$COMPOSE_FILE" exec -T postgres \
    pg_restore --list < "$STAGING_DIR/postgres.dump" >/dev/null
}

create_backup() {
  local timestamp final_archive temporary_archive
  local container_id image_id revision source_state postgres_container postgres_image postgres_version
  docker compose -f "$COMPOSE_FILE" ps --status running --services postgres \
    | grep -qx postgres || die "Compose PostgreSQL service is not running"

  mkdir -p "$BACKUP_DIR"
  chmod 700 "$BACKUP_DIR"
  prepare_staging "$BACKUP_DIR"

  docker compose -f "$COMPOSE_FILE" exec -T postgres sh -c \
    'exec pg_dump --format=custom --no-owner --no-privileges --username="$POSTGRES_USER" --dbname="$POSTGRES_DB"' \
    > "$STAGING_DIR/postgres.dump"
  chmod 600 "$STAGING_DIR/postgres.dump"
  verify_dump_catalog

  container_id="$(
    docker compose -f "$COMPOSE_FILE" ps -q modelport
  )"
  [[ -n "$container_id" ]] || die "Compose ModelPort service is not running"
  image_id="$(docker inspect "$container_id" --format '{{.Image}}')"
  revision="$(docker image inspect "$image_id" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' 2>/dev/null || true)"
  source_state="$(docker image inspect "$image_id" --format '{{index .Config.Labels "io.modelport.source-state"}}' 2>/dev/null || true)"
  postgres_container="$(docker compose -f "$COMPOSE_FILE" ps -q postgres)"
  [[ -n "$postgres_container" ]] || die "Compose PostgreSQL service is not running"
  postgres_image="$(docker inspect "$postgres_container" --format '{{.Config.Image}}')"
  postgres_version="$(docker compose -f "$COMPOSE_FILE" exec -T postgres sh -c \
    'exec psql --username="$POSTGRES_USER" --dbname="$POSTGRES_DB" --tuples-only --no-align --command="show server_version"')"
  timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
  python3 - "$STAGING_DIR/manifest.json" "$timestamp" "$(git -C "$ROOT_DIR" rev-parse HEAD)" "$image_id" "$revision" "$source_state" "$postgres_image" "$postgres_version" <<'PY'
import json
import os
import sys
from pathlib import Path

(
    path,
    generated_at,
    git_commit,
    image_id,
    revision,
    source_state,
    postgres_image,
    postgres_version,
) = sys.argv[1:]
manifest = {
    "schemaVersion": 2,
    "service": "model-port",
    "generatedAt": generated_at,
    "containsSecrets": False,
    "scope": ["postgresql", "deployment-provenance"],
    "configurationRecovery": "git-and-secret-manager",
    "source": {
        "gitCommit": git_commit,
        "imageId": image_id,
        "imageRevision": revision,
        "imageSourceState": source_state,
    },
    "database": {
        "dumpFormat": "postgresql-custom",
        "sourceImage": postgres_image,
        "sourceVersion": postgres_version.strip(),
    },
}
output = Path(path)
output.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
os.chmod(output, 0o600)
PY
  (
    cd "$STAGING_DIR"
    sha256sum manifest.json postgres.dump > SHA256SUMS
    chmod 600 SHA256SUMS
  )

  final_archive="$BACKUP_DIR/modelport-$timestamp.tar.gz"
  temporary_archive="$BACKUP_DIR/.modelport-$timestamp.tar.gz.tmp"
  tar -czf "$temporary_archive" -C "$STAGING_DIR" \
    SHA256SUMS manifest.json postgres.dump
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
  local archive="$1" require_target_major="${2:-}" namespace_count target_version source_version
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
  target_version="$(docker exec "$DRILL_CONTAINER" psql -U modelport -d modelport -Atc \
    'show server_version')"
  if [[ -n "$require_target_major" && "${target_version%%.*}" != "$require_target_major" ]]; then
    die "upgrade drill requires PostgreSQL $require_target_major, got $target_version from $POSTGRES_IMAGE"
  fi
  docker exec -i "$DRILL_CONTAINER" pg_restore --exit-on-error --no-owner \
    --no-privileges -U modelport -d modelport < "$STAGING_DIR/postgres.dump"
  namespace_count="$(docker exec "$DRILL_CONTAINER" psql -U modelport -d modelport -Atc \
    "select count(*) from modelport_state where namespace in ('auth', 'control')")"
  [[ "$namespace_count" == "2" ]] \
    || die "restored database is missing auth/control namespaces"
  source_version="$(python3 - "$STAGING_DIR/manifest.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    print(json.load(handle).get("database", {}).get("sourceVersion", "unknown"))
PY
)"
  if [[ -n "$require_target_major" ]]; then
    printf '[modelport-backup] isolated PostgreSQL upgrade drill passed: source=%s target=%s archive=%s\n' \
      "$source_version" "$target_version" "$archive"
  else
    printf '[modelport-backup] isolated restore drill passed for %s on PostgreSQL %s\n' \
      "$archive" "$target_version"
  fi
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
    upgrade-drill)
      [[ $# -eq 2 ]] || die "upgrade-drill requires one archive path"
      drill_backup "$2" 18
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
