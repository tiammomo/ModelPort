# Providers

ModelPort routes to hosted APIs and separately managed local inference
runtimes. It does not load model weights.

Two facts must remain separate:

1. **Configured support** means a Provider template and protocol adapter exist.
2. **Verified support** means an exact account/model/path passed dated
   non-stream, stream, and relevant Tool Use acceptance on a recorded commit.

A model appearing in `/v1/models` proves neither account entitlement nor
runtime compatibility.

ModelPort also embeds a versioned adaptation catalog under
`resources/catalog/provider-adaptations-v1.json`. It records exact-model context/output
metadata, input modalities, tri-state Tool Use/reasoning capabilities,
reasoning dialect/efforts, and replay constraints for the Providers ModelPort
already exposes. This catalog is owned and reviewed in this repository; it is
not synchronized from another project at runtime. Entries start `unverified`
until a separate real-provider acceptance process records evidence. Discovery
is inventory evidence only and cannot promote a capability or route.

## Built-In Provider Catalog

Defaults come from `src/config.rs`; the smaller `config.example.toml` is the
maintained first-run example. Provider catalogs change, so use the exact model
ID returned by the account or runtime.

| Provider | Protocol | Default model | Primary configuration |
| --- | --- | --- | --- |
| `cpa_codex` | OpenAI-compatible | `gpt-5.3-codex` | `CPA_CODEX_*` |
| `cpa_claude` | Anthropic | `claude-sonnet-4-6` | `CPA_CLAUDE_*` |
| `deepseek` | Anthropic | `deepseek-v4-flash` | `DEEPSEEK_ANTHROPIC_*` |
| `deepseek_openai` | OpenAI-compatible | `deepseek-v4-flash` | `DEEPSEEK_OPENAI_*` |
| `anthropic` | Anthropic | `claude-fable-5` | `ANTHROPIC_API_KEY`, `ANTHROPIC_UPSTREAM_*` |
| `openai` | OpenAI-compatible | `gpt-5.5` | `MODELPORT_OPENAI_*` |
| `openrouter` | OpenAI-compatible | `openrouter/auto` | `OPENROUTER_*` |
| `gemini` | OpenAI-compatible | `gemini-3.5-flash` | `GEMINI_*` |
| `xai` | OpenAI-compatible | `grok-3` | `XAI_*` |
| `groq` | OpenAI-compatible | `llama-3.3-70b-versatile` | `GROQ_*` |
| `dashscope` | OpenAI-compatible | `qwen-plus` | `DASHSCOPE_*` |
| `kimi` | OpenAI-compatible | `kimi-k2.6` | `MOONSHOT_API_KEY`, `KIMI_*` |
| `zhipu` | OpenAI-compatible | `glm-4.7` | `ZHIPU_*` |
| `mistral` | OpenAI-compatible | `mistral-large-latest` | `MISTRAL_*` |
| `ark` | OpenAI-compatible | `doubao-seed-1-6-250615` | `ARK_*` |
| `mimo` | OpenAI-compatible | `mimo-v2.5-pro` | `MIMO_OPENAI_*` |
| `ollama` | OpenAI-compatible | `llama3.1` | `MODELPORT_ENABLE_OLLAMA`, `OLLAMA_*` |
| `local_sglang` | OpenAI-compatible | `local-model` | `MODELPORT_ENABLE_LOCAL_SGLANG`, `SGLANG_*` |
| `local_vllm` | OpenAI-compatible | `local-model` | `MODELPORT_ENABLE_LOCAL_VLLM`, `VLLM_*` |
| `local_llamacpp` | OpenAI-compatible | `local-model` | `MODELPORT_ENABLE_LOCAL_LLAMACPP`, `LLAMACPP_*` |
| `custom` | OpenAI-compatible | `default` | `MODELPORT_ENABLE_CUSTOM`, `CUSTOM_OPENAI_*` |

The complete field and environment contract is in
[Configuration](CONFIGURATION.md#provider-environment-pattern).

## Hosted Provider Setup

1. Choose the Provider protocol and exact base URL.
2. Put its credential in the documented server-side environment variable.
3. Configure only models available to that account.
4. Run configuration validation and model discovery.
5. Call the exact `provider:model` route before enabling aliases or smart
   routing.
6. Verify streaming and Tool Use separately when the workload needs them.

The current public client boundary remains Anthropic Messages and OpenAI Chat
Completions. The adaptation catalog does not imply support for OpenAI Responses,
Bedrock/Vertex native authentication, OAuth, WebSocket, or image exchange.

Remote Providers require HTTPS by default. URLs with userinfo, query strings,
or fragments are rejected. The insecure HTTP override exposes Provider keys and
model traffic and is only for a controlled internal network.

## CPA: Codex And Claude Account Adapter

CPA means [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) in this
project. It is an optional internal credential/account adapter, not ModelPort's
public gateway and not a second source of policy truth:

```text
Codex / Claude Code / SDK
            |
         ModelPort
     auth, policy, route,
   quota, evidence, billing
            |
   +--------+---------+
   |                  |
cpa_codex         cpa_claude
OpenAI protocol   Anthropic protocol
   |                  |
   +--------+---------+
            |
   CPA OAuth/account pool
```

ModelPort deliberately exposes two Provider IDs:

- `cpa_codex` sends OpenAI-compatible requests to CPA's `/v1` client API;
- `cpa_claude` sends Anthropic requests to CPA's `/v1/messages` client API;
- both can discover the shared CPA catalog through `GET /v1/models`;
- both may use the same CPA `api-keys` value, but their ModelPort health,
  routing, evidence, and model allowlists remain separate.

Minimal host configuration:

```env
MODELPORT_ENABLE_CPA_CODEX=1
CPA_CODEX_BASE_URL=http://127.0.0.1:8317/v1
CPA_CODEX_API_KEY=replace-with-cpa-client-api-key
CPA_CODEX_MODEL=gpt-5.3-codex
CPA_CODEX_MODELS=gpt-5.3-codex

MODELPORT_ENABLE_CPA_CLAUDE=1
CPA_CLAUDE_BASE_URL=http://127.0.0.1:8317
CPA_CLAUDE_API_KEY=replace-with-cpa-client-api-key
CPA_CLAUDE_MODEL=claude-sonnet-4-6
CPA_CLAUDE_MODELS=claude-sonnet-4-6
```

Those values create the Providers only in environment-default mode. A
deployment that loads `config.toml`, including the maintained Compose path,
must also add the explicit CPA Provider records in the
[configuration recipe](CONFIGURATION.md#cpa-as-an-internal-provider).

Use `cpa_codex:gpt-5.3-codex` and
`cpa_claude:claude-sonnet-4-6` during acceptance. Provider-qualified names
avoid collisions with official OpenAI/Anthropic Providers. CPA Providers
require a non-empty explicit model allowlist, reject prefix ownership and
unknown-model passthrough, and reject a management API URL.

Keep the trust boundary narrow:

- clients connect only to ModelPort; do not publish CPA port `8317`;
- ModelPort stores only CPA's client API key; CPA owns OAuth tokens and auth
  files;
- leave CPA remote management disabled unless it has a separate administrative
  trust path;
- bind CPA to loopback for systemd or a private Docker network for containers;
- ModelPort permits CPA HTTP only for loopback, a single-label internal service
  name, or `host.docker.internal`; a public CPA hostname still requires HTTPS.

ModelPort owns Provider-level retry, fallback, cooldown, quota, and accounting.
CPA should initially use `request-retry: 0` and a bounded
`max-retry-credentials: 1`. The upstream CPA template's `0` value means
unbounded legacy credential traversal, so it is not a safe production default
behind another retrying gateway. Increase credential rotation only after
attempt counts, latency, and upstream billing evidence show that the combined
behavior is bounded.

CPA candidates enter smart routing only after direct non-stream, stream, Tool
Use, and error-path acceptance. CPA model discovery reports availability; it
does not authorize a model or prove subscription eligibility.

LiteLLM is not a runtime dependency in this topology. ModelPort may learn from
its Provider abstraction, normalized errors, routing, and cost-governance
ideas, but adding a second gateway hop would split policy, retry, and
observability ownership.

## Local Runtime Contract

A local runtime such as SGLang, vLLM, llama.cpp, Ollama, or a custom server
should expose:

- `GET /v1/models`;
- `POST /v1/chat/completions`;
- OpenAI-compatible request, response, usage, and SSE shapes for the features
  you intend to use.

Minimal example:

```toml
[providers.local_qwen]
display_name = "Local Qwen"
protocol = "openai-compat"
base_url = "http://127.0.0.1:8000/v1"
api_key_required = false
default_model = "qwen-model-id"
models = ["qwen-model-id"]
passthrough_unknown_models = false
max_tokens_field = "max_tokens"
fidelity_mode = "best_effort"

[aliases]
local = "local_qwen:qwen-model-id"
```

Inside Docker, container loopback is not the host:

```env
VLLM_BASE_URL=http://host.docker.internal:8000/v1
OLLAMA_BASE_URL=http://host.docker.internal:11434/v1
```

The runtime owns model loading, tokenizer, context capacity, and generation
limits. ModelPort owns routing, policy, aliases, pricing estimates, and stored
usage. Local pricing is internal chargeback, not a Provider invoice.

Advanced reasoning, sampling, exact token-count forwarding, stream
deduplication, buffering, and Tool Use fields are documented once in
[Configuration](CONFIGURATION.md#toml-provider-fields).

## Discovery And Verification

Dashboard model discovery calls the Provider's `/models` endpoint. Discovery
only proves that a parseable catalog was returned.

Run local checks first:

```bash
scripts/config-validate.sh
scripts/doctor.sh
```

The following commands make real or paid inference calls:

```bash
scripts/provider-matrix.sh --model provider:model
scripts/tool-use-acceptance.sh --upstream
```

For reviewable evidence:

```bash
scripts/provider-matrix.sh \
  --models provider:model-a,provider:model-b \
  --evidence artifacts/provider-matrix.json
```

The artifact records commit, source state, Provider/model, traffic class, and
outcomes without credentials or request/response bodies. Tool Use evidence is
separate and must not be inferred from a text-only matrix pass.

No real-Provider result is currently committed, so built-in entries must not be
advertised as production verified by this repository.

## Compatibility Boundaries

- OpenAI-compatible does not imply identical Tool Use, SSE, usage, or error
  semantics.
- A stream passes only after a semantic delta and valid terminal marker with no
  `event: error`; initial HTTP 200 is insufficient.
- Live streams cannot fall back after downstream headers.
- Provider usage replaces local token estimates only when the adapter recognizes
  the reported fields.
- Account permissions, pricing, and model IDs can change outside this
  repository.
- Keyless local Providers can be configured while their runtime is offline.

Use [Tool Use](TOOL_USE_COMPATIBILITY.md) for its stricter contract and
[Smart Routing](SMART_ROUTING.md) only after every candidate is independently
verified.
