# ModelPort 快速学习路径

这份路线面向第一次接触模型网关、Rust 或运维平台的人员。目标不是一次读完
所有文档，而是在 Linux 环境中逐步取得四个可验证结果：

1. 能启动并判断服务是否健康；
2. 能区分客户端密钥、ModelPort 和 Provider；
3. 能通过日志、指标和备份完成基本运维；
4. 能修改一处代码并选择正确的测试。

## 先理解一张图

```text
Claude Code / SDK
        |
        | ModelPort API Key
        v
    ModelPort --------------> PostgreSQL
        |                  状态、请求、用量、审计
        |
        | Provider Key（只保存在服务端）
        v
 DeepSeek / Anthropic / OpenAI-compatible / 本地模型
```

Dashboard 是 ModelPort 的管理界面，不是聊天应用。ModelPort 负责鉴权、路由、
策略、协议转换和证据记录，不负责运行模型。

如果要复现可选的本机 Qwen 参考路径，请阅读
[本地 Qwen 参考适配](LOCAL_INFERENCE_STACK.md)。它把静态契约检查、外部 GPU
Runtime 和 ModelPort 分成可独立验证的阶段；ModelPort 不依赖该集成或其仓库。

## 路线 A：30 分钟启动服务

适合使用者、测试人员和首次部署人员。只需要 Git、Docker 和 Docker Compose
v2，不需要先学习 Rust。

```bash
git clone https://github.com/tiammomo/ModelPort.git
cd ModelPort
cp deploy/docker/modelport.env.example .env
cp config.example.toml config.toml
```

编辑 `.env`，替换全部必填 `replace-with-...` 值。然后先运行只读检查：

```bash
scripts/doctor.sh --setup
```

看到 `doctor (setup) passed` 后，当前首次发布前的 `main` 使用贡献者源码构建路径：

```bash
scripts/build-container.sh
scripts/compose-up.sh
docker compose ps
scripts/smoke-test.sh
```

成功标准：

- `postgres` 和 `modelport` 显示 healthy；
- `scripts/smoke-test.sh` 通过；
- 可以打开 `http://127.0.0.1:33002` 并登录；
- 到这里没有调用真实模型，不会消耗 Provider 额度。

如果失败，只看第一个 `[fail]`，修复后重新运行 doctor，不要同时修改多个配置。

## 路线 B：30 分钟接入第一个客户端

先记住两类密钥：

| 密钥 | 给谁使用 | 是否可以交给客户端 |
| --- | --- | --- |
| Provider Key | ModelPort 访问上游 Provider | 不可以 |
| ModelPort API Key | Claude Code、SDK 或内部应用访问 ModelPort | 可以，但应限制作用域 |

第一次请求前先查看模型目录：

```bash
source .env
curl -fsS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:38082/v1/models
```

明确接受可能产生 Provider 费用后，再执行：

```bash
scripts/smoke-test.sh --upstream
```

然后在 Dashboard 的 Request Logs 中找到本次请求，确认 Provider、模型、状态、
延迟和用量来源。不要把估算成本当成 Provider 的正式账单。

成功标准：

- 客户端只知道 ModelPort 地址和 ModelPort Key；
- Provider Key 只存在于 `.env` 或服务端密钥系统；
- 能在日志中把客户端请求对应到一次或多次 Provider attempt。

继续阅读 [API](API.md) 和 [Provider](PROVIDERS.md)。

## 路线 C：45 分钟掌握基本运维

按以下固定顺序检查，不要先翻完整日志：

```bash
docker compose ps
scripts/smoke-test.sh
scripts/doctor.sh
docker compose logs --tail=100 modelport
```

需要掌握的三个端点：

| 端点 | 含义 |
| --- | --- |
| `/livez` | 进程仍然存活 |
| `/readyz` | 数据库和关键状态可用，可以接收流量 |
| `/metrics` | Prometheus 格式运行指标 |

如果看到 `state_conflict`，说明另一个实例已经更新 auth/control 状态。当前写入
已被拒绝而不是覆盖新数据；重新加载最新状态，并检查陈旧实例的 `/readyz`。

在升级前完成一次备份演练：

```bash
archive="$(scripts/backup-compose.sh create)"
scripts/backup-compose.sh verify "$archive"
scripts/backup-compose.sh drill "$archive"
scripts/backup-compose.sh upgrade-drill "$archive"
scripts/database-preflight.sh
```

成功标准：

- 能解释 live 与 ready 的区别；
- 能在不打印密钥的情况下收集故障信息；
- 知道 `docker compose down` 保留数据，而 `down -v` 会删除卷；
- 能完成备份验证和隔离恢复演练。
- 知道数据库大版本或数据卷不一致时必须停止整栈更新并按迁移手册处理。

继续阅读 [运维](OPERATIONS.md) 和 [生产投产](PRODUCTION.md)。
准备团队共享部署时，再按
[40 人团队第一阶段生产基线](PRODUCTION_BASELINE_40_USERS.zh-CN.md) 完成迁移、密钥和
恢复门禁。

40 人团队的队列、混合路由和双人审批不需要先发真实请求即可学习：

```bash
scripts/capacity-acceptance.sh
```

普通用户在 Dashboard 只看到自助能力；管理员在“治理与审批”页创建高风险变更。提交人
自动成为第一审批人，必须由另一个管理员批准。自动化不要共享个人密钥，应签发最长
90 天、带用途说明且模型/Provider 作用域明确的服务账号。

## 路线 D：60 分钟开始贡献代码

开发命令必须运行在 Linux 或 WSL2 中，不要混用 `/mnt/c` 下的 Windows Node、
Git 或编译器。

先运行只读工具链检查：

```bash
scripts/doctor.sh --development
```

它会验证 Rust 版本、rustfmt、Clippy、Node.js 24、npm 和 Linux C 编译器。
如果使用 NVM，先在当前 shell 激活 Node.js 24。

安装 Dashboard 依赖并运行分层检查：

```bash
npm --prefix dashboard ci
scripts/check.sh
npm --prefix dashboard run check
```

提交前运行：

```bash
scripts/check-all.sh
```

学习代码时按请求流向阅读：

1. `src/routes/client_api.rs`：HTTP 请求入口；
2. `src/exchange.rs` 和 `src/types.rs`：协议中间表示与转换；
3. `src/providers/`：Provider 适配；
4. `src/enterprise_ledger.rs`：请求、attempt、预算和审计；
5. `src/auth.rs`、`src/control.rs`、`src/storage.rs`：控制面状态。

成功标准：

- 能说明修改属于路由、协议、Provider、账本还是控制面；
- 先运行最小相关测试，再运行完整检查；
- 不在日志、测试快照或提交中写入真实密钥、Prompt 或响应。

继续阅读 [开发指南](DEVELOPMENT.md) 和 [架构](ARCHITECTURE.md)。

## 常见概念速查

| 概念 | 简单解释 |
| --- | --- |
| Provider | 真正处理模型请求的上游服务或本地运行时 |
| Gateway request | 客户端向 ModelPort 发出的一次请求 |
| Provider attempt | ModelPort 对某个 Provider 的一次实际尝试；回退时可能有多个 |
| Route | 请求选择 Provider 和模型的规则 |
| Quota | 对用户或密钥的用量限制 |
| Budget reservation | 发请求前预占预算，完成后结算或释放 |
| CAS/revision | 只有读取的版本仍是最新版本时才允许写入 |
| SSE | 流式响应使用的事件格式 |

## 最短排障决策

1. `doctor --setup` 失败：先修 Linux、Docker、文件或 placeholder。
2. 容器不健康：查看对应服务最近 100 行日志。
3. `/livez` 成功但 `/readyz` 失败：优先检查 PostgreSQL、迁移和状态 revision。
4. 返回 401/403：检查使用的是 ModelPort Key、账号状态和策略。
5. 返回 429：检查本地限流、并发限制、配额或预算。
6. 上游错误：在 Dashboard 先确认 Provider/credential 健康，再决定是否执行付费测试。

完整故障处理以 [Operations](OPERATIONS.md) 为准。
