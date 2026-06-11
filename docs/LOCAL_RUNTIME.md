# Local Runtime Integration

ModelPort does not load model weights. It routes Anthropic-compatible client traffic to an already running OpenAI-compatible inference server.

For SGLang, vLLM, llama.cpp, Ollama, or a custom local server, the important value is the served model name exposed by that runtime. In practice this is often the fine-tuned model name or the runtime alias you configured, not just the base model family name.

## Integration Contract

A local runtime should expose:

- `GET /v1/models`
- `POST /v1/chat/completions`

ModelPort points a provider at the runtime's `/v1` base URL:

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

Then route Claude Code / VS Code Claude through that provider:

```bash
export ANTHROPIC_BASE_URL=http://127.0.0.1:17878
export ANTHROPIC_AUTH_TOKEN=replace-with-the-same-local-router-token
export ANTHROPIC_MODEL=local_vllm:qwen2.5-coder-ft
```

## Built-In Provider Templates

The local providers are built in and can be enabled by environment variables.

| Provider | Default base URL | Enable flag | Model variable |
| --- | --- | --- | --- |
| `local_sglang` | `http://127.0.0.1:30000/v1` | `MODELPORT_ENABLE_LOCAL_SGLANG=1` | `SGLANG_MODEL` |
| `local_vllm` | `http://127.0.0.1:8000/v1` | `MODELPORT_ENABLE_LOCAL_VLLM=1` | `VLLM_MODEL` |
| `local_llamacpp` | `http://127.0.0.1:8080/v1` | `MODELPORT_ENABLE_LOCAL_LLAMACPP=1` | `LLAMACPP_MODEL` |

Example:

```bash
export MODELPORT_ENABLE_LOCAL_VLLM=1
export VLLM_BASE_URL=http://127.0.0.1:8000/v1
export VLLM_MODEL=qwen2.5-coder-ft
export ANTHROPIC_MODEL=local_vllm:qwen2.5-coder-ft
```

If your local runtime enforces an API key, set the matching key variable and keep authentication enabled on that provider:

```bash
export VLLM_API_KEY=replace-with-local-runtime-key
```

For file-based configuration, set `api_key_required = true` and `api_key_env = "VLLM_API_KEY"`.

## Discovering Served Models

Use the dashboard model provider card and click `发现模型`. ModelPort will call the provider's `GET /v1/models`, store the result in the latest provider test record, and display the discovered count.

This is useful when a runtime exposes a name such as:

- `qwen2.5-coder-ft`
- `deepseek-coder-33b-instruct-lora`
- `my-org/my-code-model`
- `local-model`

If discovery returns no model IDs, ModelPort falls back to the configured `models` list plus `default_model`.

## Choosing the Model Name

Use the exact model ID returned by `/v1/models` whenever possible.

If the runtime was started with a served-name or alias, use that served name in:

- provider `default_model`
- provider `models`
- `ANTHROPIC_MODEL`
- aliases such as `local = "local_vllm:qwen2.5-coder-ft"`

For fine-tuned models, this usually means the fine-tuned served model name. The base model name is only correct when the runtime exposes the base model name directly.

## Fidelity Mode

OpenAI-compatible runtimes cannot represent every Anthropic Messages feature exactly.

- `fidelity_mode = "strict"` rejects unsupported Anthropic features instead of translating them approximately.
- `fidelity_mode = "best_effort"` is the practical default for local runtimes.
- `fidelity_mode = "stability"` enables additional stream stabilization behavior when a provider repeats text fragments.

For local SGLang, vLLM, and llama.cpp, start with `best_effort`. Move to `strict` only when you prefer explicit rejection over protocol adaptation.

## Troubleshooting

If the dashboard discovery fails:

- Confirm the runtime is listening on the configured `base_url`.
- Confirm the base URL includes `/v1`.
- Open the runtime's `/v1/models` endpoint and check the returned ID.
- Check whether the runtime requires an API key.
- Keep `passthrough_unknown_models = true` while testing arbitrary local model IDs.

If routing reaches the runtime but generation fails, the served model name is the first thing to verify.
