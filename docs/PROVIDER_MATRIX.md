# Provider Compatibility Matrix

This document tracks the practical provider support status for ModelPort. The goal is to keep provider claims grounded in repeatable checks, not just configuration entries.

## Production Rule

A provider should only be described as "verified" after both paths pass through the local ModelPort gateway:

```bash
scripts/provider-matrix.sh --model <model-id>
```

For registered models:

```bash
scripts/provider-matrix.sh --all
```

The script checks:

- `POST /v1/messages` non-streaming.
- `POST /v1/messages` with `stream: true`.
- Local ModelPort auth and routing.
- Upstream HTTP/error behavior as seen through ModelPort.

It reads secrets from `.env` but does not print them.

## Current Verified Baseline

Verified on 2026-06-08 in the local WSL/VS Code environment:

| Provider | Protocol | Model | Non-stream | Stream | Notes |
| --- | --- | --- | --- | --- | --- |
| `mimo` | OpenAI-compatible | `mimo-v2.5-pro` | Verified | Verified | Tested with `BASE_URL=https://w.ciykj.cn/v1`; stream text de-duplication is enabled for Mimo. |

## Built-In Providers

These providers are built into the router. A "Pending real-key verification" status means the adapter is configured, but the repository should not claim end-to-end production verification until a real key has been tested with `scripts/provider-matrix.sh`.

| Provider | Protocol | Default Model | Status | Key Variables |
| --- | --- | --- | --- | --- |
| `mimo` | OpenAI-compatible | `mimo-v2.5-pro` | Verified | `BASE_URL`, `MIMO_OPENAI_BASE_URL`, `MIMO_OPENAI_API_KEY`, `MIMO_MODEL` |
| `deepseek` | Anthropic-compatible | `deepseek-v4-pro` | Pending real-key verification | `DEEPSEEK_ANTHROPIC_AUTH_TOKEN`, `DEEPSEEK_MODEL` |
| `anthropic` | Anthropic-compatible | `claude-sonnet-4-20250514` | Pending real-key verification | `ANTHROPIC_API_KEY`, `ANTHROPIC_UPSTREAM_MODEL` |
| `openai` | OpenAI-compatible | `gpt-4o` | Pending real-key verification | `OPENAI_API_KEY`, `OPENAI_MODEL` |
| `openrouter` | OpenAI-compatible | `openrouter/auto` | Pending real-key verification | `OPENROUTER_API_KEY`, `OPENROUTER_MODEL` |
| `gemini` | OpenAI-compatible | `gemini-2.5-flash` | Pending real-key verification | `GEMINI_API_KEY`, `GEMINI_MODEL` |
| `xai` | OpenAI-compatible | `grok-3` | Pending real-key verification | `XAI_API_KEY`, `XAI_MODEL` |
| `groq` | OpenAI-compatible | `llama-3.3-70b-versatile` | Pending real-key verification | `GROQ_API_KEY`, `GROQ_MODEL` |
| `dashscope` | OpenAI-compatible | `qwen-plus` | Pending real-key verification | `DASHSCOPE_API_KEY`, `DASHSCOPE_MODEL` |
| `kimi` | OpenAI-compatible | `kimi-k2.6` | Pending real-key verification | `MOONSHOT_API_KEY`, `KIMI_MODEL` |
| `zhipu` | OpenAI-compatible | `glm-4.7` | Pending real-key verification | `ZHIPU_API_KEY`, `ZHIPU_MODEL` |
| `mistral` | OpenAI-compatible | `mistral-large-latest` | Pending real-key verification | `MISTRAL_API_KEY`, `MISTRAL_MODEL` |
| `ark` | OpenAI-compatible | `doubao-seed-1-6-250615` | Pending real-key verification | `ARK_API_KEY`, `ARK_MODEL` |
| `ollama` | OpenAI-compatible | `llama3.1` | Pending local runtime verification | `MODELPORT_ENABLE_OLLAMA`, `OLLAMA_BASE_URL`, `OLLAMA_MODEL` |
| `custom` | OpenAI-compatible | `default` | Depends on upstream | `CUSTOM_OPENAI_BASE_URL`, `CUSTOM_OPENAI_API_KEY`, `CUSTOM_OPENAI_MODEL` |

## Acceptance Checklist

Before marking a provider as verified:

- Add the provider key and model variables to `.env`.
- Start or restart ModelPort.
- Run `scripts/doctor.sh`.
- Run `scripts/provider-matrix.sh --model <provider:model-or-model-id>`.
- For providers with multiple important models, run `scripts/provider-matrix.sh --models model-a,model-b`.
- Record the date, provider, model, protocol, and any caveats in this document.

## Known Caveats

- Some OpenAI-compatible providers use `max_tokens`, while others require `max_completion_tokens`; ModelPort supports provider-level `max_tokens_field`.
- Some streaming providers replay previous text fragments; Mimo uses `deduplicate_stream_text = true`.
- Streaming upstream failures may arrive as Anthropic SSE `event: error` with HTTP 200, so stream tests must inspect the event body.
- `openrouter`, `custom`, and `ollama` are best suited for arbitrary model passthrough.
- Image and Responses API work should remain separate from the Claude Code text path.
