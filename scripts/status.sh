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
  log "liveness: ok"
  curl_local -fsS -m 3 "$(base_url)/livez"
  printf '\n'
  if [[ -n "${MODELPORT_AUTH_TOKEN:-}" ]] && command -v node >/dev/null 2>&1; then
    readyz_file="$(mktemp)"
    trap 'rm -f "$readyz_file"' EXIT
    if curl_local -fsS -m 3 \
      -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
      "$(base_url)/readyz" > "$readyz_file" 2>/dev/null; then
      recharge_summary="$(
        node -e '
const fs = require("node:fs")
const body = JSON.parse(fs.readFileSync(process.argv[1], "utf8"))
const providers = Object.values(body.providerHealth || {})
  .filter((provider) => provider && provider.rechargeRequired)
  .map((provider) => {
    const badge = provider.rechargeBadge ? `/${provider.rechargeBadge}` : ""
    return `${provider.providerId}${badge}`
  })
console.log(providers.length > 0 ? `pending recharge: ${providers.join(", ")}` : "pending recharge: none")
        ' "$readyz_file"
      )"
      log "$recharge_summary"
    fi
  fi
else
  log "liveness: not reachable"
fi
