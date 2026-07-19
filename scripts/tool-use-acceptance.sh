#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/lib.sh"

mode="mock"
timeout_secs=60
max_tokens="${MODELPORT_TOOL_USE_MAX_TOKENS:-128}"

usage() {
  cat <<'USAGE'
Usage: scripts/tool-use-acceptance.sh [options]

Runs Tool Use compatibility checks through the local ModelPort gateway.
Default mode uses a temporary local OpenAI-compatible mock provider and does not consume upstream quota.

Options:
  --mock               Use local mock upstream. Default.
  --upstream           Use the configured default provider and make real upstream Tool Use calls.
  --timeout SECONDS    Per-request timeout. Default: 60.
  --max-tokens N       Output-token budget for real Tool Use calls. Default: 128.
                       Increase this for reasoning models that think before answering.
  -h, --help           Show this help.

Required in --mock mode:
  MODELPORT_ADMIN_PASSWORD must be set so the script can create and clean up a temporary provider.

USAGE
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --mock)
      mode="mock"
      shift
      ;;
    --upstream)
      mode="upstream"
      shift
      ;;
    --timeout)
      timeout_secs="${2:-}"
      if [[ -z "$timeout_secs" || ! "$timeout_secs" =~ ^[0-9]+$ || "$timeout_secs" -lt 1 ]]; then
        die "--timeout requires a positive integer"
      fi
      shift 2
      ;;
    --max-tokens)
      max_tokens="${2:-}"
      if [[ -z "$max_tokens" || ! "$max_tokens" =~ ^[0-9]+$ || "$max_tokens" -lt 1 ]]; then
        die "--max-tokens requires a positive integer"
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

if [[ -z "$max_tokens" || ! "$max_tokens" =~ ^[0-9]+$ || "$max_tokens" -lt 1 ]]; then
  die "MODELPORT_TOOL_USE_MAX_TOKENS must be a positive integer"
fi

load_env

for command_name in curl node; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    die "$command_name is required"
  fi
done

if ! health_ok; then
  die "ModelPort is not healthy at $(base_url). Run scripts/start.sh or docker compose up first."
fi

cookie_file="$(mktemp)"
body_file="$(mktemp)"
mock_server_file="$(mktemp)"
mock_ready_file="$(mktemp)"
mock_log_file="$(mktemp)"
temp_files=("$cookie_file" "$body_file" "$mock_server_file" "$mock_ready_file" "$mock_log_file")

admin_username="${MODELPORT_ADMIN_USERNAME:-admin}"
admin_password="${MODELPORT_ADMIN_PASSWORD:-}"
provider_id=""
mock_pid=""
mock_port=""
mock_host=""
test_model=""

cleanup() {
  if [[ -n "$provider_id" ]]; then
    curl_local -sS -m 10 -b "$cookie_file" \
      -H 'X-ModelPort-CSRF: 1' \
      -X DELETE "$(base_url)/admin/providers/$provider_id?force=true" >/dev/null 2>&1 || true
  fi
  if [[ -n "$mock_pid" ]]; then
    kill "$mock_pid" >/dev/null 2>&1 || true
    wait "$mock_pid" >/dev/null 2>&1 || true
  fi
  rm -f "${temp_files[@]}"
}
trap cleanup EXIT

ok() {
  printf '[ok] %s\n' "$*"
}

expect_status() {
  local got="$1"
  local want="$2"
  local label="$3"
  if [[ "$got" == "$want" ]]; then
    ok "$label returned HTTP $got"
  else
    printf '[fail] %s returned HTTP %s, expected %s\n' "$label" "${got:-unknown}" "$want" >&2
    sed -n '1,120p' "$body_file" >&2 || true
    exit 1
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
    process.stdout.write(typeof value === "object" ? JSON.stringify(value) : String(value));
  ' "$file" "$path"
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

message_request() {
  local model="$1"
  local stream="$2"
  node -e '
    const model = process.argv[1];
    const stream = process.argv[2] === "true";
    const maxTokens = Number(process.argv[3]);
    process.stdout.write(JSON.stringify({
      model,
      max_tokens: maxTokens,
      temperature: 0,
      stream,
      tools: [{
        name: "read_file",
        description: "Read a project file",
        input_schema: {
          type: "object",
          properties: {
            path: { type: "string" }
          },
          required: ["path"]
        }
      }],
      tool_choice: {
        type: "tool",
        name: "read_file",
        disable_parallel_tool_use: true
      },
      messages: [{
        role: "user",
        content: "Use read_file on Cargo.toml."
      }]
    }));
  ' "$model" "$stream" "$max_tokens"
}

tool_result_request() {
  local model="$1"
  node -e '
    const model = process.argv[1];
    const maxTokens = Number(process.argv[2]);
    process.stdout.write(JSON.stringify({
      model,
      max_tokens: maxTokens,
      temperature: 0,
      messages: [
        {
          role: "assistant",
          content: [{
            type: "tool_use",
            id: "toolu_acceptance_read",
            name: "read_file",
            input: { path: "Cargo.toml" }
          }]
        },
        {
          role: "user",
          content: [{
            type: "tool_result",
            tool_use_id: "toolu_acceptance_read",
            content: [{ type: "text", text: "name = \"model-port\"" }]
          }]
        },
        {
          role: "user",
          content: "Summarize the tool result in two words."
        }
      ]
    }));
  ' "$model" "$max_tokens"
}

invalid_tool_choice_request() {
  local model="$1"
  node -e '
    const model = process.argv[1];
    process.stdout.write(JSON.stringify({
      model,
      max_tokens: 32,
      tools: [{
        name: "read_file",
        input_schema: { type: "object" }
      }],
      tool_choice: { type: "tool", name: "write_file" },
      messages: [{ role: "user", content: "hello" }]
    }));
  ' "$model"
}

duplicate_tool_result_request() {
  local model="$1"
  node -e '
    const model = process.argv[1];
    process.stdout.write(JSON.stringify({
      model,
      max_tokens: 32,
      messages: [
        {
          role: "assistant",
          content: [{
            type: "tool_use",
            id: "toolu_dup",
            name: "read_file",
            input: { path: "Cargo.toml" }
          }]
        },
        {
          role: "user",
          content: [
            { type: "tool_result", tool_use_id: "toolu_dup", content: "first" },
            { type: "tool_result", tool_use_id: "toolu_dup", content: "second" }
          ]
        }
      ]
    }));
  ' "$model"
}

strict_response_guard_request() {
  local model="$1"
  local tool_name="$2"
  local prompt="$3"
  node -e '
    process.stdout.write(JSON.stringify({
      model: process.argv[1],
      max_tokens: 64,
      tools: [{
        name: process.argv[2],
        input_schema: { type: "object" }
      }],
      tool_choice: { type: "tool", name: process.argv[2] },
      messages: [{ role: "user", content: process.argv[3] }]
    }));
  ' "$model" "$tool_name" "$prompt"
}

strict_schema_guard_request() {
  local model="$1"
  node -e '
    process.stdout.write(JSON.stringify({
      model: process.argv[1],
      max_tokens: 64,
      tools: [{
        name: "read_file",
        input_schema: {
          type: "object",
          properties: {
            path: { type: "string", minLength: 1 },
            encoding: { enum: ["utf8", "base64"] }
          },
          required: ["path", "encoding"],
          additionalProperties: false
        }
      }],
      tool_choice: { type: "tool", name: "read_file" },
      messages: [{ role: "user", content: "MODELPORT_SCHEMA_MISMATCH_FIXTURE" }]
    }));
  ' "$model"
}

post_message() {
  local payload="$1"
  curl_local -sS -m "$timeout_secs" \
    -o "$body_file" \
    -w '%{http_code}' \
    -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
    -H 'Content-Type: application/json' \
    "$(base_url)/v1/messages" \
    -d "$payload" || true
}

post_message_stream() {
  local payload="$1"
  curl_local -N -sS -m "$timeout_secs" \
    -o "$body_file" \
    -w '%{http_code}' \
    -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
    -H 'Content-Type: application/json' \
    "$(base_url)/v1/messages" \
    -d "$payload" || true
}

assert_tool_response() {
  node -e '
    const fs = require("fs");
    const body = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    const tool = (body.content || []).find((block) => block?.type === "tool_use");
    if (!tool) {
      console.error("missing tool_use block");
      process.exit(1);
    }
    if (tool.name !== "read_file") {
      console.error(`unexpected tool name: ${tool.name}`);
      process.exit(1);
    }
    if (tool.input?.path !== "Cargo.toml") {
      console.error(`unexpected tool input: ${JSON.stringify(tool.input)}`);
      process.exit(1);
    }
    if (body.stop_reason !== "tool_use") {
      console.error(`unexpected stop_reason: ${body.stop_reason}`);
      process.exit(1);
    }
  ' "$body_file"
}

assert_tool_stream() {
  node -e '
    const fs = require("fs");
    const raw = fs.readFileSync(process.argv[1], "utf8");
    let sawToolStart = false;
    let inputJson = "";
    let sawStopReason = false;
    let currentEvent = "";
    for (const line of raw.split(/\r?\n/)) {
      if (line.startsWith("event:")) {
        currentEvent = line.slice(6).trim();
        continue;
      }
      if (!line.startsWith("data:")) continue;
      const data = line.slice(5).trim();
      if (!data || data === "[DONE]") continue;
      let parsed;
      try { parsed = JSON.parse(data); } catch { continue; }
      if (currentEvent === "content_block_start" && parsed?.content_block?.type === "tool_use" && parsed.content_block.name === "read_file") {
        sawToolStart = true;
      }
      if (currentEvent === "content_block_delta" && parsed?.delta?.type === "input_json_delta") {
        inputJson += String(parsed.delta.partial_json || "");
      }
      if (currentEvent === "message_delta" && parsed?.delta?.stop_reason === "tool_use") {
        sawStopReason = true;
      }
    }
    if (!sawToolStart) {
      console.error("missing streaming tool_use start");
      process.exit(1);
    }
    if (!inputJson) {
      console.error("missing streaming input_json_delta");
      process.exit(1);
    }
    let input;
    try {
      input = JSON.parse(inputJson);
    } catch (error) {
      console.error(`streaming input_json_delta did not form valid JSON: ${inputJson}`);
      process.exit(1);
    }
    if (input?.path !== "Cargo.toml") {
      console.error(`unexpected streaming tool input: ${JSON.stringify(input)}`);
      process.exit(1);
    }
    if (!sawStopReason) {
      console.error("missing streaming tool_use stop_reason");
      process.exit(1);
    }
  ' "$body_file"
}

assert_text_response() {
  node -e '
    const fs = require("fs");
    const body = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    const text = (body.content || [])
      .filter((block) => block?.type === "text")
      .map((block) => block.text || "")
      .join("");
    if (!text.trim()) {
      console.error("missing text response");
      process.exit(1);
    }
  ' "$body_file"
}

assert_mock_received_parallel_false() {
  if ! grep -q '"parallel_tool_calls":false' "$mock_log_file"; then
    printf '[fail] mock upstream did not receive parallel_tool_calls=false\n' >&2
    sed -n '1,120p' "$mock_log_file" >&2 || true
    exit 1
  fi
  ok "Anthropic disable_parallel_tool_use mapped to OpenAI parallel_tool_calls=false"
}

mock_provider_host() {
  if [[ -n "${MODELPORT_TOOL_USE_MOCK_HOST:-}" ]]; then
    printf '%s' "$MODELPORT_TOOL_USE_MOCK_HOST"
    return
  fi

  if command -v docker >/dev/null 2>&1 \
    && docker compose ps modelport --status running >/dev/null 2>&1 \
    && docker compose ps modelport --status running | grep -q 'modelport-modelport'; then
    printf '%s' 'host.docker.internal'
    return
  fi

  printf '%s' '127.0.0.1'
}

start_mock_upstream() {
  cat > "$mock_server_file" <<'NODE'
const http = require("http");
const fs = require("fs");

const readyFile = process.argv[2];
const logFile = process.argv[3];

function readBody(req) {
  return new Promise((resolve) => {
    let body = "";
    req.on("data", (chunk) => { body += chunk; });
    req.on("end", () => resolve(body));
  });
}

function writeJson(res, payload) {
  res.writeHead(200, { "content-type": "application/json" });
  res.end(JSON.stringify(payload));
}

const server = http.createServer(async (req, res) => {
  if (req.method !== "POST" || req.url !== "/v1/chat/completions") {
    res.writeHead(404, { "content-type": "application/json" });
    res.end(JSON.stringify({ error: "not found" }));
    return;
  }

  const raw = await readBody(req);
  fs.appendFileSync(logFile, `${raw}\n`, "utf8");
  const body = JSON.parse(raw || "{}");
  const lastMessage = [...(body.messages || [])].reverse().find((message) => message.role !== "system");
  const lastText = typeof lastMessage?.content === "string" ? lastMessage.content : "";

  if (body.stream) {
    res.writeHead(200, {
      "content-type": "text/event-stream",
      "cache-control": "no-cache"
    });
    res.write('data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_acceptance","function":{"name":"read_file","arguments":""}}]},"finish_reason":null,"index":0}]}\n\n');
    res.write('data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\\"path\\":\\"Cargo.toml\\"}"}}]},"finish_reason":null,"index":0}]}\n\n');
    res.write('data: {"choices":[{"delta":{},"finish_reason":"tool_calls","index":0}]}\n\n');
    res.write("data: [DONE]\n\n");
    res.end();
    return;
  }

  if (lastMessage?.role === "tool" || (body.messages || []).some((message) => message.role === "tool")) {
    writeJson(res, {
      id: "chatcmpl_tool_result_acceptance",
      choices: [{
        finish_reason: "stop",
        message: {
          role: "assistant",
          content: "tool result accepted"
        }
      }],
      usage: { prompt_tokens: 8, completion_tokens: 3 }
    });
    return;
  }

  if (lastText.includes("MODELPORT_INVALID_ARGUMENTS_FIXTURE")) {
    writeJson(res, {
      id: "chatcmpl_invalid_arguments_fixture",
      choices: [{
        finish_reason: "tool_calls",
        message: {
          role: "assistant",
          content: "",
          tool_calls: [{
            id: "call_invalid_arguments",
            type: "function",
            function: { name: "read_file", arguments: "[]" }
          }]
        }
      }]
    });
    return;
  }

  if (lastText.includes("MODELPORT_SCHEMA_MISMATCH_FIXTURE")) {
    writeJson(res, {
      id: "chatcmpl_schema_mismatch_fixture",
      choices: [{
        finish_reason: "tool_calls",
        message: {
          role: "assistant",
          content: "",
          tool_calls: [{
            id: "call_schema_mismatch",
            type: "function",
            function: {
              name: "read_file",
              arguments: "{\"path\":7,\"encoding\":\"unknown\",\"extra\":true}"
            }
          }]
        }
      }]
    });
    return;
  }

  writeJson(res, {
    id: "chatcmpl_tool_acceptance",
    choices: [{
      finish_reason: "tool_calls",
      message: {
        role: "assistant",
        content: "",
        tool_calls: [{
          id: "call_acceptance",
          type: "function",
          function: {
            name: "read_file",
            arguments: "{\"path\":\"Cargo.toml\"}"
          }
        }]
      }
    }],
    usage: { prompt_tokens: 12, completion_tokens: 4 }
  });
});

server.listen(0, "127.0.0.1", () => {
  const address = server.address();
  fs.writeFileSync(readyFile, String(address.port), "utf8");
});
NODE

  node "$mock_server_file" "$mock_ready_file" "$mock_log_file" &
  mock_pid="$!"

  for _ in $(seq 1 50); do
    if [[ -s "$mock_ready_file" ]]; then
      mock_port="$(cat "$mock_ready_file")"
      ok "mock OpenAI-compatible upstream started on 127.0.0.1:$mock_port"
      return 0
    fi
    if ! kill -0 "$mock_pid" >/dev/null 2>&1; then
      die "mock upstream exited before becoming ready"
    fi
    sleep 0.1
  done

  die "mock upstream did not become ready"
}

login_admin() {
  if [[ -z "$admin_password" ]]; then
    die "MODELPORT_ADMIN_PASSWORD is required in --mock mode"
  fi

  local status
  status="$(
    curl_local -sS -m 10 \
      -c "$cookie_file" \
      -o "$body_file" \
      -w '%{http_code}' \
      -H 'Content-Type: application/json' \
      "$(base_url)/admin/auth/login" \
      -d "$(node -e 'process.stdout.write(JSON.stringify({ username: process.argv[1], password: process.argv[2] }))' "$admin_username" "$admin_password")"
  )"
  expect_status "$status" "200" "admin login"
  json_get "$body_file" "user.id" >/dev/null
}

create_mock_provider() {
  provider_id="local_tool_acceptance_$$"
  test_model="tool-acceptance-model"
  mock_host="$(mock_provider_host)"
  local payload
  payload="$(
    node -e '
      const providerId = process.argv[1];
      const baseUrl = process.argv[2];
      const model = process.argv[3];
      process.stdout.write(JSON.stringify({
        id: providerId,
        displayName: "Tool Use Acceptance Mock",
        protocol: "openai-compat",
        baseUrl,
        apiKeyEnv: null,
        apiKeyRequired: false,
        defaultModel: model,
        models: [model],
        modelPrefixes: [],
        passthroughUnknownModels: false,
        maxTokensField: "max_completion_tokens",
        deduplicateStreamText: false,
        bufferStreamText: false,
        fidelityMode: "best_effort",
        toolUse: {
          supported: true,
          toolChoice: true,
          parallelToolCalls: true,
          streamingArguments: "best_effort",
          responseValidation: "strict"
        },
        disabled: false
      }));
    ' "$provider_id" "http://$mock_host:$mock_port/v1" "$test_model"
  )"

  local status
  status="$(admin_json POST /admin/providers "$payload")"
  expect_status "$status" "200" "create temporary Tool Use provider"
  ok "temporary provider is $provider_id using http://$mock_host:$mock_port/v1"
}

run_validation_rejections() {
  local status

  status="$(post_message "$(invalid_tool_choice_request "$test_model")")"
  expect_status "$status" "400" "undefined tool_choice rejection"
  if grep -q "must match a defined tool" "$body_file"; then
    ok "undefined tool_choice explains the tool name mismatch"
  else
    printf '[fail] undefined tool_choice error did not mention defined tool mismatch\n' >&2
    sed -n '1,120p' "$body_file" >&2 || true
    exit 1
  fi

  status="$(post_message "$(duplicate_tool_result_request "$test_model")")"
  expect_status "$status" "400" "duplicate tool_result rejection"
  if grep -q "already been answered" "$body_file"; then
    ok "duplicate tool_result explains repeated answer"
  else
    printf '[fail] duplicate tool_result error did not mention repeated answer\n' >&2
    sed -n '1,120p' "$body_file" >&2 || true
    exit 1
  fi
}

run_tool_use_roundtrip() {
  local status

  status="$(post_message "$(message_request "$test_model" false)")"
  expect_status "$status" "200" "non-streaming Tool Use"
  assert_tool_response
  ok "non-streaming OpenAI tool_calls converted to Anthropic tool_use"

  status="$(post_message_stream "$(message_request "$test_model" true)")"
  expect_status "$status" "200" "streaming Tool Use"
  if grep -Eq '^event:[[:space:]]*error' "$body_file"; then
    printf '[fail] streaming Tool Use emitted error event\n' >&2
    sed -n '1,160p' "$body_file" >&2 || true
    exit 1
  fi
  assert_tool_stream
  ok "streaming OpenAI tool_calls converted to Anthropic input_json_delta"

  status="$(post_message "$(tool_result_request "$test_model")")"
  expect_status "$status" "200" "tool_result conversation continuation"
  assert_text_response
  ok "Anthropic tool_result converted to OpenAI role=tool"
}

run_strict_response_rejections() {
  local status

  status="$(post_message "$(strict_response_guard_request "$test_model" "safe_tool" "call safe_tool")")"
  expect_status "$status" "502" "strict undeclared upstream tool rejection"
  if ! grep -q 'undeclared tool `read_file`' "$body_file"; then
    printf '[fail] strict response error did not identify the undeclared tool\n' >&2
    sed -n '1,120p' "$body_file" >&2 || true
    exit 1
  fi
  ok "strict response validation blocks undeclared upstream tool names"

  status="$(post_message "$(strict_response_guard_request "$test_model" "read_file" "MODELPORT_INVALID_ARGUMENTS_FIXTURE")")"
  expect_status "$status" "502" "strict non-object upstream arguments rejection"
  if ! grep -q 'must be a JSON object' "$body_file"; then
    printf '[fail] strict response error did not identify non-object arguments\n' >&2
    sed -n '1,120p' "$body_file" >&2 || true
    exit 1
  fi
  ok "strict response validation blocks non-object upstream arguments"

  status="$(post_message "$(strict_schema_guard_request "$test_model")")"
  expect_status "$status" "502" "strict JSON Schema mismatch rejection"
  if ! grep -q 'declared input schema' "$body_file"; then
    printf '[fail] strict response error did not identify a schema mismatch\n' >&2
    sed -n '1,120p' "$body_file" >&2 || true
    exit 1
  fi
  ok "strict response validation blocks wrong types, enums, required fields, and extras"
}

if [[ "$mode" == "mock" ]]; then
  start_mock_upstream
  login_admin
  create_mock_provider
else
  if is_placeholder_key; then
    die "cannot run upstream Tool Use acceptance because $(upstream_key_name) is missing or placeholder"
  fi
  test_model="$(default_upstream_model)"
  ok "using configured upstream model: $test_model"
fi

run_validation_rejections
run_tool_use_roundtrip

if [[ "$mode" == "mock" ]]; then
  run_strict_response_rejections
  assert_mock_received_parallel_false
fi

printf '\nModelPort Tool Use acceptance passed in %s mode.\n' "$mode"
