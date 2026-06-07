#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

if [[ -f "$ENV_FILE" ]]; then
  load_env
fi

declare -A seen=()
pids=()

pid="$(pid_from_file || true)"
if pid_running "$pid"; then
  seen["$pid"]=1
  pids+=("$pid")
fi

while read -r project_pid; do
  [[ -z "$project_pid" ]] && continue
  if [[ -z "${seen[$project_pid]:-}" ]]; then
    seen["$project_pid"]=1
    pids+=("$project_pid")
  fi
done < <(project_pids)

while read -r listen_pid; do
  [[ -z "$listen_pid" ]] && continue
  if [[ -z "${seen[$listen_pid]:-}" ]]; then
    seen["$listen_pid"]=1
    pids+=("$listen_pid")
  fi
done < <(listen_pids)

if [[ "${#pids[@]}" -eq 0 ]]; then
  log "ModelPort is not running"
  rm -f "$PID_FILE"
  exit 0
fi

log "stopping ModelPort pids: ${pids[*]}"
kill "${pids[@]}" >/dev/null 2>&1 || true

for _ in $(seq 1 10); do
  still_running=0
  for pid in "${pids[@]}"; do
    if pid_running "$pid"; then
      still_running=1
      break
    fi
  done
  [[ "$still_running" -eq 0 ]] && break
  sleep 1
done

for pid in "${pids[@]}"; do
  if pid_running "$pid"; then
    log "forcing pid $pid"
    kill -9 "$pid" >/dev/null 2>&1 || true
  fi
done

rm -f "$PID_FILE"
log "stopped"
