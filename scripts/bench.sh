#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

upstream=0
iterations=""

usage() {
  cat <<'USAGE'
Usage: scripts/bench.sh [--upstream] [-n iterations]

Measures local ModelPort endpoints without printing secrets.
Default iterations: 30 for gateway endpoints, 3 for --upstream.
USAGE
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --upstream)
      upstream=1
      shift
      ;;
    -n|--iterations)
      iterations="${2:-}"
      if [[ -z "$iterations" || ! "$iterations" =~ ^[0-9]+$ ]]; then
        die "-n/--iterations requires a positive integer"
      fi
      if (( iterations < 1 )); then
        die "-n/--iterations requires a positive integer"
      fi
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

load_env

if ! command -v curl >/dev/null 2>&1; then
  die "curl is required"
fi

if [[ -z "$iterations" ]]; then
  if [[ "$upstream" == "1" ]]; then
    iterations=3
  else
    iterations=30
  fi
fi

tmp_files=()
cleanup() {
  if [[ "${#tmp_files[@]}" -gt 0 ]]; then
    rm -f "${tmp_files[@]}"
  fi
}
trap cleanup EXIT

stats() {
  local label="$1"
  local file="$2"
  local sorted_file
  local count

  sorted_file="$(mktemp)"
  tmp_files+=("$sorted_file")
  sort -n "$file" > "$sorted_file"
  count="$(wc -l < "$sorted_file" | tr -d '[:space:]')"

  awk -v label="$label" -v count="$count" '
    {
      values[NR] = $1
      sum += $1
    }
    END {
      if (count < 1) {
        exit 1
      }
      p50_idx = int((count + 1) / 2)
      p95_idx = int((count * 95 + 99) / 100)
      if (p95_idx < 1) {
        p95_idx = 1
      }
      if (p95_idx > count) {
        p95_idx = count
      }
      printf "%-24s count=%d avg=%.3fs p50=%.3fs p95=%.3fs min=%.3fs max=%.3fs\n",
        label, count, sum / count, values[p50_idx], values[p95_idx], values[1], values[count]
    }
  ' "$sorted_file"
}

measure_health() {
  curl_local -sS -m 5 -o /dev/null -w '%{time_total}\n' "$(base_url)/livez"
}

measure_models() {
  curl_local -sS -m 5 -o /dev/null -w '%{time_total}\n' \
    -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
    "$(base_url)/v1/models"
}

measure_upstream_message() {
  local model
  model="$(default_upstream_model)"
  local payload
  payload="$(printf '{"model":"%s","max_tokens":32,"messages":[{"role":"user","content":"只回复 OK。"}]}' "$model")"

  curl_local -sS -m 120 -o /dev/null -w '%{time_total}\n' \
    -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
    -H 'Content-Type: application/json' \
    "$(base_url)/v1/messages" \
    -d "$payload"
}

run_series() {
  local label="$1"
  local fn="$2"
  local output_file
  local i

  output_file="$(mktemp)"
  tmp_files+=("$output_file")

  for i in $(seq 1 "$iterations"); do
    "$fn" >> "$output_file"
  done

  stats "$label" "$output_file"
}

log "benchmarking $(base_url) with $iterations iteration(s)"
run_series "health" measure_health
run_series "models" measure_models

if [[ "$upstream" == "1" ]]; then
  if is_placeholder_key; then
    die "$(upstream_key_name) is missing or placeholder; cannot benchmark upstream"
  fi
  run_series "messages upstream" measure_upstream_message
fi
