# 40 人团队生产基线（第一阶段）

本页是普通运维人员的执行入口。路由与 40 人基线以
[ADR-0005](adr/0005-forty-user-hybrid-routing-baseline.md) 为准；独立的模型、
Runtime Adapter、Compute Node/GPU 与 Deployment 所有权以
[ADR-0007](adr/0007-independent-model-and-gpu-control-plane.md) 为准。

本页生产基线假定启用 `MODELPORT_ENTERPRISE_MODE=1`，因此高风险写入强制双人
审批。默认 Small-Team 模式未启用 Enterprise 或
`MODELPORT_REQUIRE_DUAL_APPROVAL=1` 时，管理员可在 CSRF 防护和审计记录下直接
执行，治理变更单仍可自愿使用。

## 现在是什么状态

第一阶段仍然只有：

- 1 个 ModelPort 实例；
- 1 个本地 Qwen GPU 节点；
- 审核通过后才能接入的云 Provider；
- 1 个 Dashboard 应用，严格分为用户自助视图与管理员治理控制台。

当前不能宣称 ModelPort 高可用。40 人并发准入、四种路由模式、项目预算硬限制、双人审批
和生产级 Service Account 已进入稳定 API 与自动化验收；生产数据库切换、密钥轮换和真实
云 Provider 开通仍必须取得第二名管理员批准并在维护窗口执行。

## 已实现的请求基线

- 每用户最多 1 个本地执行、2 个本地排队；全局交互队列 16；
- `local_first` / `balanced` 预计等待超过 5 秒才会溢出到项目批准的云端；
- `local_strict` 永不外发，最多等待 60 秒，超时返回带 `Retry-After` 的 429；
- `batch` 使用独立的低优先级队列；
- 未分类和敏感数据强制 `local_strict`；客户端只能收紧项目最大模式；
- Provider、区域、API 版本和模型采用组织目录与项目子集双重白名单，禁止任意 URL；
- 本页 Enterprise 基线下，高风险变更的载荷先做 SHA-256 摘要，必须由两名不同
  管理员批准。

全新企业库首次启动必须同时提供 `MODELPORT_ADMIN_*` 与
`MODELPORT_BACKUP_ADMIN_*` 两组不同账号，系统在一次持久化写入中创建 Owner 和 Backup，
避免单管理员无法批准新增 Backup 的死锁。数据库已有用户时不会再次引导或覆盖账号。

Linux/WSL2 中运行不产生真实模型请求的容量基线：

```bash
./scripts/capacity-acceptance.sh
```

## 第一阶段已经建立的保护

### 1. 数据库更新先检查

```bash
./scripts/database-preflight.sh
```

它只读取版本、数据卷、迁移状态和状态行数量，不打印数据库密码。运行版本或数据卷与
Compose 声明不一致时必须停止，不能继续执行整栈 `docker compose up`。

日常本地 Compose 更新统一使用：

```bash
./scripts/compose-up.sh
```

数据库迁移使用 [PostgreSQL 迁移手册](POSTGRESQL_MIGRATION.md)，不能把 PostgreSQL 18
直接指向 PostgreSQL 16 的物理数据目录。

### 2. 新备份不再打包明文密钥

```bash
archive="$(./scripts/backup-compose.sh create)"
./scripts/backup-compose.sh verify "$archive"
./scripts/backup-compose.sh drill "$archive"
./scripts/backup-compose.sh upgrade-drill "$archive"
```

新的 schema-v2 归档只包含 PostgreSQL 自定义格式 Dump、校验和与部署来源信息，不包含
`.env` 和 `config.toml`。配置从 Git 恢复，密钥从 Secret Manager 恢复。

旧 schema-v1 归档仍能验证，但工具会警告它包含明文运行配置。旧归档必须按密钥材料
限制访问，并在完成凭证轮换和保留期审批后再删除。

### 3. 单实例生产模板不自带数据库

[`deploy/production/compose.single.yml`](../deploy/production/compose.single.yml)
只启动一个 ModelPort 和 Dashboard，生产数据库必须使用外部托管 PostgreSQL。运行环境
由 Secret Manager 写入仓库外、权限 `0600` 的短期文件；生产配置禁止挂载项目 `.env`。

该模板目前用于评审和迁移演练。在托管数据库、TLS CA、镜像 Digest、密钥注入与回滚
演练全部准备好以前，不要把现有服务切换到这个模板。

正式渲染前运行 `scripts/production-preflight.sh`。它会检查镜像 Digest、短期密钥文件、
数据库严格 TLS、OIDC、组织 Owner/Backup/值守文件，以及 `config.toml` 引用的 Provider
凭证，但不会打印任何密钥值。

## 投产前必须完成

- [ ] 选定平台 Owner 和 Backup，确定维护窗口与回滚负责人。
- [ ] 轮换曾出现在终端或旧备份中的 ModelPort、Provider、数据库凭证。
- [x] PostgreSQL 16 备份已在隔离 PostgreSQL 18.4 完成逻辑恢复演练。
- [ ] 确认托管 PostgreSQL `verify-full`、PITR、RPO 5 分钟、RTO 30 分钟。
- [ ] 使用固定 Digest 的 ModelPort 与 Dashboard 镜像。
- [ ] 运行 `scripts/check-all.sh`，并确保 CI 的 ShellCheck 门禁通过。
- [ ] 运行 ModelPort Provider/Tool Use 验收及每个已配置 Runtime Adapter 的验收；
  现有 Qwen 参考部署可继续使用可选的 `local-inference-stack standard` 兼容套件。
- [x] 运行 `scripts/capacity-acceptance.sh`，确认 40 人准入不变量。
- [ ] 保存不含 Prompt、回复、工具参数和密钥的验收证据。

## 第一阶段不做什么

- 不启动第二个 ModelPort；
- 不在没有维护窗口时迁移真实数据库；
- 不在仓库脚本中实现 Secret Manager；
- 不让 ModelPort 执行 Shell、数据库查询或业务工具；
- 不允许用户配置任意 OpenAI-compatible URL；
- 不绕过本页 Enterprise 基线强制的双人审批执行数据库、密钥、外发、身份或生产
  模型变更。

在本页 Enterprise 基线中，Dashboard 的“治理与审批”页用于提交、第二人审批和应用项目策略/预算；Provider、身份、
数据库、密钥和模型晋级在审批后仍由专用 Runbook 执行。对 Provider、身份与模型等已有
Dashboard 操作，第二人批准后点击“用于下一次专用操作”，随后 Dashboard 自动携带审批 ID；
API 会再次核对动作、目标和完整载荷摘要，不匹配时拒绝执行。
