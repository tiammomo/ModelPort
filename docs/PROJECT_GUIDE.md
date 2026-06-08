# ModelPort 项目指导文档

ModelPort 的最佳定位是：面向 Claude Code / VS Code Claude 的本地模型路由适配层。它不是聊天客户端，也不是云端模型平台，而是把本机开发工作流稳定接到不同模型 provider 的轻量网关。

## 一句话定位

ModelPort is a local Anthropic-compatible model gateway for Claude Code and VS Code Claude.

中文表达：

> ModelPort 是本机 Claude 模型端口，把 Claude Code 的 Anthropic Messages 请求路由到 Mimo、DeepSeek、OpenAI-compatible provider 和自定义上游。

## 当前主线

主线应保持清晰：

- 入口协议：Anthropic-compatible `/v1/messages`。
- 目标用户：使用 Claude Code / VS Code Claude 的开发者。
- 主要价值：快速切换模型、本机密钥隔离、provider 协议转换、稳定流式输出。
- 默认场景：`BASE_URL=https://w.ciykj.cn/v1` + `ANTHROPIC_MODEL=mimo-v2.5-pro`。

## 架构分层

```text
VS Code Claude / Claude Code
        |
        | Anthropic-compatible /v1/messages
        v
ModelPort local gateway
        |
        | route, auth, alias, protocol conversion
        v
Mimo / DeepSeek / OpenAI-compatible / custom provider
```

代码边界：

- `src/routes.rs`：HTTP 入口、鉴权、请求限制、响应转换。
- `src/config.rs`：provider、模型别名、默认路由和环境变量。
- `src/providers/`：Anthropic 和 OpenAI-compatible 协议适配。
- `src/http.rs`：上游 HTTP 客户端、SSE 解析、超时和响应体限制。
- `scripts/`：本机运行、自检、冒烟和 benchmark。
- `deploy/`：systemd 等生产部署模板。
- `docs/`：长期维护指导。

## 效率判断

当前中转效率足够高，适合本机长期使用和内网小团队试生产。原因是 ModelPort 只做必要的鉴权、路由和 JSON/SSE 转换；请求真正耗时通常来自上游模型生成和第三方中转网络。

不要把它建设成重型平台。短期优先级应该是：

1. 稳定文本链路。
2. 真实 provider 兼容矩阵。
3. 可观测和自检。
4. 清晰文档和部署模板。
5. 再扩展图像、Responses API 或管理面。

## GitHub 建设路线

仓库应该具备：

- README：面向使用者，回答“是什么、怎么用、怎么排查”。
- `docs/PROJECT_GUIDE.md`：面向维护者，说明定位和路线。
- `docs/PROVIDER_MATRIX.md`：面向 provider 接入，记录实测状态和验收标准。
- `docs/PERFORMANCE.md`：说明效率、瓶颈和 benchmark。
- `docs/GPT_IMAGE_2_GUIDE.md`：说明图像能力如何扩展。
- CI：fmt、test、clippy。
- Issue / PR 模板：规范反馈。
- Security policy：防止密钥泄露。
- Release notes：每次升级说明兼容性和验证结果。

## 未来拓展

建议按风险从低到高推进：

- Provider 实测矩阵：用 `scripts/provider-matrix.sh` 记录 Mimo、DeepSeek、OpenRouter、DashScope、Gemini 等真实测试状态。
- 路由策略：按模型名前缀、别名、fallback、provider 优先级扩展。
- 可观测性：请求耗时、上游状态码、provider 失败率。
- 图像能力：独立支持 `gpt-image-2` 的 Image API，不混入 Claude Code 文本主路径。
- 管理面：只在多人使用和配置复杂度上来后再做。

## Provider 验收标准

新增或声明支持某个 provider 前，至少完成：

1. 填好真实 key 和模型变量。
2. 启动或重启 ModelPort。
3. 运行 `scripts/doctor.sh`。
4. 运行 `scripts/provider-matrix.sh --model <model-id>`。
5. 同时通过非流式和流式 `/v1/messages`。
6. 把结果记录到 `docs/PROVIDER_MATRIX.md`。

如果 provider 只是在配置中存在，但没有真实 key 跑过，应标记为“待真实 key 验证”，不要对外宣称已生产验证。

## 不建议做的事

- 不要直接公网暴露。
- 不要在日志输出真实 key。
- 不要把所有 provider native API 都塞进第一阶段。
- 不要为了“任意模型”牺牲默认 Claude Code 稳定性。
- 不要把图片 base64 通过当前文本 SSE 主链路硬塞给 Claude Code。
