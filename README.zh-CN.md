# ModelPort

[![CI](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml)
[![CodeQL](https://github.com/tiammomo/ModelPort/actions/workflows/codeql.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/codeql.yml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/tiammomo/ModelPort/badge)](https://scorecard.dev/viewer/?uri=github.com/tiammomo/ModelPort)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[English](README.md) | **简体中文**

ModelPort 是面向 20–50 人研发团队的免费、自托管模型网关。管理员接好本地或
云端模型、设好使用边界，开发者复制配置即可接入；出现问题时，可以按请求
查看权限、路由、用量和计费证据。项目采用 MIT 许可证。

![ModelPort architecture overview](docs/assets/modelport-overview.svg)

## 主要能力

- 一个入口连接 Claude Code、Qwen Code 和 OpenAI SDK，支持 Messages、
  Chat Completions、流式响应和 Tool Use。
- 用户、团队、受限 API Key、配额与预算；未知和敏感数据默认仅本地执行。
- 本地与云 Provider、模型目录、确定性路由及可选智能路由。
- 五个管理入口：概览、模型接入、请求与用量、团队与策略、系统。
- PostgreSQL 持久化证据、备份恢复、指标；运维 Agent 按需开启。

v0.1.x 为 Small-Team Beta，正式支持 Linux x86_64 单实例。协议与部署边界见
[兼容矩阵](docs/COMPATIBILITY.md)。当前投入优先解决团队接入和排障，GPU 扩展
按真实需求推进，见[路线图](docs/ROADMAP.md)。

## 快速开始

需要 Linux x86_64、Git、Docker Compose v2 和 Provider 凭证；本机无需 Rust 或 Node。

```bash
git clone https://github.com/tiammomo/ModelPort.git
cd ModelPort
scripts/setup.sh
# 编辑 .env，填写 DEEPSEEK_ANTHROPIC_AUTH_TOKEN
scripts/doctor.sh --setup
scripts/build-container.sh
scripts/compose-up.sh
docker compose ps
scripts/smoke-test.sh
```

初始化会生成独立的路由器、管理员和数据库密码，写入权限为 `0600` 的 `.env`；
再次运行会保留现有配置。默认构建网关和控制台两张镜像，并运行内置 PostgreSQL。
默认端口仅绑定本机。

打开 `http://127.0.0.1:33002`，使用 `.env` 中的管理员账号密码登录，点击
**继续接入**。四步引导会从已保存配置恢复进度。使用其他 Provider、外部数据库
或已发布镜像时，参见[上手指南](docs/GETTING_STARTED.md)。

## 发送第一个请求

按引导完成 **接入模型 → 设置外发边界 → 密钥与客户端 → 核对请求结果**。
项目策略通过表单记录并应用；选择云 Provider 后仍需明确授权外发和数据分类。
接入说明按实际密钥检查权限、凭证和策略，通过后才开放配置复制。

[首次请求示例](docs/GETTING_STARTED.md#7-send-the-first-request)使用明确分类的
合成数据，可能消耗 Provider 额度。普通 `smoke-test.sh` 不调用上游。完整调用
结果以请求日志为准，管理员可从日志直接打开实际路由与计费证据。Provider 密钥
始终留在服务端。

## 文档

- [上手与排障](docs/GETTING_STARTED.md)
- [配置](docs/CONFIGURATION.md)与 [API](docs/API.md)
- [部署](docs/DEPLOYMENT.md)、[运维](docs/OPERATIONS.md)与[升级回滚](docs/UPGRADING.md)
- [全部文档](docs/README.md)与[路线图](docs/ROADMAP.md)

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
