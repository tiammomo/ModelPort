#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"
cd "$ROOT_DIR"

usage() {
  cat <<'USAGE'
Usage: scripts/dev.sh [COMMAND]

Local source development (PostgreSQL must already be available):
  run                 Run in the foreground (default).
  start               Build when needed, then start in the background.
  stop                Stop only native ModelPort processes from this checkout.
  restart             Stop, then start this checkout's native gateway.
  status              Show owned processes, liveness and Provider account state.
  logs                Show the last 80 lines of the native gateway log.
  validate            Validate configuration without contacting an upstream.
  doctor [OPTIONS]    Diagnose setup/runtime; --development checks the toolchain.
  check [--backend]   Run repository checks, or only the Rust checks.
  help                Show this help; no configuration or toolchain required.

For Docker deployments, use scripts/compose-up.sh and docker compose commands.
The original start/stop/restart/status scripts remain compatible entry points.
USAGE
}

load_optional_env() {
  if [[ -f "$ENV_FILE" ]]; then
    load_env
  else
    MODELPORT_BIND="${MODELPORT_BIND:-127.0.0.1:38082}"
  fi
}

start_gateway() {
  load_env
  require_runtime_dir
  local pid
  pid="$(pid_from_file || true)"
  if owned_pid "$pid" || [[ -n "$(project_pids)" ]]; then
    if health_ok; then
      log "ModelPort is already running at $(base_url)"
      return
    fi
    die "this checkout already has a running gateway that is not healthy; run scripts/dev.sh doctor or scripts/dev.sh restart"
  fi
  if health_ok; then
    die "$(base_url) already answers but is not managed by this checkout; inspect that deployment before starting another gateway"
  fi
  rm -f "$PID_FILE"
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
    log "ModelPort started, pid $pid"
    status_gateway
  else
    log "ModelPort failed to become healthy"
    tail -n 80 "$LOG_FILE" >&2 || true
    return 1
  fi
}

stop_gateway() {
  load_optional_env
  local pid
  local still_running
  local -A seen=()
  local pids=()
  pid="$(pid_from_file || true)"
  if owned_pid "$pid"; then
    seen["$pid"]=1
    pids+=("$pid")
  elif pid_running "$pid"; then
    log "ignoring PID file process $pid: it does not belong to this checkout"
  fi
  while read -r pid; do
    if [[ -n "$pid" && -z "${seen[$pid]:-}" ]]; then
      seen["$pid"]=1
      pids+=("$pid")
    fi
  done < <(project_pids)
  if [[ "${#pids[@]}" -eq 0 ]]; then
    log "this checkout has no running native gateway"
    rm -f "$PID_FILE"
    return
  fi
  log "stopping ModelPort pids: ${pids[*]}"
  for pid in "${pids[@]}"; do
    if owned_pid "$pid"; then
      kill -- "$pid" >/dev/null 2>&1 || true
    fi
  done
  for _ in $(seq 1 10); do
    still_running=0
    for pid in "${pids[@]}"; do
      if owned_pid "$pid"; then
        still_running=1
        break
      fi
    done
    [[ "$still_running" -eq 0 ]] && break
    sleep 1
  done
  for pid in "${pids[@]}"; do
    if owned_pid "$pid"; then
      log "forcing pid $pid"
      kill -9 -- "$pid" >/dev/null 2>&1 || true
    fi
  done
  rm -f "$PID_FILE"
  log "stopped"
}

status_gateway() {
  load_optional_env
  local pid
  local owned
  log "bind: $MODELPORT_BIND"
  log "pid file: $PID_FILE"
  log "log file: $LOG_FILE"
  pid="$(pid_from_file || true)"
  if owned_pid "$pid"; then
    log "pid file process: running ($pid)"
  elif pid_running "$pid"; then
    log "pid file process: belongs to another program ($pid)"
  else
    log "pid file process: not running"
  fi
  owned="$(project_pids | tr '\n' ' ' | sed 's/[[:space:]]*$//')"
  log "project processes: ${owned:-none}"
  if ! health_ok; then
    log "liveness: not reachable"
    return
  fi
  log "liveness: ok"
  curl_local -fsS -m 3 "$(base_url)/livez"
  printf '\n'
  if [[ -n "${MODELPORT_AUTH_TOKEN:-}" ]] && command -v node >/dev/null 2>&1; then
    # Parse through stdin so failed diagnostics need no temporary-file cleanup.
    # shellcheck disable=SC2016
    curl_local -fsS -m 3 -H "x-api-key: $MODELPORT_AUTH_TOKEN" "$(base_url)/readyz" 2>/dev/null |
      node -e '
let input = ""
process.stdin.setEncoding("utf8").on("data", chunk => { input += chunk })
process.stdin.on("end", () => {
  if (!input) return
  const providers = Object.values(JSON.parse(input).providerHealth || {})
    .filter(provider => provider && provider.rechargeRequired)
    .map(provider => `${provider.providerId}${provider.rechargeBadge ? `/${provider.rechargeBadge}` : ""}`)
  console.log(`[modelport] pending recharge: ${providers.length ? providers.join(", ") : "none"}`)
})
      ' || true
  fi
}

command_name="${1:-run}"
if [[ $# -gt 0 ]]; then shift; fi
case "$command_name" in
  help|-h|--help) usage; exit 0 ;;
  doctor) exec "$SCRIPT_DIR/doctor.sh" "$@" ;;
  check)
    case "$*" in
      "") exec "$SCRIPT_DIR/check-all.sh" ;;
      --backend) exec "$SCRIPT_DIR/check.sh" ;;
      *) die "use scripts/dev.sh check [--backend]" ;;
    esac
    ;;
  run|start|stop|restart|status|logs|validate)
    [[ $# -eq 0 ]] || die "unexpected arguments; use scripts/dev.sh help"
    ;;
  *) die "unknown command: $command_name; use scripts/dev.sh help" ;;
esac
case "$command_name" in
  run)
    load_env
    setup_cc_fallback
    log "starting ModelPort in foreground at $(base_url)"
    exec cargo run --locked --bin model-port
    ;;
  start) start_gateway ;;
  stop) stop_gateway ;;
  restart) stop_gateway; start_gateway ;;
  status) status_gateway ;;
  logs)
    [[ -f "$LOG_FILE" ]] || die "no native gateway log at $LOG_FILE; use scripts/dev.sh status"
    tail -n 80 "$LOG_FILE"
    ;;
  validate) exec "$SCRIPT_DIR/config-validate.sh" ;;
esac
