# ModelPort

[![CI](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml)

[English](README.md) | **简体中文**

ModelPort 是面向 Claude Code、VS Code Claude 和 API 客户端的自托管
Anthropic-compatible 模型网关。它用一个本地 `/v1/messages` 入口连接
Anthropic-compatible 与 OpenAI-compatible Provider，并提供路由、Tool Use
转换、鉴权、配额、请求日志、Provider 健康和轻量小团队控制台。

![ModelPort architecture overview](docs/assets/modelport-overview.svg)

ModelPort 面向单台可信主机或小型可信网络，不是公网多租户模型平台、聊天
客户端，也不负责模型推理。

## 已实现能力

- Anthropic-compatible `POST /v1/messages` 和 `GET /v1/models`。
- Anthropic 直通与 OpenAI Chat Completions 协议转换。
- Anthropic 风格 SSE 转换，包括常见 Tool Use delta。
- 模型别名、`provider:model`、精确模型和前缀路由。
- 本地 legacy token，以及带模型/Provider/IP、滚动费用窗口和用户配额策略的
  控制台 API Key。
- Provider 凭证池、冷却状态、有边界的 fallback、诊断、请求日志和
  Prometheus metrics。
- 覆盖用户、密钥、团队、配额、Provider、模型、别名、日志、健康、审计和
  脱敏诊断快照的 React 控制台。
- JSON 文件或 PostgreSQL 控制面存储，以及 Docker Compose 和 systemd 模板。

仓库内存在 Provider 配置，不等于已通过真实上游验证。带日期的验证结果应记录在
[Provider 兼容矩阵](docs/PROVIDER_MATRIX.md)。

## 技术核心

- **协议边界：** 对外保持 Anthropic Messages 契约，内部通过聚焦的 adapter
  直通 Anthropic-compatible 流量，或转换为 OpenAI Chat Completions，包括有边界
  的 SSE 与常见 Tool Use 事件转换。
- **确定性路由：** 按明确顺序解析 `provider:model`、alias、精确模型、模型前缀和
  默认 Provider；存在可用替代项时跳过冷却中的 Provider，并且只在模型可接收且
  出现 transport、protocol、429 或 5xx 等可重试失败时执行有边界的 fallback。
- **按上游尝试治理：** 鉴权及全局/身份/IP 限流先于路由执行；随后在每次上游
  尝试前检查 API Key 策略、用户配额、API Key/团队费用窗口、Provider 凭证、
  capability gate 以及 Provider/模型限流。preflight 拒绝不会扣量，只有确实发出
  的上游尝试才会记录配额或费用消耗。
- **防御式传输与流式处理：** 禁止上游重定向，对请求、响应、SSE、idle time 和
  并发设置边界；远程 Provider 默认要求 HTTPS；live stream permit 会一直持有到
  response body 完成或被丢弃。
- **单一控制面事实源：** 环境变量/TOML 基础配置与持久化控制台 override 合成
  实际运行配置；JSON 文件与 PostgreSQL 保存相同的 auth/control 逻辑文档，控制台
  只是后端客户端，不是第二套路由事实源。
- **保留证据来源的可观测性：** request ID、保留期内 usage log、Prometheus
  进程指标、健康/冷却状态和控制台聚合，会区分上游返回 usage 与本地估算。

这些机制已经实现，但不代表 ModelPort 是精确计费系统或分布式硬配额服务。
Provider 配置不等于真实上游验证；live stream 也可能在 HTTP 200 后失败，且无法
跨 Provider 重放。详见[技术核心及其边界](docs/ARCHITECTURE.md#technical-core)。

## 使用 Docker Compose 快速启动

前置条件：Docker Compose v2，以及至少一个 Provider 的有效凭证。

```bash
cp deploy/docker/modelport.env.example .env
```

编辑 `.env`，至少替换：

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

启动并查看：

```bash
docker compose up -d --build
docker compose ps
docker compose logs -f modelport
```

访问：

- 控制台：`http://127.0.0.1:5173`
- 存活检查：`http://127.0.0.1:17878/livez`
- Messages API：`http://127.0.0.1:17878/v1/messages`

使用 `MODELPORT_ADMIN_USERNAME` 和 `MODELPORT_ADMIN_PASSWORD` 登录控制台。

## 连接 Claude Code

客户端使用发布的 API 地址和同一个 router token：

```env
ANTHROPIC_BASE_URL=http://127.0.0.1:17878
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

## 验证

```bash
source .env

curl -fsS http://127.0.0.1:17878/livez

curl -fsS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/v1/models

curl -fsS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  -H 'content-type: application/json' \
  http://127.0.0.1:17878/v1/messages \
  -d '{
    "model":"deepseek-v4-flash",
    "max_tokens":96,
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

- `/readyz` 是带鉴权的诊断接口；上游降级时它目前不会自动返回失败。
- Stream 可能在初始 HTTP 200 之后通过 SSE `event: error` 失败。响应头发出后，
  live stream 完成态、最终 usage/cost、Provider 健康和 fallback 尚未完全闭环；
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

共享使用时：

1. 为真实且 active 的用户创建控制台 API Key，再设置
   `MODELPORT_REQUIRE_CONTROL_API_KEYS=1`。
2. 配置精确的可信代理 CIDR 和浏览器 Origin。
3. HTTPS 后设置 `MODELPORT_ADMIN_COOKIE_SECURE=1`。
4. 像保护凭证一样保护 PostgreSQL/JSON 状态和 CLI 备份。

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
