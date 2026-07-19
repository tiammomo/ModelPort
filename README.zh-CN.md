# ModelPort

[![CI](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml)

[English](README.md) | **简体中文**

ModelPort 是面向 Claude Code、VS Code Claude、OpenAI-compatible SDK 和 API
客户端的自托管多协议模型网关。`/v1/messages` 与
`/v1/chat/completions` 入口复用同一套鉴权、策略、配额、路由、用量结算和
Provider 健康链路，并连接 Anthropic-compatible 与 OpenAI-compatible
Provider。

项目的目标方向已调整为具备完整治理能力的多协议企业级模型网关。当前版本尚未
达到这一目标；目标架构、迁移工作流和基于证据的发布门槛见
[企业级网关路线图](docs/ENTERPRISE_ROADMAP.md)。

![ModelPort architecture overview](docs/assets/modelport-overview.svg)

ModelPort 面向单台可信主机或小型可信网络，不是公网多租户模型平台、聊天
客户端，也不负责模型推理。

## 已实现能力

- Anthropic-compatible `POST /v1/messages`、按 Provider 显式开启的精确
  `POST /v1/messages/count_tokens`、有明确兼容范围的 OpenAI-compatible
  `POST /v1/chat/completions`，以及 `GET /v1/models`。
- Anthropic 直通与 OpenAI Chat Completions 协议转换。
- Anthropic 风格 SSE 转换，包括常见 Tool Use delta、按工具完整 JSON Schema
  响应校验和 Tool 语义终态观测。
- 可选的单次非流式严格 Schema 参数修复；提示脱敏、每次尝试独立入账，并合并请求级
  Token/费用证据。
- 模型别名、`provider:model`、精确模型和前缀路由。
- 本地 legacy token，以及带模型/Provider/IP、滚动费用窗口和用户配额策略的
  控制台 API Key。
- Provider 凭证池、冷却状态、有边界的 fallback、诊断、请求日志和
  Prometheus metrics。
- 覆盖用户、密钥、团队、配额、Provider、模型、别名、日志、健康、审计、
  企业请求/尝试账本和脱敏诊断快照的 React 控制台；官方 DeepSeek provider
  支持管理员实时只读查询线上余额，充值和账单仍以 DeepSeek 控制台为准。
- 带租户外键、SQLx 连接池、rustls、事务预算预留/结算和不可变证据流水的 PostgreSQL 请求/尝试账本；
  兼容期 JSON 文件/PostgreSQL 控制面存储；Docker Compose 和 systemd 模板。

仓库内存在 Provider 配置，不等于已通过真实上游验证。带日期的验证结果应记录在
[Provider 兼容矩阵](docs/PROVIDER_MATRIX.md)。

## 技术核心

- **协议边界：** Anthropic Messages 与当前范围内的 OpenAI Chat Completions
  都先解析为强类型、协议中立的 Exchange IR，再进入共享治理链路；edge adapter
  会保留已支持的文本、角色、function tools、Tool Use ID、结束原因、usage 与有边界
  的 SSE，对未支持字段显式拒绝，不做静默丢弃。
- **确定性路由：** 按明确顺序解析 `provider:model`、alias、精确模型、模型前缀和
  默认 Provider；存在可用替代项时跳过冷却中的 Provider，并且只在模型可接收且
  出现 transport、protocol、429 或 5xx 等可重试失败时执行有边界的 fallback。
- **按上游尝试治理：** 鉴权及全局/身份/IP 限流先于路由执行；随后在每次上游
  尝试前检查 API Key 策略、用户配额、API Key/团队费用窗口、Provider 凭证、
  capability gate 以及 Provider/模型限流。preflight 拒绝不会扣量，只有确实发出
  的上游尝试才会记录配额或费用消耗；PostgreSQL 模式还会在出站前原子预留租户预算，
  并在终态按真实证据结算或释放。
- **重试与崩溃安全：** 可选的租户级 `Idempotency-Key` 声明会阻止重复 Provider
  调用；请求/尝试租约会贯穿完整 stream 交付。失去所有者且租约过期的记录会收敛
  为不计费的 `unreconciled` 证据，不会永久停留在进行中。
- **防御式传输与流式处理：** 禁止上游重定向，对请求、响应、SSE、idle time 和
  并发设置边界；远程 Provider 默认要求 HTTPS；live stream permit 会一直持有到
  response body 完成或被丢弃。
- **单一控制面事实源：** 环境变量/TOML 基础配置与持久化控制台 override 合成
  实际运行配置；迁移期内 JSON 文件与 PostgreSQL 仍保存相同的 auth/control 逻辑
  文档，同时规范化 PostgreSQL 账本会在上游调用前记录带租户作用域的请求与
  Provider 尝试。控制台只是后端客户端，不是第二套路由事实源。
- **保留证据来源的可观测性：** request ID、保留期内 usage log、Prometheus
  进程指标、健康/冷却状态和控制台聚合，会区分上游返回 usage 与本地估算；流式
  日志与健康状态在 response body 完成、失败或被丢弃时结算，而不是在初始 HTTP
  200 建立时提前判定成功。
  流式首语义延迟从首个非空正文或 Tool Call 事件计时；非流式完整生命周期延迟不会
  被误标为 TTFT。
  有界 `x-modelport-traffic-class` 可区分业务、合成和诊断调用，不保留请求正文。

这些机制已经实现；PostgreSQL 租户预算已是分布式硬准入控制，但 Provider 最终账单
仍是计费权威，兼容期用户配额/费用窗口也仍是 preflight guard，而非精确计费系统。
Provider 配置不等于真实上游验证；live stream 也可能在 HTTP 200 后失败，且无法
跨 Provider 重放。当前幂等声明会阻止第二次调用，但不会重放首次响应。详见
[技术核心及其边界](docs/ARCHITECTURE.md#technical-core)。

## 使用 Docker Compose 快速启动

前置条件：Docker Compose v2，以及至少一个 Provider 的有效凭证。

复制模板前先选择上游拓扑：

| 拓扑 | 默认 Provider | 必需上游凭据 | 说明 |
| --- | --- | --- | --- |
| 只用 DeepSeek | `deepseek` | `DEEPSEEK_ANTHROPIC_AUTH_TOKEN` | 使用官方 Anthropic-compatible 入口；支持管理员只读查询官方余额 |
| 只用本地 Qwen | `local_qwen` | 本地 runtime 无鉴权时不需要 | 需要 TOML `local_qwen` provider 与 `QWEN_LOCAL_BASE_URL`，所有 DeepSeek 变量均可省略 |
| Qwen + DeepSeek | 显式选择其一 | DeepSeek Key + 可达的 Qwen runtime | QuantPilot 推荐拓扑：Qwen 默认，DeepSeek 使用 `deepseek:<model>` 显式选择 |

上游 Provider Key 只保存在 ModelPort。应用使用独立的 legacy router token，或优先使用控制台签发、带 provider/model scope 的客户端 API Key。每个接入产品还应使用独立 Key，并由管理员把它固定绑定到唯一的 `organization/project/environment` 账本作用域；客户端请求头只能断言该绑定，不能切换到其他项目。完整 Qwen-only 和组合 TOML 示例见[配置：Provider 拓扑配方](docs/CONFIGURATION.md#provider-topology-recipes)，隔离配置见 [API Key tenant binding](docs/CONFIGURATION.md#api-key-tenant-binding)。

```bash
cp deploy/docker/modelport.env.example .env
```

如果选择 DeepSeek-only 样例，编辑 `.env` 并替换下面的值；如果选择 Qwen-only，请改用拓扑配方并省略整个 DeepSeek 区块：

```env
MODELPORT_AUTH_TOKEN=replace-with-a-long-random-local-token
ANTHROPIC_AUTH_TOKEN=replace-with-the-same-local-router-token
MODELPORT_ADMIN_USERNAME=admin
MODELPORT_ADMIN_PASSWORD=replace-with-a-long-random-admin-password
MODELPORT_POSTGRES_PASSWORD=replace-with-a-long-random-postgres-password

MODELPORT_DEFAULT_PROVIDER=deepseek
DEEPSEEK_ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic
DEEPSEEK_ANTHROPIC_AUTH_TOKEN=replace-with-a-real-provider-key
DEEPSEEK_MODEL=deepseek-v4-flash
```

`deepseek-v4-flash` 是仓库的配置样例，不代表所有账号都能使用该模型。请使用
Provider 实际为你的账号开放的精确模型 ID。

QuantPilot 长期接入时，在 ModelPort 配置 `local_qwen` 与 `deepseek`，签发只允许所需 provider/model 的客户端 Key，QuantPilot 只保存该 Key 为 `MODELPORT_API_KEY`。不要把 `DEEPSEEK_ANTHROPIC_AUTH_TOKEN` 复制到 QuantPilot。

启动并查看：

```bash
scripts/build-container.sh
docker compose up -d
docker compose ps
docker compose logs -f modelport
```

默认部署会启用 PostgreSQL，这也是当前推荐的企业模式。若部署时明确选择轻量文件模式，
使用 `docker compose -f docker-compose.yml -f docker-compose.files.yml up -d --build`；
该模式的 auth/control 数据会落盘，但企业请求与预算账本只保存在进程内存中。

访问：

- 控制台：`http://127.0.0.1:33002`
- 存活检查：`http://127.0.0.1:38082/livez`
- Messages API：`http://127.0.0.1:38082/v1/messages`
- Chat Completions API：`http://127.0.0.1:38082/v1/chat/completions`

使用 `MODELPORT_ADMIN_USERNAME` 和 `MODELPORT_ADMIN_PASSWORD` 登录控制台。

## 连接 Claude Code

客户端使用发布的 API 地址和同一个 router token：

```env
ANTHROPIC_BASE_URL=http://127.0.0.1:38082
ANTHROPIC_AUTH_TOKEN=replace-with-the-same-local-router-token
ANTHROPIC_MODEL=deepseek-v4-flash
ANTHROPIC_DEFAULT_OPUS_MODEL=deepseek-v4-flash
ANTHROPIC_DEFAULT_SONNET_MODEL=deepseek-v4-flash
ANTHROPIC_DEFAULT_HAIKU_MODEL=deepseek-v4-flash
ANTHROPIC_SMALL_FAST_MODEL=deepseek-v4-flash
CLAUDE_CODE_SUBAGENT_MODEL=deepseek-v4-flash
```

VS Code Claude 扩展可把这些变量放入其环境变量设置，然后重新加载扩展或窗口。
模型值必须与 ModelPort 的实际模型目录一致。

## 连接 OpenAI-Compatible SDK

把支持自定义 base URL 的 SDK 指向 ModelPort，并使用相同的客户端密钥：

```env
OPENAI_BASE_URL=http://127.0.0.1:38082/v1
OPENAI_API_KEY=replace-with-the-same-local-router-token
OPENAI_MODEL=deepseek-v4-flash
```

以上标准 `OPENAI_*` 名称属于**客户端进程**。如果 ModelPort 服务端要把 OpenAI
作为上游 Provider，请使用独立变量：

```env
MODELPORT_OPENAI_BASE_URL=https://api.openai.com/v1
MODELPORT_OPENAI_API_KEY=replace-with-an-openai-platform-api-key
MODELPORT_OPENAI_MODEL=gpt-5.5
```

不要把客户端的 `OPENAI_BASE_URL=http://127.0.0.1:38082/v1` 复制到 ModelPort
服务环境中，否则 OpenAI Provider 会指回网关自身。服务端旧版 `OPENAI_*` 名称仍作为
兼容回退保留，但启动和 `config validate` 会给出迁移 warning。

当前 Chat Completions 是有文档边界的文本/function-tool 兼容范围，不代表完整
OpenAI API 等价。应用接入前请阅读 [API 参考](docs/API.md#chat-completions)。

## 验证

```bash
source .env

curl -fsS http://127.0.0.1:38082/livez

curl -fsS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:38082/v1/models

curl -fsS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  -H 'content-type: application/json' \
  http://127.0.0.1:38082/v1/messages \
  -d '{
    "model":"deepseek-v4-flash",
    "max_tokens":96,
    "messages":[{"role":"user","content":"Reply exactly: OK"}]
  }'

curl -fsS \
  -H "Authorization: Bearer $MODELPORT_AUTH_TOKEN" \
  -H 'content-type: application/json' \
  http://127.0.0.1:38082/v1/chat/completions \
  -d '{
    "model":"deepseek-v4-flash",
    "messages":[{"role":"user","content":"Reply exactly: OK"}]
  }'
```

Messages 请求会产生真实上游费用。不生成文本的本地检查是：

```bash
scripts/config-validate.sh
scripts/status.sh
scripts/smoke-test.sh
```

使用 `scripts/acceptance.sh` 验证控制面，使用
`scripts/tool-use-acceptance.sh` 验证本地 mock Tool Use 链路。带
`--upstream` 的命令和 `provider-matrix.sh` 可能产生 Provider 费用。
每个 Messages 请求都必须提供正数 `max_tokens`，且不得超过
`MODELPORT_MAX_OUTPUT_TOKENS`（默认 131072）；非法值会在路由前被拒绝。

## 重要运行限制

- `/readyz` 是带鉴权的诊断接口，会检查 auth、control 和关系型账本存储；上游
  降级时它目前不会自动返回失败。
- Stream 可能在初始 HTTP 200 之后通过 SSE `event: error` 失败。响应头发出后，
  完成态、耗时、指标和 Provider 健康会在 body 完成、失败或被丢弃时统一结算；
  可识别的 Provider usage 终态事件会替换本地估算，没有该事件的 stream 仍保留
  估算值，并且下游响应头发出后不能 fallback。
  buffered 兼容模式会先完成上游，但代价是延迟首字节。
- 限流、并发 stream permit 和控制台 session 都是进程内状态；stream permit 会
  持有到响应 body 完成或释放。配额检查不是事务式预留，并发请求可能超过很紧的
  额度。
- Provider URL 校验会阻止危险的字面量地址，但不会固定 DNS 或重新校验解析后的
  私网地址。
- Auth/Control 持久化会同步写完整逻辑 JSON 文档；保留量和吞吐应维持在小团队
  设计范围内。
- token 和费用属于运维估算，不是 Provider 账单。日志用
  `upstream-returned` 标记 Provider 返回的 usage，用 `local-estimate` 标记本地
  估算；只有实际发起上游尝试才会消耗用户配额或 API Key/团队费用额度。

共享部署前请阅读[架构](docs/ARCHITECTURE.md)和[运维](docs/OPERATIONS.md)。

## 安全

除非可信局域网或同源 HTTPS 反向代理确实需要访问，否则保留默认 loopback 发布。
不要把后端直接暴露到公网。不要提交 `.env`、Provider key、完整备份、prompt 或
未经审查的敏感日志。

远程 Provider 默认必须使用 HTTPS。明文 HTTP 会暴露 Provider API Key、prompt 和
响应内容；不安全放行开关只能用于边界明确的可信内网上游。本地/custom runtime
仍可在 loopback 或受控本地网络使用 HTTP。

可选的 [OIDC 控制台登录预览](docs/OIDC.md)只负责对人类用户进行身份验证，
并签发 ModelPort 控制台会话。它不会收集、转发或代理 ChatGPT 密码、
Cookie、浏览器会话或订阅，这些也不是 OpenAI API 凭证。数据面调用方仍需
使用 ModelPort API Key；OpenAI 或其他上游 Provider 凭证始终保留在服务端。

共享使用时：

1. 为真实且 active 的用户创建控制台 API Key，再设置
   `MODELPORT_REQUIRE_CONTROL_API_KEYS=1`。
2. 配置精确的可信代理 CIDR 和浏览器 Origin。
3. HTTPS 后设置 `MODELPORT_ADMIN_COOKIE_SECURE=1`。
4. 像保护凭证一样保护 PostgreSQL/JSON 状态和 CLI 备份。

持久化 usage、ledger 和 Provider 健康错误只保留错误类别；Provider 原始正文、
Tool 校验路径、请求值、URL 与存储诊断会在进入持久化遥测前被剔除。

威胁边界和安全报告方式见 [SECURITY.md](SECURITY.md)。

## 本地开发

```bash
cp .env.example .env
# 替换所有必填 placeholder
scripts/config-validate.sh
scripts/start.sh

cd dashboard
npm ci
npm run dev
```

提交前：

```bash
scripts/check-all.sh
```

完整工具链和测试矩阵见[开发文档](docs/DEVELOPMENT.md)。

单机 PostgreSQL 部署在升级前应生成完整备份并执行隔离恢复演练：

```bash
scripts/backup-compose.sh create
scripts/backup-compose.sh drill backups/modelport-<UTC>.tar.gz
```

备份包含数据库和明文运行凭证，已被 Git 忽略并使用收敛权限，但要抵御磁盘损坏仍需
复制到加密、受控的异机存储。

## 文档

- [文档索引](docs/README.md)
- [架构](docs/ARCHITECTURE.md)
- [配置参考](docs/CONFIGURATION.md)
- [API 参考](docs/API.md)
- [运维](docs/OPERATIONS.md)
- [Docker Compose](docs/DOCKER.md)
- [systemd](docs/SYSTEMD.md)
- [Provider 兼容矩阵](docs/PROVIDER_MATRIX.md)
- [Tool Use 兼容性](docs/TOOL_USE_COMPATIBILITY.md)
- [投产验收](docs/ACCEPTANCE.md)

## 许可证

[MIT](LICENSE)
