# Tool Use Compatibility

ModelPort keeps an Anthropic-compatible Tool Use contract at the client boundary
and maps it to OpenAI function tools when required. This document describes the
implemented adapter, validation gates, capability metadata, and known limits;
it does not certify every configured provider.

## Client Contract

Requests may contain:

- top-level `tools` and `tool_choice`;
- assistant `tool_use` content blocks;
- user `tool_result` content blocks;
- streaming tool-call arguments.

An Anthropic-compatible upstream receives these fields in the Anthropic request
shape. An OpenAI-compatible upstream uses the mapping below.

## OpenAI-Compatible Mapping

| Anthropic | OpenAI-compatible |
| --- | --- |
| `tools[].name/description/input_schema` | `tools[].type=function` and `function.*` |
| `tool_choice.type=auto` | `"auto"` |
| `tool_choice.type=none` | `"none"` |
| `tool_choice.type=any` | `"required"` |
| `tool_choice.type=tool,name=X` | named function choice |
| `disable_parallel_tool_use` | inverse `parallel_tool_calls` |
| assistant `tool_use` | assistant `tool_calls` |
| user `tool_result` | `role=tool`, matching `tool_call_id` |

OpenAI `tool_calls`, plus legacy `function_call`, map back to Anthropic
`tool_use`. Finish reasons `tool_calls` and `function_call` map to
`stop_reason=tool_use`. Function arguments must become an Anthropic input object;
non-object or invalid JSON is preserved below `_raw_arguments` instead of being
silently discarded.

## Validation Before Routing

The gateway rejects malformed Tool Use before contacting a provider:

- `tools` must be an array of objects with unique 1–64 character names using
  ASCII letters, digits, `_`, or `-`;
- descriptions must be strings and `input_schema`, when present, must be an
  object schema;
- `tool_choice` must be `auto`, `any`, `none`, or `tool`; a named choice must
  reference a declared tool when definitions are present;
- `any` and named `tool` choices require a non-empty tool list;
- assistant `tool_use` requires a unique non-empty ID, valid name, and object
  input;
- user `tool_result` must reference an earlier unanswered assistant tool ID;
- tool count and serialized size obey the configured guardrails.

Provider capability checks then reject Tool Use entirely, reject tool choice,
or reject an explicitly enabled parallel-call request when that provider says it
cannot support it.

`parallel_tool_calls=false` in provider metadata is not a complete server-side
single-tool transformer. When the client omits an explicit parallel preference,
the upstream's own default can still matter. Verify local runtimes in practice.

## Streaming

ModelPort emits Anthropic-style events:

- `content_block_start` for text or tool blocks;
- `content_block_delta` with `text_delta` or `input_json_delta`;
- `content_block_stop`;
- `message_delta` with mapped stop reason;
- `message_stop`.

OpenAI tool calls are tracked by upstream call index. Arguments that arrive
before the function name are buffered. A missing name produces a synthetic
`tool` name rather than dropping argument data. With
`streaming_arguments="delta"`, argument fragments remain incremental. The
`cumulative` and `best_effort` modes enable replay deduplication and retain the
best complete JSON object found at stream completion. Anthropic providers use
native pass-through semantics.

The stream can still end with `event: error` after HTTP 200. A half-written JSON
argument is not guaranteed recoverable, and a synthetically named tool is a
compatibility fallback rather than a semantically verified call.

The live handshake requires a non-204 2xx response with
`text/event-stream`. Native Anthropic completion requires `message_stop`;
OpenAI-compatible completion requires `[DONE]` or `finish_reason`, after which
ModelPort emits `message_stop`. EOF without that signal is an error, not a
successful partial Tool Use result.

With `buffer_stream_text=true`, the OpenAI-compatible adapter awaits and maps a
complete non-stream response before it returns local SSE. Tool blocks are then
emitted as `content_block_start`, one complete serialized `input_json_delta`,
and `content_block_stop`. Upstream HTTP/JSON/conversion errors occur before the
local SSE response and can use normal fallback; reported upstream usage is also
available to accounting. This is not live tool-argument streaming and delays
the first downstream event until the generation has completed.

## Capability Configuration

```toml
[providers.example.tool_use]
supported = true
tool_choice = true
parallel_tool_calls = true
streaming_arguments = "delta" # native | delta | cumulative | best_effort
```

The first three fields participate in capability validation.
`streaming_arguments` also controls OpenAI-compatible Tool Use stream handling:
`delta` forwards incremental fragments, while `cumulative` and `best_effort`
enable argument replay deduplication and bounded complete-JSON recovery. The
setting does not normalize every provider-specific schema or certify that the
upstream really follows the declared mode.

Similarly, `fidelity_mode="stability"` labels a stream-rewriting configuration;
it does not enable a rewrite flag. `fidelity_mode="strict"` actively rejects
unsupported Anthropic-to-OpenAI features and cannot be combined with configured
stream text rewriting. Tool-argument handling is selected independently by
`tool_use.streaming_arguments`; repeated text output still requires
`deduplicate_stream_text` or `buffer_stream_text` explicitly.

## Acceptance

Mock-backed adapter acceptance:

```bash
scripts/tool-use-acceptance.sh
```

It creates a temporary local OpenAI-compatible provider and covers non-stream
mapping, streaming `input_json_delta`, `tool_result` continuation, malformed
requests, and parallel-choice mapping before cleanup.

Real provider certification, which may cost money:

```bash
scripts/tool-use-acceptance.sh --upstream
```

Record a dated result in [Provider Matrix](PROVIDER_MATRIX.md). A mock pass means
the gateway adapter works for the fixture; it says nothing about a provider's
schema limits, tool-choice support, argument streaming, or account entitlement.

## Deferred Work

- A provider-neutral Tool IR and provider-specific schema transformation.
- Strong enforcement/normalization of single-tool runtimes.
- Final live-stream lifecycle accounting and fallback after response headers.
- Provider-specific argument repair beyond the current bounded best effort.
- A committed real-provider verification ledger.
