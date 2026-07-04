# ModelPort 面试问题与回答清单

这份文档用于面试前快速复习。回答不追求背诵，重点是把项目讲成一个有边界、有难点、有取舍、有验证的工程系统。

## 1. 项目定位

### Q1: ModelPort 是什么？

A: ModelPort 是一个本地大模型路由网关，面向个人开发者、小团队和初创团队。它统一接收客户端模型请求，在本地完成鉴权、权限策略、模型路由、Provider 协议适配、Tool Use、限流预算、日志指标和 Provider 健康管理。

### Q2: 一句话怎么介绍这个项目？

A: ModelPort 把本地大模型调用从简单 API 转发升级成一个可治理、可诊断、可验证的轻量模型网关。

### Q3: 它和普通 API Proxy 有什么区别？

A: 普通 Proxy 主要做请求转发；ModelPort 还处理协议语义转换、Tool Use 校验、Provider 能力门禁、API Key 和 Team Policy、IP allowlist、quota、Provider health、fallback、请求日志和 Prometheus metrics。

### Q4: 为什么项目定位是个人和小团队，而不是企业平台？

A: 因为早期最重要的是低运维、高可靠、安全收益明显。个人和小团队更需要 Docker Compose、本地配置、PostgreSQL、轻量控制台和明确的自检脚本，而不是一开始引入 Kubernetes、Redis、OIDC、复杂多租户或服务网格。

### Q5: ModelPort 的核心价值是什么？

A: 核心价值是统一模型调用入口，让多客户端、多协议、多 Provider、多凭证和多种失败模式都能在一个本地网关中治理。

### Q6: 如何证明它不是只适配某个客户端或某个模型？

A: 架构上 Provider 是协议和配置驱动的，支持 Anthropic-compatible、OpenAI-compatible、本地运行时和自定义 Provider。DeepSeek 可以作为标准样例，但不是能力边界。

## 2. 总体架构

### Q7: ModelPort 的主链路是什么？

A: 主链路是 `Client Apps -> ModelPort Gateway -> Provider Pool`。客户端请求进入网关后，经过鉴权、策略、模型解析、协议适配和 Provider 调用，再把响应统一映射回客户端期望的协议形态。

### Q8: 后端为什么选择 Rust + Axum？

A: 网关是长时间运行的 I/O 服务，需要稳定并发、低运行成本和明确错误处理。Rust 的类型系统适合表达协议结构、配置校验和错误分类；Axum 足够轻量，适合构建本地网关服务。

### Q9: 控制面和数据面怎么区分？

A: 数据面负责 `/v1/messages` 等模型调用链路，面向 API Key 或 legacy token。控制面负责 Dashboard、用户、Provider、Key、日志和配置管理，使用 Admin Session 和 CSRF 保护。调用模型的 token 不能直接管理系统。

### Q10: 一次请求在后端大概怎么走？

A: 请求进入后，先做 body size、鉴权、权限和 quota 检查；再解析 model alias，选择 provider/model/credential/fallback；然后根据 Provider 协议做直通或转换；上游响应回来后做错误映射、token 和费用记录、request log、metrics 和 Provider health 更新。

### Q11: 模型路由怎么设计？

A: 路由先把用户传入的 model 解析成具体 provider 和真实模型，再根据 provider 配置、凭证状态和 fallback 候选决定调用路径。入口层不直接耦合所有 provider 细节，避免路由逻辑变成巨大条件分支。

### Q12: Dashboard 在项目里承担什么角色？

A: Dashboard 是轻量控制面，主要用于管理用户、API Key、Provider、模型、额度、请求日志、健康状态和系统设置。它不是聊天客户端，也不是营销页面。

## 3. 协议层

### Q13: 协议适配解决什么问题？

A: 不同 Provider 的请求和响应语义不同。客户端可能期望 Anthropic Messages，而上游可能是 OpenAI-compatible。ModelPort 要把 content blocks、tool use、streaming、错误和 usage 映射成客户端稳定理解的协议。

### Q14: Anthropic Messages 和 OpenAI Chat 最大差异是什么？

A: Anthropic 更偏 content block 模型，比如 `text`、`tool_use`、`tool_result`；OpenAI Chat 更偏 role message 模型，工具调用通过 `tool_calls` 和 `role=tool` 表达。

### Q15: 关键字段怎么映射？

A: `stop_sequences` 映射到 `stop`；`max_tokens` 根据 Provider 配置映射到 `max_tokens` 或 `max_completion_tokens`；Anthropic content blocks 映射到 OpenAI role messages；`tool_use` 映射到 `tool_calls`；`tool_result` 映射到 `role=tool`。

### Q16: 返回时怎么映射 stop reason？

A: OpenAI `finish_reason=length` 映射为 Anthropic `stop_reason=max_tokens`；`finish_reason=tool_calls` 或 legacy `function_call` 映射为 `stop_reason=tool_use`；普通 stop 映射为 `end_turn`。

### Q17: 为什么需要错误映射？

A: 客户端期望稳定的错误类型。上游可能返回各种状态码和结构，网关需要把它们映射成 Anthropic 风格 error event 或统一错误响应，避免客户端处理不了。

### Q18: fidelity 检查有什么意义？

A: 不是所有 Anthropic 字段都能被 OpenAI-compatible Provider 无损表达。fidelity 检查可以提前发现无法保真的字段，避免静默丢语义。

### Q19: 为什么没有一开始做一个完整内部 IR？

A: 当前 Provider 数量和协议差异还可控，结构化 adapter 更简单、更容易测试。内部 IR 会增加抽象成本。等多个 Provider 需要 schema transformation、argument repair 或复杂 replay diagnostics 时，再引入更合适。

## 4. Streaming

### Q20: Streaming 的主要难点是什么？

A: 上游 SSE 事件和客户端期望的事件不一定一致。尤其是文本或工具参数可能分片、累计回放、顺序不稳定，网关必须把它们归一化成稳定的 Anthropic SSE。

### Q21: Anthropic SSE 输出顺序是什么？

A: 典型顺序是 `message_start -> content_block_start -> content_block_delta -> content_block_stop -> message_delta -> message_stop`。

### Q22: 什么是累计回放问题？

A: 有些 Provider 每次不是只发新增 delta，而是不断重复发送从开头累计到当前的完整字符串。如果直接转发，客户端会看到重复文本或重复 arguments。ModelPort 会只保留新增 suffix。

### Q23: 工具参数 streaming 为什么更难？

A: 因为 arguments 通常是 JSON 片段，可能分多次到达，也可能先到 arguments 后到 name，还可能被上游累计回放。网关需要缓存、去重，并以 Anthropic `input_json_delta` 形式输出。

### Q24: 非对象 arguments 怎么处理？

A: Anthropic `tool_use.input` 期望是对象。如果 OpenAI-compatible 上游返回字符串、数组或非法 JSON，ModelPort 会包装为 `_raw_arguments`，既保留信息又保证结构合法。

## 5. Tool Use

### Q25: Tool Use 为什么是项目重点？

A: Tool Use 是代码类客户端最敏感的能力。普通文本格式错一点还能容忍，但工具调用的 id、role、arguments、result 配对或 streaming 事件出错，会直接导致客户端工具执行链断掉。

### Q26: Tool Use 的完整链路是什么？

A: 完整链路是 `Declare Tools -> Assistant tool_use -> Client Executes Tool -> User tool_result -> Model Continues`。网关要保证这条链路的因果关系。

### Q27: 请求侧做了哪些 Tool Use 校验？

A: `tools` 必须是数组；tool name 必须唯一且合法；`input_schema` 必须是 object；`tool_choice` 只允许 `auto / any / none / tool`；named `tool_choice` 必须命中已声明 tool；`tool_use.id` 不可为空或重复；`tool_result` 必须引用历史 `tool_use`；重复 `tool_result` 会被拒绝。

### Q28: `tool_choice` 怎么映射到 OpenAI-compatible？

A: `tool_choice.auto` 映射为 `"auto"`；`none` 映射为 `"none"`；`any` 映射为 `"required"`；`tool` 映射为 named function choice。

### Q29: `disable_parallel_tool_use` 怎么处理？

A: Anthropic 的 `disable_parallel_tool_use=true` 会映射为 OpenAI 的 `parallel_tool_calls=false`。同时 Provider capability matrix 可以声明某个 Provider 是否支持 parallel tool calls。

### Q30: assistant `tool_use` 怎么映射？

A: Anthropic assistant `tool_use` block 会转换成 OpenAI assistant message 的 `tool_calls`，其中 `name` 进入 function name，`input` 序列化成 function arguments。

### Q31: user `tool_result` 怎么映射？

A: Anthropic user `tool_result` block 会转换成 OpenAI `role=tool` message，并通过 `tool_call_id` 关联之前的 tool call。

### Q32: OpenAI `tool_calls` 怎么映射回 Anthropic？

A: OpenAI response 里的 `tool_calls` 会变成 Anthropic content block 中的 `tool_use`，并把 function arguments 解析为 `input`。legacy `function_call` 也会兼容成 `tool_use`。

### Q33: 为什么要拒绝重复 tool result？

A: 同一个工具调用只能被回答一次。重复 `tool_result` 会破坏工具调用状态机，导致模型上下文歧义。提前拒绝比把错误交给上游更可控。

### Q34: Provider Tool Use 能力矩阵有什么用？

A: 不同 Provider 对 Tool Use、`tool_choice`、parallel tool calls 和 streaming arguments 支持程度不同。能力矩阵可以在路由前做门禁，也能在 Dashboard 展示 Provider 能力。

### Q35: Tool Use acceptance 脚本验证什么？

A: 它用 mock OpenAI-compatible upstream 验证非流式 Tool Use response mapping、流式 `input_json_delta`、`tool_result` continuation、非法 Tool Use 请求拒绝、`disable_parallel_tool_use` 到 `parallel_tool_calls=false` 的映射。

## 6. 鉴权与权限

### Q36: 当前有哪些鉴权方式？

A: 有 legacy token、Dashboard Admin Session、API Key。legacy token 主要兼容数据面；Admin Session 用于控制台登录；API Key 用于标识调用身份和挂载 quota、policy。

### Q37: 为什么需要 CSRF？

A: Dashboard 使用 session cookie，控制台写操作需要防止跨站请求伪造。CSRF 保护可以避免用户登录控制台后被其他站点诱导发起管理操作。

### Q38: Team Policy 控制什么？

A: Team Policy 可以限制调用方允许使用哪些 provider 和 model，避免所有 API Key 都拥有同样权限。

### Q39: IP Allowlist 有什么意义？

A: 它限制 API Key 或调用身份的来源地址，适合小团队内网、本机或固定出口场景，降低 key 泄露后的风险面。

### Q40: 控制面和数据面为什么必须分开？

A: 模型调用权限和系统管理权限不是一回事。数据面凭证可以发请求，但不能创建用户、修改 Provider 或查看所有日志。分开后权限边界更清晰。

## 7. 安全治理

### Q41: Provider URL SSRF Guard 防什么？

A: 防止可配置的 Provider Base URL 被设置成 metadata IP、内网地址、带 userinfo 的 URL 或其他危险地址，从而通过网关探测本机和内网资源。

### Q42: 为什么要禁止 URL userinfo？

A: userinfo 可能隐藏账号密码或绕过审查，比如 `https://user:pass@example.com`。禁止它可以减少凭证泄露和 URL 混淆风险。

### Q43: 为什么要限制请求体大小？

A: 防止超大请求消耗内存、拖慢服务或造成拒绝服务。网关应在进入协议转换和上游调用前做边界控制。

### Q44: Secret Redaction 做什么？

A: 它会在日志、错误和诊断信息中脱敏 API key、Bearer token、数据库 URL 密码等敏感内容，避免排障时泄露密钥。

### Q45: CORS / Origin Guard 解决什么问题？

A: 它限制控制台和 API 的跨源访问，避免非可信网页直接调用本地网关或控制面接口。

## 8. 稳定性

### Q46: Rate Limit 和 Quota 的区别是什么？

A: Rate Limit 更偏单位时间请求控制，防止瞬时流量过大；Quota/Budget 更偏长期用量或花费控制，防止 API Key 或团队超额消耗。

### Q47: Credential Pool 有什么作用？

A: 同一个 Provider 可以配置多个上游凭证。凭证池可以支持轮询、手动选择、故障切换和账号级健康记录，提高可用性。

### Q48: Cooldown 是什么？

A: 当某个 Provider 或 credential 出现限流、账号问题或可恢复失败时，网关可以把它放入冷却，短时间内减少继续打到同一个故障点。

### Q49: Fallback 怎么讲？

A: Fallback 是当当前 provider 或 credential 不可用时，基于候选配置切换到其他可用路径。它依赖健康状态、错误分类和路由策略，不能盲目重试。

### Q50: 为什么要识别余额不足？

A: 余额不足和普通 500 不一样。它通常需要用户充值或切换账号。ModelPort 会把它归类为 account issue，设置 `rechargeRequired=true`，并展示 `代充值` 标记。

### Q51: 上游错误如何分类？

A: 可以分为 `rate_limit`、`account`、`auth`、`transport`、`upstream_protocol`、`server_error`。分类后才能给出正确的 cooldown、fallback 和用户提示。

## 9. 可观测性

### Q52: Request Logs 记录什么？

A: 记录时间、状态、provider/model、调用身份、状态码、耗时、tokens、费用、网络来源、retry/fallback、错误上下文和请求 id 等，方便排查和审计。

### Q53: Metrics 有什么价值？

A: Prometheus metrics 可以从服务维度观察请求量、成功失败、延迟和消息调用指标，适合长期运行和外部监控集成。

### Q54: Provider Health 展示什么？

A: 展示 provider 或 credential 的请求数、成功率、连续失败、最后错误、最后状态码、冷却状态、余额不足标记和推荐操作。

### Q55: Trace ID 为什么重要？

A: Trace ID 可以把客户端请求、后端日志、上游调用和 Dashboard 记录串起来。出现问题时，可以用一个 id 快速定位整条链路。

### Q56: `readyz` 和 `livez` 有什么区别？

A: `livez` 判断服务进程是否活着；`readyz` 判断服务是否准备好处理真实请求，并可包含存储和 Provider 健康等信息。

## 10. 前端与产品体验

### Q57: Dashboard 为什么不是普通 CRUD？

A: Dashboard 是网关控制面，重点是让用户管理 Provider、模型、API Key、额度、日志和健康状态。它需要面向排障和重复运维场景，而不是单纯表单增删改查。

### Q58: 请求日志页面为什么重要？

A: 网关排障最常看请求日志。它需要同时展示状态、provider、身份、模型、耗时、tokens、费用、网络来源、错误上下文和 retry/fallback 信息。

### Q59: 登录页和视觉设计为什么值得做？

A: 作为本地控制台，登录页是用户对项目完成度的第一印象。视觉上不需要营销化，但要体现技术控制台的可信度和清晰结构。

## 11. 工程化验证

### Q60: 这个项目怎么验证？

A: 后端跑 `cargo fmt -- --check`、`cargo test`、`cargo clippy --all-targets --all-features -- -D warnings`；前端跑 `npm run build` 和 lint；配置跑 `scripts/config-validate.sh`；部署跑 `docker compose up -d --build`；最后跑 `scripts/smoke-test.sh` 和 `scripts/tool-use-acceptance.sh`。

### Q61: 为什么 smoke test 不默认跑真实上游生成？

A: 真实上游调用会消耗额度，也会受账号状态影响。默认 smoke test 更适合验证本地网关健康、鉴权、模型列表和配置。真实 Provider 可以用单独 acceptance 或 `--upstream` 模式认证。

### Q62: Tool Use 为什么需要单独 acceptance？

A: Tool Use 涉及多轮状态和 streaming，普通 smoke test 覆盖不够。单独 acceptance 可以用 mock upstream 稳定复现工具调用、流式参数和非法请求场景。

### Q63: 如何证明改动没有破坏协议兼容？

A: 看三层验证：单测覆盖字段映射和边界校验；acceptance 覆盖端到端 Tool Use；smoke test 覆盖运行时健康。必要时再用真实 Provider 跑 provider matrix。

## 12. 取舍与演进

### Q64: 为什么不引入 Redis 做限流？

A: 当前定位是单机本地或小团队轻量部署，进程内或数据库辅助的策略已经够用。Redis 会增加部署成本。只有多实例共享限流状态时才值得引入。

### Q65: 为什么不做 Kubernetes？

A: Kubernetes 适合规模化和复杂运维，但 ModelPort 当前优先低运维。Docker Compose 对个人和小团队更直接，systemd 或 Compose 已能满足多数部署。

### Q66: 为什么不做 OIDC/SSO？

A: OIDC 适合组织级身份体系，但会增加配置和运维复杂度。当前 Admin Session、用户和 API Key 更适合本地小团队。等外部组织接入或企业 SSO 成为真实需求再做。

### Q67: 下一阶段最值得做什么？

A: 继续增强 Tool Use compatibility、Provider acceptance matrix、账号池健康趋势、fallback 策略解释、日志检索体验和文档化部署流程。

### Q68: 什么时候应该引入内部 IR？

A: 当 Provider 数量变多，协议差异开始扩散到多个 adapter，并且需要统一处理 schema transformation、argument repair、tool replay diagnostics 时，引入内部 IR 才能降低复杂度。

### Q69: 项目最大的长期风险是什么？

A: 最大风险是协议适配复杂度膨胀。每个 Provider 都有细微差异，如果没有能力矩阵、acceptance 测试和清晰边界，后续会变成难维护的特例堆积。

### Q70: 如何控制长期复杂度？

A: 保持模块边界清晰，Provider 差异用配置和能力矩阵表达；新增行为必须补测试；文档记录 provider 实测状态；不要过早引入重平台能力。

## 13. 结束话术

### Q71: 最后怎么总结项目？

A: ModelPort 的价值不是把请求转出去，而是把本地大模型调用变成一个可治理、可诊断、可验证、可逐步扩展的工程系统。它在小团队场景下用轻量方案解决了协议兼容、Tool Use、安全边界、稳定性和可观测性这些真实网关问题。
