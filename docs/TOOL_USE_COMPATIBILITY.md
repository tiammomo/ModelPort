# Tool Use Compatibility

ModelPort accepts Anthropic Tool Use and the scoped OpenAI Chat Completions
function-tool contract at its client edges. A typed Exchange IR maps either
edge to Anthropic or OpenAI-compatible Providers. This document describes the
implemented adapters, validation gates, capability metadata, and known limits;
it does not certify every configured Provider.

Protocol references: [Anthropic tool-call handling](https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls)
and [OpenAI function calling](https://developers.openai.com/api/docs/guides/function-calling).

## Client Contract

Requests may contain:

- top-level `tools` and `tool_choice`;
- assistant `tool_use` content blocks;
- user `tool_result` content blocks;
- streaming tool-call arguments.

The OpenAI edge accepts function definitions, assistant `tool_calls`, tool-role
results, named/auto/required/none choices, and `parallel_tool_calls`. An
Anthropic-compatible upstream receives these as Anthropic Tool Use blocks; an
OpenAI-compatible upstream receives the native function-tool form.

### DeepSeek Anthropic bridge

The configured `deepseek` provider uses DeepSeek's official Anthropic base URL
`https://api.deepseek.com/anthropic`. Native `/v1/messages` requests preserve
Anthropic thinking blocks. Requests entering through `/v1/chat/completions`
cannot round-trip those blocks in later OpenAI assistant/tool messages, while
DeepSeek enables thinking by default and requires previous thinking blocks to
be replayed. ModelPort therefore sends `thinking.type=disabled` only on this
OpenAI-to-DeepSeek-Anthropic bridge. This also makes named/required tool choice
and the following tool-result turn compatible. Other Anthropic providers and
native Anthropic requests are unchanged.

## OpenAI-Compatible Mapping

| Anthropic | OpenAI-compatible |
| --- | --- |
| `tools[].name/description/input_schema/strict` | `tools[].type=function` and `function.*` |
| `tool_choice.type=auto` | `"auto"` |
| `tool_choice.type=none` | `"none"` |
| `tool_choice.type=any` | `"required"` |
| `tool_choice.type=tool,name=X` | named function choice |
| `disable_parallel_tool_use` | inverse `parallel_tool_calls` |
| assistant `tool_use` | assistant `tool_calls` |
| user `tool_result` | `role=tool`, matching `tool_call_id` |

In best-effort conversion, `tool_result.is_error=true` is represented inside
the OpenAI tool-role content with a stable `ModelPort tool execution error`
marker so a local model does not lose the failure signal. Strict fidelity still
rejects this conversion because Chat Completions has no exact `is_error` field.

OpenAI `tool_calls`, plus legacy `function_call`, map back to Anthropic
`tool_use`. Finish reasons `tool_calls` and `function_call` map to
`stop_reason=tool_use`. Function arguments must become an Anthropic input object;
non-object or invalid JSON is preserved below `_raw_arguments` instead of being
silently discarded.

## Validation Before Routing

The gateway rejects malformed Tool Use before contacting a provider:

- `tools` must be an array of objects with unique 1–64 character names using
  ASCII letters, digits, `_`, or `-`;
- descriptions must be strings; `input_schema` is required and must declare an
  object schema; the schema must compile locally and may only use local JSON
  Pointer `$ref`/`$dynamicRef` references; optional `strict` must be boolean;
- `tool_choice` must be `auto`, `any`, `none`, or `tool`; a named choice must
  reference a declared tool when definitions are present;
- `any` and named `tool` choices require a non-empty tool list;
- assistant `tool_use` requires a unique non-empty ID, valid name, and object
  input;
- user `tool_result` must reference an earlier unanswered assistant tool ID;
- tool results must immediately follow the assistant tool call, return all
  pending results in one user message, and precede text in that message;
- tool count and serialized size obey the configured guardrails.

Provider capability checks then reject Tool Use entirely, reject tool choice,
or reject an explicitly enabled parallel-call request when that provider says it
cannot support it. For an OpenAI-compatible provider declaring
`parallel_tool_calls=false`, ModelPort also injects the upstream parameter when
the client omitted a preference. Strict response validation verifies that the
upstream did not return multiple calls anyway.

## Streaming

For `/v1/messages`, ModelPort emits Anthropic-style events:

- `content_block_start` for text or tool blocks;
- `content_block_delta` with `text_delta` or `input_json_delta`;
- `content_block_stop`;
- `message_delta` with mapped stop reason;
- `message_stop`.

For `/v1/chat/completions`, native or converted tool calls use OpenAI
`chat.completion.chunk` deltas with indexed `tool_calls`, function name and
argument fragments, a `tool_calls` finish reason, and `[DONE]`.

OpenAI tool calls are tracked by upstream call index. Arguments that arrive
before the function name are buffered. In best-effort response mode, a missing
name produces a synthetic `tool` name rather than dropping argument data;
strict response mode emits a protocol error instead. With
`streaming_arguments="delta"`, argument fragments remain incremental. The
`cumulative` and `best_effort` modes enable replay deduplication and retain the
best complete JSON object found at stream completion. Anthropic providers use
native pass-through semantics.

The stream can still end with an Anthropic error event or OpenAI error data
after HTTP 200. A half-written JSON argument is not guaranteed recoverable.
Strict response validation requires the delivered aggregate to be a JSON object
and verifies the returned name, call count, and arguments against the selected
tool's complete `input_schema` (including nested types, required properties,
enums, numeric/string constraints, and `additionalProperties`). Response
validation reports the failing JSON Pointer while masking instance values, so
tool arguments are not copied into errors or logs.
In live delta mode, `content_block_start` and argument deltas can precede this
aggregate validation. A rejected call ends with an error event and no valid
`content_block_stop`; clients must never execute a tool before that successful
terminal block signal. Buffered stream mode validates before emitting blocks.

The live handshake requires a non-204 2xx response with
`text/event-stream`. Native Anthropic completion requires `message_stop`;
OpenAI-compatible completion requires `[DONE]` or `finish_reason`, after which
ModelPort emits the terminal form expected by the originating client protocol.
EOF without that signal is an error, not a successful partial Tool Use result.

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
response_validation = "best_effort" # best_effort | strict
repair_invalid_arguments = false
```

The first three fields participate in capability validation.
`streaming_arguments` also controls OpenAI-compatible Tool Use stream handling:
`delta` forwards incremental fragments, while `cumulative` and `best_effort`
enable argument replay deduplication and bounded complete-JSON recovery. The
setting does not normalize every provider-specific schema or certify that the
upstream really follows the declared mode.

`response_validation="strict"` adds a response-side contract for
OpenAI-compatible providers. It is opt-in so providers with non-standard output
can retain compatibility mode, while a certified local runtime can fail closed
before a malformed or hallucinated call reaches the tool executor.

`repair_invalid_arguments=true` is a separate, opt-in reliability policy for a
strict OpenAI-compatible provider. It applies only to non-stream Anthropic
Messages responses rejected by the declared JSON Schema. ModelPort schedules
one same-provider retry as a normal ledger attempt and sends a fixed correction
instruction that contains no tool arguments, validation paths, or Provider
body. A second failure is not repaired again. Live streams are excluded because
tool argument fragments may already have crossed the client boundary; tool
execution errors remain the application's responsibility.

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
requests, parallel-choice mapping, and strict rejection of undeclared tool
names and non-object arguments before cleanup.

Real provider certification, which may cost money:

```bash
scripts/tool-use-acceptance.sh --upstream
```

Reasoning models may spend the original 128-token test budget before emitting
the final text after a tool result. Increase only the acceptance request budget
without changing gateway limits:

```bash
scripts/tool-use-acceptance.sh --upstream --max-tokens 2048
```

`MODELPORT_TOOL_USE_MAX_TOKENS` provides the equivalent environment override.

The streaming acceptance check concatenates all `input_json_delta.partial_json`
fragments before parsing them. Providers may split a valid JSON string at any
token boundary; a single SSE event is not required to contain a complete tool
argument value.

Record a dated result in [Provider Matrix](PROVIDER_MATRIX.md). A mock pass means
the gateway adapter works for the fixture; it says nothing about a provider's
schema limits, tool-choice support, argument streaming, or account entitlement.

## Deferred Work

- A provider-neutral Tool IR and provider-specific schema transformation.
- Schema normalization beyond strict names, object arguments, call IDs, tool
- Cross-provider schema normalization for provider-specific dialects.
- Provider-specific argument repair beyond the current bounded best effort.
- A committed real-provider verification ledger.
