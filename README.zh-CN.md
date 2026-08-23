# ModelPort

[![CI](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml)
[![CodeQL](https://github.com/tiammomo/ModelPort/actions/workflows/codeql.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/codeql.yml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/tiammomo/ModelPort/badge)](https://scorecard.dev/viewer/?uri=github.com/tiammomo/ModelPort)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[English](README.md) | **简体中文**

ModelPort v0.1.x 是面向 20–50 人内部研发团队的免费 MIT 开源、自托管 LLM
网关，聚焦本地模型与获批云 Provider 的受治理混合使用。Claude Code、SDK 和
内部应用可以通过一个入口统一获得鉴权、逻辑模型路由、配额、用量、Provider
健康和请求证据。Small-Team Beta 以中文产品体验为优先，同时维护英文 API 与
运维文档。

已经批准的长期方向是独立的混合模型与 GPU 控制平面：托管 API Provider
保持一等能力，本地 Qwen 只是可替换的 Runtime Adapter 示例；v0.1.x 尚未实现的
Compute 和 Deployment API 不会被描述成已交付。边界见
[架构](docs/ARCHITECTURE.md)与
[ADR-0007](docs/adr/0007-independent-model-and-gpu-control-plane.md)。

![ModelPort architecture overview](docs/assets/modelport-overview.svg)

## 主要能力

- `POST /v1/messages`、`POST /v1/chat/completions`、`GET /v1/models` 和
  显式开启的精确 Token 计数。
- Anthropic 与 OpenAI-compatible Provider 适配、受限流式传输和 Tool Use
  转换。
- 可选的 CPA Codex/Claude 账号通道；CPA 只作为内部 Provider，统一受
  ModelPort 的策略、路由和证据边界管理。
- 确定性路由，以及支持 Shadow、稳定灰度和持久化决策证据的可解释智能路由。
- 有作用域的客户端 API Key、用户、团队、配额、消费控制、Provider 凭证池、
  冷却和受限回退。
- React 运维控制台和 PostgreSQL 请求、用量、预算与审计账本。
- 默认关闭的确定性只读运维 Agent：提供持久化事件中心、受限离线队列、恢复证据
  和通用 Webhook；可由管理员选择基础模型并优先推荐本地模型，但不执行 Shell、
  SQL 或自动改配置。
- Docker Compose、systemd、备份恢复、Prometheus 指标和验收脚本。

ModelPort 当前只正式支持 Linux x86_64 单实例、可信主机或小型可信网络。它
不宣称企业级或高可用，也不是公网多租户服务、模型运行时、聊天界面、支付
系统或 Provider 账单。扩大部署范围前请阅读[兼容矩阵](docs/COMPATIBILITY.md)、
[生产投产](docs/PRODUCTION.md)和[路线图](docs/ROADMAP.md)。

## 快速开始

要求：Linux x86_64、Git、Docker、Docker Compose v2，以及至少一个 Provider
凭证。`v0.1.0` 确实出现在 GitHub Releases 页面后，以下正式用户路径会直接
拉取预构建镜像，不编译 Rust 或控制台。默认示例使用 DeepSeek 的
Anthropic-compatible 接口。

```bash
git clone --branch v0.1.0 --depth 1 https://github.com/tiammomo/ModelPort.git
cd ModelPort
cp deploy/docker/modelport.env.example .env
cp config.example.toml config.toml
```

编辑 `.env`，替换所有必填的 `replace-with-...` 值。至少设置不同的路由器、
管理员、PostgreSQL 和 Provider 凭证。首次本地测试时，让
`MODELPORT_AUTH_TOKEN` 与客户端侧 `ANTHROPIC_AUTH_TOKEN` 保持一致。

```bash
export MODELPORT_COMPOSE_FILE="$PWD/deploy/release/compose.yml"
scripts/doctor.sh --setup
docker compose -f "$MODELPORT_COMPOSE_FILE" pull
scripts/compose-up.sh
docker compose -f "$MODELPORT_COMPOSE_FILE" ps
scripts/smoke-test.sh
```

在 `v0.1.0` 标签和 GHCR 镜像尚未真实发布前，上述 Release 命令会明确失败；
仓库代码变更本身不能假装外部发布已发生。发布前测试当前 `main` 或参与贡献时，
使用源码构建路径：

```bash
git clone https://github.com/tiammomo/ModelPort.git
cd ModelPort
cp deploy/docker/modelport.env.example .env
cp config.example.toml config.toml
# 替换必填 placeholder
export MODELPORT_COMPOSE_FILE="$PWD/docker-compose.yml"
scripts/build-container.sh
MODELPORT_LOCAL_BUILD=1 scripts/compose-up.sh
```

打开 `http://127.0.0.1:33002`，使用
`MODELPORT_ADMIN_USERNAME`/`MODELPORT_ADMIN_PASSWORD` 登录。

使用本地 Qwen、其他 Provider、生产加固或排障时，继续阅读经过验证的
[上手指南](docs/GETTING_STARTED.md)。
可选 Agent 请按[安全上线指南](docs/OPS_AGENT.md)先运行 Shadow，再切换只读事件
写入；它与 ModelPort 一样免费开源。
首个 Release 发布后，从源码构建镜像属于
[贡献者开发流程](docs/DEVELOPMENT.md)，不是普通安装步骤。

## 发送第一个请求

云端外发默认拒绝，必须先为请求所属项目落地显式策略。在控制台打开
**治理与变更审批**，选择 `project_policy.upsert`，将目标设为
`org_local/prj_default/env_default`，并记录下面这个最小示例策略：

```json
{
  "organizationId": "org_local",
  "projectId": "prj_default",
  "environmentId": "env_default",
  "maximumMode": "cloud_first",
  "defaultClassification": "unknown",
  "allowedProviders": ["deepseek"],
  "allowedModels": ["deepseek-v4-flash"],
  "allowedRegions": ["global"],
  "allowedApiVersions": ["anthropic-v1"],
  "cloudEnabled": true
}
```

填写明确的变更原因，提交后再应用。默认免费小团队模式允许同一管理员在
CSRF 和审计保护下应用这条已记录变更；启用 Enterprise 模式或
`MODELPORT_REQUIRE_DUAL_APPROVAL=1` 后，必须先由另一名管理员审批。
该边界只允许文档中的 DeepSeek 模型/API 路径；没有显式安全分类的请求仍只允许
本地执行。

```bash
source .env

curl -fsS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  -H 'content-type: application/json' \
  -H 'x-modelport-data-classification: public' \
  -H 'x-modelport-hybrid-mode: cloud_first' \
  http://127.0.0.1:38082/v1/messages \
  -d '{
    "model":"deepseek-v4-flash",
    "max_tokens":96,
    "messages":[{"role":"user","content":"Reply exactly: OK"}]
  }'
```

该请求可能消耗 Provider 额度。`scripts/smoke-test.sh` 只做本地检查；明确希望
发送付费合成请求时再使用 `scripts/smoke-test.sh --upstream`。

Claude Code：

```env
ANTHROPIC_BASE_URL=http://127.0.0.1:38082
ANTHROPIC_AUTH_TOKEN=<MODELPORT_AUTH_TOKEN>
ANTHROPIC_MODEL=deepseek-v4-flash
```

OpenAI-compatible SDK：

```env
OPENAI_BASE_URL=http://127.0.0.1:38082/v1
OPENAI_API_KEY=<ModelPort 客户端 API Key>
OPENAI_MODEL=deepseek-v4-flash
```

共享部署应使用控制台签发的有作用域客户端 API Key。Provider 密钥只保留在
ModelPort 服务端，不能复制到客户端应用。

## 文档

按任务选择文档，不需要通读全部内容：

- [上手指南](docs/GETTING_STARTED.md)：安装、首次登录、首次请求和启动排障。
- [快速学习路径](docs/LEARNING_PATH.zh-CN.md)：面向使用者、接入人员、运维和
  贡献者的 30–60 分钟分层课程。
- [本地 Qwen 参考适配](docs/LOCAL_INFERENCE_STACK.md)：原始集成的可选
  Linux/WSL2 兼容性流程，不构成 ModelPort 架构依赖。
- [配置参考](docs/CONFIGURATION.md)：环境变量和 TOML。
- [API 参考](docs/API.md)：客户端和控制面接口契约。
- [Provider](docs/PROVIDERS.md)：托管 Provider、本地运行时和兼容性证据。
- [智能路由](docs/SMART_ROUTING.md)：评分、Shadow、灰度和回滚。
- [部署](docs/DEPLOYMENT.md)：Docker Compose、systemd 和生产拓扑。
- [运维](docs/OPERATIONS.md)：健康、日志、指标、备份、保留策略、事故和升级。
- [兼容矩阵](docs/COMPATIBILITY.md)：Tier 1 平台及实验性/不支持边界。
- [告警处置手册](docs/OBSERVABILITY_RUNBOOK.md)：官方告警、Grafana 面板和
  事故处置。
- [升级与回滚](docs/UPGRADING.md)：安全停机、备份、迁移、验收及应用/数据库
  成对回滚。
- [生产投产](docs/PRODUCTION.md)：上线与发布验收。
- [开发](docs/DEVELOPMENT.md)：贡献者工作流和测试矩阵。
- [文档索引](docs/README.md)：按角色导航。

## 安全与支持

保持后端和 PostgreSQL 端口私有。共享部署应使用同源 HTTPS、精确可信代理
CIDR、安全 Cookie、CSRF 防护和控制台 API Key。不要提交 `.env`、Provider
密钥、备份、Prompt、响应或原始敏感日志。

请阅读[安全策略](SECURITY.md)、[隐私说明](PRIVACY.md)、
[支持政策](SUPPORT.md)和[项目治理](GOVERNANCE.md)。ModelPort 是免费自托管
软件；本项目不提供付费版本、托管服务或社区支持 SLA。

## 本地开发

```bash
cp .env.example .env
cp config.example.toml config.toml
# 替换必填 placeholder
scripts/start.sh

cd dashboard
npm ci
npm run dev
```

提交变更前：

```bash
scripts/check-all.sh
```

## 许可证

[MIT](LICENSE)
