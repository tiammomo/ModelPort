# 本地 Qwen 参考适配（兼容性文档）

本文只保留原始 `local-inference-stack` 集成的可选 Linux/WSL2 复现路径，供
现有部署迁移和兼容性回归使用。它不是 ModelPort 的安装前提、模型/GPU
事实来源或架构依赖。新的本地运行时必须遵循
[ADR-0007](adr/0007-independent-model-and-gpu-control-plane.md) 定义的通用
Runtime Adapter 边界，不能依赖本文中的仓库目录、脚本或环境变量。
通用 v1alpha1 capability 契约和离线验证方式见
[Runtime Adapter Capability Contract](RUNTIME_ADAPTER.md)。

当前 v0.1.x 仍只把本地 Qwen 暴露为一个 OpenAI-compatible Provider；尚未交付
Compute Inventory 或 Deployment 生命周期 API。下面的兼容性命令先验证配置
契约，再分别启动两个项目，最后才做真实生成。两个仓库可以放在任意目录；所有
命令都显式传路径，不依赖相邻 checkout。

## 先分清职责

```text
Claude Code / SDK
        |
        | ModelPort API Key，Anthropic/OpenAI-compatible
        v
    ModelPort :38082
        | 认证、路由、Token 准入、Tool 协议、账本
        |
        | Docker DNS: qwen-runtime:8080/v1
        v
 external Qwen runtime integration
        | 当前参考实现：模型制品、llama.cpp、GPU、Profile、验收证据
        v
   Qwen3.5 GGUF
```

ModelPort v0.1.x 不下载或运行模型；参考 Runtime 不签发客户端密钥、不执行业务
工具。未来 ModelPort 将通过 Runtime Adapter 管理 desired state、observed
inventory 和执行证据，实际推理仍由外部 Runtime 完成。宿主机
`127.0.0.1:18080` 只用于直接诊断；容器间调用不经过这个端口。

## 第 1 阶段：5 分钟只读检查

在 Linux/WSL2 Shell 中设置两个真实路径：

```bash
export MODELPORT_PROJECT_DIR=/path/to/ModelPort
export LOCAL_INFERENCE_STACK_DIR=/path/to/local-inference-stack
```

先查看硬件、制品和当前资源是否满足部署条件。这个命令不下载、不启动，也不改
GPU 状态：

```bash
cd "$LOCAL_INFERENCE_STACK_DIR"
./scripts/model-manager.py plan --json
```

重点看 `readyToDeploy`、`resourceAvailableNow`、固定 revision、SHA256、许可证和
`caveats`。`readyToDeploy=false` 时停在这里，处理资源或审批问题，不绕过准入。

先校验 ModelPort 自有的通用 capability 和 Qwen 参考 Fixture：

```bash
cd "$MODELPORT_PROJECT_DIR"
./scripts/runtime-adapter-check.sh --json
```

成功结果包含 `"valid":true`，且不读取另一个仓库。仅为回归原始联合部署时，
再显式进入已弃用的跨仓库兼容检查：

```bash
./scripts/local-inference-check.sh \
  --stack-dir "$LOCAL_INFERENCE_STACK_DIR"
```

成功标准是 `Compatibility check passed`。它检查 Provider、容器内地址、模型、
逻辑别名、Reasoning、Tool Use、精确 Token 计数、混合路由头和 40 人队列基线，以及各档输入/输出限制；不会
请求模型。

默认请求不写路由头也会按 `local_strict` 处理。仅当项目策略已批准云端，客户端才可用
`x-modelport-hybrid-mode: local_first` 或 `balanced`；`unknown` / `sensitive` 分类无论何种
请求头都不会离开本地。后台任务显式发送 `x-modelport-traffic-class: batch`。

## 第 2 阶段：准备 ModelPort

全新本地 Qwen 配置可以从维护的示例开始：

```bash
cd "$MODELPORT_PROJECT_DIR"
cp deploy/docker/modelport.env.example .env
cp deploy/local-inference/modelport.local-qwen.toml config.toml
```

如果已经有 `config.toml`，不要覆盖；把示例中的 `local_qwen`、三个逻辑别名和
`token_counting` 段合并进去。编辑 `.env`：

```env
MODELPORT_DEFAULT_PROVIDER=local_qwen
QWEN_LOCAL_BASE_URL=http://qwen-runtime:8080/v1
ANTHROPIC_BASE_URL=http://127.0.0.1:38082
ANTHROPIC_MODEL=qwen3.5-code
```

同时替换 ModelPort、管理员和 PostgreSQL 的 placeholder 密码。Qwen-only 配置
不需要 DeepSeek Key；客户端只拿 ModelPort Key。

```bash
./scripts/doctor.sh --setup
./scripts/runtime-adapter-check.sh
```

这两个命令仍然不启动服务、不生成内容。

## 第 3 阶段：受控启动

只有 local-inference-stack 的计划返回 `readyToDeploy=true`，且操作者明确批准
下载/选择/启动后，才按照该仓库的首次部署指南处理模型。为避免共享网络的启动
顺序歧义，先启动 ModelPort 基础栈，再启动推理 Runtime：

```bash
cd "$MODELPORT_PROJECT_DIR"
./scripts/build-container.sh
./scripts/compose-up.sh

cd "$LOCAL_INFERENCE_STACK_DIR"
# 按该仓库计划输出和审批流程下载、选择后：
./scripts/runtime.sh start latency
```

不要用裸 `docker compose up` 启动推理容器；`runtime.sh` 会执行制品、Profile、
主机准入和共享网络检查。

## 第 4 阶段：从健康到真实验收

先做不生成内容的检查：

```bash
cd "$LOCAL_INFERENCE_STACK_DIR"
./scripts/runtime.sh status
curl --noproxy '*' -fsS http://127.0.0.1:18080/health

cd "$MODELPORT_PROJECT_DIR"
./scripts/smoke-test.sh
curl --noproxy '*' -fsS http://127.0.0.1:38082/livez
```

明确要占用本地 GPU 做联合验收后，再运行：

```bash
cd "$LOCAL_INFERENCE_STACK_DIR"
MODELPORT_PROJECT_DIR="$MODELPORT_PROJECT_DIR" \
  ./scripts/acceptance-suite.sh standard
```

`standard` 包含真实生成、Reasoning、长上下文、Token 计数和 Tool Use 路径，不是
静态检查。正式联合发布再使用 `local-inference-check.sh --release`，它还会要求
两个仓库干净且 ModelPort commit 与部署清单一致。

## 三个逻辑档位

| 逻辑模型 | 默认思考预算 | 推荐输入工作集 | 最大输出 |
| --- | ---: | ---: | ---: |
| `qwen3.5-fast` | 关闭 / 512 | 24,576 | 4,096 |
| `qwen3.5-code` | 4,096 | 57,344 | 16,384 |
| `qwen3.5-deep` | 16,384 | 94,208 | 32,768 |

三档共享同一份权重，不会增加显存占用。ModelPort 在进入推理 Slot 前执行精确
Token 计数；超出逻辑档位或 131,072 硬上下文时返回可操作的 4xx，不静默截断。

## 最短排障顺序

1. 联合检查失败：只修复第一个 `FAIL`，不要同时改两个仓库的多项配置。
2. `readyToDeploy=false`：回到 `plan --json` 的 `caveats`，不要启动 Runtime。
3. `qwen-runtime` 无法解析：检查两个容器是否都连接
   `modelport_default`，以及 Runtime 的网络别名。
4. `18080/health` 成功但 `38082/livez` 失败：问题在 ModelPort 进程或 Compose。
5. `/livez` 成功但请求被拒：检查 ModelPort Key、逻辑模型和返回的 Token 准入信息。

在这个历史参考流程中，`local-inference-stack` 的契约文件只用于验证该适配
Fixture，不是 ModelPort 的跨仓库事实来源。v1alpha1 当前只定义 capability，
不代表库存、认证传输或生命周期 API 已交付。影响历史 Fixture 的接口、模型、
Reasoning、Token 限制或 Tool Use 变化仍需同步更新兼容测试和 `standard` 验收；
新的核心功能不得读取该仓库的内部文件。
