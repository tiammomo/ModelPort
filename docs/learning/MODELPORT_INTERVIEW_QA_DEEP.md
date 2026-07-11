# ModelPort 深度面试问题与回答

> **非规范、基于版本的讲解材料。** 最后复核于 2026-07-11。代码继续演进时，
> 以 [Architecture](../ARCHITECTURE.md)、[Configuration](../CONFIGURATION.md)、
> [API](../API.md) 和 [Operations](../OPERATIONS.md) 为准。本文不得作为真实
> Provider 验证记录，也不得省略 stream 完成态、quota 并发、DNS SSRF 和同步整
> 文档持久化限制。

这份文档是高级工程师面试版题库。它不是普通功能清单，而是围绕 ModelPort 的工程边界、失败模式、取舍、验证和长期演进来组织。

建议使用方式：

1. 先用 [ModelPort 技术面经](./MODELPORT_INTERVIEW_GUIDE.md) 串完整讲述。
2. 再用本文按专题刷追问。
3. 面试时先给简短回答，再根据面试官兴趣展开到代码、测试和取舍。

## A. Tool Use 深度问题

### Q1: 为什么 Tool Use 不能只做字段映射？

**考察点：**
面试官想看你是否理解 Tool Use 是一个多轮协议状态机，而不是简单 JSON 改名。

**简短回答：**
Tool Use 的核心是保持工具调用链路的因果关系。一次调用从 `tools` 声明开始，经过 assistant `tool_use`、客户端执行、user `tool_result`，再回到模型继续生成。只改字段名会漏掉 id 配对、role 约束、流式参数和 provider 能力差异。

**深入展开：**
如果不做结构化校验，错误的 `tool_result` 可能引用不存在的 `tool_use.id`，或者同一个工具调用被回答两次，客户端和模型都会进入不一致状态。ModelPort 在请求进入 provider 前先运行 `validate_anthropic_tooling`，验证工具定义、`tool_choice`、content block 和历史引用，再在 provider 边界用 `validate_anthropic_tool_capabilities` 做能力门禁。这样非法请求不会打到上游，也不会在日志里变成难排查的 upstream 500。

**代码/文档依据：**
- `src/tool_use.rs`
- `src/types.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- 如果 provider 自己也会校验，为什么网关还要校验？
- Tool Use 状态机最容易坏在哪一环？
- 哪些错误应该 400，哪些应该交给上游？

**回答注意：**
不要说 “Tool Use 就是 `tool_use` 到 `tool_calls` 的字段替换”。要强调 id、role、turn reference、provider capability 和 streaming 共同构成协议正确性。

### Q2: `tools` 校验为什么要在路由前完成？

**考察点：**
面试官想看你是否能把输入校验、故障定位和上游成本联系起来。

**简短回答：**
`tools` 是客户端和模型之间的契约，必须在网关入口先保证形状正确。ModelPort 要求 `tools` 是数组，tool item 是对象，name 存在且唯一，`description` 是字符串，`input_schema` 是 object schema。这样可以把客户端错误稳定归类为 invalid request，而不是消耗上游额度。

**深入展开：**
如果网关不提前校验，某些 provider 可能直接拒绝，某些可能静默忽略字段，最终客户端表现不一致。ModelPort 的 `validate_tool_definitions` 会收集 tool name 到 `HashSet`，用于后续校验 named `tool_choice` 和 assistant `tool_use.name` 是否命中已声明工具。这里的取舍是只做结构和关键语义校验，不在当前阶段实现完整 JSON Schema validator，避免把轻量网关做成 schema 执行器。

**代码/文档依据：**
- `src/tool_use.rs`
- `src/routes/client_api.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- 为什么只要求 `input_schema.type=object`？
- 是否应该校验 `required/properties` 的完整 JSON Schema？
- tool name 规则对不同 provider 是否一致？

**回答注意：**
要讲清楚当前校验是协议安全边界，不是完整 schema 语义执行。不要承诺已经做了完整 JSON Schema 校验。

### Q3: `tool_choice` 校验有哪些边界？

**考察点：**
面试官想看你是否理解工具选择控制和 declared tools 之间的关系。

**简短回答：**
ModelPort 要求 `tool_choice` 是对象，`type` 必须是 `auto / any / none / tool`。如果是 named tool choice，`name` 必须是字符串、合法工具名，并且在声明过的 tools 中存在。`any` 和 `tool` 还要求请求中至少声明一个 tool。

**深入展开：**
如果 `tool_choice.type=tool` 但没有 name，上游可能给出模糊错误；如果 name 不在 tools 里，模型会被要求调用不存在的工具。ModelPort 在 `validate_tool_choice_shape` 中提前拒绝这些情况，并检查 `disable_parallel_tool_use` 必须是 boolean。这个设计把客户端可修复错误定位在网关入口，避免 provider 差异影响用户体验。

**代码/文档依据：**
- `src/tool_use.rs`
- `src/types.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- `tool_choice.any` 为什么要求至少一个 tool？
- named `tool_choice` 在没有 tools 时是否允许？
- `disable_parallel_tool_use` 为什么不是 provider 侧再判断？

**回答注意：**
不要把 `tool_choice` 只讲成 OpenAI 的字段。它在 Anthropic contract 中有自己的语义，转换只是后续一步。

### Q4: 为什么要校验 `tool_use / tool_result` 的因果关系？

**考察点：**
面试官想看你是否理解多轮对话里工具调用的状态一致性。

**简短回答：**
`tool_result` 必须回答前面 assistant 发出的 `tool_use`，否则模型上下文会出现无来源结果。ModelPort 维护 seen 和 pending 两个集合，确保 `tool_result.tool_use_id` 必须匹配历史 `tool_use.id`，并且每个 tool use 只能被回答一次。

**深入展开：**
如果不校验，客户端可能把另一个请求的工具结果塞进当前对话，或者重复提交同一个工具结果。上游 provider 可能接受这些脏上下文，导致模型输出不可预测。`validate_tool_turn_references` 用 `seen_tool_use_ids` 记录所有出现过的工具调用，用 `pending_tool_use_ids` 记录尚未回答的工具调用。`tool_result` 命中后从 pending 中移除，第二次回答同一 id 会被拒绝。

**代码/文档依据：**
- `src/tool_use.rs`
- `src/types.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- 如果 tool_result 比 tool_use 先出现怎么办？
- 并行工具调用如何区分多个 pending id？
- 是否允许一个 user message 回答多个 tool use？

**回答注意：**
要强调这是协议层保护，不是业务层工具执行。ModelPort 不执行工具，只守住工具调用上下文。

### Q5: 为什么要拒绝重复 `tool_use.id`？

**考察点：**
面试官想看你是否能解释 id 唯一性和 continuation 的关系。

**简短回答：**
`tool_use.id` 是后续 `tool_result.tool_use_id` 的锚点，重复会让结果无法确定归属。ModelPort 在遍历 assistant tool blocks 时用集合检测重复 id，发现重复直接返回 invalid request。

**深入展开：**
如果两个不同工具调用共享同一个 id，客户端执行后的结果无法区分应该回给哪个 tool call。尤其在 parallel tool calls 场景下，重复 id 会让 pending 状态错误收敛。ModelPort 选择在入口拒绝，而不是尝试重写 id，因为重写会破坏客户端和上游之间的可追踪性。

**代码/文档依据：**
- `src/tool_use.rs`
- `src/routes.rs` 测试用例
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- 能不能自动生成新 id？
- 重复 id 是否可能来自 provider 返回？
- 并行工具调用如何保证 id 唯一？

**回答注意：**
不要说网关会随意改客户端 id。请求侧 id 是客户端上下文的一部分，自动改写可能造成更隐蔽的问题。

### Q6: 为什么重复 `tool_result` 要拒绝？

**考察点：**
面试官想看你是否能讲清楚工具状态机的完成态。

**简短回答：**
一个 `tool_use` 只能有一个对应结果。ModelPort 在 `tool_result` 命中 pending id 后移除该 id，再次提交同一个 `tool_use_id` 时会返回 “has already been answered”。

**深入展开：**
如果允许重复结果，模型会看到同一个工具调用被执行两次，而且结果可能不一致。对代码类客户端来说，这会破坏文件读取、命令执行或搜索工具的确定性。ModelPort 的取舍是严格拒绝重复结果，而不是合并内容，因为合并策略属于工具业务语义，网关不应猜测。

**代码/文档依据：**
- `src/tool_use.rs`
- `src/types.rs` 测试
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- 如果工具执行失败后想重试怎么办？
- 一个 `tool_result` 能不能包含多个 content block？
- 并行工具结果乱序回来怎么办？

**回答注意：**
要区分 “同一 tool_use 重复回答” 和 “多个 pending tool_use 分别回答”。后者是允许的，前者要拒绝。

### Q7: Anthropic `tools[]` 到 OpenAI `tools[].function` 怎么转换？

**考察点：**
面试官想看你是否了解两个协议工具声明的结构差异。

**简短回答：**
Anthropic 工具声明直接有 `name/description/input_schema`；OpenAI-compatible 工具声明通常是 `type=function`，下面挂 `function.name`、`function.description` 和 `function.parameters`。ModelPort 在 `convert_tools` 中做结构映射，缺省 schema 会给一个空 object schema。

**深入展开：**
如果直接透传 Anthropic `tools`，OpenAI-compatible provider 可能无法识别工具。ModelPort 的转换保持工具名和描述不变，把 `input_schema` 放到 OpenAI 的 `parameters`。这个转换前已经做过工具定义校验，所以 adapter 可以假设基本结构合法。边界是不同 provider 对 JSON Schema 支持深度不一样，当前不做 provider-specific schema rewrite。

**代码/文档依据：**
- `src/types.rs`
- `src/tool_use.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- OpenAI function parameters 和 Anthropic input_schema 是否完全等价？
- 如果 provider 不支持某些 schema 关键字怎么办？
- 为什么不做 schema 降级？

**回答注意：**
不要宣称 schema 完全无损。当前是结构映射，深层 schema 能力依赖 provider。

### Q8: `tool_choice.any` 为什么映射成 OpenAI `required`？

**考察点：**
面试官想看你是否理解不同协议的语义等价，而不是只记字段名。

**简短回答：**
Anthropic 的 `tool_choice.type=any` 表示模型必须选择某个工具，但不限定具体工具。OpenAI-compatible 中对应语义更接近 `"required"`，表示必须产生 tool call。

**深入展开：**
如果把 `any` 映射成 `auto`，模型可能选择不调用工具，客户端语义就被弱化。ModelPort 在 `convert_tool_choice` 里明确把 `any` 转成 `required`。这里也解释了为什么 `any` 必须要求至少有一个工具，否则 “必须调用任意工具” 没有可执行对象。

**代码/文档依据：**
- `src/types.rs`
- `src/tool_use.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- `auto`、`none`、`any`、`tool` 分别怎么映射？
- 如果 provider 不支持 `required` 怎么办？
- named tool choice 怎么转换？

**回答注意：**
要讲语义，不要只背 “any 到 required”。如果 provider 不支持，应通过 capability 或 provider 实测矩阵记录，而不是静默降级。

### Q9: `disable_parallel_tool_use` 为什么映射到 `parallel_tool_calls=false`？

**考察点：**
面试官想看你是否能解释并行工具调用控制在协议转换中的位置。

**简短回答：**
Anthropic 用 `disable_parallel_tool_use` 表达禁止并行工具调用；OpenAI-compatible 用 `parallel_tool_calls` 表达是否允许并行。语义方向相反，所以 `disable_parallel_tool_use=true` 要映射为 `parallel_tool_calls=false`。

**深入展开：**
这个字段不仅是请求转换问题，也和 Provider 能力有关。有些本地 runtime 或 provider 不支持 parallel tool calls，ModelPort 的 `ToolUseConfig.parallel_tool_calls` 会在路由前拦截不支持的请求。如果不拦截，模型可能返回多个 tool calls，而客户端或 provider 无法稳定处理。

**代码/文档依据：**
- `src/types.rs`
- `src/tool_use.rs`
- `src/config.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- 为什么字段取反？
- 如果客户端没有传 `disable_parallel_tool_use` 怎么办？
- provider 不支持并行时怎么拒绝？

**回答注意：**
不要把它说成 “固定关闭并行”。只有请求明确禁用或 provider capability 不支持时才处理。

### Q10: assistant `tool_use` 到 assistant `tool_calls` 的关键点是什么？

**考察点：**
面试官想看你是否理解 assistant 消息中工具调用的结构化转换。

**简短回答：**
Anthropic assistant message 可以包含 text block 和 `tool_use` block。转换到 OpenAI-compatible 时，text 合并到 assistant `content`，`tool_use` 转成 `tool_calls`，其中 `id` 保留，`name` 进入 function name，`input` 序列化为 function arguments。

**深入展开：**
如果不保留 id，后续 user `tool_result` 就无法映射到 OpenAI 的 `tool_call_id`。如果不把 input 序列化为字符串，OpenAI-compatible function arguments 结构也不匹配。ModelPort 的 `convert_assistant_message` 会保留 text，同时收集多个 tool calls。边界是未知 block 会被忽略或转换为文本时要谨慎，避免引入不可保真的语义。

**代码/文档依据：**
- `src/types.rs`
- `src/tool_use.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- assistant 同时有 text 和 tool_use 怎么办？
- input 序列化失败怎么办？
- 如果 tool_use 缺少 name 怎么处理？

**回答注意：**
要讲清楚 OpenAI `function.arguments` 是字符串，而 Anthropic `input` 是对象。

### Q11: user `tool_result` 到 `role=tool` 怎么转换？

**考察点：**
面试官想看你是否理解工具结果 continuation 的 role 语义。

**简短回答：**
Anthropic user message 中的 `tool_result` 会被转换成 OpenAI `role=tool` message，`tool_use_id` 映射为 `tool_call_id`，content 转成文本。这样 OpenAI-compatible provider 能继续关联前一次 assistant tool call。

**深入展开：**
如果 user message 同时包含普通 text 和 tool_result，ModelPort 会先把累积 text 作为 user message 输出，再输出 role=tool message。这避免把自然语言和工具执行结果混在同一个 OpenAI tool message 中。边界是 `tool_result.content` 可能是 string 或 blocks，当前通过 `content_to_text` 做文本化，适合多数代码客户端场景。

**代码/文档依据：**
- `src/types.rs`
- `src/tool_use.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- tool_result content 是数组时怎么处理？
- user text 和 tool_result 混合时顺序如何保持？
- 为什么 role 要变成 tool？

**回答注意：**
不要说 user `tool_result` 仍然是 user role 直接透传。OpenAI-compatible 需要 `role=tool` 才能关联 tool call。

### Q12: OpenAI `tool_calls` 如何映射回 Anthropic `tool_use`？

**考察点：**
面试官想看你是否理解响应方向的协议归一化。

**简短回答：**
ModelPort 会遍历 OpenAI response 的 `message.tool_calls`，为每个 call 生成 Anthropic `tool_use` block，保留 id 和 function name，并把 function arguments 解析成 `input` 对象。

**深入展开：**
如果 OpenAI 没有返回 id，ModelPort 会生成 `toolu_` 前缀 id，避免 Anthropic 侧缺少必要字段。arguments 解析时，如果是对象就直接作为 input；如果是非对象或非法 JSON，就包装成 `_raw_arguments`。这样既维持 Anthropic 结构合法，也不丢失 provider 返回的原始信息。

**代码/文档依据：**
- `src/types.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- 为什么要生成 id？
- 非法 JSON 怎么处理？
- 多个 tool_calls 的顺序如何保留？

**回答注意：**
不要宣称所有 arguments 都能修复成业务对象。当前保证结构合法和信息保留，不做业务级参数修复。

### Q13: 为什么兼容 legacy `function_call`？

**考察点：**
面试官想看你是否考虑 provider 和 OpenAI-compatible 实现的历史差异。

**简短回答：**
一些 OpenAI-compatible provider 可能仍然返回旧版 `function_call`，而不是 `tool_calls`。ModelPort 在响应映射和 streaming 中都兼容 `function_call`，把它转换为 Anthropic `tool_use`。

**深入展开：**
如果不兼容 legacy `function_call`，同样是 OpenAI-compatible 的 provider 在 Tool Use 场景下会表现不一致。ModelPort 的处理方式是：如果没有标准 `tool_calls`，但存在 `function_call`，就生成一个 `tool_use` block，id 用 `toolu_` UUID。streaming 路径里也用单独的 `function_call` state 处理旧格式增量。

**代码/文档依据：**
- `src/types.rs`
- `src/providers/openai_stream.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- `function_call` 和 `tool_calls` 同时出现怎么办？
- legacy function_call 如何生成 id？
- 是否会长期保留这类兼容？

**回答注意：**
要说清楚这是兼容 OpenAI-compatible 生态差异，不是鼓励新 provider 继续使用旧协议。

### Q14: `input_json_delta` 在 streaming 中怎么产生？

**考察点：**
面试官想看你是否理解 Anthropic SSE 中工具参数的流式表达。

**简短回答：**
OpenAI stream 里 tool function arguments 可能以增量字符串出现。ModelPort 把这些增量转换成 Anthropic `content_block_delta`，其中 `delta.type=input_json_delta`，`partial_json` 是当前应输出的参数片段。

**深入展开：**
当 tool name 到达后，ModelPort 先发 `content_block_start`，再发参数 delta。如果 arguments 先于 name 到达，会暂存在 `ToolState.pending_arguments` 中，等 name 出现并 start block 后再输出。如果启用了 deduplicate 模式，ModelPort 会收集 raw arguments 并尝试恢复 best complete JSON object，避免累计回放导致重复输出。

**代码/文档依据：**
- `src/providers/openai_stream.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- name 没来但 arguments 来了怎么办？
- deduplicate 模式为什么最后可能输出完整 JSON？
- `content_block_start` 什么时候发？

**回答注意：**
不要把 streaming 描述成简单逐行转发。这里有状态缓存、block index 和 start/stop 生命周期。

### Q15: delta、cumulative、best_effort 三类 streaming 参数如何理解？

**考察点：**
面试官想看你是否能分类 provider streaming 行为。

**简短回答：**
delta 表示每次只发新增片段；cumulative 表示每次发从开头到当前的累计字符串；best_effort 表示 provider 行为不稳定，需要尽量去重和恢复完整 JSON。ModelPort 通过 provider `tool_use.streaming_arguments` 选择工具参数策略；普通文本的重复回放则由独立的 `deduplicate_stream_text` 处理。

**深入展开：**
标准 delta 可以直接输出 pending arguments。累计回放不能直接输出，否则客户端会看到重复 JSON 片段。ModelPort 的 `text_delta` 会根据已见内容计算新增 suffix，`best_complete_json_object` 会从多个候选字符串中提取最长完整对象。当前取舍是做通用去重和恢复，不做 provider-specific 的复杂参数语义修复。

**代码/文档依据：**
- `src/providers/openai_stream.rs`
- `src/config.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- `text_delta` 如何处理重叠片段？
- 如果 provider 输出多个 JSON 对象怎么办？
- best_effort 会不会误删合法重复内容？

**回答注意：**
要承认去重是工程折中，重点解决 provider 回放和稳定性，不保证理解业务层 JSON 语义。

### Q16: 工具参数半截 JSON 怎么处理？

**考察点：**
面试官想看你是否理解流式 JSON 的不完整性。

**简短回答：**
流式 arguments 经常不是完整 JSON，ModelPort 不会要求每个 chunk 都能 parse。它会把片段作为 `partial_json` 输出，或者在 deduplicate 模式下缓存多个来源，最终尝试恢复最长完整 JSON object。

**深入展开：**
如果每个 chunk 都强行 parse，不完整片段会被误判为 upstream protocol error。Anthropic 的 `input_json_delta` 本来就允许分片传输，所以 ModelPort 可以输出 partial JSON。只有在非流式响应映射时，才会用 `parse_tool_arguments` 解析完整 arguments；解析失败则包装 `_raw_arguments`。

**代码/文档依据：**
- `src/providers/openai_stream.rs`
- `src/types.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- streaming 中什么时候 parse JSON？
- 非流式和流式处理为什么不一样？
- 如何验证半截 JSON 不会导致错误？

**回答注意：**
不要说每个 streaming chunk 都必须是合法 JSON。关键是 Anthropic SSE 允许 `partial_json`。

### Q17: name 和 arguments 顺序不稳定怎么处理？

**考察点：**
面试官想看你是否考虑实际 provider 的乱序增量。

**简短回答：**
ModelPort 的 `ToolState` 会分别记录 name、upstream id、pending arguments 和 started 状态。如果 arguments 先到，先缓存；等 name 到达后再发 `content_block_start` 和之前缓存的参数。

**深入展开：**
Anthropic `content_block_start` 需要带 tool name。如果 arguments 先到就直接 start，会缺少 name；如果丢弃 arguments，又会丢工具参数。ModelPort 的状态机选择缓存 arguments，并在 name 出现后启动 block。若流结束时仍没有 name，但已有参数数据，也会用 synthetic `"tool"` name 启动 block，避免静默丢数据。

**代码/文档依据：**
- `src/providers/openai_stream.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- 没有 name 时为什么不报错？
- synthetic tool name 会不会误导客户端？
- 多个 tool call index 如何区分？

**回答注意：**
要讲这是容错策略。它保留信息，但 provider 兼容性仍应通过 acceptance 标注。

### Q18: 非对象 arguments 为什么包装为 `_raw_arguments`？

**考察点：**
面试官想看你是否理解协议契约和信息保留之间的取舍。

**简短回答：**
Anthropic `tool_use.input` 需要对象，但 OpenAI-compatible provider 可能返回字符串、数组或非法 JSON。ModelPort 把这类值包装到 `{ "_raw_arguments": ... }`，既保持 Anthropic 结构合法，又不丢上游返回内容。

**深入展开：**
如果直接透传非对象，会破坏客户端对 `input` 的结构预期；如果直接丢弃，会让工具调用不可恢复。包装 `_raw_arguments` 是中间选择。它不承诺业务参数已经修复，只保证协议层合法和信息可见。

**代码/文档依据：**
- `src/types.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- 客户端如何处理 `_raw_arguments`？
- 为什么不直接返回错误？
- 什么时候应该做更强 argument repair？

**回答注意：**
不要说 `_raw_arguments` 是最佳业务格式。它是兼容兜底。

### Q19: Provider Tool Use capability matrix 解决什么问题？

**考察点：**
面试官想看你是否能把 provider 差异产品化和配置化。

**简短回答：**
不同 provider 对 `tool_use`、`tool_choice`、parallel tool calls 和 streaming arguments 支持不一样。ModelPort 用 `ToolUseConfig` 描述能力，在路由前做门禁，并在 Dashboard 和文档中展示 provider 能力；其中 `streaming_arguments` 还会实际选择 OpenAI-compatible 工具参数流处理策略。

**深入展开：**
如果没有能力矩阵，所有 provider 都会被假设支持完整 Tool Use，一旦 provider 不支持就会产生模糊上游错误。`validate_anthropic_tool_capabilities` 会根据 `supported`、`tool_choice` 和 `parallel_tool_calls` 拦截不兼容请求。`streaming_arguments="delta"` 保留增量参数片段，`cumulative` 和 `best_effort` 启用回放去重与完整 JSON 恢复；这仍是轻量配置，不是上游兼容性认证，也不是重型 provider plugin 系统。

**代码/文档依据：**
- `src/config.rs`
- `src/tool_use.rs`
- `src/routes/admin_providers.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- capability 应该由配置还是自动探测？
- provider 支持一部分 tool_choice 怎么办？
- Dashboard 如何展示这些能力？

**回答注意：**
不要说矩阵能自动保证所有 provider 正确。它是门禁和声明，仍需要 acceptance 验证。

### Q20: 为什么暂不引入完整 Tool IR？

**考察点：**
面试官想看你是否有抽象克制，而不是见到协议差异就堆架构。

**简短回答：**
当前差异还可以用结构化 adapter 和能力矩阵控制，完整 Tool IR 会增加抽象层、迁移成本和测试成本。ModelPort 先保证现有 Anthropic/OpenAI-compatible 路径可靠，等 provider 数量和差异复杂度真正上来再引入 IR。

**深入展开：**
IR 适合在多 provider 都需要深度 schema transformation、argument repair、tool replay diagnostics 时引入。现在如果提前做，容易为了未来不确定需求牺牲当前可读性。现阶段的边界是 `tool_use.rs` 负责校验，`types.rs` 负责非流式转换，`openai_stream.rs` 负责 streaming 归一化，模块边界已经能支撑当前规模。

**代码/文档依据：**
- `src/tool_use.rs`
- `src/types.rs`
- `src/providers/openai_stream.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- 引入 IR 的触发条件是什么？
- 当前 adapter 会不会演变成特例堆？
- 如果新增 Gemini native protocol 怎么办？

**回答注意：**
不要把 “没有 IR” 说成能力不足。要说这是针对当前项目阶段的工程取舍。

### Q21: `tool-use-acceptance.sh` 为什么比普通 smoke test 更重要？

**考察点：**
面试官想看你是否知道协议能力需要专门验收。

**简短回答：**
普通 smoke test 主要验证网关健康和基础接口，覆盖不了多轮 Tool Use 和 streaming 参数。`tool-use-acceptance.sh` 用 mock OpenAI-compatible upstream 稳定验证非流式映射、流式 `input_json_delta`、`tool_result` continuation、非法请求拒绝和 parallel tool calls 映射。

**深入展开：**
Tool Use 的风险在边界，而不是 happy path 文本回复。mock upstream 能构造真实 provider 难稳定复现的情况，比如累计参数回放、非法 tool_choice、`disable_parallel_tool_use` 映射。真实上游认证可以用 `--upstream`，但默认不跑真实上游，避免消耗额度和受账号状态影响。

**代码/文档依据：**
- `scripts/tool-use-acceptance.sh`
- `docs/TOOL_USE_COMPATIBILITY.md`
- `src/routes.rs` 测试

**可能追问：**
- acceptance 和 unit test 分工是什么？
- 为什么默认不用真实 provider？
- 如何把新 provider 纳入认证？

**回答注意：**
不要把 acceptance 说成代替单测。它是端到端协议验收，和单测互补。

### Q22: Tool Use 如果失败，怎么向面试官解释排障路径？

**考察点：**
面试官想看你能否把协议、日志和验证串起来排障。

**简短回答：**
我会先看请求是否通过 `validate_anthropic_tooling`，再看 provider capability 是否允许 Tool Use，然后看协议转换后的 OpenAI body，最后看 streaming 事件和 request log。失败点不同，对应的修复也不同。

**深入展开：**
如果入口报 invalid request，多半是 tools、tool_choice 或 tool_result 引用问题；如果 provider 返回 400，可能是 provider 不支持某种 schema 或 tool_choice；如果 streaming 结果重复，重点看 `deduplicate_stream_text` 和 `streaming_arguments` 配置；如果客户端没有收到 `message_stop`，重点看 SSE 解析和 stream idle timeout。验证上先跑 unit test，再跑 `scripts/tool-use-acceptance.sh`。

**代码/文档依据：**
- `src/tool_use.rs`
- `src/types.rs`
- `src/providers/openai_stream.rs`
- `src/http.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- 如何定位是客户端问题还是 provider 问题？
- 什么日志最关键？
- 什么时候需要抓 upstream raw response？

**回答注意：**
不要只说 “看日志”。要把入口校验、能力门禁、adapter、stream 和 request log 分层讲。

## B. 协议层深度问题

### Q23: Anthropic Messages 和 OpenAI Chat 的根本语义差异是什么？

**考察点：**
面试官想看你是否理解协议模型，而不是只看字段。

**简短回答：**
Anthropic Messages 更像 content block 协议，message content 可以由 `text/tool_use/tool_result` 等 block 组成；OpenAI Chat 更像 role message 协议，工具调用通过 assistant `tool_calls` 和 `role=tool` continuation 表达。ModelPort 的 adapter 要在这两种语义之间保持尽量稳定的映射。

**深入展开：**
如果把 Anthropic content blocks 粗暴拼成字符串，Tool Use、thinking、tool_result 等结构语义都会丢失。ModelPort 对 user 和 assistant 分别处理：assistant text 与 tool_use 转成 content 和 tool_calls，user text 与 tool_result 转成 user message 和 tool role message。这个差异也是为什么协议层和 Tool Use 强相关，不能拆成完全独立的字符串代理。

**代码/文档依据：**
- `src/types.rs`
- `src/tool_use.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- content block 中的未知类型怎么处理？
- tool_result 为什么不能直接拼进 user content？
- 是否支持所有 Anthropic block？

**回答注意：**
不要宣称完全无损支持所有 Anthropic 特性。当前对核心文本和 Tool Use 做结构化适配，对无法保真的能力需要 fidelity 边界。

### Q24: content blocks 到 role messages 的转换有什么取舍？

**考察点：**
面试官想看你能否解释转换中的信息保留与兼容性取舍。

**简短回答：**
转换时要尽量保留 provider 能理解的语义：text 进入普通 content，assistant tool_use 进入 tool_calls，user tool_result 进入 role=tool。对于不适合 OpenAI-compatible 表达的 block，只能文本化或通过 fidelity 检查提前暴露风险。

**深入展开：**
ModelPort 的 `convert_message` 根据 role 分派到 `convert_assistant_message` 或 `convert_user_message`。assistant 会收集 text 和 tool_calls；user 会把普通 text 和 tool_result 拆成不同 message。这个设计保证了工具 continuation 的 role 正确。代价是某些 Anthropic 特定 block 无法完整表达，需要通过 `fidelity.rs` 识别，避免用户误以为完全无损。

**代码/文档依据：**
- `src/types.rs`
- `src/fidelity.rs`

**可能追问：**
- system blocks 如何处理？
- thinking/cache_control 等字段怎么办？
- 是否应该引入 IR 统一表达？

**回答注意：**
要讲清楚 “兼容核心路径” 和 “完全无损” 不是一回事。

### Q25: 什么时候走 Anthropic-compatible 直通，什么时候走 OpenAI-compatible 转换？

**考察点：**
面试官想看你是否理解 provider protocol 的边界。

**简短回答：**
Provider 配置里有 protocol。Anthropic-compatible provider 走直通，保留客户端 Anthropic 请求形态；OpenAI-compatible provider 走 adapter，把 Anthropic request 转成 OpenAI Chat/SSE，再把响应转回 Anthropic-compatible。

**深入展开：**
直通的好处是减少转换损耗，适合 DeepSeek 官方 Anthropic-compatible 这类参考路径。转换适合 OpenAI-compatible、本地 runtime 或自定义 OpenAI 风格上游。`send_message_attempt` 根据 `ProviderProtocol` 选择 `providers::anthropic::messages` 或 `providers::openai_compat::messages`。无论哪条路径，入口的请求边界和 Tool Use 能力门禁仍然执行。

**代码/文档依据：**
- `src/routes/client_api.rs`
- `src/providers/anthropic.rs`
- `src/providers/openai_compat.rs`
- `src/config.rs`

**可能追问：**
- 直通路径是否还做 Tool Use 校验？
- Provider protocol 配错会怎样？
- 未来支持 native Gemini 怎么扩展？

**回答注意：**
不要把 DeepSeek 说成唯一主线。它只是默认配置样例，不能替代带日期的真实验证；协议边界支持多 Provider。

### Q26: SSE event 映射为什么要统一成 Anthropic 风格？

**考察点：**
面试官想看你是否理解客户端契约优先级。

**简短回答：**
ModelPort 的客户端侧契约是 Anthropic-compatible，所以不管上游是 OpenAI-compatible 还是非流式响应，最终都要给客户端稳定的 Anthropic SSE 事件序列。这样客户端不需要理解每个 provider 的 streaming 差异。

**深入展开：**
OpenAI stream 通常是 choices delta，Anthropic stream 是 message 和 content block 生命周期。ModelPort 用 `message_start`、`content_block_start`、`content_block_delta`、`content_block_stop`、`message_delta`、`message_stop` 重建客户端期望的结构。如果不统一，客户端需要按 provider 写分支，网关价值会下降。

**代码/文档依据：**
- `src/providers/openai_stream.rs`
- `src/types.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- 非流式 OpenAI 响应怎么变成 stream？
- 如果 upstream stream 不是 SSE 怎么处理？
- message_stop 为什么重要？

**回答注意：**
要强调这是客户端契约，不是为了形式统一而统一。

### Q27: `stop_reason / finish_reason` 为什么不能直接透传？

**考察点：**
面试官想看你是否理解终止原因对客户端状态机的影响。

**简短回答：**
不同协议对结束原因命名不同。OpenAI `finish_reason=length` 应映射为 Anthropic `max_tokens`，`tool_calls/function_call` 应映射为 `tool_use`，普通 `stop` 映射为 `end_turn`。直接透传会让 Anthropic-compatible 客户端误判生成状态。

**深入展开：**
Tool Use 场景尤其依赖 stop reason。如果模型返回工具调用，但 stop reason 没变成 `tool_use`，客户端可能不会进入工具执行流程。ModelPort 在非流式和流式路径都有 `map_finish_reason`，保证终止原因一致。

**代码/文档依据：**
- `src/types.rs`
- `src/providers/openai_stream.rs`

**可能追问：**
- 未知 finish_reason 怎么处理？
- length 和 max_tokens 的语义是否完全一致？
- stop_sequence 怎么表达？

**回答注意：**
不要说所有 finish reason 都能完全保真。未知值当前按 `end_turn` 兜底，是保守兼容策略。

### Q28: `max_tokens / max_completion_tokens` 为什么要配置化？

**考察点：**
面试官想看你是否考虑 OpenAI-compatible provider 的接口差异。

**简短回答：**
不同 OpenAI-compatible provider 对输出 token 字段支持不完全一致。有的用 `max_tokens`，有的用 `max_completion_tokens`，有的兼容两者。ModelPort 用 `MaxTokensField` 配置决定发送哪个字段。

**深入展开：**
如果固定一个字段，会导致部分 provider 不识别或行为异常。`anthropic_to_openai_request` 根据 provider 配置把 Anthropic `max_tokens` 映射到对应字段。这个设计把差异放在 provider config 中，而不是在转换逻辑里写 provider id 特例，利于长期维护。

**代码/文档依据：**
- `src/types.rs`
- `src/config.rs`

**可能追问：**
- 两个字段同时发送会不会冲突？
- 新 provider 怎么确定该用哪个字段？
- 默认值怎么选？

**回答注意：**
要讲 provider 配置驱动，避免说 “OpenAI-compatible 都一样”。

### Q29: `stop_sequences` 到 `stop` 的映射有什么边界？

**考察点：**
面试官想看你是否理解停止序列是 provider 能力的一部分。

**简短回答：**
Anthropic 使用 `stop_sequences`，OpenAI-compatible 使用 `stop`。ModelPort 在请求转换时直接把 `stop_sequences` 复制到 `stop`。边界是不同 provider 对 stop 的数量、长度和行为可能不完全一致。

**深入展开：**
这个字段转换比较直接，但不能过度承诺完全一致。某些 provider 可能忽略 stop，或者对多 stop sequence 支持有限。当前 ModelPort 做结构映射，不在网关内模拟 stop 截断，因为那会影响 streaming 和 token usage 的准确性。

**代码/文档依据：**
- `src/types.rs`
- `src/config.rs`

**可能追问：**
- provider 忽略 stop 怎么办？
- 是否应该在网关层截断输出？
- streaming 中遇到 stop sequence 如何处理？

**回答注意：**
不要宣称 stop 行为完全由网关保证。网关当前负责映射和边界说明。

### Q30: 错误映射为什么要分 Anthropic error event？

**考察点：**
面试官想看你是否关注客户端错误契约。

**简短回答：**
错误是否进入 SSE 取决于下游响应头是否已经发出。ModelPort 会让连接失败、上游非 2xx 和转换失败尽量在 pre-header 阶段走正常 HTTP 错误与 fallback；只有真实流在 HTTP 200 之后才用 Anthropic 风格 `error` event 表达后续失败。

**深入展开：**
普通 OpenAI streaming 路径会先 await 上游连接并检查初始状态，因此连接错误和初始非 2xx 不会先返回本地 200；如果已经开始向客户端发 SSE，后续帧解析或上游中断才由 `openai_stream_to_anthropic` 产生协议内 error event。`buffer_stream_text=true` 更进一步：先 await 完整非流式响应并完成 OpenAI 到 Anthropic 转换，再创建本地 SSE，所以其上游 HTTP/JSON/转换失败也都是 pre-header HTTP 错误。统一错误映射与 redaction 避免直接暴露 provider 原始敏感内容。

**代码/文档依据：**
- `src/types.rs`
- `src/providers/openai_stream.rs`
- `src/error.rs`

**可能追问：**
- 上游 429 映射成什么？
- 鉴权失败和权限失败怎么区分？
- streaming 过程中错误怎么结束？

**回答注意：**
不要直接暴露上游完整错误体，尤其可能包含 secret。要提到 sanitization。

### Q31: fidelity 检查应该怎么向面试官解释？

**考察点：**
面试官想看你是否知道兼容性不只是转换成功，还要知道哪些语义会丢。

**简短回答：**
fidelity 检查用于判断 Anthropic 请求在转成 OpenAI-compatible 时是否可能丢失关键语义。它不是为了阻止所有差异，而是让不可保真的字段显性化，避免静默错配。

**深入展开：**
比如某些 content block、`tool_choice` 额外字段、thinking/cache control 等并不一定能被 OpenAI-compatible provider 表达。`fidelity.rs` 会审计请求中无法保留的字段。这个能力适合和 provider capability matrix 配合，形成 “能支持什么，不能支持什么” 的明确边界。

**代码/文档依据：**
- `src/fidelity.rs`
- `src/types.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- fidelity 是 hard reject 还是 warning？
- 新 Anthropic 字段出现怎么办？
- 如何避免误杀可兼容字段？

**回答注意：**
不要把 fidelity 说成完整形式化验证。它是工程级兼容审计。

### Q32: Provider native 能力边界怎么管理？

**考察点：**
面试官想看你是否能防止 provider 特例失控。

**简短回答：**
ModelPort 把 provider 差异尽量放到配置和能力矩阵里，比如 protocol、max token 字段、tool use 支持、streaming argument mode、base URL 和模型列表。adapter 只处理协议类别，不在核心路径散落大量 provider id 特例。

**深入展开：**
如果每接一个 provider 都在路由层写 if/else，项目很快会难维护。当前策略是：已知协议类别走统一 adapter，provider 个性用配置表达；真实兼容性用 provider matrix 和 acceptance 记录。只有当某 provider native API 和现有协议差异巨大时，才考虑新增 provider module 或内部 IR。

**代码/文档依据：**
- `src/config.rs`
- `src/providers/mod.rs`
- `docs/PROVIDER_MATRIX.md`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- provider-specific bug 怎么处理？
- 什么情况新增独立 adapter？
- 配置能力矩阵会不会太弱？

**回答注意：**
要强调当前是轻量治理，不是宣称所有 provider native 能力都已覆盖。

### Q33: 为什么请求边界校验放在 provider 调用前？

**考察点：**
面试官想看你是否关注成本、安全和可诊断性。

**简短回答：**
请求边界校验可以在消耗上游额度前拒绝明显错误，包括空 messages、非法 role、超大 body、过多 tools，以及缺失、为零或超过全局上限的 `max_tokens`。这样错误归因明确，也能降低滥用风险。

**深入展开：**
`validate_message_request` 校验 model 长度、messages 数量、messages JSON 大小、system JSON 大小、tools 数量和 tools JSON 大小。它还要求 `max_tokens > 0` 且不超过 `MODELPORT_MAX_OUTPUT_TOKENS`，限制 role 只能是 `user/assistant`，content 必须是 string 或 array。没有这些限制，大请求会消耗内存和上游资源，错误也会变成 provider 差异化响应。

**代码/文档依据：**
- `src/routes/client_api.rs`
- `src/routes.rs`
- `src/tool_use.rs`

**可能追问：**
- 为什么 role 只允许 user/assistant？
- 这些限制怎么配置？
- 是否会影响兼容性？

**回答注意：**
要说明这是本地网关的安全边界，不是完整复刻所有 provider 可能接受的输入。

### Q34: OpenAI-compatible 非 SSE 响应怎么处理？

**考察点：**
面试官想看你是否考虑上游响应不符合预期的情况。

**简短回答：**
SSE 握手必须得到非 204 的 2xx 和 `text/event-stream`。缺失或错误 Content-Type 会在下游 HTTP 200 前失败；错误 body 会受字节上限、总读取 timeout 和 idle timeout 约束，再做 secret redaction。

**深入展开：**
很多 provider 在错误时不会按 SSE 返回，而是返回 JSON 或纯文本错误。`post_json_sse` 在握手阶段检查状态和 media type，并用 `response_body_with_timeouts` 有界读取错误体，避免 slow-drip 永久占用连接。握手后的 native Anthropic 流必须出现 `message_stop`，OpenAI-compatible 流必须出现 `[DONE]` 或 `finish_reason`；缺少终止信号会在已有 HTTP 200 上发 `event: error`，而不是把半截流当成功。

**代码/文档依据：**
- `src/http.rs`
- `src/providers/openai_stream.rs`

**可能追问：**
- stream idle timeout 怎么处理？
- 错误 body 多大时截断？
- 如何区分正常 keepalive comment 和非 SSE body？

**回答注意：**
不要忽略上游错误响应形态。真实 provider 经常在错误时不遵守成功流协议。

### Q35: 协议层如何避免把项目讲窄成某一个 provider？

**考察点：**
面试官想看你是否能抽象项目价值。

**简短回答：**
我会把 DeepSeek 讲成默认配置样例，而不是已验证结果或能力边界。ModelPort 的协议层按 provider protocol 和 capability 工作，核心是 Anthropic-compatible 客户端契约和 OpenAI-compatible adapter，而不是绑定某一个模型。

**深入展开：**
当前 provider 列表和 public model rows 会根据配置展示官方、第三方、本地和自定义来源。路由通过 `AppConfig.resolve`、provider order 和 protocol 选择路径。这样新 provider 的接入重点是配置、模型列表、base URL、安全校验和 acceptance，而不是改业务主链路。

**代码/文档依据：**
- `src/config.rs`
- `src/routes/client_api.rs`
- `docs/PROVIDER_MATRIX.md`
- `docs/PROJECT_GUIDE.md`

**可能追问：**
- 新增 provider 的最小步骤是什么？
- 如何证明 provider 真兼容？
- 本地 runtime 和云 provider 有何差异？

**回答注意：**
不要反复只讲 Claude Code 和 DeepSeek。要讲多协议、多 Provider、本地路由入口。

### Q36: 为什么协议转换和模型路由要分开？

**考察点：**
面试官想看你是否能拆清路由决策和协议适配两个变化轴。

**简短回答：**
模型路由决定请求打到哪个 provider、哪个模型、哪个 credential；协议转换决定请求体和响应体如何适配。二者分开后，新增模型别名或 fallback 不会影响 adapter，新增 OpenAI-compatible provider 也不需要改路由主流程。

**深入展开：**
`messages` handler 先 authenticate、validate、resolve model，再生成 route attempts。每个 attempt 到 `send_message_attempt` 后才根据 `ProviderProtocol` 选择 provider module。如果把路由和协议转换混在一起，fallback、credential pool 和 Provider health 会被 adapter 细节污染，长期很容易变成不可维护的 provider 特例堆。

**代码/文档依据：**
- `src/routes/client_api.rs`
- `src/config.rs`
- `src/providers/anthropic.rs`
- `src/providers/openai_compat.rs`

**可能追问：**
- route attempts 何时生成？
- credential pool 在路由前还是路由后生效？
- provider protocol 配错如何发现？

**回答注意：**
不要把所有逻辑都说成在 adapter 里完成。adapter 只负责协议，路由和治理在入口与控制面。

### Q37: 为什么 OpenAI-compatible 非流式响应也能转换成 Anthropic stream？

**考察点：**
面试官想看你是否理解客户端 stream contract 和上游能力不一致时的兜底。

**简短回答：**
如果客户端请求 stream，但上游或适配路径拿到的是完整非流式响应，ModelPort 可以把完整响应拆成 Anthropic SSE 事件输出。这样客户端仍然得到一致的 `message_start/content_block/message_stop` 生命周期。

**深入展开：**
OpenAI-compatible adapter 会先发送非流式请求，await 完整响应、提取 usage，并调用 `openai_response_to_anthropic` 完成协议转换；只有这些步骤成功后才创建本地 SSE response。`openai_complete_to_anthropic_stream` 本身只负责把已经转换好的 Anthropic message 逐个 content block 分块：文本按稳定 chunk 输出，tool_use 用 `input_json_delta` 输出完整 input。这样上游 HTTP 或转换错误仍可在响应头前失败/fallback，usage header 也让 request accounting 使用上游 token；代价是必须等待完整生成才有首字节，而且下游取消时上游生成早已结束。

**代码/文档依据：**
- `src/providers/openai_stream.rs`
- `src/types.rs`

**可能追问：**
- 这和真实 streaming 有什么差异？
- 是否会影响 token usage？
- 工具调用 block 如何输出？

**回答注意：**
要讲清这是兼容兜底，不要把它说成和真实上游 streaming 完全一样。

### Q38: 协议层如何处理 usage 和 cost？

**考察点：**
面试官想看你是否关注网关的治理能力，而不只是请求响应。

**简短回答：**
ModelPort 会先基于请求估算 usage，用于 quota 检查；非流式上游返回后如果本地 response header 带有解析出的 usage，会用它覆盖估算并写入 metrics 和 usage log。日志用 `billingMode=upstream-returned` 和 `local-estimate` 明确 provenance。普通 live stream 当前通常仍使用请求估算；`buffer_stream_text=true` 会先完成非流式上游并把 reported usage 放进内部 response header，因此能覆盖估算，但仍不代表精确账单或下游交付对账。

**深入展开：**
`estimate_usage` 用完整请求 JSON 的字符数粗略估算 input tokens，并用 `max_tokens` 估算 output tokens。非流式调用以及 buffered stream 在上游完整返回后，`pricing::usage_from_headers` 如果能解析到 Provider usage，就替换 estimate。普通 live stream 的最终 usage 尚未回灌；即使有 returned usage，本地价格表也决定这些值仍不能当账单。只有 `last_sent` 存在，也就是实际发起过上游尝试，才会增加 quota 和 spend ledger；attempt-level preflight 进入 recorder 时是零 usage，早期 ingress 拒绝可能没有持久行，两者都不收费。

**代码/文档依据：**
- `src/routes/client_api.rs`
- `src/pricing.rs`
- `src/control.rs`
- `src/metrics.rs`

**可能追问：**
- 请求前估算不准怎么办？
- provider 不返回 usage 怎么办？
- cache read/write tokens 如何记录？

**回答注意：**
不要宣称所有 Provider 都能精确计费。当前是估算和部分非流式 returned usage 的结合。

## C. 安全与鉴权深度问题

### Q39: legacy token 和 API Key 为什么要并存？

**考察点：**
面试官想看你是否能平衡兼容性和安全演进。

**简短回答：**
legacy token 适合本地快速接入和兼容旧客户端；API Key 适合控制面管理、绑定用户、team、quota、policy 和 IP allowlist。ModelPort 默认保留 legacy 兼容，但可以通过 `MODELPORT_REQUIRE_CONTROL_API_KEYS` 要求使用控制面 API Key。

**深入展开：**
如果一开始移除 legacy token，会提高迁移成本；如果只保留 legacy token，就无法做精细权限和用量归属。创建 API Key 时服务端要求 owner 是真实 active 用户并覆盖 canonical username；每次数据面鉴权还会联查 owner 仍存在且 active。`authenticate_client` 先尝试 control store 的 API Key，再根据 security policy 决定是否允许 legacy router token。这个过渡设计适合个人和小团队逐步升级安全模型。

**代码/文档依据：**
- `src/routes.rs`
- `src/control.rs`
- `src/config.rs`

**可能追问：**
- 什么时候应该关闭 legacy token？
- API Key 如何绑定 quota 和 team？
- legacy 请求如何记录身份？

**回答注意：**
不要把 legacy token 说成长期最佳实践。它是兼容通道，生产化应逐步切到 API Key。

### Q40: Admin Session 和数据面 API Key 为什么要分开？

**考察点：**
面试官想看你是否理解控制面和数据面的权限隔离。

**简短回答：**
数据面 API Key 用于模型调用，控制面 Admin Session 用于管理用户、Provider、模型、Key 和日志。两者分开可以避免调用凭证被滥用为管理凭证，也能让控制台写操作使用 CSRF 和 Origin Guard。

**深入展开：**
`require_admin_user`、`require_console_user`、`require_api_key_writer` 分别表达不同控制面权限。admin 能创建和管理完整 Key policy；普通 user 只能读取自己的 Key、修改 name/group、revoke 或 delete，不能 create/restore/改 policy；viewer 只读。数据面 `authenticate_client` 只返回 `ClientIdentity`，不会授予管理操作权限。如果两者混用，泄露一个调用 key 就可能修改 provider，这是本地网关尤其要避免的横向权限扩大。

**代码/文档依据：**
- `src/routes.rs`
- `src/auth.rs`
- `src/control.rs`

**可能追问：**
- 普通 user 和 admin 有什么区别？
- API key owner 能不能管理自己的 key？
- 控制台读写权限如何拆分？

**回答注意：**
不要说 “登录了就都能做”。要强调 role、owner 和控制台写保护。

### Q41: 为什么 Dashboard 写操作需要 CSRF？

**考察点：**
面试官想看你是否理解 session cookie 场景的 Web 安全风险。

**简短回答：**
Dashboard 使用 session cookie，浏览器会自动带 cookie。没有 CSRF 防护时，用户登录控制台后可能被其他网页诱导发起管理写操作。ModelPort 要求写操作带 `x-modelport-csrf`，并校验 Origin/Referer。

**深入展开：**
`require_console_write_protection` 先检查 CSRF header，再调用 `validate_admin_request_origin`。Origin 可以是同源，或者在 `MODELPORT_ALLOWED_ORIGINS` 中显式配置；loopback dev host 也有兼容判断。这个设计不追求企业 SSO，但能覆盖本地控制台最关键的 CSRF 风险。

**代码/文档依据：**
- `src/routes.rs`
- `src/auth.rs`

**可能追问：**
- 为什么 GET 不要求 CSRF？
- Origin 缺失怎么处理？
- 为什么提供 `MODELPORT_DISABLE_CSRF`？

**回答注意：**
不要把 CSRF 当作 API Key 场景的问题。它主要保护 cookie session 控制面。

### Q42: Team Policy 怎么限制 provider 和 model？

**考察点：**
面试官想看你是否能解释小团队权限模型。

**简短回答：**
Team Policy 可以限制允许的 provider 和 model，支持精确匹配和通配前缀。请求进入后，API Key 的身份和 team 信息会参与 quota 和 policy 检查，避免所有 key 都能调用所有模型。

**深入展开：**
`policy.rs` 提供 `enforce_model_policy` 和 `enforce_provider_policy`，既检查 requested model，也检查 resolved model，防止用户通过 alias 绕过模型限制。通配符支持 `*` 和后缀 `*` 的前缀匹配，足够覆盖小团队常见的 provider/model 分组，而不引入复杂 ABAC/RBAC 系统。只要任意 active 或 revoked API Key 仍引用 team，删除 team 就返回 400，要求先 reassign/delete Key，避免删除动作静默解除 policy、扩大权限。

**代码/文档依据：**
- `src/policy.rs`
- `src/control.rs`
- `src/routes/client_api.rs`

**可能追问：**
- alias 会不会绕过 policy？
- 为什么不用复杂 RBAC？
- provider policy 和 model policy 谁先检查？

**回答注意：**
不要宣称这是企业级权限系统。它是小团队场景下的轻量策略。

### Q43: IP Allowlist 如何设计，为什么支持 CIDR？

**考察点：**
面试官想看你是否考虑 API Key 泄露后的收敛手段。

**简短回答：**
IP Allowlist 用于限制 API Key 的来源地址，支持精确 IP 和 CIDR。这样小团队可以限制到本机、内网网段或固定出口，降低 key 泄露后的可利用范围。

**深入展开：**
`normalize_ip_rules` 会校验 IP 或 CIDR 格式，`enforce_ip_policy` 会在请求时解析 client IP，并检查是否命中规则。client IP 的来源会结合 trusted proxies，只有 peer 是可信代理时才使用 `x-forwarded-for` 等头，避免客户端伪造来源地址。

**代码/文档依据：**
- `src/policy.rs`
- `src/routes.rs`
- `src/routes/client_api.rs`

**可能追问：**
- 为什么不能直接信任 `x-forwarded-for`？
- IPv6 怎么处理？
- 没有 client IP 时怎么办？

**回答注意：**
要强调 trusted proxy，否则 IP allowlist 很容易被伪造请求头绕过。

### Q44: Provider URL SSRF Guard 防什么？

**考察点：**
面试官想看你是否理解可配置 URL 带来的服务端请求伪造风险。

**简短回答：**
Provider base URL 是可配置项，如果不限制，攻击者可能把它直接配置成 metadata IP、本机管理端口或内网字面量地址。ModelPort 在请求前校验 scheme、userinfo、fragment 和危险的字面量 IP；非 local/custom Provider 还默认强制 HTTPS。`MODELPORT_ALLOW_INSECURE_PROVIDER_HTTP=1` 只适合可信内网，因为 HTTP 会明文暴露 Provider key、prompt 和 response。当前不会解析域名后再次阻止私网地址，所以仍需出站网络策略。

**深入展开：**
`send_message_attempt` 每次请求前调用 `validate_provider_base_url_for_request`，Provider credential base URL 写入时也会校验。配置项 `MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS` 是显式逃生口，适合本地 runtime 场景，但默认应保持保守。这个设计把 SSRF 防线放在 provider 调用边界，而不是只靠 UI 表单。

**代码/文档依据：**
- `src/routes/client_api.rs`
- `src/config.rs`
- `src/provider_credentials.rs`

**可能追问：**
- 本地模型 provider 怎么办？
- 为什么写入时和请求时都校验？
- metadata IP 包括哪些风险？

**回答注意：**
不要说 “本地项目无所谓 SSRF”。本地网关能访问开发机和内网，更需要守住 URL 边界。

### Q45: 为什么要禁止 URL userinfo？

**考察点：**
面试官想看你是否关注 URL 解析歧义和凭证泄露。

**简短回答：**
URL userinfo 可能把账号密码藏在 URL 中，也可能造成 host 审查歧义。ModelPort 禁止 provider URL 包含 userinfo，避免凭证出现在配置、日志和错误中。

**深入展开：**
例如 `https://user:pass@example.com` 这种 URL 会让 secret 混在 endpoint 中。更糟的是，复杂 URL 可能让人工审查误判实际 host。Provider API Key 应通过环境变量名管理，而不是塞到 URL。Provider partial update 还用 `clearApiKeyEnv=true` 明确表达清空；省略 `apiKeyEnv` 表示保留，同时发送非空值和 clear flag 会被拒绝。这个边界也和 secret redaction 互补，减少敏感信息进入系统的机会。

**代码/文档依据：**
- `src/config.rs`
- `src/provider_credentials.rs`
- `src/http.rs`

**可能追问：**
- 如果 provider 需要 basic auth 怎么办？
- userinfo 禁止会影响兼容性吗？
- 是否会记录 base URL？

**回答注意：**
要强调凭证路径统一使用 env，而不是把 secret 放 URL。

### Q46: 为什么要禁止 metadata IP？

**考察点：**
面试官想看你是否理解云环境和本地容器环境的特殊风险。

**简短回答：**
metadata IP 可能暴露云实例凭证、容器环境信息或内部服务信息。Provider URL 如果能指向这些地址，网关就可能成为 SSRF 跳板。

**深入展开：**
即使 ModelPort 当前面向本地和小团队，也可能运行在云主机或容器里。攻击者如果能配置 provider URL，就可能请求 `169.254.169.254` 等 metadata endpoint。ModelPort 的 URL policy 默认阻断这类地址，只有显式 private provider allowance 才允许本地 runtime 场景。

**代码/文档依据：**
- `src/config.rs`
- `src/provider_credentials.rs`

**可能追问：**
- 私有 IP 和 loopback 是否都禁止？
- Docker Compose 下访问 host 怎么处理？
- allow_private_provider_urls 有什么风险？

**回答注意：**
不要把 allow_private_provider_urls 当默认建议。它是本地模型场景的受控开关。

### Q47: Secret Redaction 做到哪些层面？

**考察点：**
面试官想看你是否能把安全和可观测性结合起来。

**简短回答：**
ModelPort 会在上游错误体和诊断输出中脱敏敏感字段，包括 `api_key`、authorization、access token、secret、password、credential，以及纯文本里的 `Bearer`、`sk-`、`sk_m` 片段。

**深入展开：**
`sanitize_error_text` 会优先尝试把错误体解析为 JSON 并递归脱敏敏感 key；如果不是 JSON，则做文本片段脱敏。错误体还会按字符数截断，防止日志过大。这样既能保留排障信息，又降低 key 泄露风险。

**代码/文档依据：**
- `src/http.rs`
- `src/storage.rs`
- `src/routes/logs_view.rs`

**可能追问：**
- 如果 secret 不符合 sk- 格式怎么办？
- redaction 会不会误伤普通文本？
- 日志里是否保存 provider raw body？

**回答注意：**
不要说 redaction 能覆盖所有 secret 模式。它覆盖常见 key 和 marker，仍要避免把 secret 放到不该出现的位置。

### Q48: CSRF / Origin 写保护如何保护控制面？

**考察点：**
面试官想看你是否理解浏览器侧攻击面。

**简短回答：**
控制面写操作会检查 CSRF header 和 Origin/Referer，允许同源或配置过的 origin，本地 loopback dev port 也有兼容。这样可以降低非可信网页直接驱动已登录用户控制台的风险。它不是通用 CORS，也不是鉴权替代品。

**深入展开：**
`validate_admin_request_origin` 会从 `origin` 或 `referer` 解析 host，与请求 host 做同源匹配，或与 `MODELPORT_ALLOWED_ORIGINS` 比较。`console_host_matches` 对 loopback hostname 做了开发友好处理。Origin/Referer 缺失时允许非浏览器请求；后端也不返回通用 CORS header。因此正确部署方式仍是 Dashboard/API 同源反代。

**代码/文档依据：**
- `src/routes.rs`
- `src/auth.rs`

**可能追问：**
- Origin 缺失为什么允许？
- 是否有 SameSite cookie？
- 为什么 loopback 特殊处理？

**回答注意：**
要把它讲成控制面写保护的一部分，不要单独夸成完整 Web 安全体系。

### Q49: 请求体大小限制为什么要放在网关层？

**考察点：**
面试官想看你是否关注资源消耗和 DoS 风险。

**简短回答：**
大模型请求可能包含很多 messages、system blocks 和 tools schema。如果不限制，攻击者或误用方可以用超大 JSON 消耗内存和 CPU。ModelPort 在 Axum 层和业务校验层都设置了限制。

**深入展开：**
路由层有 `DefaultBodyLimit::max(max_request_body_bytes)` 和 `ConcurrencyLimitLayer`；业务层 `validate_message_request` 又限制 messages 数量、JSON 字符数、system JSON 字符数、tools 数量和 tools JSON 字符数。这是两层保护：一层防止 HTTP body 过大，一层给协议字段更清晰的错误。

**代码/文档依据：**
- `src/routes.rs`
- `src/routes/client_api.rs`
- `src/config.rs`

**可能追问：**
- body limit 和 messages JSON limit 有什么区别？
- 限制值怎么配置？
- 太严格会不会影响大上下文请求？

**回答注意：**
要讲这是可配置边界，适合个人和小团队默认安全，不是固定不可调。

### Q50: 错误响应最小暴露怎么做？

**考察点：**
面试官想看你是否知道错误信息既要可诊断又不能泄密。

**简短回答：**
ModelPort 会把内部错误映射成有限类型，并对上游错误体做 redaction 和 truncation。Streaming 场景用 Anthropic error event 返回，避免把内部结构或完整 provider body 直接暴露。

**深入展开：**
上游错误中可能包含 API key、request body 片段、账号信息或 provider 内部错误。`AppError` 到响应的映射控制 HTTP status 和错误类型，`sanitize_error_body` 控制 body 内容。Provider health 和 request logs 保存必要摘要，比如 status code、last error、failure kind，而不是无限制保存原始 body。

**代码/文档依据：**
- `src/error.rs`
- `src/http.rs`
- `src/types.rs`
- `src/control.rs`

**可能追问：**
- 如何平衡排障和脱敏？
- 错误体截断多长？
- Dashboard 是否展示完整错误？

**回答注意：**
不要把上游原始错误完整展示当成 “透明”。真实系统要先考虑 secret 和噪声。

### Q51: 安全 headers 在这个项目里有什么作用？

**考察点：**
面试官想看你是否关注控制台浏览器安全基本面。

**简短回答：**
ModelPort 在响应中添加 `X-Content-Type-Options: nosniff`、`X-Frame-Options: DENY`、`Referrer-Policy: no-referrer` 和 `Permissions-Policy`，减少 MIME sniffing、点击劫持、referrer 泄露和不必要浏览器能力暴露。

**深入展开：**
这些 headers 不是重安全体系，但属于低成本高收益的默认加固。对本地控制台来说，防止被 iframe 嵌入和减少 referrer 泄露尤其有价值。它和 CSRF、Origin Guard、session cookie 共同构成控制面基础防护。

**代码/文档依据：**
- `src/routes.rs`

**可能追问：**
- 为什么不加 CSP？
- Dashboard 由 nginx 提供时是否还需要后端 headers？
- 这些 headers 对 API 客户端有影响吗？

**回答注意：**
不要把安全 headers 夸成完整安全方案。它们是基础加固。

### Q52: trusted proxy 设计为什么重要？

**考察点：**
面试官想看你是否理解反向代理后真实 IP 的可信边界。

**简短回答：**
只有 peer addr 是可信代理时，ModelPort 才会使用 `x-forwarded-for`、`x-real-ip` 或 `cf-connecting-ip` 作为 client IP。否则直接用 TCP peer IP，防止客户端伪造转发头绕过 IP allowlist 和 rate limit。

**深入展开：**
`TrustedProxyConfig` 默认信任 loopback，并可通过 `MODELPORT_TRUSTED_PROXIES` 增加规则。`client_ip` 仅在 peer trusted 时读取 forwarded header，并从连接 peer 开始对 XFF 右向左剥离明确可信的代理 hop，选择第一个不可信地址；不会直接采用攻击者可控的最左值。Docker Nginx 用 `$remote_addr` 覆盖 XFF，并用 `$http_host` 保留 Host 端口，避免 Origin/Host 校验错位。

**代码/文档依据：**
- `src/routes.rs`
- `src/policy.rs`

**可能追问：**
- 默认为什么信任 loopback？
- 多级代理取哪个 IP？
- trusted proxy 配错有什么风险？

**回答注意：**
要明确 forwarded headers 不是天然可信，必须由可信代理边界保护。

### Q53: 登录安全做了哪些基础防护？

**考察点：**
面试官想看你是否关注控制台账号安全，而不只是 API Key。

**简短回答：**
Admin 登录使用 Argon2 存密码哈希，session token 存 hash，有 TTL；失败登录有锁定机制；用户状态必须 active；当前管理员不能把自己降权或禁用。

**深入展开：**
`AuthStore::login` 会 normalize username、验证密码、记录失败次数，超过阈值进入 lockout。session token 生成后只保存 hash，cookie secure 可通过 env 控制。`update_user` 防止当前用户移除自己的 admin access，避免把控制面锁死。这些是小团队控制台最实用的账号安全边界。

**代码/文档依据：**
- `src/auth.rs`
- `src/routes/admin_users.rs`

**可能追问：**
- 为什么用 Argon2？
- session TTL 如何配置？
- 当前管理员为什么不能降权自己？

**回答注意：**
不要宣称已具备企业 IAM。当前是本地控制面账号安全，不是组织级身份体系。

### Q54: 安全能力有哪些当前没有做，触发条件是什么？

**考察点：**
面试官想看你是否能诚实说明边界和演进条件。

**简短回答：**
当前没有默认引入 OIDC、企业 SSO、复杂多租户、集中审计平台和服务网格。触发条件是外部组织接入、多团队隔离要求、合规审计要求或多实例部署成为真实需求。

**深入展开：**
ModelPort 当前优先小团队低运维，Admin Session、API Key、Team Policy、IP allowlist、CSRF、SSRF Guard 和 redaction 已覆盖主要风险。如果一开始做 OIDC 和复杂租户，会显著提高配置和运维成本。未来可以按需求增加 OIDC、细粒度 RBAC、审计导出和密钥管理系统集成。

**代码/文档依据：**
- `docs/PROJECT_GUIDE.md`
- `docs/DOCKER.md`
- `src/auth.rs`
- `src/control.rs`

**可能追问：**
- 哪些场景必须上 OIDC？
- 多实例下 session 怎么共享？
- 审计日志如何外部化？

**回答注意：**
不要说 “不需要企业能力”。要说当前阶段不默认引入，条件成熟后再做。

## D. 稳定性深度问题

### Q55: Rate Limit 为什么按多个维度做？

**考察点：**
面试官想看你是否理解流量控制不能只看全局 QPS。

**简短回答：**
ModelPort 的 RateLimiter 支持 global、API key、IP、provider 和 model 多维度窗口。这样既能防止全局打爆，也能限制单个 key、单个来源、单个 provider 或热点模型。

**深入展开：**
`RateLimiter::check` 为每个请求构造多个窗口 key，用 `VecDeque` 存时间戳并按滑动窗口清理。命中限制时返回 `RateLimited`，包含 `retry_after_secs`。Streaming 另有 `MODELPORT_MAX_CONCURRENT_STREAMS` semaphore，默认继承总请求并发；permit 被封装进 response body，直到 body 完成/drop 才释放，耗尽时在上游调用前返回 429。两种限制都是进程内状态，适合单机本地和小团队；多实例共享限流是未来触发 Redis 或数据库中心化限流的条件。

**代码/文档依据：**
- `src/routes.rs`
- `src/routes/client_api.rs`
- `src/error.rs`

**可能追问：**
- 进程内限流多实例怎么办？
- 为什么 provider/model 维度也要限？
- retry-after 如何计算？

**回答注意：**
不要宣称当前限流是分布式的。它是单实例低运维方案。

### Q56: Quota / Budget 和 Rate Limit 有什么本质区别？

**考察点：**
面试官想看你是否能区分瞬时流量控制和长期用量治理。

**简短回答：**
Rate Limit 控制单位时间请求速率，解决瞬时流量和滥用；Quota/Budget 控制一段周期内的 tokens、requests 或 cost，解决长期花费和团队配额。两者都需要，但目标不同。

**深入展开：**
`check_quotas` 会在上游调用前基于估算 usage 检查 API Key policy 和用户 quota；只有真实 upstream attempt 开始后，`record_usage` 才会累加 returned usage 或估算。用户 quota 的 daily/weekly/monthly 是 UTC 自然日、周一开始的自然周、自然月。API Key/Team spend 则是 rolling 5h/24h/7d/30d，`rateLimited` 只是“启用周期费用限额”的兼容字段名，不是请求速率开关。滚动 spend 用小时桶，边界会保守计入最老交叠整小时。check/update 不是事务式预留，并发请求仍可能同时通过。

**代码/文档依据：**
- `src/control.rs`
- `src/routes/client_api.rs`
- `src/policy.rs`

**可能追问：**
- 请求前估算不准会怎样？
- quota 何时 reset？
- daily/monthly budget 如何处理？

**回答注意：**
不要混淆 rate limit 和 quota。一个是短窗口频率，一个是周期预算。

### Q57: Credential Pool 如何提高稳定性？

**考察点：**
面试官想看你是否理解多账号管理和故障切换。

**简短回答：**
Credential Pool 允许同一 provider 配多个上游账号，并支持 `manual`、`failover`、`round_robin` 模式。请求时选择可用 credential，失败时记录账号级健康，必要时自动切到可用账号。

**深入展开：**
`select_provider_credential_locked` 会过滤 disabled、缺少 env key、仍在 cooldown 的 credential。`failover` 优先当前 active，可用时保持稳定；`round_robin` 在可用账号间轮换；`manual` 保留用户选择。自动模式一个 usable credential 都没有时返回 None 并让该 Provider fail closed，再由候选 Provider fallback；不会退回 disabled/cooldown/missing-env 账号。失败后如果模式不是 manual，且 failure kind 可轮转，系统会自动切换并记录 activity。

**代码/文档依据：**
- `src/control.rs`
- `src/provider_credentials.rs`
- `src/provider_status.rs`

**可能追问：**
- manual 模式为什么不自动切？
- 账号 key 为什么只保存 env name？
- round_robin 如何选择下一个？

**回答注意：**
不要说 Credential Pool 一定解决所有上游失败。它解决账号级和部分可恢复失败，provider 整体不可用仍需要 fallback。

### Q58: credential health 和 provider health 为什么要分开？

**考察点：**
面试官想看你是否能区分账号问题和 provider 问题。

**简短回答：**
provider health 描述整个 provider 的状态，credential health 描述某个账号或 key 的状态。分开后，余额不足、账号限流这类问题可以切账号，而 provider 整体不可用则需要 fallback 到其他 provider。

**深入展开：**
`record_provider_outcome_for_credential` 同时更新 provider health 和 credential health。credential health 有 `last_used_at`、成功失败数、连续失败、cooldown 等字段。`provider_in_cooldown` 还会判断如果 provider 在 cooldown 但 credential pool 有可用账号，是否可以继续使用而不是整体跳过 provider。

**代码/文档依据：**
- `src/control.rs`
- `src/control_view.rs`
- `src/provider_status.rs`

**可能追问：**
- provider 在 cooldown 时为什么还能用？
- 账号问题和 provider 问题怎么分类？
- Dashboard 如何展示两层 health？

**回答注意：**
要避免把 provider health 简化成单个布尔状态。它是分类和决策输入。

### Q59: failure kind 分类为什么重要？

**考察点：**
面试官想看你是否能从错误分类推导恢复策略。

**简短回答：**
不同错误需要不同恢复策略。`rate_limit` 适合等待或切账号；`account` 适合检查 key、余额或切账号；`config` 适合检查 env；`upstream_unavailable` 适合切 provider；未知错误需要看日志。

**深入展开：**
`provider_failure_guidance` 根据 status code 和 error text 分类，并返回建议。`should_rotate_provider_credential` 只对 `account/rate_limit/config` 这类可能账号级恢复的失败轮转。这样避免盲目轮转无法解决的问题，比如 provider 整体 503。

**代码/文档依据：**
- `src/provider_status.rs`
- `src/control.rs`

**可能追问：**
- 为什么 401/403 归 account？
- 429 如何处理？
- 500 是否应该轮转 credential？

**回答注意：**
不要说所有失败都重试。分类的价值就是避免盲目重试。

### Q60: cooldown 的设计思路是什么？

**考察点：**
面试官想看你是否理解熔断和降载的轻量实现。

**简短回答：**
Cooldown 用于短时间避开连续失败的 provider 或 credential，减少重复打到故障点。ModelPort 根据连续失败次数、429、5xx 和 account failure 设置不同冷却时长。

**深入展开：**
Provider 只会在 transport、upstream protocol、429、5xx 或明确 account/config 等可熔断 failure kind 上进入 cooldown；普通非重试型 4xx 虽会计入 failure 统计，但不会仅凭连续次数熔断整个 Provider。credential 的 account failure 会使用更长冷却，因为余额或账号权限问题通常不是短时间恢复。`route_attempts` 会跳过 provider cooldown；credential selection 会过滤 credential cooldown。这个设计是轻量熔断，不引入复杂 circuit breaker。

**代码/文档依据：**
- `src/provider_status.rs`
- `src/control.rs`
- `src/routes/client_api.rs`

**可能追问：**
- cooldown 时间怎么定？
- 成功后如何清除 cooldown？
- provider cooldown 和 credential cooldown 冲突怎么办？

**回答注意：**
要讲这是小团队级别的简单有效策略，不是完整服务网格熔断器。

### Q61: fallback 为什么不能盲目重试？

**考察点：**
面试官想看你是否理解重试可能放大故障和成本。

**简短回答：**
盲目重试可能放大上游故障、消耗额度、产生重复工具调用或增加延迟。ModelPort 只对 transport、upstream protocol、429 和 5xx 等可恢复错误尝试 fallback，不对鉴权、权限、请求非法等错误重试。

**深入展开：**
`is_retryable_message_error` 明确区分可重试错误。`route_attempts` 生成 provider order 中可用的候选，跳过 cooldown provider，并确保候选 provider 能匹配 requested model、primary model、model prefix 或 passthrough。重试次数和 fallback_from_provider 会记录到 usage log，方便诊断。

**代码/文档依据：**
- `src/routes/client_api.rs`
- `src/control.rs`

**可能追问：**
- Tool Use 请求 fallback 是否安全？
- 非幂等请求能不能重试？
- fallback 后模型不一致怎么办？

**回答注意：**
要承认 fallback 对模型一致性有影响，所以候选要基于配置和可接受模型匹配，而不是随便切。

### Q62: insufficient_balance 为什么要单独识别？

**考察点：**
面试官想看你是否能把技术错误转成可操作诊断。

**简短回答：**
余额不足不是普通上游失败，它需要充值或切换账号。ModelPort 会优先识别 `insufficient_balance`、英文变体和中文“余额不足”，标记 `rechargeRequired=true`，并展示 `等待充值` badge。

**深入展开：**
如果把余额不足当成普通 500，系统可能不断重试或错误 fallback，用户也不知道该充值。`provider_account_issue` 在 generic account error 前优先检测余额变体，`provider_health_row` 会将它显示成 recharge required。credential health 也会记录账号级问题，便于切账号。

**代码/文档依据：**
- `src/provider_status.rs`
- `src/control_view.rs`
- `src/control.rs`

**可能追问：**
- 为什么 402 有时表现为 500？
- rechargeRequired 和 auth error 有何区别？
- “等待充值”标记是否影响路由？

**回答注意：**
不要把“等待充值”讲成支付系统。当前是诊断和运营状态，不是内置充值平台。

### Q63: rechargeRequired=true 的意义是什么？

**考察点：**
面试官想看你是否能把后端状态和前端可见性联系起来。

**简短回答：**
`rechargeRequired=true` 把 provider account issue 转成控制台可见状态。用户可以在 Dashboard 看到该 provider 或 credential 需要处理余额，而不是只看到请求失败。

**深入展开：**
这个字段是 Provider Health View 的输出，不是单纯日志文本。它让前端展示 badge、推荐操作和健康状态时有结构化依据。后续如果做通知或自动运维，也可以基于这个字段触发提醒。当前它不自动充值，也不替用户决定支付行为。

**代码/文档依据：**
- `src/control_view.rs`
- `src/provider_status.rs`
- `src/routes/dashboard_view.rs`

**可能追问：**
- 它和 failure_kind 什么关系？
- 是否会触发 credential cooldown？
- 如何避免重复活动记录？

**回答注意：**
要说清这是结构化诊断字段，不是商业闭环。

### Q64: Provider Health 如何服务 Dashboard 和路由？

**考察点：**
面试官想看你是否理解健康状态不是只给用户看的。

**简短回答：**
Provider Health 一方面给 Dashboard 展示状态、成功率、错误和建议操作；另一方面参与路由决策，比如 cooldown provider 会被 route attempts 跳过，credential health 会影响 credential selection。

**深入展开：**
`provider_health_rows` 输出控制台需要的结构化状态，`provider_in_cooldown` 用于 `route_attempts`。请求结束后，`record_provider_outcome_for_credential` 立即更新 provider 和 credential 的成功失败、连续失败、last error 和 cooldown。这样观测和治理共享同一份状态，而不是前端单独推断。

**代码/文档依据：**
- `src/control.rs`
- `src/control_view.rs`
- `src/routes/client_api.rs`
- `src/routes/dashboard_view.rs`

**可能追问：**
- provider health 是否持久化？
- 成功请求如何恢复健康？
- health 和 metrics 有什么区别？

**回答注意：**
不要把 health 和 metrics 混为一谈。health 是状态和治理输入，metrics 是聚合观测。

### Q65: 为什么不直接对所有失败做多次重试？

**考察点：**
面试官想看你是否理解重试风暴和成本风险。

**简短回答：**
模型请求成本高、延迟长，而且 Tool Use 可能涉及客户端执行链。对所有失败重试会放大上游压力、增加费用，还可能造成语义重复。ModelPort 只对明确可恢复的错误尝试 fallback。

**深入展开：**
鉴权失败、权限失败、请求非法、quota exceeded 都是确定性错误，重试没有意义。429、5xx、transport 和 upstream protocol error 才更可能通过换 provider 或等待恢复解决。这个策略比 “失败就重试三次” 更保守，也更符合小团队成本控制。

**代码/文档依据：**
- `src/routes/client_api.rs`
- `src/error.rs`
- `src/provider_status.rs`

**可能追问：**
- 是否应该指数退避？
- streaming 请求中途失败能不能重试？
- 重试会不会改变模型输出？

**回答注意：**
要承认 streaming 中途失败自动重试很危险，当前不应轻率承诺。

### Q66: quota 检查为什么在 provider 调用前做？

**考察点：**
面试官想看你是否理解预算门禁和成本控制。

**简短回答：**
Provider 调用会产生真实成本，所以 quota 必须在调用前检查。ModelPort 用估算 usage 先做门禁，请求完成后再记录实际或估算 usage。

**深入展开：**
如果只在请求后扣减，超额请求已经发生，预算控制没有意义。`check_quotas` 会对 API Key policy 和用户 quota 执行检查；`record_usage` 只在确实发送过 upstream attempt 时更新 quota/spend。attempt-level policy/capability/credential/provider-rate 拒绝若进入 recorder 会记零 usage，入口鉴权/结构校验/global rate/stream permit 等更早拒绝可能没有持久行，但都不消耗预算。边界是估算可能不准、并发 check/update 不是 reservation，而且 rolling spend 使用小时桶保守计边界，因此它不是精确计费系统。

**代码/文档依据：**
- `src/control.rs`
- `src/routes/client_api.rs`
- `src/usage.rs`

**可能追问：**
- 估算过高会不会误拒？
- 并发下 quota 是否有竞态？
- 如何支持团队级预算？

**回答注意：**
不要把它宣传成支付级账务。它是小团队调用预算控制。

### Q67: 稳定性后续最值得演进什么？

**考察点：**
面试官想看你是否能给出有触发条件的路线图。

**简短回答：**
短期可以增强 provider acceptance matrix、健康趋势和 fallback 策略解释；中期可以做 credential 权重、长期成功率和更细冷却原因；多实例后再考虑分布式限流和共享状态。

**深入展开：**
当前不引入 Redis 是为了低运维。触发条件是多实例部署、多个网关节点需要共享 rate limit、quota 或 provider health。内部可以先把 rate limiter 抽象成接口，再提供 Redis/Postgres-backed 实现。不要在单机阶段提前引入外部依赖。

**代码/文档依据：**
- `src/routes.rs`
- `src/control.rs`
- `docs/PROJECT_GUIDE.md`

**可能追问：**
- 分布式限流怎么做？
- provider health 多实例怎么合并？
- fallback 策略如何避免震荡？

**回答注意：**
要明确哪些是当前已做，哪些是未来触发条件。不要假装已经有分布式治理。

## E. 可观测性深度问题

### Q68: Request Logs 为什么是网关核心能力？

**考察点：**
面试官想看你是否理解网关的排障入口。

**简短回答：**
网关夹在客户端和 provider 中间，失败时用户首先需要知道是谁、什么时候、调用了哪个模型、走了哪个 provider、花了多久、消耗多少、是否 retry/fallback、错误是什么。Request Logs 把这些信息结构化记录下来。

**深入展开：**
`record_usage` 写入 usage log，字段包括 identity、api key、team、model、resolved model、provider、protocol、stream、status、tokens、cost、`billingMode`、latency、retry_count、fallback_from_provider、client_ip 和 error_message。`billingMode` 区分 `upstream-returned` 与 `local-estimate`；attempt-level preflight 行是零 usage，更早的 ingress 拒绝可能只出现在 route metrics/错误响应而没有 usage row。Dashboard Logs 页面基于这些数据展示调用明细。没有这层日志，排障只能靠散落的服务日志。

**代码/文档依据：**
- `src/control.rs`
- `src/routes/client_api.rs`
- `src/routes/logs_view.rs`

**可能追问：**
- 日志是否会泄露 secret？
- usage log 和 tracing log 有什么区别？
- 日志保留策略是什么？

**回答注意：**
不要只说 “有日志”。要说清楚记录字段和排障路径。

### Q69: Request ID 在请求链路中起什么作用？

**考察点：**
面试官想看你是否会设计跨层关联。

**简短回答：**
Request ID 让客户端响应、ModelPort tracing 日志和 Dashboard usage record 可以关联。ModelPort 使用 `x-request-id`，没有时由中间件生成并在响应与请求记录中保留。

**深入展开：**
`routes.rs` 使用 `SetRequestIdLayer` 和 `PropagateRequestIdLayer`，message handler 与 usage record 保存该值。两条内置协议 adapter 还会把它转发给上游，排障时可以用用户提供的 request ID 定位本服务日志、Dashboard 记录，并在上游支持时继续关联。上游仍可能忽略或替换它；它也没有 span/trace parent 语义，不是完整分布式 trace。

**代码/文档依据：**
- `src/routes.rs`
- `src/routes/client_api.rs`

**可能追问：**
- request id 是否传给上游？
- Dashboard 是否显示 request id？
- 分布式 trace 如何演进？

**回答注意：**
不要宣称当前已经是完整 OpenTelemetry tracing。当前只是 ModelPort 范围内的 request-ID 关联。

### Q70: Prometheus Metrics 当前记录哪些信息？

**考察点：**
面试官想看你是否能区分日志和指标。

**简短回答：**
Metrics 记录服务 uptime、route 请求数/成功数/失败数/耗时，以及按 provider/model/stream 维度的 message 请求数、成功失败、耗时、tokens、cache tokens 和 cost estimate。

**深入展开：**
`Metrics` 是内存计数器，`/metrics` 以 Prometheus text format 输出，并要求客户端鉴权。它适合单机或小团队做轻量监控。长期如果要多实例聚合，可以让 Prometheus scrape 多实例，或者把 metrics 和 usage log 分别接入外部系统。

**代码/文档依据：**
- `src/metrics.rs`
- `src/routes/ops.rs`

**可能追问：**
- 重启后 metrics 会丢吗？
- metrics 和 usage log 为什么都需要？
- metrics endpoint 为什么要鉴权？

**回答注意：**
要承认当前 metrics 是内存级，不是长期账务存储。

### Q71: `readyz / livez` 为什么要分开？

**考察点：**
面试官想看你是否理解存活探测和就绪探测的区别。

**简短回答：**
`livez` 只说明进程能响应；`readyz` 需要鉴权并实际读取 auth/control storage，成功后返回 provider health、storage path 和 providers 等诊断信息。存储读失败会返回 not-ready，但某个 Provider 降级不会，所以它是存储 readiness 而不是全 Provider gate。

**深入展开：**
`health` 在未鉴权时默认只返回 minimal body，除非开启 detailed public health。这样避免公开暴露 provider 和存储信息。`readyz` 要求 `authenticate_client` 并检查持久层可读性，适合 smoke/K8s 带 header 的存储 readiness，但不主动探测上游 Provider。

**代码/文档依据：**
- `src/routes/ops.rs`
- `src/routes.rs`
- `scripts/smoke-test.sh`

**可能追问：**
- health 和 readyz 有什么区别？
- 为什么 readyz 需要鉴权？
- Kubernetes 场景会怎么配置？

**回答注意：**
不要把 detailed health 默认公开。当前默认是最小暴露。

### Q72: Provider Health Dashboard 应该怎么看？

**考察点：**
面试官想看你是否能把后端状态转成用户可理解诊断。

**简短回答：**
Provider Health Dashboard 展示 provider 是否 healthy/degraded/cooldown、成功率、连续失败、最后错误、账户问题、充值标记和推荐操作。它把底层错误分类转成用户能行动的信息。

**深入展开：**
Provider Health 来自 control store 的 provider health record，Dashboard view 会结合 runtime status 和 persisted usage 生成页面数据。余额不足会保留 `rechargeBadge`，cooldown 会展示状态。它不是单纯红绿灯，而是包含行动建议的诊断面板。

**代码/文档依据：**
- `src/control_view.rs`
- `src/routes/dashboard_view.rs`
- `src/provider_status.rs`

**可能追问：**
- success rate 如何计算？
- cooldown 状态如何产生？
- provider health 和 credential health 如何同时展示？

**回答注意：**
不要把 Dashboard 状态当成纯前端计算。后端输出结构化健康字段。

### Q73: token / cost / latency 为什么要同时记录？

**考察点：**
面试官想看你是否理解模型网关的运营指标。

**简短回答：**
tokens 反映模型消耗，cost 反映预算，latency 反映体验。三者一起看，才能判断某个 provider 是慢、贵，还是请求本身上下文太大。

**深入展开：**
`record_message` 记录 tokens 和 cost estimate 到 metrics，`record_usage` 记录到 request log。非流式响应和 buffered stream 尽量使用解析出的 Provider usage；普通 live stream 通常仍是请求估算，且当前 duration/success 更接近 stream 接受阶段。Dashboard 的范围趋势由服务端对选定窗口内全部 retained usage 做聚合，不是拿当前日志页拼图；response 还会标记 persisted/metrics-estimate/empty 来源、是否估算以及 retention 是否已达上限。它可以展示运营趋势，但不能把这些值当最终账单或完整生成 SLO。

**代码/文档依据：**
- `src/routes/client_api.rs`
- `src/metrics.rs`
- `src/control.rs`
- `src/pricing.rs`

**可能追问：**
- cache tokens 为什么单独记录？
- provider 不返回 usage 怎么办？
- cost estimate 准确吗？

**回答注意：**
不要把 cost estimate 讲成精确账单。它是运营估算和预算控制依据。

### Q74: retry / fallback 记录有什么价值？

**考察点：**
面试官想看你是否能把恢复行为可视化。

**简短回答：**
如果请求成功但经历了 retry/fallback，用户应该知道它不是一次干净成功。ModelPort 记录 `retry_count` 和 `fallback_from_provider`，便于发现 provider 不稳定或 fallback 策略频繁触发。

**深入展开：**
没有 retry/fallback 记录，用户只看到成功，长期会误判系统健康。`messages` handler 在 attempt index 大于 0 时增加 retry count，并记录 first provider 作为 fallback source。usage log 保存这些字段，Logs 页面可展示。后续可以基于这些字段做 fallback 活动趋势和告警。

**代码/文档依据：**
- `src/routes/client_api.rs`
- `src/control.rs`
- `src/routes/logs_view.rs`

**可能追问：**
- fallback 后响应模型不同怎么办？
- retry count 是否包括同 provider credential 切换？
- 如何判断 fallback 过于频繁？

**回答注意：**
不要把成功率单独作为健康判断。成功但频繁 fallback 也是风险。

### Q75: 诊断脚本在项目里怎么定位？

**考察点：**
面试官想看你是否重视可操作验证，而不是只写文档。

**简短回答：**
诊断脚本是本地可重复验证入口，包括配置校验、smoke test、Tool Use acceptance 和 provider matrix。它们把手工排查步骤固化下来，降低小团队使用成本。

**深入展开：**
`scripts/config-validate.sh` 调用后端配置校验，`scripts/smoke-test.sh` 检查 liveness、authenticated readiness 和模型列表，`scripts/tool-use-acceptance.sh` 验证 Tool Use 端到端。默认 smoke test 不跑真实上游生成，避免消耗额度；需要真实 provider 认证时再显式加 upstream 模式。

**代码/文档依据：**
- `scripts/config-validate.sh`
- `scripts/smoke-test.sh`
- `scripts/tool-use-acceptance.sh`
- `docs/ACCEPTANCE.md`

**可能追问：**
- smoke test 为什么不覆盖真实生成？
- acceptance 如何接入 CI？
- provider matrix 记录什么？

**回答注意：**
不要把脚本当成附属品。对本地网关来说，诊断脚本就是产品可靠性的一部分。

### Q76: smoke test 如何证明服务可用？

**考察点：**
面试官想看你是否理解 smoke test 的范围和边界。

**简短回答：**
smoke test 检查 `/livez`、认证的 `/readyz` 和 `/v1/models`，能证明服务进程、鉴权、配置加载、Provider 列表和控制面存储基本可用。

**深入展开：**
它不默认证明真实上游生成成功，因为那会消耗额度并受账号状态影响。比如当前某 provider 余额不足时，readyz 仍可返回整体服务健康，同时展示 provider degraded 和 rechargeRequired。这个边界很重要：smoke test 证明网关基础链路，不等于所有上游账号都正常。

**代码/文档依据：**
- `scripts/smoke-test.sh`
- `src/routes/ops.rs`
- `src/routes/client_api.rs`

**可能追问：**
- provider degraded 时 smoke test 是否失败？
- 如何测试真实上游？
- health body 中 secret 是否脱敏？

**回答注意：**
要清楚区分网关健康和上游账号健康。

### Q77: 可观测性后续怎么演进？

**考察点：**
面试官想看你是否知道从单机观测到团队观测的路径。

**简短回答：**
短期增强日志筛选、provider health 趋势和错误分布；中期增加导出和告警；多实例后再接 OpenTelemetry、集中日志或外部 metrics 存储。

**深入展开：**
当前内存 metrics 和持久 usage log 已能覆盖本地和小团队。后续如果部署到多节点，需要外部 scrape、集中日志和 trace correlation。演进要基于真实运维需求，而不是一开始上复杂 observability stack。

**代码/文档依据：**
- `src/metrics.rs`
- `src/control.rs`
- `docs/PROJECT_GUIDE.md`

**可能追问：**
- OpenTelemetry 怎么接？
- usage log 如何导出？
- 多实例 metrics 如何聚合？

**回答注意：**
不要宣称当前已具备完整分布式追踪。讲清现状和触发条件。

### Q78: 如何用观测数据定位一次 Tool Use 失败？

**考察点：**
面试官想看你能否跨模块排障。

**简短回答：**
先用 request id 找 usage log，确认 provider/model/status/error/retry/fallback；再看是否入口校验失败、provider capability 拒绝、上游错误还是 streaming 解析错误；最后用 Tool Use acceptance 复现协议边界。

**深入展开：**
如果错误是 invalid request，多半看 `tool_use.rs` 校验；如果是 upstream protocol，看 `http.rs` 和 `openai_stream.rs`；如果是 provider account/rate_limit，看 Provider Health；如果只是输出重复，看 streaming dedup 配置和 `text_delta` 行为。这样排障是分层的，不是盲目看所有日志。

**代码/文档依据：**
- `src/tool_use.rs`
- `src/providers/openai_stream.rs`
- `src/http.rs`
- `src/control.rs`
- `scripts/tool-use-acceptance.sh`

**可能追问：**
- 如何区分请求格式错和 provider 不支持？
- 如何复现 streaming 回放？
- 哪些日志字段最有用？

**回答注意：**
不要停在 “看 logs”。要说明从 request log 到模块定位的路径。

## F. 工程化与取舍深度问题

### Q79: 为什么选择 Rust + Axum，而不是 Node/Python？

**考察点：**
面试官想看你是否能把技术选型和网关问题本身联系起来，而不是背语言优缺点。

**简短回答：**
ModelPort 是长连接、流式转发、协议转换和安全边界都比较重的网关，Rust 的类型系统和错误处理很适合把协议状态、配置和失败分类做清楚。Axum 基于 Tower 生态，能比较自然地组合 body limit、concurrency limit、trace、middleware 和路由。

**深入展开：**
这个项目的难点不是普通 CRUD，而是请求在多个层之间流动：鉴权、策略、限流、provider 选择、协议转换、streaming、日志、metrics。Rust 能把 `AppError`、provider config、request type、tool state 等关键结构显式化，减少隐式 `any` 和运行时字段错误。Axum 的优势是够轻，适合本地网关，不需要引入重型框架。取舍是开发速度比 Node/Python 慢一些，但换来协议边界更稳、并发和内存行为更可控。不能说 Node/Python 做不了，只是这个项目更看重网关长期稳定性和类型约束。

**代码/文档依据：**
- `src/main.rs`
- `src/routes.rs`
- `src/types.rs`
- `src/http.rs`

**可能追问：**
- Rust 对 streaming 有什么具体收益？
- Axum 和 Actix 怎么取舍？
- 如果团队 Rust 能力弱怎么办？

**回答注意：**
不要把语言选择讲成信仰。要落到 SSE、协议结构、错误分类和中间件组合这些真实收益。

### Q80: 为什么当前不引入 Redis？

**考察点：**
面试官想看你是否理解 “低运维优先” 和 “未来可演进” 的平衡。

**简短回答：**
当前定位是个人和小团队，本地单实例场景下进程内 rate limit、provider health 和 usage log 已经能覆盖主要需求。Redis 的触发条件是多实例部署，需要共享限流窗口、quota 状态、provider cooldown 或 credential health。

**深入展开：**
如果一开始引入 Redis，会增加部署、备份、故障恢复和配置复杂度，对本地网关用户不划算。ModelPort 当前把 rate limit 放在 `RateLimiter` 中，把 provider/credential health 放在控制面状态里，把 request usage 记录到持久层。这个设计可以先稳定单机体验。未来如果要横向扩容，可以把限流、健康状态和 session store 抽象成可替换后端，再提供 Redis 或 Postgres-backed 实现。

**代码/文档依据：**
- `src/routes.rs`
- `src/control.rs`
- `src/provider_status.rs`
- `src/provider_credentials.rs`

**可能追问：**
- 多实例下当前会有什么问题？
- Redis 引入后哪些状态要共享？
- 为什么不是直接用 Postgres 做限流？

**回答注意：**
不要说 “永远不用 Redis”。正确说法是当前不需要，触发条件是共享状态成为真实瓶颈。

### Q81: 为什么当前不使用 Kubernetes？

**考察点：**
面试官想看你是否避免过度工程化。

**简短回答：**
ModelPort 当前更适合 Docker Compose 或 systemd：部署简单、可复现、维护成本低。Kubernetes 的触发条件是多实例、滚动发布、自动扩缩容、集中配置和团队已有 K8s 运维能力。

**深入展开：**
本地模型网关的主要价值是把协议、路由、鉴权和 provider 管理跑稳，而不是一开始搭平台。如果用 K8s，用户需要理解 ingress、secret、PVC、health probe、metrics scrape 等一堆额外概念。当前 `docker compose up -d --build` 足够让小团队跑起来。未来若进入 K8s，`/livez` 可作 liveness；带认证 header 的 `/readyz` 可作存储 readiness，但上游 Provider 是否必须健康应由单独策略决定。

**代码/文档依据：**
- `docker-compose.yml`
- `deploy/`
- `src/routes/ops.rs`
- `scripts/smoke-test.sh`

**可能追问：**
- readyz 在 K8s 里该不该依赖所有 provider？
- Compose 到 K8s 迁移需要改哪些配置？
- 多实例下 sticky session 是否必要？

**回答注意：**
不要把不用 K8s 说成技术能力不足。重点是项目阶段、用户画像和运维成本。

### Q82: 为什么当前不接 OIDC/SSO？

**考察点：**
面试官想看你是否能区分本地控制台身份和企业身份体系。

**简短回答：**
当前 Admin Session、Argon2 密码、CSRF、Origin Guard 和控制面 API Key 已能覆盖小团队的基础访问控制。OIDC/SSO 的触发条件是接入企业组织、统一身份、离职回收、合规审计和集中权限管理。

**深入展开：**
ModelPort 的控制面不是公网 SaaS 管理平台，默认更接近本地部署的 Admin Console。如果一开始接 OIDC，会引入 issuer、client secret、redirect URI、token validation、group claims、logout、session sync 等复杂度。现阶段更应该把基础安全做好：会话安全、CSRF、防暴力登录、控制面和数据面隔离、API Key policy。未来可以在 `auth.rs` 外侧增加 identity provider adapter，把 OIDC claim 映射到本地用户/team。

**代码/文档依据：**
- `src/auth.rs`
- `src/routes.rs`
- `src/policy.rs`
- `src/routes/ops.rs`

**可能追问：**
- OIDC 接入后如何映射 team？
- 当前 session 存储能否支撑多实例？
- 控制面 API Key 和 OIDC 的关系是什么？

**回答注意：**
不要宣称当前已经具备企业 SSO。要把未来触发条件讲清楚。

### Q83: 为什么不做复杂多租户？

**考察点：**
面试官想看你是否理解产品定位和数据隔离成本。

**简短回答：**
当前 ModelPort 用用户、team、policy、quota 和 API Key 满足小团队治理，不做 SaaS 级复杂多租户。触发条件是多个外部组织共用同一网关实例，并要求数据、账单、密钥、审计完全隔离。

**深入展开：**
复杂多租户不只是数据库多一个 `tenant_id`，还涉及密钥隔离、日志隔离、RBAC、审计、provider 配额归属、跨租户访问测试和迁移策略。对个人和初创团队来说，这些成本很高。当前的正确边界是：一个团队或小组织内部做访问控制和预算治理。未来如果要提供托管版或多组织共享实例，再引入 tenant model 和更严格的隔离。

**代码/文档依据：**
- `src/auth.rs`
- `src/policy.rs`
- `src/control.rs`
- `src/storage.rs`

**可能追问：**
- team policy 和 tenant 有什么区别？
- 如果两个团队共享 provider key 怎么算账？
- 多租户下 request log 如何隔离？

**回答注意：**
不要把 team policy 包装成完整多租户。它是小团队权限治理，不是 SaaS tenant 隔离。

### Q84: 为什么优先 Docker Compose 部署？

**考察点：**
面试官想看你是否能从用户落地角度做架构选择。

**简短回答：**
Docker Compose 对本地网关最友好：后端、数据库、Dashboard 和环境变量可以一次性启动，学习成本低，也便于 smoke test。它能覆盖当前用户最常见的个人机器、小云主机和小团队内网部署。

**深入展开：**
Compose 的价值在于降低第一次跑起来的门槛。ModelPort 需要管理 provider key、管理员账号、数据库和前后端服务，如果要求用户手工配置多个进程，很容易出错。Compose 不是长期唯一方案，但它是 MVP 到小团队稳定使用阶段的合理默认。systemd 适合长期单机守护，K8s 适合规模化多实例，这三者是不同阶段的部署选择。

**代码/文档依据：**
- `docker-compose.yml`
- `.env.example`
- `deploy/`
- `scripts/config-validate.sh`
- `scripts/smoke-test.sh`

**可能追问：**
- Compose 下 secret 怎么管理？
- 数据库备份怎么做？
- systemd 和 Compose 怎么取舍？

**回答注意：**
不要把 Compose 讲成玩具。它是低运维阶段的合适选择，但要承认多实例能力有限。

### Q85: 什么情况下引入内部协议 IR？

**考察点：**
面试官想看你是否知道抽象的触发条件，而不是为了抽象而抽象。

**简短回答：**
当前没有完整 Tool/Message IR，主要靠 Anthropic/OpenAI-compatible adapter 和 capability matrix 管理差异。引入 IR 的触发条件是 provider 数量增长、协议差异变复杂、需要统一 schema transformation、argument repair、tool replay diagnostics 或跨协议测试生成。

**深入展开：**
IR 的好处是把 “客户端协议” 和 “provider native 协议” 解耦，统一表达 message、tool call、tool result、stop reason、usage 和 streaming event。但代价也明显：要维护转换器、兼容性矩阵、测试金字塔和迁移路径。现阶段 `tool_use.rs`、`types.rs` 和 `openai_stream.rs` 的边界还能支撑主要协议。未来如果接入更多 native provider，IR 可以作为中间层逐步引入，而不是一次性替换所有 adapter。

**代码/文档依据：**
- `src/tool_use.rs`
- `src/types.rs`
- `src/providers/openai_stream.rs`
- `docs/TOOL_USE_COMPATIBILITY.md`

**可能追问：**
- IR 第一版应该包含哪些字段？
- Streaming event 是否也要 IR？
- 如何避免 IR 变成最大公约数导致能力丢失？

**回答注意：**
不要说 “没有 IR 是缺陷”。要说当前阶段 adapter 更直接，IR 是 provider 复杂度上来后的演进。

### Q86: 什么情况下引入分布式限流？

**考察点：**
面试官想看你是否能识别单机状态在多实例下的失效点。

**简短回答：**
当 ModelPort 多实例部署，并且 API Key、IP、provider 或 model 的限流必须在节点间共享时，就需要分布式限流。当前进程内限流适合单实例，不适合多个网关节点共同承载同一组客户端。

**深入展开：**
多实例下，如果每个节点各自维护窗口，实际总额度会变成 N 倍，quota 和 provider 保护都会失真。引入分布式限流可以基于 Redis、Postgres 或专门 rate-limit service。工程上应该先抽象限流接口，再把当前内存实现作为默认实现，外部实现作为可选增强。这样不破坏个人用户的低运维体验。

**代码/文档依据：**
- `src/routes.rs`
- `src/control.rs`
- `src/policy.rs`

**可能追问：**
- sliding window 在 Redis 怎么做？
- 多实例 quota 和 rate limit 有什么区别？
- 分布式限流失败时应该 fail open 还是 fail closed？

**回答注意：**
不要把当前进程内限流说成多实例可用。要明确边界。

### Q87: 如何避免 provider 特例不断堆积？

**考察点：**
面试官想看你是否具备长期维护 provider matrix 的意识。

**简短回答：**
核心策略是把 provider 差异放到配置、能力矩阵和 provider adapter 边界中，而不是在路由主流程里堆 `if provider == ...`。协议类别统一走 Anthropic-compatible 或 OpenAI-compatible，个性能力通过 `ToolUseConfig`、max token 字段和 base URL policy 表达。

**深入展开：**
provider 特例一旦散落在路由、鉴权、日志和转换层，就会拖慢每次接入新模型。ModelPort 现在把路由选择放在 `client_api.rs`，provider 状态在 `provider_status.rs`，credential pool 在 `provider_credentials.rs/control.rs`，协议转换在 `types.rs/openai_stream.rs`。未来如果某个 provider native API 差异非常大，应新增 adapter 模块和测试，而不是污染通用路径。

**代码/文档依据：**
- `src/routes/client_api.rs`
- `src/provider_status.rs`
- `src/provider_credentials.rs`
- `src/types.rs`
- `src/tool_use.rs`

**可能追问：**
- capability matrix 如何落到配置？
- provider-specific error 怎么分类？
- 什么时候新增 provider module？

**回答注意：**
不要说 “所有 provider 都完全兼容”。要强调能力矩阵和测试证据。

### Q88: 如何保证协议和 Tool Use 的测试覆盖？

**考察点：**
面试官想看你是否知道高风险逻辑该怎么验证。

**简短回答：**
Tool Use 和协议转换要用单元测试、路由测试、acceptance 脚本和 smoke test 分层覆盖。单元测试覆盖字段映射和校验，acceptance 覆盖端到端协议行为，smoke test 覆盖服务可用性。

**深入展开：**
协议层最怕 “happy path 能跑，边界场景坏掉”。比如 duplicate `tool_result`、arguments 半截 JSON、name/arguments 顺序不稳定、legacy `function_call`、provider 不支持 parallel tool calls，都需要专门测试。`scripts/tool-use-acceptance.sh` 的价值是用 mock upstream 构造真实 provider 不稳定复现的边界。修改服务行为时还要同步 scripts 和文档，避免代码、说明和验收标准脱节。

**代码/文档依据：**
- `src/tool_use.rs`
- `src/types.rs`
- `src/providers/openai_stream.rs`
- `scripts/tool-use-acceptance.sh`
- `scripts/smoke-test.sh`

**可能追问：**
- mock upstream 和真实 upstream 各自覆盖什么？
- 怎么防止 streaming 测试不稳定？
- CI 中哪些测试必须跑？

**回答注意：**
不要只说 “cargo test”。要讲测试分层和具体边界。

### Q89: 长期迭代如何排优先级？

**考察点：**
面试官想看你是否能按阶段治理项目，而不是堆功能。

**简短回答：**
近期优先低风险高收益：Tool Use 兼容性、request logs、provider health、配置校验、Dashboard 可用性。中期提升小团队稳定使用：credential pool、quota、fallback、诊断脚本、更多 provider matrix。长期在真实触发后再做多实例、OIDC、分布式限流和内部 IR。

**深入展开：**
这个项目的原则是先把本地网关最核心链路做可靠：客户端进来、协议正确转换、上游稳定调用、失败可解释、日志可追踪。不要一开始做企业平台，因为那会拖慢核心协议能力。长期路线应该围绕用户增长和 provider 数量演进：当 provider 差异变多，做 IR；当多实例出现，做共享状态；当组织接入，做 OIDC/RBAC；当合规要求出现，做审计导出。

**代码/文档依据：**
- `docs/PROJECT_GUIDE.md`
- `docs/learning/MODELPORT_INTERVIEW_GUIDE.md`
- `src/routes/client_api.rs`
- `src/control.rs`

**可能追问：**
- 当前最优先的一件事是什么？
- 哪些需求应该拒绝？
- 如何判断架构演进时机？

**回答注意：**
不要给一个大而空的路线图。要用 “触发条件” 解释长期能力。

### Q90: 面试官质疑 “这只是个人项目” 时怎么回应？

**考察点：**
面试官想看你是否能证明项目复杂度来自真实工程问题，而不是包装话术。

**简短回答：**
我会承认它不是重企业平台，但它解决的是一个真实网关问题：多协议、多 provider、Tool Use、streaming、安全边界、限流、quota、fallback 和可观测性。项目规模不靠团队人数证明，而靠问题边界、失败处理和验证体系证明。

**深入展开：**
可以拿一次请求链路说明复杂度：入口鉴权和 policy，路由选择和 credential pool，Anthropic/OpenAI-compatible 转换，Tool Use 校验和 streaming 事件归一化，provider health/fallback，usage log/metrics。再强调取舍：没有 Redis/K8s/OIDC 是因为当前定位不需要，不是没想过。等多实例、组织身份、复杂租户成为真实需求时，有明确演进路径。

**代码/文档依据：**
- `src/routes/client_api.rs`
- `src/tool_use.rs`
- `src/providers/openai_stream.rs`
- `src/auth.rs`
- `src/metrics.rs`
- `docs/learning/MODELPORT_INTERVIEW_GUIDE.md`

**可能追问：**
- 哪一块最能体现技术深度？
- 当前最大不足是什么？
- 如果用户量涨 10 倍怎么改？

**回答注意：**
不要硬吹企业级。高级面试更看重边界诚实和工程判断。

### Q91: 当前最明显的技术债是什么？

**考察点：**
面试官想看你是否能坦诚识别项目风险，并给出可落地改进。

**简短回答：**
后端整体模式已经比较清楚，但 `routes.rs` 和部分控制面逻辑仍偏大，长期可以按 middleware、rate limit、CSRF、health、client API 拆得更细。前端如果页面继续增长，也需要把大页面拆成领域组件和数据 hooks。

**深入展开：**
技术债不等于现在必须重构。当前最重要的是保持功能稳定和测试覆盖，避免大范围无关重构。触发拆分的条件是：文件变动频繁冲突、测试难写、模块职责不清或新增功能必须理解太多上下文。后端可以优先把 security middleware、rate limiter、ops routes、client routes 的边界继续清晰化；前端可以把 request logs、provider health、login visual 和 settings 拆成可复用组件。每次拆分都要保证行为测试不变。

**代码/文档依据：**
- `src/routes.rs`
- `src/routes/client_api.rs`
- `src/routes/ops.rs`
- `src/control.rs`
- `dashboard/src/pages/`

**可能追问：**
- 哪个文件应该先拆？
- 怎么避免重构引入回归？
- 如何判断抽象是否过度？

**回答注意：**
不要把技术债讲成项目很乱。要说明已有边界和下一步可控治理。

### Q92: 配置变更为什么必须同步文档、部署和脚本？

**考察点：**
面试官想看你是否理解网关类项目的可运维性。

**简短回答：**
配置就是这个项目的行为边界。新增安全、限流、provider 或协议配置时，如果只改代码，不同步 `.env.example`、deploy env、README 和验证脚本，用户很容易部署出和开发环境不一致的网关。

**深入展开：**
ModelPort 很多能力都由环境变量或 provider 配置控制，比如 request body limit、provider URL policy、auth mode、timeout、stream idle timeout、public health detail 等。配置缺省值必须保守，文档必须说明用途和风险，脚本必须能检查常见错误。这样小团队不需要专职运维也能稳定使用。

**代码/文档依据：**
- `.env.example`
- `deploy/`
- `src/config.rs`
- `scripts/config-validate.sh`
- `README.md`

**可能追问：**
- 新增 env var 的流程是什么？
- 哪些配置必须默认关闭？
- config validate 应该检查什么？

**回答注意：**
不要把配置当成附属工作。网关项目里，错误配置往往就是生产事故。

## 附录 A: 3 分钟项目介绍话术

**开场怎么说：**
ModelPort 是一个面向个人开发者、小团队和初创团队的本地大模型路由网关。它不只是把请求转发到某个模型，而是在本地统一处理多协议适配、Provider 路由、Tool Use、Streaming、鉴权、限流、Quota、Credential Pool、Fallback、Provider Health、Request Logs、Metrics 和 Dashboard。项目定位不是重企业平台，所以我刻意没有一开始引入 Redis、Kubernetes、OIDC 或复杂多租户，而是优先做低运维、高可靠、高安全收益的能力。

**中间怎么展开：**
一次请求进来后，先经过 HTTP body limit、鉴权、IP allowlist、team policy 和 quota 检查；然后按 model/provider 规则做路由，选择可用 credential；接着根据 provider protocol 决定直通 Anthropic-compatible，还是转换到 OpenAI-compatible；上游返回后再把响应、streaming event、usage、cost、latency、retry/fallback 信息记录到日志和 metrics。失败时会按 failure kind 分类，更新 provider/credential health，并决定是否 cooldown 或 fallback。

**Tool Use 怎么重点讲：**
Tool Use 是我会重点讲的一块，因为它最能体现协议层复杂度。ModelPort 不是简单字段替换，而是校验 `tools`、`tool_choice`、`tool_use/tool_result` 因果关系，拒绝重复 `tool_use.id` 和重复 `tool_result`；同时把 Anthropic `tool_use` 和 OpenAI `tool_calls` 双向转换，处理 legacy `function_call`，在 streaming 中处理 `input_json_delta`、半截 JSON、name/arguments 顺序不稳定和 cumulative 参数回放。

**最后怎么收尾：**
这个项目我最想表达的是工程取舍：它没有假装自己一开始就是企业级平台，但它在本地网关真正容易出问题的地方做了治理，包括协议正确性、安全边界、失败分类和可观测性。长期演进上，等 provider 差异变多再引入 IR，等多实例部署再引入分布式限流和共享 health，等组织身份需求明确再接 OIDC。

**面试官追问时怎么切换到代码和测试证据：**
如果追问 Tool Use，我会看 `src/tool_use.rs`、`src/types.rs`、`src/providers/openai_stream.rs` 和 `scripts/tool-use-acceptance.sh`。如果追问安全，我会看 `src/auth.rs`、`src/routes.rs`、`src/policy.rs` 和 `src/http.rs`。如果追问稳定性和观测，我会看 `src/routes/client_api.rs`、`src/control.rs`、`src/provider_status.rs`、`src/metrics.rs` 和 request logs。

## 附录 B: 10 分钟技术深挖路线

**开场怎么说：**
我会先说明 ModelPort 的定位：本地大模型路由网关，核心不是某一个模型，而是把不同客户端协议和不同 provider 能力接到一个可治理的本地入口上。项目面向小团队，所以架构重点是稳定、低运维和边界清晰。

**中间怎么展开：**
第 1-2 分钟讲整体链路：client request、auth/policy、routing、credential pool、provider call、response mapping、usage log/metrics。第 3-5 分钟讲协议层：Anthropic Messages 和 OpenAI Chat 的语义差异、content blocks 到 role messages、stop reason 和 finish reason、SSE event 归一化、max token 字段差异和错误映射。第 6-8 分钟讲稳定性和安全：Provider URL 字面量地址 guard 与 DNS 边界、secret redaction、rate limit、非事务 quota、failure kind、cooldown、fallback 和 rechargeRequired。第 9-10 分钟讲可观测性和取舍：request ID、进程内 metrics、诊断型 readyz，以及为什么 stream 最终对账和分布式能力仍是后续工作。

**Tool Use 怎么重点讲：**
在协议层部分我会把 Tool Use 单独拉出来：先讲请求校验，再讲双向转换，最后讲 streaming。请求校验包括工具定义、`tool_choice`、id 唯一性、pending tool result；双向转换包括 Anthropic `tools/tool_use/tool_result` 到 OpenAI `tools/tool_calls/role=tool`，以及 OpenAI `tool_calls/function_call` 回 Anthropic `tool_use`；streaming 则讲 `input_json_delta`、半截 JSON、arguments 早于 name、非对象 arguments 包装和 provider capability matrix。

**最后怎么收尾：**
我会主动讲当前没有做的东西：没有完整 Tool IR、没有 Redis 分布式限流、没有 Kubernetes、没有 OIDC、没有复杂多租户。原因不是忽略，而是项目阶段不需要；每一项都有触发条件和演进路径。

**面试官追问时怎么切换到代码和测试证据：**
如果对方问 “怎么证明不是纸面设计”，我会切到 `scripts/tool-use-acceptance.sh`、`scripts/smoke-test.sh`、`scripts/config-validate.sh`，以及 `cargo test` 中围绕 tool use、streaming、policy、provider health 的测试。回答时尽量把每个设计点对应到文件，而不是只讲概念。

## 附录 C: 30 分钟系统设计面试讲法

**开场怎么说：**
我会把题目定义成 “设计一个本地大模型路由网关”。需求分三层：数据面要兼容 Anthropic/OpenAI-compatible、支持 streaming 和 Tool Use；控制面要管理 API Key、provider、credential、quota、logs 和 health；非功能需求是安全、低运维、可诊断、适合小团队。

**中间怎么展开：**
前 5 分钟讲需求和边界：支持多客户端、多 provider、本地部署、Dashboard 管理，不一开始做 SaaS 多租户和企业 SSO。接下来 8 分钟讲架构：Axum 路由层、auth/policy、rate limiter、client API、provider adapter、credential pool、control state、metrics 和 dashboard。再用 7 分钟讲请求生命周期：入口校验、模型解析、provider/credential selection、非事务 quota pre-check、字面量 URL guard、upstream call、streaming mapping，以及非流式 usage/health/fallback 闭环。然后讲 CSRF/Origin 写保护、IP allowlist、best-effort redaction、DNS 与 stream 生命周期边界。最后讲 request ID、进程内 metrics、诊断与后续演进。

**Tool Use 怎么重点讲：**
系统设计里 Tool Use 可以作为最有技术含量的深挖点。我会画出四段：客户端传 `tools/tool_choice`，模型返回 assistant `tool_use`，客户端执行工具并返回 user `tool_result`，模型继续生成。然后说明 ModelPort 在这里做三类保证：入口语义校验、跨协议映射、streaming 状态机。重点讲 `validate_tool_turn_references` 的 seen/pending 集合、`disable_parallel_tool_use` 到 `parallel_tool_calls=false`、OpenAI `tool_calls` 到 Anthropic `tool_use`、legacy `function_call`、`input_json_delta`、半截 JSON 和 `_raw_arguments`。

**最后怎么收尾：**
收尾时我会强调架构取舍：当前阶段选择单机友好的内存限流和 Compose 部署，换来低运维；对高风险点做强校验、强日志和脚本验收；等多实例和企业身份需求真实出现，再引入 Redis/OIDC/K8s/IR。这样既展示当前工程深度，也不把未来能力吹成已经完成。

**面试官追问时怎么切换到代码和测试证据：**
如果追问数据面，我会打开 `src/routes/client_api.rs`、`src/types.rs`、`src/providers/openai_stream.rs`。如果追问控制面和权限，我会打开 `src/auth.rs`、`src/policy.rs`、`src/control.rs`。如果追问 provider 稳定性，我会打开 `src/provider_status.rs` 和 `src/provider_credentials.rs`。如果追问验证，我会讲 `scripts/tool-use-acceptance.sh` 覆盖 Tool Use，`scripts/smoke-test.sh` 覆盖服务可用性，`scripts/config-validate.sh` 覆盖配置错误，并说明哪些真实上游测试需要显式启用以避免消耗额度。
