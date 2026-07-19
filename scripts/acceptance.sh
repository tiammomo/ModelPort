#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

upstream=0
case "${1:-}" in
  "")
    ;;
  --upstream)
    upstream=1
    ;;
  -h|--help)
    cat <<'USAGE'
Usage: scripts/acceptance.sh [--upstream]

Runs a lightweight production acceptance check for personal and small-team deployments.
Default mode does not call the upstream model provider.
Use --upstream to also make one real /v1/messages request through the created API key.
USAGE
    exit 0
    ;;
  *)
    die "unknown argument: $1"
    ;;
esac

load_env

dashboard_url="${MODELPORT_DASHBOARD_URL:-http://127.0.0.1:33002}"
admin_username="${MODELPORT_ADMIN_USERNAME:-admin}"
admin_password="${MODELPORT_ADMIN_PASSWORD:-}"
acceptance_model="$(default_upstream_model)"

cookie_file="$(mktemp)"
headers_file="$(mktemp)"
body_file="$(mktemp)"
backup_file="$(mktemp)"
temp_files=("$cookie_file" "$headers_file" "$body_file" "$backup_file")

created_user_id=""
created_key_id=""
created_team_id=""

cleanup() {
  if [[ -n "$created_key_id" ]]; then
    curl_local -sS -m 10 -b "$cookie_file" \
      -H 'X-ModelPort-CSRF: 1' \
      -X DELETE "$(base_url)/admin/api-keys/$created_key_id" >/dev/null 2>&1 || true
  fi
  if [[ -n "$created_user_id" ]]; then
    curl_local -sS -m 10 -b "$cookie_file" \
      -H 'X-ModelPort-CSRF: 1' \
      -X DELETE "$(base_url)/admin/users/$created_user_id" >/dev/null 2>&1 || true
  fi
  if [[ -n "$created_team_id" ]]; then
    curl_local -sS -m 10 -b "$cookie_file" \
      -H 'X-ModelPort-CSRF: 1' \
      -X DELETE "$(base_url)/admin/teams/$created_team_id" >/dev/null 2>&1 || true
  fi
  rm -f "${temp_files[@]}"
}
trap cleanup EXIT

ok() {
  printf '[ok] %s\n' "$*"
}

warn() {
  printf '[warn] %s\n' "$*" >&2
}

require_command() {
  local name="$1"
  if command -v "$name" >/dev/null 2>&1; then
    ok "$name is available"
  else
    die "$name is required"
  fi
}

json_get() {
  local file="$1"
  local path="$2"
  node -e '
    const fs = require("fs");
    const data = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    const parts = process.argv[2].split(".");
    let value = data;
    for (const part of parts) {
      if (value == null || !(part in value)) process.exit(2);
      value = value[part];
    }
    if (typeof value === "object") {
      process.stdout.write(JSON.stringify(value));
    } else {
      process.stdout.write(String(value));
    }
  ' "$file" "$path"
}

expect_status() {
  local got="$1"
  local want="$2"
  local label="$3"
  if [[ "$got" == "$want" ]]; then
    ok "$label returned HTTP $got"
  else
    printf '[fail] %s returned HTTP %s, expected %s\n' "$label" "${got:-unknown}" "$want" >&2
    sed -n '1,80p' "$body_file" >&2 || true
    exit 1
  fi
}

modelport_cli() {
  if [[ -x "$RELEASE_BIN" ]]; then
    "$RELEASE_BIN" "$@"
  elif [[ -x "$DEBUG_BIN" ]]; then
    "$DEBUG_BIN" "$@"
  else
    setup_cc_fallback
    cargo run --quiet -- "$@"
  fi
}

admin_json() {
  local method="$1"
  local path="$2"
  local payload="${3:-}"
  if [[ -n "$payload" ]]; then
    curl_local -sS -m 20 -b "$cookie_file" -c "$cookie_file" \
      -o "$body_file" -w '%{http_code}' \
      -X "$method" \
      -H 'Content-Type: application/json' \
      -H 'X-ModelPort-CSRF: 1' \
      "$(base_url)$path" \
      -d "$payload"
  else
    curl_local -sS -m 20 -b "$cookie_file" -c "$cookie_file" \
      -o "$body_file" -w '%{http_code}' \
      -X "$method" \
      -H 'X-ModelPort-CSRF: 1' \
      "$(base_url)$path"
  fi
}

message_payload() {
  local max_tokens="$1"
  node -e '
    const model = process.argv[1];
    const maxTokens = Number(process.argv[2]);
    process.stdout.write(JSON.stringify({
      model,
      max_tokens: maxTokens,
      messages: [{ role: "user", content: "Reply with: ModelPort acceptance OK." }]
    }));
  ' "$acceptance_model" "$max_tokens"
}

require_command curl
require_command node

if [[ -z "$admin_password" ]]; then
  die "MODELPORT_ADMIN_PASSWORD is required for acceptance login"
fi

if health_ok; then
  ok "liveness endpoint is reachable: $(base_url)/livez"
else
  die "liveness endpoint is not reachable: $(base_url)/livez"
fi

dashboard_status="$(
  curl_local -sS -m 5 -o /dev/null -w '%{http_code}' "$dashboard_url" || true
)"
if [[ "$dashboard_status" == "200" ]]; then
  ok "dashboard is reachable: $dashboard_url"
else
  warn "dashboard returned HTTP ${dashboard_status:-unknown}: $dashboard_url"
fi

login_status="$(
  curl_local -sS -m 10 \
    -D "$headers_file" \
    -c "$cookie_file" \
    -o "$body_file" \
    -w '%{http_code}' \
    -H 'Content-Type: application/json' \
    "$(base_url)/admin/auth/login" \
    -d "$(node -e 'process.stdout.write(JSON.stringify({ username: process.argv[1], password: process.argv[2] }))' "$admin_username" "$admin_password")"
)"
expect_status "$login_status" "200" "admin login"
json_get "$body_file" "user.id" >/dev/null

models_status="$(
  curl_local -sS -m 10 \
    -o "$body_file" \
    -w '%{http_code}' \
    -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
    "$(base_url)/v1/models"
)"
expect_status "$models_status" "200" "authenticated /v1/models"

suffix="$(date +%s)"
username="acceptance_$suffix"
create_user_payload="$(
  node -e '
    const username = process.argv[1];
    process.stdout.write(JSON.stringify({
      username,
      email: `${username}@modelport.local`,
      password: "acceptance-password-123",
      role: "user",
      status: "active"
    }));
  ' "$username"
)"
status="$(admin_json POST /admin/users "$create_user_payload")"
expect_status "$status" "200" "create acceptance user"
created_user_id="$(json_get "$body_file" "id")"

create_team_payload="$(
  node -e '
    const suffix = process.argv[1];
    process.stdout.write(JSON.stringify({
      name: `acceptance-team-${suffix}`,
      slug: `acceptance-${suffix}`,
      dailyLimitUsd: 1,
      monthlyLimitUsd: 10,
      allowedModels: [],
      allowedProviders: [],
      status: "active"
    }));
  ' "$suffix"
)"
status="$(admin_json POST /admin/teams "$create_team_payload")"
expect_status "$status" "200" "create acceptance team"
created_team_id="$(json_get "$body_file" "id")"

create_key_payload="$(
  node -e '
    process.stdout.write(JSON.stringify({
      userId: process.argv[1],
      username: process.argv[2],
      name: "acceptance-key",
      group: "acceptance",
      teamId: process.argv[3]
    }));
  ' "$created_user_id" "$username" "$created_team_id"
)"
status="$(admin_json POST /admin/api-keys "$create_key_payload")"
expect_status "$status" "200" "create acceptance API key"
created_key_id="$(json_get "$body_file" "id")"
created_api_key="$(json_get "$body_file" "key")"

ip_policy_payload='{"ipRestricted":true,"allowedIps":["203.0.113.10"],"spendLimitUsd":0}'
status="$(admin_json PUT "/admin/api-keys/$created_key_id" "$ip_policy_payload")"
expect_status "$status" "200" "enable API key IP restriction"

status="$(
  curl_local -sS -m 10 \
    -o "$body_file" \
    -w '%{http_code}' \
    -H "x-api-key: $created_api_key" \
    -H "X-Forwarded-For: 198.51.100.10" \
    -H 'Content-Type: application/json' \
    "$(base_url)/v1/messages" \
    -d "$(message_payload 64)" || true
)"
expect_status "$status" "403" "IP restriction rejection"

quota_policy_payload='{"ipRestricted":false,"allowedIps":[],"spendLimitUsd":0.000001}'
status="$(admin_json PUT "/admin/api-keys/$created_key_id" "$quota_policy_payload")"
expect_status "$status" "200" "enable tiny spend limit"

status="$(
  curl_local -sS -m 10 \
    -o "$body_file" \
    -w '%{http_code}' \
    -H "x-api-key: $created_api_key" \
    -H 'Content-Type: application/json' \
    "$(base_url)/v1/messages" \
    -d "$(message_payload 4096)" || true
)"
expect_status "$status" "429" "spend limit rejection"

status="$(
  curl_local -sS -m 10 \
    -b "$cookie_file" \
    -o "$body_file" \
    -w '%{http_code}' \
    "$(base_url)/admin/audit"
)"
expect_status "$status" "200" "audit events"
audit_total="$(json_get "$body_file" "total")"
if [[ "$audit_total" =~ ^[0-9]+$ && "$audit_total" -gt 0 ]]; then
  ok "audit log has $audit_total event(s)"
else
  die "audit log is empty after acceptance operations"
fi

modelport_cli backup export "$backup_file" >/dev/null
modelport_cli backup validate "$backup_file" >/dev/null
ok "backup export and validate succeeded"

if [[ "$upstream" == "1" ]]; then
  if is_placeholder_key; then
    die "cannot run upstream acceptance because $(upstream_key_name) is missing or placeholder"
  fi

  upstream_policy_payload='{"ipRestricted":true,"allowedIps":["203.0.113.10"],"spendLimitUsd":0}'
  status="$(admin_json PUT "/admin/api-keys/$created_key_id" "$upstream_policy_payload")"
  expect_status "$status" "200" "prepare API key for upstream call"

  status="$(
    curl_local -sS -m 90 \
      -o "$body_file" \
      -w '%{http_code}' \
      -H "x-api-key: $created_api_key" \
      -H "X-Forwarded-For: 203.0.113.10" \
      -H 'Content-Type: application/json' \
      "$(base_url)/v1/messages" \
      -d "$(message_payload 96)" || true
  )"
  if [[ "$status" =~ ^2[0-9][0-9]$ ]]; then
    ok "real upstream message returned HTTP $status"
  else
    printf '[fail] real upstream message returned HTTP %s\n' "${status:-unknown}" >&2
    sed -n '1,80p' "$body_file" >&2 || true
    exit 1
  fi
else
  ok "real upstream message skipped; run scripts/acceptance.sh --upstream to include it"
fi

status="$(admin_json DELETE "/admin/api-keys/$created_key_id")"
expect_status "$status" "200" "cleanup acceptance API key"
created_key_id=""

status="$(admin_json DELETE "/admin/users/$created_user_id")"
expect_status "$status" "200" "cleanup acceptance user"
created_user_id=""

status="$(admin_json DELETE "/admin/teams/$created_team_id")"
expect_status "$status" "200" "cleanup acceptance team"
created_team_id=""

printf '\nModelPort acceptance passed for personal/small-team deployment.\n'
