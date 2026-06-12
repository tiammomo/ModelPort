# ModelPort

[![CI](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml)

[English](README.md) | **简体中文**

![ModelPort architecture overview](docs/assets/modelport-overview.svg)

_架构概览：Claude 客户端调用本机 ModelPort，ModelPort 负责鉴权、路由、协议转换、流式输出和指标，再转发到已配置的上游 provider。_

**ModelPort 是 Claude Code / VS Code Claude 的本地模型网关。**

它在本机暴露 Anthropic-compatible `/v1/messages` 接口，把 Claude Code / VS Code Claude 的请求路由到 Mimo、DeepSeek、Anthropic、OpenAI-compatible provider、OpenRouter、Ollama 或自定义上游。核心目标是：不改变编辑器工作流，让本机开发环境稳定使用不同基础模型。

## 当前结论

ModelPort 已经进入当前定位下的**可投产阶段**：

- 适合个人长期使用、本机常驻、VS Code Claude / Claude Code 日常开发。
- 适合内网小团队生产或试生产，但建议放在可信网络或反向代理之后。
- 不建议直接暴露到公网。
- 不是多租户 SaaS 网关，不提供用户体系、计费、额度、审计留存和细粒度 RBAC。

当前已真实验证：

- 第三方 Mimo base URL：`https://w.ciykj.cn/v1`
- 模型：`mimo-v2.5-pro`
- 非流式 `/v1/messages`
- 流式 `/v1/messages`
- VS Code Claude Windows/WSL settings
- `doctor`、`provider-matrix`、`/metrics` 和配置校验

## 核心能力

- 本地 token 鉴权，默认禁止无鉴权启动。
- Anthropic-compatible 入口协议。
- OpenAI-compatible 上游协议转换。
- `provider:model`、模型别名、模型前缀、未知模型透传。
- 原生 `reqwest` / `rustls` HTTP 传输，不依赖系统 `curl` 子进程。
- 上游连接池、连接超时、请求超时、流式空闲超时。
- 请求体、响应体和并发上限。
- Mimo 稳定流式输出，避免重放片段污染 Claude Code。
- `doctor` 运行态自检、`config validate` 静态配置校验、provider matrix 实测。
- Prometheus 文本 `/metrics`。
- Docker Compose、systemd、快速启动脚本、GitHub Actions CI。

## 项目定位

ModelPort 的定位不是“大而全模型聚合平台”，而是一个轻量、本地、可控的开发者模型路由适配层。

- 对用户：让 Claude Code 接入市面常用代码模型的本地端口。
- 对开发者：Anthropic Messages API 到多 provider 的轻量协议转换网关。
- 对长期演进：本地 AI provider control plane 的最小核心，负责模型命名、路由、协议转换、密钥隔离和 provider 策略。

## 项目文档

- [docs/PROJECT_GUIDE.md](docs/PROJECT_GUIDE.md)：定位、架构边界和长期路线。
- [docs/PROVIDER_MATRIX.md](docs/PROVIDER_MATRIX.md)：provider 兼容矩阵、实测状态和验收标准。
- [docs/LOCAL_RUNTIME.md](docs/LOCAL_RUNTIME.md)：SGLang、vLLM、llama.cpp、Ollama 和自定义本地运行时接入。
- [docs/PERFORMANCE.md](docs/PERFORMANCE.md)：效率、benchmark、运行指标和投产调优。
- [docs/GITHUB_SETUP.md](docs/GITHUB_SETUP.md)：GitHub 仓库设置、分支保护和 release 建议。
- [docs/GPT_IMAGE_2_GUIDE.md](docs/GPT_IMAGE_2_GUIDE.md)：后续图像能力扩展指导。

## 快速开始

![ModelPort quick-start flow](docs/assets/modelport-quickstart.svg)

_快速开始图：准备 `.env`、校验配置、启动本地网关，然后让 VS Code Claude 通过 ModelPort 使用 `mimo-v2.5-pro`。_

### 1. 安装依赖

Linux / WSL 建议：

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config jq
```

需要 Rust toolchain。`jq` 不是运行必需项，但会让 JSON 检查和 `--all` provider matrix 更方便。

### 2. 准备配置

```bash
cd /home/tiammomo/projects/dev/ModelPort
cp .env.example .env
```

编辑 `.env`，最少需要这些变量：

```bash
MODELPORT_BIND=127.0.0.1:17878
MODELPORT_AUTH_TOKEN=replace-with-a-long-random-local-token
MODELPORT_ADMIN_USERNAME=admin
MODELPORT_ADMIN_PASSWORD=replace-with-a-long-random-admin-password
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

- `MODELPORT_AUTH_TOKEN` 是 Claude Code 调用本地 ModelPort 的 token。
- `ANTHROPIC_AUTH_TOKEN` 必须和 `MODELPORT_AUTH_TOKEN` 一致。
- `MODELPORT_ADMIN_USERNAME` 和 `MODELPORT_ADMIN_PASSWORD` 只用于管理后台登录。
- `MIMO_OPENAI_API_KEY` 必须是真实上游 key，不能是占位符。
- `.env` 已被 `.gitignore` 忽略，不要提交真实密钥。

启动前做静态配置校验：

```bash
scripts/config-validate.sh
```

已经安装 release 二进制后，也可以直接运行：

```bash
model-port config validate
```

当前如果没有填 DeepSeek key，校验会给 warning，但默认 Mimo 链路不受影响。

### 3. 启动服务

后台启动：

```bash
scripts/start.sh
```

查看状态：

```bash
scripts/status.sh
```

停止和重启：

```bash
scripts/stop.sh
scripts/restart.sh
```

开发调试前台运行：

```bash
scripts/dev.sh
```

### 4. 验证服务

本机完整自检：

```bash
scripts/doctor.sh
```

真实 Mimo 上游验证：

```bash
scripts/doctor.sh --upstream
scripts/smoke-test.sh --upstream
```

provider 非流式和流式兼容性验证：

```bash
scripts/provider-matrix.sh --model mimo-v2.5-pro
```

验证所有已注册模型：

```bash
scripts/provider-matrix.sh --all
```

`--all` 会产生真实上游调用成本。

### 5. VS Code Claude 接入

在 VS Code 用户级 `settings.json` 配置 Claude Code 插件环境变量。

Linux / WSL 常见路径：

```bash
/home/tiammomo/.config/Code/User/settings.json
```

Windows 路径在 WSL 中通常是：

```bash
/mnt/c/Users/pearf/AppData/Roaming/Code/User/settings.json
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

配置后重启 VS Code 或重新加载 Claude Code 窗口，再发送一个简单问题。ModelPort 日志中应出现 `routing message request`。

## 常用命令

```bash
scripts/config-validate.sh
scripts/start.sh
scripts/status.sh
scripts/doctor.sh --upstream
scripts/provider-matrix.sh --model mimo-v2.5-pro
scripts/bench.sh
scripts/restart.sh
```

## API

### `GET /health`

不需要 token：

```bash
curl http://127.0.0.1:17878/health
```

### `GET /v1/models`

需要 token：

```bash
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/v1/models
```

### `POST /v1/messages`

非流式：

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

流式：

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

### `GET /metrics`

Prometheus 文本格式，需要 token：

```bash
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/metrics
```

当前指标包括：

- `modelport_uptime_seconds`
- `modelport_route_requests_total`
- `modelport_route_successes_total`
- `modelport_route_failures_total`
- `modelport_route_duration_ms_total`
- `modelport_message_requests_total`
- `modelport_message_successes_total`
- `modelport_message_failures_total`
- `modelport_message_duration_ms_total`

鉴权 header 支持：

```http
x-api-key: <MODELPORT_AUTH_TOKEN>
```

或：

```http
Authorization: Bearer <MODELPORT_AUTH_TOKEN>
```

管理后台使用账号登录，不再直接使用网关 token。第一个管理员会从 `MODELPORT_ADMIN_USERNAME` 和 `MODELPORT_ADMIN_PASSWORD` 初始化；如果没有配置后台密码，ModelPort 仅为了本地迁移会回退使用 `MODELPORT_AUTH_TOKEN` 作为初始密码。

## 模型切换

直接设置模型：

```bash
export ANTHROPIC_MODEL=mimo-v2.5-pro
export ANTHROPIC_MODEL=deepseek-v4-pro
export ANTHROPIC_MODEL=qwen-plus
```

强制指定 provider：

```bash
export ANTHROPIC_MODEL=mimo:mimo-v2.5-pro
export ANTHROPIC_MODEL=openrouter:anthropic/claude-sonnet-4
export ANTHROPIC_MODEL=gemini:gemini-2.5-flash
export ANTHROPIC_MODEL=custom:any-model-name-from-your-upstream
```

配置别名：

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

`openrouter`、`custom`、`ollama` 最适合未知模型透传和任意模型热切换。

## 本地 OpenAI-compatible 运行时

基于 SGLang、vLLM、llama.cpp、Ollama 或其他 OpenAI-compatible server 部署的本地模型，都可以接入 ModelPort。把 provider 指向本地运行时的 `/v1` base URL 即可；如果本地服务没有鉴权，保持 `api_key_required = false`。模型名建议使用 `/v1/models` 暴露的真实 served model ID；对于微调部署，这通常是微调后的 served name，而不只是基座模型系列名。

```toml
[providers.local_vllm]
display_name = "Local vLLM"
protocol = "openai-compat"
base_url = "http://127.0.0.1:8000/v1"
api_key_required = false
default_model = "local-model"
models = ["local-model"]
passthrough_unknown_models = true
max_tokens_field = "max_tokens"
fidelity_mode = "best_effort"

[aliases]
local = "local_vllm:local-model"
```

常见 base URL：SGLang 通常是 `http://127.0.0.1:30000/v1`，vLLM 通常是 `http://127.0.0.1:8000/v1`，llama.cpp 的 OpenAI-compatible server 通常是 `http://127.0.0.1:8080/v1`。如果你希望遇到无法表达的 Anthropic 特性时直接报错，再把该 provider 设为 `fidelity_mode = "strict"`。

模型发现、served name 判断和本地运行时排障详见 [docs/LOCAL_RUNTIME.md](docs/LOCAL_RUNTIME.md)。

这些本地运行时也已经作为可选内置 provider 注册。例如：

```bash
export MODELPORT_ENABLE_LOCAL_VLLM=1
export VLLM_BASE_URL=http://127.0.0.1:8000/v1
export VLLM_MODEL=qwen2.5-coder
export ANTHROPIC_MODEL=local_vllm:qwen2.5-coder
```

## Provider

| Provider | 协议 | 关键环境变量 |
| --- | --- | --- |
| `mimo` | OpenAI-compatible | `BASE_URL`, `MIMO_OPENAI_BASE_URL`, `MIMO_OPENAI_API_KEY`, `MIMO_MODEL` |
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
| `local_sglang` | OpenAI-compatible | `MODELPORT_ENABLE_LOCAL_SGLANG`, `SGLANG_BASE_URL`, `SGLANG_MODEL` |
| `local_vllm` | OpenAI-compatible | `MODELPORT_ENABLE_LOCAL_VLLM`, `VLLM_BASE_URL`, `VLLM_MODEL` |
| `local_llamacpp` | OpenAI-compatible | `MODELPORT_ENABLE_LOCAL_LLAMACPP`, `LLAMACPP_BASE_URL`, `LLAMACPP_MODEL` |

Mimo 已完成真实 baseline 验证。其他 provider 配置已内置，但需要真实 key 跑过 `scripts/provider-matrix.sh` 后再标记为 verified，详见 [docs/PROVIDER_MATRIX.md](docs/PROVIDER_MATRIX.md)。

## 配置文件

默认不需要配置文件，环境变量即可。需要固定 provider、别名或优先级时：

```bash
mkdir -p ~/.config/modelport
cp config.example.toml ~/.config/modelport/config.toml
```

也可以指定其他路径：

```bash
MODELPORT_CONFIG=/path/to/config.toml model-port config validate
```

真实密钥建议继续通过环境变量传入，不建议写死在 `config.toml`。

关键 provider 字段：

- `provider_order`：前缀匹配优先级。
- `models`：显式模型名。
- `model_prefixes`：模型名前缀匹配。
- `passthrough_unknown_models`：未知模型是否透传。
- `max_tokens_field`：OpenAI-compatible token 字段策略。
- `deduplicate_stream_text`：处理会重放文本片段的流式上游，Mimo 默认开启。
- `buffer_stream_text`：把不稳定上游流转换为稳定下游 SSE，Mimo 默认开启。
- `fidelity_mode`：`strict` 拒绝有损的 Anthropic-to-OpenAI-compatible 转换，`best_effort` 保持兼容优先，`stability` 允许为不稳定上游改写流式文本。
- `[aliases]`：模型别名，可指向 provider、模型名或 `provider:model`。

服务级变量：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `MODELPORT_BIND` | `127.0.0.1:17878` | 监听地址。生产建议保持本机地址或放在反向代理之后。 |
| `MODELPORT_MAX_REQUEST_BODY_BYTES` | `33554432` | 单请求体大小上限。 |
| `MODELPORT_MAX_CONCURRENT_REQUESTS` | `64` | 并发请求上限。 |
| `MODELPORT_HTTP_CONNECT_TIMEOUT_SECS` | `10` | 上游连接超时。 |
| `MODELPORT_HTTP_REQUEST_TIMEOUT_SECS` | `600` | 非流式请求总超时。 |
| `MODELPORT_HTTP_STREAM_IDLE_TIMEOUT_SECS` | `300` | 流式上游空闲超时。 |
| `MODELPORT_HTTP_MAX_RESPONSE_BYTES` | `33554432` | 非流式响应体和错误响应体上限。 |
| `MODELPORT_INCLUDE_UNAVAILABLE_PROVIDERS` | unset | 设置为 `1` 时展示未配置 key 的 provider。 |
| `MODELPORT_ALLOW_NO_AUTH` | unset | 仅隔离测试可设为 `1`，投产不要启用。 |
| `MODELPORT_TRUSTED_PROXIES` | `127.0.0.1,::1` | 只有这些代理来源的 `X-Forwarded-For` / `X-Real-IP` 会被信任。Docker 模板默认额外包含 `172.16.0.0/12`。 |
| `MODELPORT_ALLOWED_ORIGINS` | unset | 控制台写请求的额外允许 Origin，通常不需要设置。 |
| `MODELPORT_DISABLE_CSRF` | unset | 仅本地紧急调试可设为 `1`，投产不要启用。 |

## 长期运行

### 后台脚本

```bash
scripts/start.sh
scripts/status.sh
tail -f .modelport/model-port.log
```

### Docker Compose

```bash
cp deploy/docker/modelport.env.example .env
# 编辑 .env，填好 MODELPORT_AUTH_TOKEN、MODELPORT_ADMIN_PASSWORD 和 provider key
docker compose up -d --build
docker compose logs -f modelport
```

默认启动两个轻量容器：

- `modelport`：后端 API、路由、鉴权、控制面数据。
- `dashboard`：静态后台 UI，并反代 `/admin`、`/v1` 到后端。

默认只暴露本机端口：

- 后台：`http://127.0.0.1:5173`
- API：`http://127.0.0.1:17878/v1/messages`

控制面数据保存在 Docker named volume `modelport-data`。更多说明见 [docs/DOCKER.md](docs/DOCKER.md)。

试生产验收：

```bash
scripts/acceptance.sh
# 包含一次真实上游模型请求：
scripts/acceptance.sh --upstream
```

验收脚本会创建临时用户和 API Key，验证 IP/额度拒绝、审计和备份校验，并在结束时清理临时资源。更多说明见 [docs/ACCEPTANCE.md](docs/ACCEPTANCE.md)。

停止：

```bash
docker compose down
```

### systemd

```bash
scripts/build-release.sh
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

日志：

```bash
journalctl -u modelport -f
```

WSL 默认不一定启用 systemd。不可用时使用后台脚本或 `tmux`。

## 排查

推荐日志级别：

```bash
RUST_LOG=model_port=info,tower_http=info
```

更详细排查：

```bash
RUST_LOG=model_port=debug,tower_http=info
```

| 现象 | 含义 | 处理 |
| --- | --- | --- |
| 启动时报缺少 token | 没有设置 `MODELPORT_AUTH_TOKEN` 或 `ANTHROPIC_AUTH_TOKEN` | 设置长随机本地 token。 |
| `config validate` 报 placeholder | key 或 token 还是占位符 | 替换成真实值。 |
| `/v1/models` 返回 401 | 客户端 token 缺失或不一致 | 检查 `x-api-key` 和 `ANTHROPIC_AUTH_TOKEN`。 |
| 上游返回 `INVALID_API_KEY` | 已连到上游，但上游 key 错误 | 替换真实 `MIMO_OPENAI_API_KEY`。 |
| VS Code Claude 没走 ModelPort | 插件未读取环境变量或未重载 | 重启 VS Code，检查 `ANTHROPIC_BASE_URL`。 |
| 请求超时 | 上游或网络慢 | 先检查上游和线路，再调大超时。 |
| 流式返回 `event: error` | 上游流式请求失败，ModelPort 已转换错误 | 看错误 message 和 ModelPort 日志。 |
| 大请求返回 413 | 请求体超过限制 | 调大 `MODELPORT_MAX_REQUEST_BODY_BYTES`。 |

## 升级与回滚

升级前：

```bash
scripts/check.sh
scripts/config-validate.sh
scripts/doctor.sh --upstream
```

升级：

```bash
git pull
scripts/build-release.sh
scripts/restart.sh
```

systemd：

```bash
sudo install -m 0755 target/release/model-port /usr/local/bin/model-port
sudo systemctl restart modelport
journalctl -u modelport -f
```

回滚时切回上一版二进制或上一版 git commit，重新构建并重启。

## 脚本速查

| 脚本 | 作用 |
| --- | --- |
| `scripts/config-validate.sh` | 静态校验配置，不启动服务。 |
| `scripts/start.sh` | 构建 release 二进制并后台启动。 |
| `scripts/stop.sh` | 停止当前项目的 ModelPort 进程。 |
| `scripts/restart.sh` | 停止后重新后台启动。 |
| `scripts/status.sh` | 查看 PID、日志位置和 `/health` 状态。 |
| `scripts/doctor.sh` | 检查配置、服务、鉴权、VS Code settings 和关键端点。 |
| `scripts/doctor.sh --upstream` | 在 doctor 基础上验证真实 Mimo 上游回复。 |
| `scripts/provider-matrix.sh` | 验证指定模型的非流式和流式兼容性。 |
| `scripts/provider-matrix.sh --all` | 验证 `/v1/models` 中全部模型，会产生真实上游调用成本。 |
| `scripts/bench.sh` | 测量本机 `/health` 和 `/v1/models` 延迟。 |
| `scripts/bench.sh --upstream` | 测量真实 `/v1/messages` 上游延迟，会产生模型调用成本。 |
| `scripts/dev.sh` | 加载 `.env` 后前台 `cargo run`。 |
| `scripts/smoke-test.sh` | 验证本机网关和鉴权。 |
| `scripts/smoke-test.sh --upstream` | 验证真实上游模型回复。 |
| `scripts/build-release.sh` | 构建 `target/release/model-port`。 |
| `scripts/check.sh` | 运行 fmt、test、clippy。 |
| `scripts/install-deps-ubuntu.sh` | 在 Ubuntu / WSL 上安装基础依赖。 |

## 非目标

ModelPort 有意保持小而清晰：

- 不做聊天客户端。
- 不做云端模型聚合平台。
- 不做计费、额度、用户体系。
- 不实现模型推理，只做协议适配和本地路由。
- 不把图像 base64 混入 Claude Code 文本主链路。
- 不追求所有 provider native API，优先支持 Anthropic-compatible 和 OpenAI-compatible。
