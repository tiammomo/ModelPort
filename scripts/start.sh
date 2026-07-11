#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

load_env
require_runtime_dir

if health_ok; then
  log "ModelPort is already running at $(base_url)"
  exit 0
fi

pid="$(pid_from_file || true)"
if pid_running "$pid"; then
  log "found existing process from $PID_FILE: $pid"
else
  rm -f "$PID_FILE"
fi

if ! release_is_fresh || [[ "${MODELPORT_FORCE_BUILD:-0}" == "1" ]]; then
  "$SCRIPT_DIR/build-release.sh"
fi

log "starting ModelPort in background at $(base_url)"
log "log file: $LOG_FILE"
if command -v setsid >/dev/null 2>&1; then
  setsid "$RELEASE_BIN" >> "$LOG_FILE" 2>&1 < /dev/null &
else
  nohup "$RELEASE_BIN" >> "$LOG_FILE" 2>&1 < /dev/null &
fi
pid="$!"
echo "$pid" > "$PID_FILE"

if wait_for_health 30 1; then
  log "ModelPort started, pid $(cat "$PID_FILE")"
  "$SCRIPT_DIR/status.sh"
else
  log "ModelPort failed to become healthy"
  tail -n 80 "$LOG_FILE" >&2 || true
  exit 1
fi
