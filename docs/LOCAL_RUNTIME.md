# Local Runtime Integration

ModelPort does not load model weights. It routes Anthropic-compatible client
traffic to a separately managed OpenAI-compatible inference server such as
SGLang, vLLM, llama.cpp, Ollama, or a custom runtime.

## Contract

The runtime should expose:

- `GET /v1/models`
- `POST /v1/chat/completions`

Use the exact served model ID returned by the runtime, including any fine-tuned
alias. Example TOML:

```toml
[providers.local_vllm]
display_name = "Local vLLM"
protocol = "openai-compat"
base_url = "http://127.0.0.1:8000/v1"
api_key_required = false
default_model = "qwen2.5-coder-ft"
models = ["qwen2.5-coder-ft"]
passthrough_unknown_models = true
max_tokens_field = "max_tokens"
fidelity_mode = "best_effort"

[aliases]
local = "local_vllm:qwen2.5-coder-ft"
```

Then select:

```env
ANTHROPIC_BASE_URL=http://127.0.0.1:17878
ANTHROPIC_AUTH_TOKEN=<MODELPORT_AUTH_TOKEN>
ANTHROPIC_MODEL=local_vllm:qwen2.5-coder-ft
```

## Built-In Templates

| Provider | Default base URL | Enable flag | Model/key variables |
| --- | --- | --- | --- |
| `ollama` | `http://127.0.0.1:11434/v1` | `MODELPORT_ENABLE_OLLAMA=1` | `OLLAMA_MODEL`, optional `OLLAMA_API_KEY` |
| `local_sglang` | `http://127.0.0.1:30000/v1` | `MODELPORT_ENABLE_LOCAL_SGLANG=1` | `SGLANG_MODEL`, optional `SGLANG_API_KEY` |
| `local_vllm` | `http://127.0.0.1:8000/v1` | `MODELPORT_ENABLE_LOCAL_VLLM=1` | `VLLM_MODEL`, optional `VLLM_API_KEY` |
| `local_llamacpp` | `http://127.0.0.1:8080/v1` | `MODELPORT_ENABLE_LOCAL_LLAMACPP=1` | `LLAMACPP_MODEL`, optional `LLAMACPP_API_KEY` |
| `custom` | `http://127.0.0.1:8000/v1` | `MODELPORT_ENABLE_CUSTOM=1` or any custom value | `CUSTOM_OPENAI_MODEL`, optional `CUSTOM_OPENAI_API_KEY` |

Example:

```env
MODELPORT_ENABLE_LOCAL_VLLM=1
VLLM_BASE_URL=http://127.0.0.1:8000/v1
VLLM_MODEL=qwen2.5-coder-ft
```

If authentication is required, set the matching key and, for TOML, use
`api_key_required=true` plus `api_key_env`.

## Docker Host Runtime

The backend container's loopback is not the host. Compose supplies a host
gateway:

```env
VLLM_BASE_URL=http://host.docker.internal:8000/v1
OLLAMA_BASE_URL=http://host.docker.internal:11434/v1
```

Only route to a trusted runtime. Current URL checks allow local/custom loopback
and do not inspect a hostname's resolved IP, so firewall and Docker network
policy remain part of the trust boundary. Base URLs with userinfo, a query
string, or a fragment are rejected; put any runtime credential in its documented
API-key environment variable, not the URL.

HTTP is intentionally available to local/custom Provider classes for loopback
and controlled local-runtime traffic. Non-local/custom Providers require HTTPS
unless `MODELPORT_ALLOW_INSECURE_PROVIDER_HTTP=1` is explicitly set. Do not use
that override casually: plain HTTP exposes the Provider API key and complete
prompt/response traffic to the network path.

## Model Discovery

The dashboard sends the CSRF-protected control request
`POST /admin/providers/{provider_id}/models`. The backend then calls the
upstream `<base_url>/models`, corresponding to `GET /v1/models` when the base
ends in `/v1`. Because discovery stores the latest provider-test result and an
audit event, the admin endpoint has no GET alias. The dashboard displays the
discovered IDs; if discovery is empty, configured `models` and `default_model`
remain the routing catalog.

Discovery proves only that the endpoint returned a parseable catalog. Run a
non-stream and stream request before calling a runtime compatible.

## Fidelity And Tool Use

- `fidelity_mode="strict"` rejects Anthropic features that the OpenAI request
  conversion cannot preserve.
- `fidelity_mode="best_effort"` performs the normal adapter mapping and is the
  practical starting point for local runtimes.
- `fidelity_mode="stability"` is a label used when explicit stream rewrite flags
  are configured. It does not enable deduplication or buffering by itself.

For repeated cumulative text, set `deduplicate_stream_text=true` only after a
real reproduction. `buffer_stream_text=true` changes streaming into complete
generation and protocol conversion followed by locally chunked SSE. Upstream
errors can then fail or fallback before local HTTP 200, and reported upstream
usage enters normal accounting. The cost is full-generation time to first byte;
downstream cancellation happens after the upstream generation is already done,
and successful local delivery is not tracked.

Local templates set `parallel_tool_calls=false` and
`streaming_arguments="best_effort"`. The latter enables argument replay
deduplication and recovery of the best complete JSON object available, but it is
not a complete normalizer. The runtime can still have different schema,
tool-choice, and argument-delta behavior. Certify with:

```bash
scripts/provider-matrix.sh --model local_vllm:qwen2.5-coder-ft
scripts/tool-use-acceptance.sh --upstream
```

These are real inference calls.

## Troubleshooting

- Confirm the runtime listens on the configured network namespace/address.
- Confirm the base URL ends at the API base, normally `/v1`, not
  `/chat/completions`.
- Query `/v1/models` and use its exact served ID.
- Check key requirements and the provider's `max_tokens_field`.
- Keep arbitrary passthrough only while the runtime/model catalog is controlled.
- For live streaming, require a 2xx status other than 204,
  `Content-Type: text/event-stream`, and an OpenAI `[DONE]` or
  `finish_reason`; a missing termination signal becomes an SSE error.
- Inspect the complete SSE body for `event: error`; HTTP 200 at stream start is
  not a completed generation.
- Remember that final live-stream usage, provider health, and fallback after
  headers are current ModelPort lifecycle limits.
