# ModelPort Learning

这里放 ModelPort 的学习、复盘和面试材料。

> **非规范文档。** 这些材料用于讲解设计思路，不定义 API、配置、部署或已验证
> Provider。权威现状以 [架构](../ARCHITECTURE.md)、[配置](../CONFIGURATION.md)、
> [API](../API.md) 和 [运维](../OPERATIONS.md) 为准。图片是概念图，可能省略执行
> 顺序和当前限制。最后统一复核：2026-07-11。

## 面试材料

- [ModelPort 技术面经](./MODELPORT_INTERVIEW_GUIDE.md)
- [ModelPort 面试问题与回答清单](./MODELPORT_INTERVIEW_QA.md)
- [ModelPort 深度面试 Q&A](./MODELPORT_INTERVIEW_QA_DEEP.md)

这套面经配套 5 张技术图，建议按下面顺序讲：

1. 项目总览：先讲 ModelPort 的定位和完整能力面。
2. 请求生命周期：把一次请求从客户端到 Provider 再回来的链路讲清楚。
3. Tool Use 深挖：重点讲工具调用协议、校验、映射和流式参数处理。
4. 协议适配与 Streaming：讲 Anthropic/OpenAI-compatible 的语义差异。
5. 安全、稳定性、可观测性：证明项目不是简单转发，而是一个可治理网关。

如果要刷题复习，先看问题清单；如果要准备高级工程师深挖，重点看深度面试 Q&A；如果要组织完整表达，再看技术面经。

讲解时必须明确当前边界：`readyz` 检查存储但不是全 Provider 门禁；浏览器保护是
CSRF/Origin 写保护而不是通用 CORS；request ID 不是完整分布式 trace；stream
完成态、并发配额、DNS SSRF 和同步整文档持久化仍有限制。

## 配套图片

图片目前放在 [../assets](../assets)：

- [modelport-technical-interview-map.png](../assets/modelport-technical-interview-map.png)
- [modelport-interview-01-request-lifecycle.png](../assets/modelport-interview-01-request-lifecycle.png)
- [modelport-interview-02-tool-use-deep-dive.png](../assets/modelport-interview-02-tool-use-deep-dive.png)
- [modelport-interview-03-protocol-streaming.png](../assets/modelport-interview-03-protocol-streaming.png)
- [modelport-interview-04-security-stability-observability.png](../assets/modelport-interview-04-security-stability-observability.png)
