# ModelPort

[![CI](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml)

[中文](#modelport) | [English](#english-readme)

**ModelPort 是 Claude Code / VS Code Claude 的本地模型端口。**

它在本机暴露 Anthropic-compatible `/v1/messages` 接口，让 Claude Code / VS Code Claude 插件可以在不改变编辑器工作流的前提下，路由到 Mimo、DeepSeek、Anthropic、OpenAI-compatible provider、OpenRouter、Ollama 或自定义上游。

![ModelPort 项目含义图](docs/assets/modelport-overview.svg)

## 投产结论

当前项目已经进入**可投产阶段**，适合个人长期使用、本机常驻、内网小团队生产/试生产。它已经具备长期运行所需的核心能力：

- 本地 token 鉴权，默认禁止无鉴权启动。
- 原生 reqwest/rustls HTTP 传输，不再依赖系统 `curl`。
- 上游连接池、连接超时、非流式请求超时、流式空闲超时。
- 真实上游 HTTP 状态透传，流式上游错误会转换为 Anthropic SSE `event: error`。
- 请求 ID、路由日志、上游状态日志。
- 请求体大小限制、并发请求限制、响应体大小限制。
- Mimo 第三方地址 `BASE_URL=https://w.ciykj.cn/v1` 和模型 `mimo-v2.5-pro` 已适配。
- 覆盖模型路由、OpenAI-compatible 转换、上游 401、流式错误、请求过大的自动化测试。
- 提供快速启动脚本、doctor 自检、Docker Compose、systemd 模板和 GitHub Actions CI。

投产边界也要明确：

- 适合绑定 `127.0.0.1` 给本机 Claude Code / VS Code Claude 使用。
- 适合在内网小团队前面加反向代理后使用。
- 不建议直接暴露到公网。
- 暂不作为多租户 SaaS 网关使用，因为它不做用户体系、计费、额度、审计留存和细粒度 RBAC。
- 真实可用还取决于上游 API key。仓库示例里的 `MIMO_OPENAI_API_KEY` 是占位符，必须换成真实 key。

## 项目定位

ModelPort 的定位不是“大而全模型聚合平台”，而是一个轻量、本地、可控的开发者模型路由适配层。

- 对用户：让 Claude Code 接入市面主流代码模型的本地端口。
- 对开发者：Anthropic Messages API 到多 provider 的轻量路由/协议转换网关。
- 对长期演进：本地 AI provider control plane 的最小核心，负责模型命名、路由、协议转换、密钥隔离和 fallback 策略。

核心能力：

- Anthropic-compatible API 直接 pass-through，例如 Anthropic 官方、DeepSeek Anthropic 格式。
- OpenAI-compatible API 自动转换，例如 Mimo、OpenAI、OpenRouter、Gemini、Qwen/DashScope、Kimi、GLM、Grok、Groq、Mistral、Doubao/Ark、Ollama、自定义中转。
- `provider:model`、模型别名、模型前缀和未知模型透传，服务 Claude Code 里的快速切模型场景。
- 密钥保留在本机环境变量里，不依赖云端控制面。

## 效率与好用性

当前中转效率足够支撑 Claude Code / VS Code Claude 的本机长期使用，也足够支撑内网小团队试生产。ModelPort 只做必要的鉴权、路由、JSON 转换和 SSE 转换；大多数耗时来自上游模型生成和第三方中转网络。

可以用内置 benchmark 查看本机端口耗时：

```bash
scripts/bench.sh
```

真实上游 benchmark 会产生模型调用成本，默认只跑 3 次：

```bash
scripts/bench.sh --upstream
```

更详细的效率判断、瓶颈和调优建议见 [docs/PERFORMANCE.md](docs/PERFORMANCE.md)。

## 项目文档

- [docs/PROJECT_GUIDE.md](docs/PROJECT_GUIDE.md)：项目定位、架构边界、长期路线。
- [docs/PROVIDER_MATRIX.md](docs/PROVIDER_MATRIX.md)：provider 兼容矩阵、实测状态和验收标准。
- [docs/PERFORMANCE.md](docs/PERFORMANCE.md)：中转效率、benchmark 和投产调优。
- [docs/GITHUB_SETUP.md](docs/GITHUB_SETUP.md)：GitHub 仓库设置、分支保护、release 建议。
- [docs/GPT_IMAGE_2_GUIDE.md](docs/GPT_IMAGE_2_GUIDE.md)：基于 OpenAI `gpt-image-2` 的后续图像能力扩展指导。

小米 MiMo 官方 OpenAI-compatible base URL 当前是 `https://api.xiaomimimo.com/v1`。如果你买的是第三方中转服务，可以设置服务商给你的通用 `BASE_URL`，例如 `BASE_URL=https://w.ciykj.cn/v1`；`MIMO_OPENAI_BASE_URL` 存在时会优先覆盖 `BASE_URL`。

DeepSeek 官方 Anthropic base URL 是 `https://api.deepseek.com/anthropic`。默认模型列表包含 `deepseek-v4-pro`、`deepseek-v4-flash`，并保留旧的 `deepseek-chat`、`deepseek-reasoner` 用于兼容。

## 快速开始

![ModelPort 快速上手图](docs/assets/modelport-quickstart.svg)

### 1. 安装依赖

需要 Rust toolchain。Linux / WSL 建议安装基础 C/C++ 编译工具，用于构建 TLS 依赖：

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config jq
```

`jq` 不是运行必需项，只用于命令行检查 JSON。如果机器上暂时没有 `jq`，可以用 `node` 或直接看响应文本。

### 2. 准备配置

```bash
cd /home/tiammomo/projects/dev/ModelPort
cp .env.example .env
```

编辑 `.env`，最少需要填这几项：

```bash
MODELPORT_BIND=127.0.0.1:17878
MODELPORT_AUTH_TOKEN=replace-with-a-long-random-local-token
MODELPORT_DEFAULT_PROVIDER=mimo

BASE_URL=https://w.ciykj.cn/v1
MIMO_OPENAI_API_KEY=replace-with-real-mimo-api-key
MIMO_MODEL=mimo-v2.5-pro

ANTHROPIC_BASE_URL=http://127.0.0.1:17878
ANTHROPIC_AUTH_TOKEN=replace-with-the-same-local-router-token
ANTHROPIC_MODEL=mimo-v2.5-pro
ANTHROPIC_DEFAULT_OPUS_MODEL=mimo-v2.5-pro
ANTHROPIC_DEFAULT_SONNET_MODEL=mimo-v2.5-pro
ANTHROPIC_DEFAULT_HAIKU_MODEL=mimo-v2.5-pro
ANTHROPIC_SMALL_FAST_MODEL=mimo-v2.5-pro
CLAUDE_CODE_SUBAGENT_MODEL=mimo-v2.5-pro
```

注意：

- `MODELPORT_AUTH_TOKEN` 是 Claude Code 调用 ModelPort 的本地 token。
- `ANTHROPIC_AUTH_TOKEN` 必须和 `MODELPORT_AUTH_TOKEN` 一致。
- `MIMO_OPENAI_API_KEY` 是第三方 Mimo 上游 key，不能用占位符。
- `.env` 已被 `.gitignore` 忽略，不要提交真实 key。

启动前可以先做静态配置校验：

```bash
scripts/config-validate.sh
```

它会检查默认 provider、provider base URL、placeholder key、alias 循环、非本机监听地址等问题，但不会打印真实密钥。已经安装 release 二进制后，也可以直接运行：

```bash
model-port config validate
```

### 3. 启动服务

推荐快速启动后台服务：

```bash
scripts/start.sh
```

查看状态：

```bash
scripts/status.sh
```

停止服务：

```bash
scripts/stop.sh
```

重启服务：

```bash
scripts/restart.sh
```

开发时也可以前台运行：

```bash
scripts/dev.sh
```

看到类似日志即表示服务已启动：

```text
ModelPort listening on http://127.0.0.1:17878
```

### 4. 验证服务

推荐先跑完整本机自检：

```bash
scripts/doctor.sh
```

再使用内置冒烟测试：

```bash
scripts/smoke-test.sh
```

上面只验证本机网关和鉴权。填入真实 `MIMO_OPENAI_API_KEY` 后，可以验证真实上游模型回复：

```bash
scripts/doctor.sh --upstream
scripts/smoke-test.sh --upstream
```

需要验证某个 provider/model 的非流式和流式兼容性时：

```bash
scripts/provider-matrix.sh --model mimo-v2.5-pro
```

需要验证 `/v1/models` 返回的全部已注册模型时：

```bash
scripts/provider-matrix.sh --all
```

健康检查不需要 token：

```bash
curl http://127.0.0.1:17878/health
```

模型列表需要 token：

```bash
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/v1/models
```

非流式消息测试：

```bash
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  http://127.0.0.1:17878/v1/messages \
  -d '{
    "model": "mimo-v2.5-pro",
    "max_tokens": 128,
    "messages": [
      {
        "role": "user",
        "content": "用一句话回复：ModelPort 已连接。"
      }
    ]
  }'
```

流式消息测试：

```bash
curl -N -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  http://127.0.0.1:17878/v1/messages \
  -d '{
    "model": "mimo-v2.5-pro",
    "max_tokens": 128,
    "stream": true,
    "messages": [
      {
        "role": "user",
        "content": "流式回复：你好。"
      }
    ]
  }'
```

如果返回：

```json
{"code":"INVALID_API_KEY","message":"Invalid API key"}
```

说明 ModelPort 已经成功连到上游 `BASE_URL`，但 `MIMO_OPENAI_API_KEY` 不正确或仍是占位符。

## VS Code Claude 接入

当前推荐方式是在 VS Code 用户级 `settings.json` 里配置 Claude Code 插件环境变量。

Linux / WSL 常见路径：

```bash
/home/tiammomo/.config/Code/User/settings.json
```

推荐配置：

```json
{
  "claudeCode.selectedModel": "mimo-v2.5-pro",
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
      "value": "mimo-v2.5-pro"
    },
    {
      "name": "ANTHROPIC_DEFAULT_OPUS_MODEL",
      "value": "mimo-v2.5-pro"
    },
    {
      "name": "ANTHROPIC_DEFAULT_SONNET_MODEL",
      "value": "mimo-v2.5-pro"
    },
    {
      "name": "ANTHROPIC_DEFAULT_HAIKU_MODEL",
      "value": "mimo-v2.5-pro"
    },
    {
      "name": "ANTHROPIC_SMALL_FAST_MODEL",
      "value": "mimo-v2.5-pro"
    },
    {
      "name": "CLAUDE_CODE_SUBAGENT_MODEL",
      "value": "mimo-v2.5-pro"
    }
  ]
}
```

配置后：

1. 先启动 ModelPort。
2. 重启 VS Code 或重新加载 Claude Code 插件窗口。
3. 在 VS Code Claude 中选择或使用 `mimo-v2.5-pro`。
4. 发起一次简单问题，观察 ModelPort 日志中是否出现 `routing message request`。

本机当前已配置为：

- `ANTHROPIC_BASE_URL=http://127.0.0.1:17878`
- `ANTHROPIC_MODEL=mimo-v2.5-pro`
- `claudeCode.selectedModel=mimo-v2.5-pro`

## 长期运行

### 前台运行

适合调试：

```bash
RUST_LOG=model_port=info,tower_http=info scripts/dev.sh
```

### 后台运行

简单本机常驻直接用脚本：

```bash
scripts/start.sh
scripts/status.sh
tail -f .modelport/model-port.log
```

脚本会使用 release 二进制后台启动，PID 写入 `.modelport/model-port.pid`，日志写入 `.modelport/model-port.log`。

也可以用 `tmux`：

```bash
tmux new -s modelport
scripts/dev.sh
```

需要退出窗口但保持服务运行时，按 `Ctrl-b` 再按 `d`。

### Docker Compose 运行

适合把 ModelPort 固定成一个本机服务。先确认 `.env` 中已经填好真实 token 和上游 key，然后运行：

```bash
docker compose up -d --build
docker compose logs -f modelport
```

Compose 会把容器内监听地址改成 `0.0.0.0:17878`，但宿主机只暴露到 `127.0.0.1:17878`，便于 VS Code Claude 本机访问。

停止：

```bash
docker compose down
```

### systemd 运行

如果是标准 Linux 主机，可以编译二进制后用 systemd：

```bash
cargo build --release
sudo install -m 0755 target/release/model-port /usr/local/bin/model-port
sudo mkdir -p /etc/modelport
sudo cp deploy/systemd/modelport.env.example /etc/modelport/modelport.env
sudo nano /etc/modelport/modelport.env
sudo chmod 600 /etc/modelport/modelport.env
sudo cp deploy/systemd/modelport.service /etc/systemd/system/modelport.service
```

启动：

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now modelport
sudo systemctl status modelport
journalctl -u modelport -f
```

`/etc/modelport/modelport.env` 必须填真实 `MODELPORT_AUTH_TOKEN`、`ANTHROPIC_AUTH_TOKEN` 和 `MIMO_OPENAI_API_KEY`；不要把真实 env 文件提交到 git。

WSL 默认不一定启用 systemd；如果 systemd 不可用，用 `tmux` 或进程管理器即可。

## 模型切换

默认 provider 是 `mimo`。未知 Claude 原生模型名会安全兜底到 `mimo-v2.5-pro`，避免 VS Code 插件把 `claude-...` 原样发给 Mimo 导致失败。

常用切换方式有三种。

### 方式一：直接设置模型名

```bash
export ANTHROPIC_MODEL=mimo-v2.5-pro
export ANTHROPIC_MODEL=deepseek-v4-pro
export ANTHROPIC_MODEL=qwen-plus
```

### 方式二：使用 `provider:model`

强制发给指定 provider：

```bash
export ANTHROPIC_MODEL=mimo:mimo-v2.5-pro
export ANTHROPIC_MODEL=openrouter:anthropic/claude-sonnet-4
export ANTHROPIC_MODEL=gemini:gemini-2.5-flash
export ANTHROPIC_MODEL=custom:any-model-name-from-your-upstream
```

### 方式三：配置别名

在 `~/.config/modelport/config.toml` 中配置：

```toml
[aliases]
sonnet = "openrouter:anthropic/claude-sonnet-4"
qwen = "dashscope:qwen-plus"
mimo = "mimo:mimo-v2.5-pro"
```

然后：

```bash
export ANTHROPIC_MODEL=sonnet
```

默认环境配置会始终注册 `mimo` 和 `deepseek`。其他 provider 只要设置对应 API key、base URL、模型变量，或 `MODELPORT_ENABLE_<PROVIDER>=1`，就会自动注册。复制 `config.example.toml` 时，缺少 key 的非默认 provider 会自动跳过；如果你想在 `/v1/models` 里展示未填 key 的 provider，可以设置 `MODELPORT_INCLUDE_UNAVAILABLE_PROVIDERS=1`。

`openrouter`、`custom`、`ollama` 支持未知模型名原样透传，最适合“任意模型”热切换。

## Provider 列表

| Provider | 协议 | 关键环境变量 |
| --- | --- | --- |
| `mimo` | OpenAI-compatible | `MIMO_OPENAI_API_KEY`, `MIMO_MODEL` |
| `deepseek` | Anthropic-compatible | `DEEPSEEK_ANTHROPIC_AUTH_TOKEN`, `DEEPSEEK_MODEL` |
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

## 配置文件

默认不需要配置文件，ModelPort 会读取环境变量。需要固定 provider、别名或优先级时，可以把示例复制到用户配置目录：

```bash
mkdir -p ~/.config/modelport
cp config.example.toml ~/.config/modelport/config.toml
```

也可以用 `MODELPORT_CONFIG=/path/to/config.toml` 指定其他配置文件。

真实密钥仍建议通过环境变量传入，不建议写死在 `config.toml`：

- `MODELPORT_AUTH_TOKEN`：Claude Code 调用本机端口时使用的本地 token。
- `DEEPSEEK_ANTHROPIC_AUTH_TOKEN`：DeepSeek API key。
- `BASE_URL` / `MIMO_OPENAI_BASE_URL`：小米/Mimo 第三方 OpenAI-compatible `/v1` 地址；`MIMO_OPENAI_BASE_URL` 优先。
- `MIMO_OPENAI_API_KEY`：小米/Mimo 第三方 OpenAI-compatible API key。
- 其他 provider 的 key 看上面的 provider 表；复制 `config.example.toml` 后，建议只保留你实际使用的 provider 块。

provider 配置支持这些路由字段：

- `provider_order`：前缀匹配的优先级，遇到模型前缀冲突时靠它决定。
- `models`：显式模型名，精确匹配后会保留原模型名发给上游。
- `model_prefixes`：模型名前缀匹配，例如 `gemini-`、`qwen-`。
- `passthrough_unknown_models`：未知模型是否原样透传给该 provider。`openrouter`、`custom`、`ollama` 默认适合开启。
- `max_tokens_field`：OpenAI-compatible 上游的 token 字段，可选 `max_completion_tokens`、`max_tokens`、`both`。
- `deduplicate_stream_text`：处理会重放流式文本片段的上游。Mimo 默认开启，标准 OpenAI-compatible provider 默认关闭。
- `[aliases]`：左边是 Claude/VS Code 看到的模型名，右边可以是 provider ID、模型名，或 `provider:model`。

服务级运行参数：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `MODELPORT_BIND` | `127.0.0.1:17878` | 监听地址。生产建议保持本机地址，不要直接公网暴露。 |
| `MODELPORT_MAX_REQUEST_BODY_BYTES` | `33554432` | 单请求体大小上限。 |
| `MODELPORT_MAX_CONCURRENT_REQUESTS` | `64` | 并发请求上限。 |
| `MODELPORT_HTTP_CONNECT_TIMEOUT_SECS` | `10` | 上游连接超时。 |
| `MODELPORT_HTTP_REQUEST_TIMEOUT_SECS` | `600` | 非流式请求总超时。 |
| `MODELPORT_HTTP_STREAM_IDLE_TIMEOUT_SECS` | `300` | 流式上游空闲超时。 |
| `MODELPORT_HTTP_MAX_RESPONSE_BYTES` | `33554432` | 非流式响应体和错误响应体上限。 |
| `MODELPORT_HTTP_USER_AGENT` | `model-port/<version>` | 上游请求 User-Agent。 |
| `MODELPORT_INCLUDE_UNAVAILABLE_PROVIDERS` | unset | 设置为 `1` 时展示未配置 key 的 provider。 |
| `MODELPORT_ALLOW_NO_AUTH` | unset | 仅隔离测试可设为 `1`，投产不要启用。 |

## API 接口

- `GET /health`
- `GET /metrics`
- `GET /v1/models`
- `POST /v1/messages`

`POST /v1/messages` 同时支持非流式和 `stream: true`。DeepSeek provider 直接转发 Anthropic SSE；OpenAI-compatible provider 会把 OpenAI SSE chunk 转换为 Anthropic SSE event。

`GET /metrics` 输出 Prometheus 文本格式，包含 route 和 message provider/model 维度的请求数、成功/失败数和耗时累计。该接口需要和 `/v1/models` 一样携带本地 token：

```bash
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/metrics
```

### 鉴权

客户端必须带以下任一 header：

```http
x-api-key: <MODELPORT_AUTH_TOKEN>
```

或：

```http
Authorization: Bearer <MODELPORT_AUTH_TOKEN>
```

## 日志与排查

推荐日志级别：

```bash
RUST_LOG=model_port=info,tower_http=info
```

需要更详细上游排查时：

```bash
RUST_LOG=model_port=debug,tower_http=info
```

常见问题：

| 现象 | 含义 | 处理 |
| --- | --- | --- |
| 启动时报缺少 token | 没有设置 `MODELPORT_AUTH_TOKEN` 或 `ANTHROPIC_AUTH_TOKEN` | 在 `.env` 中设置长随机 token。 |
| `/v1/models` 返回 401 | 客户端没带 token 或 token 不一致 | 检查 `x-api-key` / `ANTHROPIC_AUTH_TOKEN`。 |
| 上游返回 `INVALID_API_KEY` | 已经连到第三方上游，但上游 key 不对 | 替换真实 `MIMO_OPENAI_API_KEY`。 |
| VS Code Claude 没走 ModelPort | 插件没有读取环境变量或未重载 | 重启 VS Code，确认 settings 中 `ANTHROPIC_BASE_URL`。 |
| 请求卡住后报 timeout | 上游长时间无响应或网络问题 | 调大 `MODELPORT_HTTP_REQUEST_TIMEOUT_SECS` 或检查上游状态。 |
| 流式返回 `event: error` | 上游流式请求失败，ModelPort 已转换错误 | 看错误 message 和 ModelPort 日志。 |
| 大请求返回 413 | 请求体超过限制 | 调大 `MODELPORT_MAX_REQUEST_BODY_BYTES`。 |

## 升级与回滚

升级前：

```bash
scripts/check.sh
scripts/doctor.sh --upstream
```

升级步骤：

```bash
git pull
scripts/build-release.sh
scripts/restart.sh
```

如果用 systemd：

```bash
sudo install -m 0755 target/release/model-port /usr/local/bin/model-port
sudo systemctl restart modelport
journalctl -u modelport -f
```

回滚时切回上一版二进制或上一版 git commit，重新 `cargo build --release` 并重启服务。

## 非目标和边界

ModelPort 有意保持小而清晰：

- 不做聊天客户端。
- 不做云端模型聚合平台。
- 不做计费、额度、用户体系。
- 不实现模型推理，只做协议适配和本地路由。
- 不追求所有 provider 的 native API，优先支持 Anthropic-compatible 和 OpenAI-compatible。

这版优先覆盖 Claude Code 常用路径：文本消息、system、tool schema、tool use/tool result、流式文本和流式 tool call 参数。图片、document、server tool、MCP 专用块暂时不转换。

HTTP 传输层使用原生 reqwest/rustls 客户端，支持连接池、真实 HTTP 状态、请求超时、流式空闲超时和错误响应体上限。

## 脚本速查

| 脚本 | 作用 |
| --- | --- |
| `scripts/start.sh` | 构建 release 二进制并后台启动 ModelPort。 |
| `scripts/stop.sh` | 停止当前项目的 ModelPort 进程。 |
| `scripts/restart.sh` | 停止后重新后台启动。 |
| `scripts/status.sh` | 查看 PID、日志位置和 `/health` 状态。 |
| `scripts/doctor.sh` | 检查 `.env`、本机服务、鉴权、VS Code settings 和关键配置。 |
| `scripts/doctor.sh --upstream` | 在 doctor 基础上验证真实 Mimo 上游消息回复。 |
| `scripts/config-validate.sh` | 静态校验 ModelPort 配置，不启动服务。 |
| `scripts/provider-matrix.sh` | 验证指定模型的非流式和流式 provider 兼容性。 |
| `scripts/provider-matrix.sh --all` | 验证 `/v1/models` 中全部已注册模型，会产生真实上游调用成本。 |
| `scripts/bench.sh` | 测量本机 `/health` 和 `/v1/models` 延迟。 |
| `scripts/bench.sh --upstream` | 测量真实 `/v1/messages` 上游调用延迟，会产生模型调用成本。 |
| `scripts/dev.sh` | 加载 `.env` 后前台 `cargo run`，适合开发调试。 |
| `scripts/smoke-test.sh` | 验证本机网关和鉴权。 |
| `scripts/smoke-test.sh --upstream` | 验证真实上游模型回复，需要真实 Mimo key。 |
| `scripts/build-release.sh` | 构建 `target/release/model-port`。 |
| `scripts/check.sh` | 运行 fmt、test、clippy。 |
| `scripts/install-deps-ubuntu.sh` | 在 Ubuntu / WSL 上安装 `build-essential pkg-config jq`。 |

---

## English README

**ModelPort is a local model gateway for Claude Code and VS Code Claude.**

It exposes an Anthropic-compatible `/v1/messages` endpoint on your machine, while routing requests to Mimo, DeepSeek, Anthropic, OpenAI-compatible providers, OpenRouter, Ollama, or a custom upstream without changing your Claude Code workflow.

### Production Status

ModelPort is now ready for production use within its intended scope:

- Personal long-running local usage.
- A persistent local gateway for VS Code Claude / Claude Code.
- Small internal team production or pilot deployments behind a trusted network boundary.

The production boundary is intentional:

- Bind to `127.0.0.1` for local development by default.
- Use a reverse proxy if you deploy it for a small internal team.
- Do not expose it directly to the public internet.
- It is not a multi-tenant SaaS gateway. It does not provide user accounts, billing, quotas, audit retention, or fine-grained RBAC.
- Real availability still depends on valid upstream API keys.

Production-ready capabilities include:

- Local token authentication, with unauthenticated startup disabled by default.
- Native `reqwest` / `rustls` HTTP transport.
- Connection pooling, connect timeout, request timeout, stream idle timeout, and response body limits.
- Upstream HTTP status propagation and Anthropic SSE error conversion.
- Request IDs, route logs, and upstream status logs.
- Request body limits and concurrency limits.
- Mimo third-party base URL support: `BASE_URL=https://w.ciykj.cn/v1`.
- Mimo model support: `mimo-v2.5-pro`.
- CI, production scripts, doctor checks, Docker Compose, and systemd templates.

### Project Positioning

ModelPort is not a full model aggregation platform. It is a lightweight, local, controllable model routing and protocol adaptation layer.

- For users: a local port that lets Claude Code use popular coding models.
- For developers: a lightweight gateway from Anthropic Messages API to multiple providers.
- For long-term evolution: a minimal local AI provider control plane for model naming, routing, protocol conversion, key isolation, and fallback behavior.

Core routing modes:

- Anthropic-compatible pass-through providers, such as Anthropic and DeepSeek Anthropic format.
- OpenAI-compatible providers, such as Mimo, OpenAI, OpenRouter, Gemini, Qwen/DashScope, Kimi, GLM, Grok, Groq, Mistral, Doubao/Ark, Ollama, and custom relays.
- `provider:model` routing, aliases, model prefixes, and unknown-model passthrough for fast model switching.

### Documentation

- [docs/PROJECT_GUIDE.md](docs/PROJECT_GUIDE.md): positioning, architecture boundaries, and roadmap.
- [docs/PROVIDER_MATRIX.md](docs/PROVIDER_MATRIX.md): provider compatibility matrix, verified status, and acceptance checklist.
- [docs/PERFORMANCE.md](docs/PERFORMANCE.md): gateway efficiency, benchmarks, and production tuning.
- [docs/GITHUB_SETUP.md](docs/GITHUB_SETUP.md): GitHub repository setup, branch protection, and release suggestions.
- [docs/GPT_IMAGE_2_GUIDE.md](docs/GPT_IMAGE_2_GUIDE.md): future image capability extension notes.

### Quick Start

Install dependencies on Linux / WSL:

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config jq
```

Prepare the environment file:

```bash
cd /home/tiammomo/projects/dev/ModelPort
cp .env.example .env
```

Edit `.env` and set at least:

```bash
MODELPORT_BIND=127.0.0.1:17878
MODELPORT_AUTH_TOKEN=replace-with-a-long-random-local-token
MODELPORT_DEFAULT_PROVIDER=mimo

BASE_URL=https://w.ciykj.cn/v1
MIMO_OPENAI_API_KEY=replace-with-real-mimo-api-key
MIMO_MODEL=mimo-v2.5-pro

ANTHROPIC_BASE_URL=http://127.0.0.1:17878
ANTHROPIC_AUTH_TOKEN=replace-with-the-same-local-router-token
ANTHROPIC_MODEL=mimo-v2.5-pro
ANTHROPIC_DEFAULT_OPUS_MODEL=mimo-v2.5-pro
ANTHROPIC_DEFAULT_SONNET_MODEL=mimo-v2.5-pro
ANTHROPIC_DEFAULT_HAIKU_MODEL=mimo-v2.5-pro
ANTHROPIC_SMALL_FAST_MODEL=mimo-v2.5-pro
CLAUDE_CODE_SUBAGENT_MODEL=mimo-v2.5-pro
```

Important notes:

- `MODELPORT_AUTH_TOKEN` is the local token used by Claude Code to call ModelPort.
- `ANTHROPIC_AUTH_TOKEN` must match `MODELPORT_AUTH_TOKEN`.
- `MIMO_OPENAI_API_KEY` must be a real upstream key.
- `.env` is ignored by git. Never commit real secrets.

Validate configuration before startup:

```bash
scripts/config-validate.sh
```

It checks the default provider, provider base URLs, placeholder keys, alias cycles, non-loopback bind addresses, and related static configuration issues without printing secrets. After installing the release binary, you can also run:

```bash
model-port config validate
```

Start the service:

```bash
scripts/start.sh
```

Check status:

```bash
scripts/status.sh
```

Restart or stop:

```bash
scripts/restart.sh
scripts/stop.sh
```

### Verification

Run the local doctor check:

```bash
scripts/doctor.sh
```

Run gateway smoke tests:

```bash
scripts/smoke-test.sh
```

After setting a real Mimo key, verify the real upstream route:

```bash
scripts/doctor.sh --upstream
scripts/smoke-test.sh --upstream
```

Check non-streaming and streaming compatibility for one provider/model:

```bash
scripts/provider-matrix.sh --model mimo-v2.5-pro
```

Check every registered model returned by `/v1/models`:

```bash
scripts/provider-matrix.sh --all
```

Health check:

```bash
curl http://127.0.0.1:17878/health
```

Authenticated model list:

```bash
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/v1/models
```

Non-streaming message test:

```bash
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  http://127.0.0.1:17878/v1/messages \
  -d '{
    "model": "mimo-v2.5-pro",
    "max_tokens": 128,
    "messages": [
      {
        "role": "user",
        "content": "Reply in one short sentence: ModelPort is connected."
      }
    ]
  }'
```

Streaming message test:

```bash
curl -N -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  http://127.0.0.1:17878/v1/messages \
  -d '{
    "model": "mimo-v2.5-pro",
    "max_tokens": 128,
    "stream": true,
    "messages": [
      {
        "role": "user",
        "content": "Stream a short hello."
      }
    ]
  }'
```

If the upstream returns:

```json
{"code":"INVALID_API_KEY","message":"Invalid API key"}
```

ModelPort has reached the configured upstream `BASE_URL`, but the upstream key is invalid or still a placeholder.

### VS Code Claude Setup

Configure Claude Code extension environment variables in your VS Code user `settings.json`.

Common Linux / WSL path:

```bash
/home/tiammomo/.config/Code/User/settings.json
```

Common Windows path from WSL:

```bash
/mnt/c/Users/pearf/AppData/Roaming/Code/User/settings.json
```

Recommended configuration:

```json
{
  "claudeCode.selectedModel": "mimo-v2.5-pro",
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
      "value": "mimo-v2.5-pro"
    },
    {
      "name": "ANTHROPIC_DEFAULT_OPUS_MODEL",
      "value": "mimo-v2.5-pro"
    },
    {
      "name": "ANTHROPIC_DEFAULT_SONNET_MODEL",
      "value": "mimo-v2.5-pro"
    },
    {
      "name": "ANTHROPIC_DEFAULT_HAIKU_MODEL",
      "value": "mimo-v2.5-pro"
    },
    {
      "name": "ANTHROPIC_SMALL_FAST_MODEL",
      "value": "mimo-v2.5-pro"
    },
    {
      "name": "CLAUDE_CODE_SUBAGENT_MODEL",
      "value": "mimo-v2.5-pro"
    }
  ]
}
```

Then:

1. Start ModelPort.
2. Reload VS Code or the Claude Code extension window.
3. Use `mimo-v2.5-pro`.
4. Ask a simple question and check ModelPort logs for `routing message request`.

### Long-Running Deployment

Foreground mode for debugging:

```bash
RUST_LOG=model_port=info,tower_http=info scripts/dev.sh
```

Background mode for local production:

```bash
scripts/start.sh
scripts/status.sh
tail -f .modelport/model-port.log
```

Docker Compose:

```bash
docker compose up -d --build
docker compose logs -f modelport
```

The Compose file binds the host port to `127.0.0.1:17878` while using `0.0.0.0:17878` inside the container.

systemd on a standard Linux host:

```bash
cargo build --release
sudo install -m 0755 target/release/model-port /usr/local/bin/model-port
sudo mkdir -p /etc/modelport
sudo cp deploy/systemd/modelport.env.example /etc/modelport/modelport.env
sudo nano /etc/modelport/modelport.env
sudo chmod 600 /etc/modelport/modelport.env
sudo cp deploy/systemd/modelport.service /etc/systemd/system/modelport.service
sudo systemctl daemon-reload
sudo systemctl enable --now modelport
sudo systemctl status modelport
```

Follow logs:

```bash
journalctl -u modelport -f
```

WSL may not have systemd enabled by default. Use the background scripts or `tmux` if systemd is unavailable.

### Model Switching

Set a direct model name:

```bash
export ANTHROPIC_MODEL=mimo-v2.5-pro
export ANTHROPIC_MODEL=deepseek-v4-pro
export ANTHROPIC_MODEL=qwen-plus
```

Force a specific provider with `provider:model`:

```bash
export ANTHROPIC_MODEL=mimo:mimo-v2.5-pro
export ANTHROPIC_MODEL=openrouter:anthropic/claude-sonnet-4
export ANTHROPIC_MODEL=gemini:gemini-2.5-flash
export ANTHROPIC_MODEL=custom:any-model-name-from-your-upstream
```

Use aliases in `~/.config/modelport/config.toml`:

```toml
[aliases]
sonnet = "openrouter:anthropic/claude-sonnet-4"
qwen = "dashscope:qwen-plus"
mimo = "mimo:mimo-v2.5-pro"
```

Then:

```bash
export ANTHROPIC_MODEL=sonnet
```

`openrouter`, `custom`, and `ollama` are the best choices for arbitrary model passthrough.

### Providers

| Provider | Protocol | Key Variables |
| --- | --- | --- |
| `mimo` | OpenAI-compatible | `MIMO_OPENAI_API_KEY`, `MIMO_MODEL` |
| `deepseek` | Anthropic-compatible | `DEEPSEEK_ANTHROPIC_AUTH_TOKEN`, `DEEPSEEK_MODEL` |
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

### API

- `GET /health`
- `GET /metrics`
- `GET /v1/models`
- `POST /v1/messages`

`POST /v1/messages` supports both non-streaming requests and `stream: true`.

`GET /metrics` returns Prometheus text metrics for route and message provider/model counters, success/failure counters, and accumulated duration. It requires the same local token as `/v1/models`:

```bash
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/metrics
```

Authentication requires either:

```http
x-api-key: <MODELPORT_AUTH_TOKEN>
```

or:

```http
Authorization: Bearer <MODELPORT_AUTH_TOKEN>
```

### Troubleshooting

Recommended logs:

```bash
RUST_LOG=model_port=info,tower_http=info
```

Debug logs:

```bash
RUST_LOG=model_port=debug,tower_http=info
```

| Symptom | Meaning | Fix |
| --- | --- | --- |
| Startup fails with missing token | `MODELPORT_AUTH_TOKEN` or `ANTHROPIC_AUTH_TOKEN` is not set | Set a long random local token in `.env`. |
| `/v1/models` returns 401 | Client token is missing or mismatched | Check `x-api-key` and `ANTHROPIC_AUTH_TOKEN`. |
| Upstream returns `INVALID_API_KEY` | ModelPort reached the upstream, but the upstream key is invalid | Replace `MIMO_OPENAI_API_KEY`. |
| VS Code Claude does not use ModelPort | Extension did not load the environment variables | Restart VS Code and verify `ANTHROPIC_BASE_URL`. |
| Request times out | Upstream or network is slow | Increase `MODELPORT_HTTP_REQUEST_TIMEOUT_SECS` or check upstream health. |
| Stream returns `event: error` | Upstream streaming request failed | Check the error message and ModelPort logs. |
| Large request returns 413 | Request body exceeds the configured limit | Increase `MODELPORT_MAX_REQUEST_BODY_BYTES`. |

### Upgrade And Rollback

Before upgrading:

```bash
scripts/check.sh
scripts/doctor.sh --upstream
```

Upgrade:

```bash
git pull
scripts/build-release.sh
scripts/restart.sh
```

For systemd:

```bash
sudo install -m 0755 target/release/model-port /usr/local/bin/model-port
sudo systemctl restart modelport
journalctl -u modelport -f
```

Rollback by checking out a previous commit or restoring a previous binary, then rebuild and restart.

### Script Reference

| Script | Purpose |
| --- | --- |
| `scripts/start.sh` | Build the release binary if needed and start ModelPort in the background. |
| `scripts/stop.sh` | Stop ModelPort processes for this project. |
| `scripts/restart.sh` | Stop and start the background service. |
| `scripts/status.sh` | Show PID, log path, listener, and `/health` status. |
| `scripts/doctor.sh` | Check `.env`, local service, auth, VS Code settings, and key configuration. |
| `scripts/doctor.sh --upstream` | Run doctor plus a real Mimo upstream message call. |
| `scripts/config-validate.sh` | Validate ModelPort configuration without starting the service. |
| `scripts/provider-matrix.sh` | Check non-streaming and streaming compatibility for selected models. |
| `scripts/provider-matrix.sh --all` | Check every registered model returned by `/v1/models`. This may incur model cost. |
| `scripts/bench.sh` | Measure local `/health` and `/v1/models` latency. |
| `scripts/bench.sh --upstream` | Measure real `/v1/messages` upstream latency. This may incur model cost. |
| `scripts/dev.sh` | Load `.env` and run `cargo run` in the foreground. |
| `scripts/smoke-test.sh` | Verify local gateway and authentication. |
| `scripts/smoke-test.sh --upstream` | Verify a real upstream model reply. |
| `scripts/build-release.sh` | Build `target/release/model-port`. |
| `scripts/check.sh` | Run fmt, tests, and clippy. |
| `scripts/install-deps-ubuntu.sh` | Install `build-essential`, `pkg-config`, and `jq` on Ubuntu / WSL. |
