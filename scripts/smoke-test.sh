#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

load_env
mode="${1:-gateway}"

log "checking health: $(base_url)/health"
curl_local -fsS -m 5 "$(base_url)/health"
printf '\n'

log "checking authenticated model list"
curl_local -fsS -m 5 \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  "$(base_url)/v1/models"
printf '\n'

if [[ "$mode" != "--upstream" ]]; then
  if is_placeholder_key; then
    log "gateway is healthy; upstream message test skipped because MIMO_OPENAI_API_KEY is placeholder"
  else
    log "gateway is healthy; run scripts/smoke-test.sh --upstream to test real model replies"
  fi
  exit 0
fi

if is_placeholder_key; then
  die "MIMO_OPENAI_API_KEY is missing or placeholder; cannot test real upstream model reply"
fi

log "checking upstream message route"
body_file="$(mktemp)"
trap 'rm -f "$body_file"' EXIT

status="$(
  curl_local -sS -m 60 \
    -o "$body_file" \
    -w '%{http_code}' \
    -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
    -H 'Content-Type: application/json' \
    "$(base_url)/v1/messages" \
    -d '{
      "model": "mimo-v2.5-pro",
      "max_tokens": 256,
      "messages": [
        {
          "role": "user",
          "content": "用一句话回复：ModelPort upstream OK。"
        }
      ]
    }'
)"

cat "$body_file"
printf '\n'

if [[ "$status" -lt 200 || "$status" -ge 300 ]]; then
  die "upstream message route returned HTTP $status"
fi

log "upstream smoke test passed"
