# ModelPort Docker Compose

ModelPort 面向个人和小团队时，推荐保持轻量：

| 组件 | 默认 | 作用 |
| --- | --- | --- |
| `postgres` | 是 | PostgreSQL 16，保存控制面持久化状态；默认只在 Docker 内网监听，不占用宿主机 5432。 |
| `modelport` | 是 | Rust 后端、Anthropic-compatible API、控制面 API、鉴权、路由、日志和配额。 |
| `dashboard` | 是 | 静态后台 UI，并把 `/admin`、`/v1`、`/health`、`/metrics` 反代到后端。 |
| `modelport-postgres` | 是 | PostgreSQL 数据卷，保存用户、API Key、审计、用量和路由配置。 |
| `modelport-data` | 是 | 后端辅助数据卷，可放 CLI 备份文件；未配置数据库时会作为 JSON 文件存储位置。 |
| Caddy/Nginx | 否 | 只有需要局域网域名、HTTPS 或统一入口时再加。 |
| Prometheus | 否 | 已有 `/metrics`，如果你已有监控系统再接入即可。 |
| Redis/队列 | 否 | 当前控制面数据量小，不建议为个人/小团队默认引入。 |

## 启动

```bash
cp deploy/docker/modelport.env.example .env
nano .env
docker compose up -d --build
```

访问：

- 后台：`http://127.0.0.1:5173`
- API：`http://127.0.0.1:17878/v1/messages`

Claude Code / VS Code Claude 继续使用：

```bash
ANTHROPIC_BASE_URL=http://127.0.0.1:17878
ANTHROPIC_AUTH_TOKEN=<same-as-MODELPORT_AUTH_TOKEN>
ANTHROPIC_MODEL=mimo-v2.5-pro
```

## 日常命令

```bash
docker compose ps
docker compose logs -f modelport
docker compose logs -f dashboard
docker compose restart modelport
docker compose down
```

升级镜像但保留数据：

```bash
docker compose up -d --build
```

清理容器但保留数据：

```bash
docker compose down
```

连数据一起清理：

```bash
docker compose down -v
```

## 数据和备份

Docker Compose 默认启用内部 PostgreSQL，控制面数据保存在 named volume `modelport-postgres`。数据库服务没有 `ports` 映射，只在 Docker 内网通过 `postgres:5432` 给 `modelport` 使用，因此不会和宿主机已有的 PostgreSQL 端口冲突。

应用层使用一张 `modelport_state` 表保存两份 `jsonb` 状态：

| namespace | 内容 |
| --- | --- |
| `auth` | 管理用户、角色、状态、密码哈希。 |
| `control` | API Key 哈希、配额、用量、审计、路由配置、provider 测试记录。 |

如果不设置 `MODELPORT_DATABASE_URL`，ModelPort 会回退到文件存储：`/data/admin-auth.json` 和 `/data/control-plane.json`。从旧版文件存储升级到 PostgreSQL 时，如果数据库为空，启动时会自动导入这两个 JSON 文件。

后台“系统设置 -> 运维”可以导出脱敏控制面快照，适合排查和留档，但不用于完整恢复。

完整恢复使用 CLI 备份，里面包含密码哈希和 API Key 哈希等认证材料。请像保护密钥一样保护该文件：

```bash
docker compose exec modelport model-port backup export /data/modelport-backup.json
docker compose exec modelport model-port backup validate /data/modelport-backup.json
```

恢复会先备份当前数据文件，再写入备份内容：

```bash
docker compose stop modelport dashboard
docker compose run --rm modelport model-port backup restore /data/modelport-backup.json --yes
docker compose up -d
```

也可以直接备份 volume：

```bash
docker run --rm \
  -v modelport_modelport-postgres:/var/lib/postgresql/data:ro \
  -v "$PWD":/backup \
  debian:bookworm-slim \
  tar czf /backup/modelport-postgres.tgz -C /var/lib/postgresql/data .
```

更推荐用 PostgreSQL 自带逻辑备份：

```bash
docker compose exec postgres pg_dump -U modelport modelport > modelport.sql
```

恢复到同一 compose 项目时：

```bash
docker compose exec -T postgres psql -U modelport modelport < modelport.sql
```

## PostgreSQL 端口冲突

默认 `postgres` 服务没有 `ports:`，所以不会绑定宿主机 `5432`，也不会影响你机器上已有的 PostgreSQL。

只有你需要从宿主机直接连这个数据库调试时，才建议临时加端口映射，并选择一个不冲突的端口，例如：

```yaml
postgres:
  ports:
    - "127.0.0.1:15432:5432"
```

这样宿主机连 `127.0.0.1:15432`，容器内部仍然是 `postgres:5432`。

## 访问范围

默认 compose 只发布到本机：

```env
MODELPORT_API_PUBLISH=127.0.0.1:17878
MODELPORT_DASHBOARD_PUBLISH=127.0.0.1:5173
```

如果要给局域网访问，改成：

```env
MODELPORT_API_PUBLISH=0.0.0.0:17878
MODELPORT_DASHBOARD_PUBLISH=0.0.0.0:5173
```

对外网或跨网络访问，建议放在 Caddy/Nginx 后面，并启用 HTTPS。`deploy/docker/Caddyfile.example` 提供了最小反代示例。

## 可信代理和控制台写保护

ModelPort 默认只信任来自本机代理的 `X-Forwarded-For` / `X-Real-IP`。Docker 模板额外设置：

```env
MODELPORT_TRUSTED_PROXIES=127.0.0.1,::1,172.16.0.0/12
```

这是为了让 dashboard 容器反代到后端时仍能保留真实客户端 IP。如果你把 API 直接暴露到局域网，并且不希望信任整个 Docker bridge 网段，可以改成更精确的反代容器 IP 或自建反代 IP。

控制台写操作要求前端带 `X-ModelPort-CSRF`，并校验 `Origin` / `Referer` 是否与当前 Host 匹配。常规 dashboard 使用不需要额外配置。只有反代改写 Host 导致不匹配时，才需要：

```env
MODELPORT_ALLOWED_ORIGINS=https://modelport.example.com
```

`MODELPORT_DISABLE_CSRF=1` 只建议本地紧急调试使用。

## 本机模型运行时

容器内的 `127.0.0.1` 指向容器自己。如果要连接宿主机上的 Ollama、vLLM、SGLang 或自定义 OpenAI-compatible 服务，用：

```env
OLLAMA_BASE_URL=http://host.docker.internal:11434/v1
CUSTOM_OPENAI_BASE_URL=http://host.docker.internal:8000/v1
```

`docker-compose.yml` 已配置 `host.docker.internal:host-gateway`。

## 为什么暂时不加 Redis 或队列

ModelPort 当前主要是单机个人/小团队网关，PostgreSQL 已经足够保存控制面状态：

- 部署和备份仍然简单。
- 单机并发足够。
- 不增加额外缓存一致性和队列消费成本。

只有当你需要多实例横向扩容、异步计费或复杂审计留存时，再考虑 Redis/队列/对象存储。
