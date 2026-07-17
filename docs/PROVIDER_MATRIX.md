# Provider Compatibility Matrix

This document separates two different facts:

1. **Built-in configuration** means ModelPort has a provider template and a
   protocol adapter path.
2. **Verified** means a real account/model passed dated non-stream, stream, and
   relevant Tool Use acceptance through a specific commit.

Configuration is not verification. A model listed by `/v1/models` may still be
unavailable to the account or runtime.

The Protocol column describes the Provider edge, not the client edge. Both
`/v1/messages` and the documented `/v1/chat/completions` compatibility slice
can select either Provider protocol when the pre-egress fidelity/capability
checks accept the request.

## Built-In Catalog

Defaults below are derived from the built-in catalog in `src/config.rs` as of
2026-07-15. The shipped `config.example.toml` is a smaller DeepSeek-only example.
Provider catalogs change; an environment model override is inserted into that
provider's runtime model list.

| Provider | Protocol | Code default model | Tool-argument mode | Primary variables |
| --- | --- | --- | --- | --- |
| `deepseek` | Anthropic | `deepseek-v4-flash` | native | `DEEPSEEK_ANTHROPIC_AUTH_TOKEN`, `DEEPSEEK_ANTHROPIC_BASE_URL`, `DEEPSEEK_MODEL` |
| `deepseek_openai` | OpenAI-compatible | `deepseek-v4-flash` | delta | `DEEPSEEK_OPENAI_API_KEY`, `DEEPSEEK_OPENAI_BASE_URL`, `DEEPSEEK_OPENAI_MODEL` |
| `mimo` | OpenAI-compatible | `mimo-v2.5-pro` | delta | `MIMO_OPENAI_API_KEY`, `MIMO_OPENAI_BASE_URL`/`BASE_URL`, `MIMO_MODEL` |
| `anthropic` | Anthropic | `claude-fable-5` | native | `ANTHROPIC_API_KEY`, `ANTHROPIC_UPSTREAM_BASE_URL`, `ANTHROPIC_UPSTREAM_MODEL` |
| `openai` | OpenAI-compatible | `gpt-5.5` | delta | `MODELPORT_OPENAI_API_KEY`, `MODELPORT_OPENAI_BASE_URL`, `MODELPORT_OPENAI_MODEL` (legacy `OPENAI_*` fallbacks) |
| `openrouter` | OpenAI-compatible | `openrouter/auto` | delta | `OPENROUTER_API_KEY`, `OPENROUTER_BASE_URL`, `OPENROUTER_MODEL` |
| `gemini` | OpenAI-compatible | `gemini-3.5-flash` | delta | `GEMINI_API_KEY`, `GEMINI_OPENAI_BASE_URL`, `GEMINI_MODEL` |
| `xai` | OpenAI-compatible | `grok-3` | delta | `XAI_API_KEY`, `XAI_BASE_URL`, `XAI_MODEL` |
| `groq` | OpenAI-compatible | `llama-3.3-70b-versatile` | delta | `GROQ_API_KEY`, `GROQ_BASE_URL`, `GROQ_MODEL` |
| `dashscope` | OpenAI-compatible | `qwen-plus` | delta | `DASHSCOPE_API_KEY`, `DASHSCOPE_BASE_URL`, `DASHSCOPE_MODEL` |
| `kimi` | OpenAI-compatible | `kimi-k2.6` | delta | `MOONSHOT_API_KEY`, `KIMI_BASE_URL`, `KIMI_MODEL` |
| `zhipu` | OpenAI-compatible | `glm-4.7` | delta | `ZHIPU_API_KEY`, `ZHIPU_BASE_URL`, `ZHIPU_MODEL` |
| `mistral` | OpenAI-compatible | `mistral-large-latest` | delta | `MISTRAL_API_KEY`, `MISTRAL_BASE_URL`, `MISTRAL_MODEL` |
| `ark` | OpenAI-compatible | `doubao-seed-1-6-250615` | delta | `ARK_API_KEY`, `ARK_BASE_URL`, `ARK_MODEL` |
| `ollama` | OpenAI-compatible | `llama3.1` | best_effort | `MODELPORT_ENABLE_OLLAMA`, `OLLAMA_BASE_URL`, `OLLAMA_MODEL` |
| `custom` | OpenAI-compatible | `default` | best_effort | `CUSTOM_OPENAI_BASE_URL`, `CUSTOM_OPENAI_API_KEY`, `CUSTOM_OPENAI_MODEL` |
| `local_sglang` | OpenAI-compatible | `local-model` | best_effort | `MODELPORT_ENABLE_LOCAL_SGLANG`, `SGLANG_BASE_URL`, `SGLANG_MODEL` |
| `local_vllm` | OpenAI-compatible | `local-model` | best_effort | `MODELPORT_ENABLE_LOCAL_VLLM`, `VLLM_BASE_URL`, `VLLM_MODEL` |
| `local_llamacpp` | OpenAI-compatible | `local-model` | best_effort | `MODELPORT_ENABLE_LOCAL_LLAMACPP`, `LLAMACPP_BASE_URL`, `LLAMACPP_MODEL` |

Fallback credential names include `DEEPSEEK_API_KEY`, `GOOGLE_API_KEY`,
`QWEN_API_KEY`, `KIMI_API_KEY`, and `VOLCENGINE_API_KEY`. Every template also
supports a comma-separated `*_MODELS` catalog; see
[Configuration](CONFIGURATION.md#provider-environment-pattern).

Anthropic templates default to Tool Use support, tool choice, parallel calls,
and native arguments. General OpenAI-compatible templates default to delta
arguments. Ollama/SGLang/vLLM/llama.cpp default to `parallel_tool_calls=false`;
custom and local templates default to `best_effort`. For OpenAI-compatible
streams, `delta` preserves incremental argument fragments, while `cumulative`
and `best_effort` enable argument replay deduplication and complete-JSON
recovery. A configured mode remains an expectation, not real-provider
verification.

## Committed Verification Ledger

No dated, reproducible real-provider result is currently committed. Therefore
none of the built-in entries above should be advertised as production verified
by this repository. DeepSeek with `deepseek-v4-flash` is the configured sample
path only.

Add results in this format without secrets:

| Date | Commit | Provider/model | Endpoint ownership | Non-stream | Stream body | Tool Use | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| _YYYY-MM-DD_ | `_sha_` | `_provider:model_` | official/local/third-party | pass/fail | pass/fail | pass/fail/n-a | account tier, caveat, script version |

“Stream pass” requires a text/tool delta and no `event: error`; HTTP 200 alone
does not pass. “Tool Use pass” requires the real upstream path, not only the
local mock adapter test.

## Verification Procedure

Start with the gateway running and the exact account/model configured:

```bash
scripts/config-validate.sh
scripts/doctor.sh
scripts/provider-matrix.sh --model provider:model
```

For multiple models:

```bash
scripts/provider-matrix.sh --models provider:model-a,provider:model-b
```

For real Tool Use certification:

```bash
scripts/tool-use-acceptance.sh --upstream
```

These commands make paid provider calls. Record failures as evidence too. The
default `scripts/tool-use-acceptance.sh` proves the local adapter with a mock;
it does not certify a provider.

## Interpretation Limits

- Provider model names, account permissions, pricing, and APIs change outside
  this repository. Confirm them with the provider before deployment.
- OpenAI-compatible does not imply identical Tool Use, SSE, usage, or error
  semantics.
- Live-stream verification must observe a non-204 2xx
  `text/event-stream` handshake and the protocol's termination marker, not only
  an initial HTTP 200 or a text delta.
- `max_tokens_field`, text replay, and tool-argument behavior are provider-level
  compatibility settings.
- A keyless local/custom provider can be present in file configuration and
  appear in `/v1/models` while its runtime is offline.
- Provider base URLs reject userinfo, query strings, and fragments. Credentials
  belong in their environment-backed adapter headers, not the URL.
- Non-local/non-custom Provider bases require HTTPS by default. The explicit
  insecure-HTTP override is only for a trusted internal network because it
  exposes Provider credentials and model traffic in plaintext.
- Stream errors can occur after headers. The in-process body finalizer records
  terminal request and Provider outcome evidence, but cannot replay fallback
  after downstream headers or reconcile an attempt lost with the process.
- Pricing values are estimates and require separate regression tests in
  `src/pricing.rs`; provider billing remains authoritative.
