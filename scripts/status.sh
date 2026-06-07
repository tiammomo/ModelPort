#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

if [[ -f "$ENV_FILE" ]]; then
  load_env
else
  MODELPORT_BIND="${MODELPORT_BIND:-127.0.0.1:17878}"
fi

log "bind: $MODELPORT_BIND"
log "pid file: $PID_FILE"
log "log file: $LOG_FILE"

pid="$(pid_from_file || true)"
if pid_running "$pid"; then
  log "pid file process: running ($pid)"
else
  log "pid file process: not running"
fi

project_pids="$(project_pids | tr '\n' ' ' | sed 's/[[:space:]]*$//')"
if [[ -n "$project_pids" ]]; then
  log "project processes: $project_pids"
else
  log "project processes: none"
fi

listen_pids="$(listen_pids | tr '\n' ' ' | sed 's/[[:space:]]*$//')"
if [[ -n "$listen_pids" ]]; then
  log "listener pids: $listen_pids"
else
  log "listener pids: none"
fi

if health_ok; then
  log "health: ok"
  curl_local -fsS -m 3 "$(base_url)/health"
  printf '\n'
else
  log "health: not reachable"
fi
