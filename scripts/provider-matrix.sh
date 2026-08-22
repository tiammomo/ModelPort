#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

models=()
all_models=0
run_non_stream=1
run_stream=1
timeout_secs=120
evidence_file=""

usage() {
  cat <<'USAGE'
Usage: scripts/provider-matrix.sh [options]

Runs real compatibility checks through the local ModelPort gateway.
Secrets are read from .env but never printed.

Options:
  --model MODEL          Test one model. Can be repeated.
  --models A,B,C         Test a comma-separated model list.
  --all                  Test each provider-qualified model from /v1/models once.
  --non-stream-only      Only test non-streaming /v1/messages.
  --stream-only          Only test streaming /v1/messages.
  --timeout SECONDS      Per-request timeout. Default: 120.
  --evidence FILE        Write a secret-free, machine-readable JSON evidence record.
  -h, --help             Show this help.

Default model: ANTHROPIC_MODEL, then DEEPSEEK_MODEL, then deepseek-v4-flash.
USAGE
}

add_csv_models() {
  local raw="$1"
  local item

  IFS=',' read -r -a parts <<< "$raw"
  for item in "${parts[@]}"; do
    item="$(printf '%s' "$item" | xargs)"
    if [[ -n "$item" ]]; then
      models+=("$item")
    fi
  done
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --model)
      if [[ -z "${2:-}" ]]; then
        die "--model requires a value"
      fi
      models+=("$2")
      shift 2
      ;;
    --models)
      if [[ -z "${2:-}" ]]; then
        die "--models requires a comma-separated value"
      fi
      add_csv_models "$2"
      shift 2
      ;;
    --all)
      all_models=1
      shift
      ;;
    --non-stream-only)
      run_stream=0
      shift
      ;;
    --stream-only)
      run_non_stream=0
      shift
      ;;
    --timeout)
      timeout_secs="${2:-}"
      if [[ -z "$timeout_secs" || ! "$timeout_secs" =~ ^[0-9]+$ || "$timeout_secs" -lt 1 ]]; then
        die "--timeout requires a positive integer"
      fi
      shift 2
      ;;
    --evidence)
      if [[ -z "${2:-}" ]]; then
        die "--evidence requires a file path"
      fi
      evidence_file="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

if [[ "$run_non_stream" == "0" && "$run_stream" == "0" ]]; then
  die "at least one of non-stream or stream checks must be enabled"
fi

load_env

for command_name in curl node; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    die "$command_name is required"
  fi
done

tmp_files=()
results_file="$(mktemp)"
tmp_files+=("$results_file")
cleanup() {
  if [[ "${#tmp_files[@]}" -gt 0 ]]; then
    rm -f "${tmp_files[@]}"
  fi
}
trap cleanup EXIT

declare -A model_providers=()
declare -A model_provider_ids=()

fetch_model_catalog() {
  local body_file

  body_file="$(mktemp)"
  tmp_files+=("$body_file")
  curl_local -fsS -m 10 \
    -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
    "$(base_url)/v1/models" > "$body_file"

  while IFS=$'\t' read -r model provider provider_id; do
    [[ -z "$model" ]] && continue
    model_providers["$model"]="$provider"
    model_provider_ids["$model"]="$provider_id"
    if [[ "$all_models" == "1" && "$model" == *:* ]]; then
      models+=("$model")
    fi
  done < <(
    # JavaScript template literals must not be expanded by the shell.
    # shellcheck disable=SC2016
    node -e '
      const fs = require("fs");
      const body = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
      const seen = new Set();
      for (const item of body.data || []) {
        if (!item?.id || seen.has(item.id)) continue;
        seen.add(item.id);
        process.stdout.write(`${item.id}\t${item.display_name || ""}\t${item.owned_by || "unknown"}\n`);
      }
    ' "$body_file"
  )
}

request_payload() {
  local model="$1"
  local stream="$2"

  # This matrix probes transport/protocol compatibility. Reasoning is covered
  # by a dedicated acceptance suite; disabling it here prevents a small probe
  # budget from being consumed before the first final-answer token.
  printf '{"model":"%s","max_tokens":256,"stream":%s,"thinking":{"type":"disabled"},"messages":[{"role":"user","content":"只回复 OK。不要解释。"}]}' "$model" "$stream"
}

non_stream_has_text() {
  local body_file="$1"

  node -e '
    const fs = require("fs");
    const body = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    const blocks = Array.isArray(body.content) ? body.content : [];
    const text = blocks
      .filter((block) => block?.type === "text" && typeof block.text === "string")
      .map((block) => block.text.trim())
      .join("");
    process.exit(text.length > 0 ? 0 : 1);
  ' "$body_file"
}

stream_has_text() {
  local body_file="$1"

  node -e '
    const fs = require("fs");
    const raw = fs.readFileSync(process.argv[1], "utf8");
    let event = "";
    let hasText = false;
    let terminal = false;
    for (const line of raw.split(/\r?\n/)) {
      if (line.startsWith("event:")) {
        event = line.slice(6).trim();
        if (event === "message_stop") terminal = true;
        continue;
      }
      if (!line.startsWith("data:")) continue;
      const data = line.slice(5).trim();
      if (data === "[DONE]") {
        terminal = true;
        continue;
      }
      if (!data || event !== "content_block_delta") continue;
      try {
        const parsed = JSON.parse(data);
        const text = parsed?.delta?.text;
        if (typeof text === "string" && text.trim().length > 0) hasText = true;
      } catch {}
    }
    process.exit(hasText && terminal ? 0 : 1);
  ' "$body_file"
}

check_non_stream() {
  local model="$1"
  local body_file
  local status

  body_file="$(mktemp)"
  tmp_files+=("$body_file")

  status="$(
    curl_local -sS -m "$timeout_secs" \
      -o "$body_file" \
      -w '%{http_code}' \
      -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
      -H 'x-modelport-traffic-class: synthetic' \
      -H 'Content-Type: application/json' \
      "$(base_url)/v1/messages" \
      -d "$(request_payload "$model" false)" || true
  )"

  if [[ ! "$status" =~ ^[0-9]+$ ]]; then
    printf 'FAIL: curl failed'
    return 1
  fi

  if [[ "$status" -lt 200 || "$status" -ge 300 ]]; then
    printf 'FAIL: HTTP %s' "$status"
    return 1
  fi

  if non_stream_has_text "$body_file"; then
    printf 'PASS'
    return 0
  fi

  printf 'FAIL: empty text'
  return 1
}

check_stream() {
  local model="$1"
  local body_file
  local status

  body_file="$(mktemp)"
  tmp_files+=("$body_file")

  status="$(
    curl_local -N -sS -m "$timeout_secs" \
      -o "$body_file" \
      -w '%{http_code}' \
      -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
      -H 'x-modelport-traffic-class: synthetic' \
      -H 'Content-Type: application/json' \
      "$(base_url)/v1/messages" \
      -d "$(request_payload "$model" true)" || true
  )"

  if [[ ! "$status" =~ ^[0-9]+$ ]]; then
    printf 'FAIL: curl failed'
    return 1
  fi

  if [[ "$status" -lt 200 || "$status" -ge 300 ]]; then
    printf 'FAIL: HTTP %s' "$status"
    return 1
  fi

  if grep -Eq '^event:[[:space:]]*error' "$body_file"; then
    printf 'FAIL: event error'
    return 1
  fi

  if stream_has_text "$body_file"; then
    printf 'PASS'
    return 0
  fi

  printf 'FAIL: missing stream text or terminal event'
  return 1
}

fetch_model_catalog

if [[ "$all_models" == "1" ]]; then
  mapfile -t models < <(printf '%s\n' "${models[@]}" | awk 'NF && !seen[$0]++')
fi

if [[ "${#models[@]}" -eq 0 && "$all_models" == "1" ]]; then
  die "/v1/models returned no provider-qualified model IDs"
fi

if [[ "${#models[@]}" -eq 0 ]]; then
  models+=("$(default_upstream_model)")
fi

if ! health_ok; then
  die "ModelPort is not healthy at $(base_url). Run scripts/start.sh first."
fi

log "checking provider compatibility through $(base_url)"
printf '| Model | Provider | Non-stream | Stream |\n'
printf '| --- | --- | --- | --- |\n'

failures=0
for model in "${models[@]}"; do
  non_stream_result='SKIP'
  stream_result='SKIP'
  provider="${model_providers[$model]:-unknown}"
  provider_id="${model_provider_ids[$model]:-unknown}"

  if [[ "$run_non_stream" == "1" ]]; then
    if non_stream_result="$(check_non_stream "$model")"; then
      :
    else
      failures=$((failures + 1))
    fi
  fi

  if [[ "$run_stream" == "1" ]]; then
    if stream_result="$(check_stream "$model")"; then
      :
    else
      failures=$((failures + 1))
    fi
  fi

  printf "| \`%s\` | %s | %s | %s |\n" "$model" "$provider" "$non_stream_result" "$stream_result"
  safe_model="${model//$'\t'/ }"
  safe_model="${safe_model//$'\n'/ }"
  safe_provider="${provider//$'\t'/ }"
  safe_provider="${safe_provider//$'\n'/ }"
  safe_provider_id="${provider_id//$'\t'/ }"
  safe_provider_id="${safe_provider_id//$'\n'/ }"
  printf '%s\t%s\t%s\t%s\t%s\n' \
    "$safe_model" "$safe_provider" "$safe_provider_id" "$non_stream_result" "$stream_result" >> "$results_file"
done

if [[ -n "$evidence_file" ]]; then
  evidence_parent="$(dirname "$evidence_file")"
  mkdir -p "$evidence_parent"
  evidence_commit="$(git rev-parse HEAD 2>/dev/null || printf 'unknown')"
  if [[ -z "$(git status --porcelain --untracked-files=normal 2>/dev/null)" ]]; then
    evidence_source_state="clean"
  else
    evidence_source_state="dirty"
  fi
  node - "$results_file" "$evidence_file" "$evidence_commit" "$evidence_source_state" "$failures" <<'NODE'
const fs = require('fs')
const [resultsPath, evidencePath, commit, sourceState, failureCount] = process.argv.slice(2)
const checks = fs.readFileSync(resultsPath, 'utf8')
  .split(/\r?\n/)
  .filter(Boolean)
  .map((line) => {
    const [model, providerLabel, providerId, nonStream, stream] = line.split('\t')
    return {
      model,
      providerId,
      providerLabel,
      nonStream,
      stream,
      toolUse: 'not-run',
    }
  })
const evidence = {
  schemaVersion: 1,
  generatedAt: new Date().toISOString(),
  commit,
  sourceState,
  trafficClass: 'synthetic',
  passed: Number(failureCount) === 0,
  checks,
}
fs.writeFileSync(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 })
NODE
  printf '\nVerification evidence written to %s\n' "$evidence_file"
fi

if [[ "$failures" -gt 0 ]]; then
  printf '\nModelPort provider matrix failed: %d failed check(s).\n' "$failures" >&2
  exit 1
fi

printf '\nModelPort provider matrix passed for %d model(s).\n' "${#models[@]}"
