# ModelPort

[![CI](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml)

[English](README.md) | **简体中文**

ModelPort 是面向 Claude Code / VS Code Claude 的本地模型网关。它在本机暴露 Anthropic-compatible `/v1/messages` 接口，然后把请求路由到 DeepSeek、Mimo、Anthropic、OpenAI-compatible provider、OpenRouter、Ollama 或自定义本地运行时。

它的目标很明确：不改变编辑器工作流，通过一个本地、可观测、带鉴权的端口切换不同模型。

![ModelPort architecture overview](docs/assets/modelport-overview.svg)

## 当前实际配置

当前工作区已经按下面配置启动并验证：

| 项目 | 当前值 |
| --- | --- |
| Dashboard | `http://127.0.0.1:5173` |
| 本地 API | `http://127.0.0.1:17878` |
| 默认 provider | `deepseek` |
| Claude 模型 | `deepseek-v4-pro` |
| 存储 | Docker Compose PostgreSQL volume |

`mimo-v2.5-pro` 仍然是已支持模型，但它是否可用取决于上游额度、余额和限流状态。当前本机 Claude settings 已经指向 `deepseek-v4-pro`。

## 实际界面

下面截图来自当前正在运行的本地 Dashboard，不是 mock 图。

### 仪表盘

仪表盘展示 API Key 状态、请求量、Token、费用估算、成功率、provider 健康状态、模型分布和最近调用。

![ModelPort dashboard overview](docs/assets/dashboard-overview.png)

### 模型与 Provider 管理

模型页展示已注册模型、默认路由、provider 映射、别名、provider 生命周期控制和模型库存。

![ModelPort model management](docs/assets/dashboard-models.png)

### 系统设置

系统设置包含上线检查、服务参数、认证、限流、provider 凭证、运行诊断、备份导出和配置热加载。

![ModelPort system settings](docs/assets/dashboard-settings.png)

## 核心能力

- 使用 `x-api-key` 或 `Authorization: Bearer` 鉴权本地客户端。
- 接收 Claude Code / VS Code Claude 的 Anthropic Messages API 请求。
- 转换到上游 Anthropic-compatible 或 OpenAI-compatible provider API。
- 支持 `provider:model`、别名、显式模型 ID 和模型前缀路由。
- 记录请求、延迟、重试、输入/输出/cache token、provider 健康状态和费用估算。
- 提供 Web Dashboard 管理 API Key、用户、团队/项目、配额、日志、provider 配置、模型库存、别名、备份和运行参数。
- 支持 Docker Compose、本地源码开发、systemd、Prometheus metrics 和投产验收脚本。

ModelPort 适合个人和可信小团队环境，不建议直接暴露到公网。

## 快速开始

最快完整启动方式是 Docker Compose：后端 API、Dashboard UI 和内部 PostgreSQL。

```bash
cp deploy/docker/modelport.env.example .env
```

编辑 `.env`，至少设置：

```bash
MODELPORT_AUTH_TOKEN=replace-with-a-long-random-local-token
ANTHROPIC_AUTH_TOKEN=replace-with-the-same-local-router-token
MODELPORT_ADMIN_USERNAME=admin
MODELPORT_ADMIN_PASSWORD=replace-with-a-long-random-admin-password
MODELPORT_POSTGRES_PASSWORD=replace-with-a-long-random-postgres-password

MODELPORT_DEFAULT_PROVIDER=deepseek
DEEPSEEK_ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic
DEEPSEEK_ANTHROPIC_AUTH_TOKEN=replace-with-real-deepseek-api-key
DEEPSEEK_MODEL=deepseek-v4-pro

ANTHROPIC_BASE_URL=http://127.0.0.1:17878
ANTHROPIC_MODEL=deepseek-v4-pro
ANTHROPIC_DEFAULT_OPUS_MODEL=deepseek-v4-pro
ANTHROPIC_DEFAULT_SONNET_MODEL=deepseek-v4-pro
ANTHROPIC_DEFAULT_HAIKU_MODEL=deepseek-v4-pro
ANTHROPIC_SMALL_FAST_MODEL=deepseek-v4-pro
CLAUDE_CODE_SUBAGENT_MODEL=deepseek-v4-pro
```

启动：

```bash
docker compose up -d --build
docker compose ps
```

打开：

- Dashboard：`http://127.0.0.1:5173`
- 健康检查：`http://127.0.0.1:17878/health`
- Messages API：`http://127.0.0.1:17878/v1/messages`

使用 `MODELPORT_ADMIN_USERNAME` 和 `MODELPORT_ADMIN_PASSWORD` 登录 Dashboard。

## Claude Code 接入

在 VS Code Claude / Claude Code 里配置本地网关地址，并使用 `.env` 中相同的 router token。

常见 settings 路径：

```bash
# Linux / WSL
~/.config/Code/User/settings.json

# WSL 里看到的 Windows 路径
/mnt/c/Users/<you>/AppData/Roaming/Code/User/settings.json
```

推荐配置：

```json
{
  "claudeCode.selectedModel": "deepseek-v4-pro",
  "claudeCode.environmentVariables": [
    {
      "name": "ANTHROPIC_BASE_URL",
      "value": "http://127.0.0.1:17878"
    },
    {
      "name": "ANTHROPIC_AUTH_TOKEN",
      "value": "replace-with-the-same-local-router-token"
    },
    {
      "name": "ANTHROPIC_MODEL",
      "value": "deepseek-v4-pro"
    },
    {
      "name": "ANTHROPIC_DEFAULT_OPUS_MODEL",
      "value": "deepseek-v4-pro"
    },
    {
      "name": "ANTHROPIC_DEFAULT_SONNET_MODEL",
      "value": "deepseek-v4-pro"
    },
    {
      "name": "ANTHROPIC_DEFAULT_HAIKU_MODEL",
      "value": "deepseek-v4-pro"
    },
    {
      "name": "ANTHROPIC_SMALL_FAST_MODEL",
      "value": "deepseek-v4-pro"
    },
    {
      "name": "CLAUDE_CODE_SUBAGENT_MODEL",
      "value": "deepseek-v4-pro"
    }
  ]
}
```

修改 settings 后，重新加载 VS Code 或重启 Claude Code 会话。

## 验证服务

本地健康检查：

```bash
curl http://127.0.0.1:17878/health
```

带鉴权的模型列表：

```bash
source .env
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/v1/models
```

真实 `deepseek-v4-pro` 调用：

```bash
source .env
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  http://127.0.0.1:17878/v1/messages \
  -d '{
    "model": "deepseek-v4-pro",
    "max_tokens": 96,
    "messages": [
      {
        "role": "user",
        "content": "Reply exactly: OK"
      }
    ]
  }'
```

Provider 兼容性检查：

```bash
scripts/provider-matrix.sh --model deepseek-v4-pro
```

完整本地检查：

```bash
scripts/config-validate.sh
scripts/status.sh
scripts/acceptance.sh
```

`scripts/acceptance.sh --upstream` 和 `scripts/provider-matrix.sh --all` 会产生真实上游调用，可能产生费用。

## API

| Endpoint | 鉴权 | 作用 |
| --- | --- | --- |
| `GET /health` | 不需要 | 运行状态和 provider 健康状态。 |
| `GET /v1/models` | 需要 | Anthropic 风格模型列表。 |
| `POST /v1/messages` | 需要 | Anthropic-compatible messages API。 |
| `GET /metrics` | 需要 | Prometheus 文本指标。 |
| `/admin/*` | Cookie session | Dashboard 和控制面 API。 |

支持的鉴权 header：

```http
x-api-key: <MODELPORT_AUTH_TOKEN>
Authorization: Bearer <MODELPORT_AUTH_TOKEN>
```

Dashboard 使用账号登录，不直接使用 router token。首个管理员来自 `MODELPORT_ADMIN_USERNAME` 和 `MODELPORT_ADMIN_PASSWORD`。

## Providers

| Provider | 协议 | 主要环境变量 |
| --- | --- | --- |
| `deepseek` | Anthropic-compatible | `DEEPSEEK_ANTHROPIC_AUTH_TOKEN`, `DEEPSEEK_MODEL` |
| `deepseek_openai` | OpenAI-compatible | `DEEPSEEK_OPENAI_API_KEY`, `DEEPSEEK_OPENAI_MODEL`, `DEEPSEEK_API_KEY` |
| `mimo` | OpenAI-compatible | `BASE_URL`, `MIMO_OPENAI_BASE_URL`, `MIMO_OPENAI_API_KEY`, `MIMO_MODEL` |
| `anthropic` | Anthropic-compatible | `ANTHROPIC_API_KEY`, `ANTHROPIC_UPSTREAM_MODEL` |
| `openai` | OpenAI-compatible | `OPENAI_API_KEY`, `OPENAI_MODEL` |
| `openrouter` | OpenAI-compatible | `OPENROUTER_API_KEY`, `OPENROUTER_MODEL` |
| `gemini` | OpenAI-compatible | `GEMINI_API_KEY`, `GEMINI_MODEL` |
| `xai` | OpenAI-compatible | `XAI_API_KEY`, `XAI_MODEL` |
| `groq` | OpenAI-compatible | `GROQ_API_KEY`, `GROQ_MODEL` |
| `dashscope` | OpenAI-compatible | `DASHSCOPE_API_KEY`, `DASHSCOPE_MODEL` |
| `kimi` | OpenAI-compatible | `MOONSHOT_API_KEY`, `KIMI_MODEL` |
| `zhipu` | OpenAI-compatible | `ZHIPU_API_KEY`, `ZHIPU_MODEL` |
| `mistral` | OpenAI-compatible | `MISTRAL_API_KEY`, `MISTRAL_MODEL` |
| `ark` | OpenAI-compatible | `ARK_API_KEY`, `ARK_MODEL` |
| `ollama` | OpenAI-compatible | `MODELPORT_ENABLE_OLLAMA`, `OLLAMA_MODEL` |
| `custom` | OpenAI-compatible | `CUSTOM_OPENAI_BASE_URL`, `CUSTOM_OPENAI_MODEL` |
| `local_sglang` | OpenAI-compatible | `MODELPORT_ENABLE_LOCAL_SGLANG`, `SGLANG_BASE_URL`, `SGLANG_MODEL` |
| `local_vllm` | OpenAI-compatible | `MODELPORT_ENABLE_LOCAL_VLLM`, `VLLM_BASE_URL`, `VLLM_MODEL` |
| `local_llamacpp` | OpenAI-compatible | `MODELPORT_ENABLE_LOCAL_LLAMACPP`, `LLAMACPP_BASE_URL`, `LLAMACPP_MODEL` |

兼容状态和实测记录见 [docs/PROVIDER_MATRIX.md](docs/PROVIDER_MATRIX.md)。

## 模型切换

直接设置模型：

```bash
export ANTHROPIC_MODEL=deepseek-v4-pro
export ANTHROPIC_MODEL=mimo-v2.5-pro
export ANTHROPIC_MODEL=qwen-plus
```

强制指定 provider：

```bash
export ANTHROPIC_MODEL=deepseek:deepseek-v4-pro
export ANTHROPIC_MODEL=mimo:mimo-v2.5-pro
export ANTHROPIC_MODEL=openrouter:anthropic/claude-sonnet-4
export ANTHROPIC_MODEL=custom:any-model-name-from-your-upstream
```

在 `config.toml` 中配置别名：

```toml
[aliases]
main = "deepseek:deepseek-v4-pro"
mimo = "mimo:mimo-v2.5-pro"
local = "local_vllm:qwen2.5-coder"
```

然后使用：

```bash
export ANTHROPIC_MODEL=main
```

Dashboard 中的别名、provider 顺序、默认 provider、provider 生命周期和模型库存变更可以运行时生效。监听地址、并发上限等服务级参数仍然需要重启后端。

## 本地开发

只启动后端：

```bash
cp .env.example .env
scripts/start.sh
scripts/status.sh
```

Dashboard 开发服务：

```bash
cd dashboard
npm ci
npm run dev
```

Vite Dashboard 默认监听 `http://127.0.0.1:5173`，并把 `/admin`、`/v1`、`/health` 和 `/metrics` 代理到后端。

前台运行后端：

```bash
scripts/dev.sh
```

提交前检查：

```bash
scripts/check.sh
cd dashboard
npm run lint
npm run build
```

## 运维

常用 Docker 命令：

```bash
docker compose ps
docker compose logs -f modelport
docker compose restart modelport
docker compose down
```

备份和校验：

```bash
docker compose exec modelport model-port backup export /data/modelport-backup.json
docker compose exec modelport model-port backup validate /data/modelport-backup.json
```

Prometheus metrics：

```bash
source .env
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/metrics
```

常用脚本：

| 脚本 | 作用 |
| --- | --- |
| `scripts/config-validate.sh` | 不启动服务，静态校验配置。 |
| `scripts/start.sh` | 构建并后台启动本地后端。 |
| `scripts/stop.sh` | 停止由脚本启动的本地后端。 |
| `scripts/restart.sh` | 重启本地后端。 |
| `scripts/status.sh` | 查看 PID、日志路径、监听端口和 `/health` 状态。 |
| `scripts/doctor.sh` | 检查 env、服务、鉴权、VS Code settings 和关键 endpoint。 |
| `scripts/provider-matrix.sh` | 验证指定模型的非流式和流式兼容性。 |
| `scripts/acceptance.sh` | 运行个人/小团队投产验收检查。 |
| `scripts/bench.sh` | 测量本地和可选上游延迟。 |
| `scripts/build-release.sh` | 构建 `target/release/model-port`。 |
| `scripts/check.sh` | 运行 fmt、tests 和 clippy。 |

## 故障排查

| 现象 | 含义 | 处理 |
| --- | --- | --- |
| 启动提示缺少 token | `MODELPORT_AUTH_TOKEN` 或 `ANTHROPIC_AUTH_TOKEN` 未设置 | 设置两个值，并确保一致。 |
| `/v1/models` 返回 401 | 客户端 token 缺失或不匹配 | 检查 `x-api-key` 或 `ANTHROPIC_AUTH_TOKEN`。 |
| Claude Code 仍使用旧模型 | VS Code 还没加载新的 settings | Reload VS Code 或重启 Claude Code 会话。 |
| Provider 是 `degraded` 或 `cooldown` | 最近上游调用失败 | 在 Dashboard settings/logs 中测试 provider，并检查上游额度和状态。 |
| 上游返回 403 | Provider 账号或 key 被拒绝 | 检查上游 key、账号权限和余额。 |
| 上游返回 429 | Provider 限流或额度耗尽 | 等待、降流量或切换 provider。 |
| 大请求返回 413 | 请求体超过配置上限 | 增大 `MODELPORT_MAX_REQUEST_BODY_BYTES`。 |
| 流式返回 `event: error` | 本地请求开始后，上游流式失败 | 查看请求日志和后端日志。 |

推荐后端日志级别：

```bash
RUST_LOG=model_port=info,tower_http=info
```

## 文档

- [docs/PROJECT_GUIDE.md](docs/PROJECT_GUIDE.md)：项目定位、架构边界和路线。
- [docs/PROVIDER_MATRIX.md](docs/PROVIDER_MATRIX.md)：provider 兼容矩阵和验证流程。
- [docs/ACCEPTANCE.md](docs/ACCEPTANCE.md)：投产验收清单。
- [docs/DOCKER.md](docs/DOCKER.md)：Docker Compose 部署和 PostgreSQL 持久化。
- [docs/LOCAL_RUNTIME.md](docs/LOCAL_RUNTIME.md)：SGLang、vLLM、llama.cpp、Ollama 和自定义本地运行时。
- [docs/PERFORMANCE.md](docs/PERFORMANCE.md)：benchmark 和运行调优。
- [docs/GITHUB_SETUP.md](docs/GITHUB_SETUP.md)：release 和仓库设置建议。
- [dashboard/README.md](dashboard/README.md)：Dashboard 开发和 E2E 测试说明。

## 非目标

ModelPort 刻意保持小而清晰：

- 它不是聊天客户端。
- 它不是云端模型聚合平台。
- 它不是企业 IAM、外部计费或公网多租户 SaaS。
- 它不在本地运行模型推理，只做协议适配和路由。
- 它不追求覆盖所有 provider 原生 API，而是优先支持 Anthropic-compatible 和 OpenAI-compatible API。
