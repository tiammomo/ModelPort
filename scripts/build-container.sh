#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

allow_dirty=0
if [[ "${1:-}" == "--allow-dirty" ]]; then
  allow_dirty=1
  shift
fi
if [[ "$#" -ne 0 ]]; then
  die "usage: scripts/build-container.sh [--allow-dirty]"
fi

source_revision="$(git -C "$ROOT_DIR" rev-parse HEAD)"
source_state="clean"
if [[ -n "$(git -C "$ROOT_DIR" status --porcelain=v1)" ]]; then
  source_state="dirty"
  if [[ "$allow_dirty" != "1" ]]; then
    die "refusing to build a release image from a dirty worktree; commit the reviewed changes or use --allow-dirty for local testing"
  fi
fi

log "building ModelPort image revision=$source_revision source_state=$source_state"
MODELPORT_SOURCE_REVISION="$source_revision" \
MODELPORT_SOURCE_STATE="$source_state" \
  docker compose build modelport

image_id="$(docker image inspect modelport:local --format '{{.Id}}')"
image_revision="$(docker image inspect modelport:local --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')"
image_state="$(docker image inspect modelport:local --format '{{index .Config.Labels "io.modelport.source-state"}}')"

if [[ "$image_revision" != "$source_revision" || "$image_state" != "$source_state" ]]; then
  die "built image provenance labels do not match the requested source state"
fi
log "built modelport:local id=$image_id revision=$image_revision source_state=$image_state"
