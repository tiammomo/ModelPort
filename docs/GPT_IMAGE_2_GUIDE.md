# GPT Image 2 扩展指导

这份文档用于指导 ModelPort 后续接入 `gpt-image-2`。当前 ModelPort 的主路径是 Claude Code / VS Code Claude 使用的 Anthropic-compatible `/v1/messages` 文本协议；`gpt-image-2` 应作为独立图像能力扩展，不建议混入 Claude Code 文本中转主链路。

## README 图示策略

README 里的项目含义图和快速上手图采用“图像模型出方向稿，仓库提交可编辑 SVG”的方式维护：

- `docs/assets/modelport-overview.svg`：解释 ModelPort 在 Claude Code、VS Code Claude 和上游 provider 之间的位置。
- `docs/assets/modelport-quickstart.svg`：解释从 `.env` 到 VS Code Claude 连通 Mimo 的四步流程。

这种方式比直接引用外部生成图片更稳：GitHub 能长期渲染，文字可搜索，后续改模型名、端口、provider 时可以直接编辑 SVG。

图示维护原则：

- 只展示已经实现或已经验证的能力，不把路线图能力画成已完成能力。
- 单行文字要短，代码片段要放在足够宽的区域内，避免 GitHub README 缩放后遮挡。
- 优先使用克制的浅色技术图，不使用外部 logo、复杂截图或密集小字。

可用于 `gpt-image-2` 重新生成方向稿的 prompt：

```text
Create a clean technical README architecture infographic for an open source developer tool called "ModelPort". Wide 16:9 aspect ratio, modern GitHub README style, crisp readable labels, generous spacing. Visual concept: VS Code Claude / Claude Code on the left sends Anthropic Messages API to a local gateway box labeled "ModelPort" in the center, then ModelPort routes to configured providers on the right: Mimo v2.5 Pro, DeepSeek, Anthropic, OpenAI-compatible, OpenRouter, Ollama / Custom. Show only implemented or validated ideas as small badges: Local token auth, Model routing, Protocol transform, Streaming SSE, Mimo text de-dup, Health and metrics. Use a professional light theme with white background, deep teal and graphite accents, small orange highlight for Mimo. Avoid clutter, no fake UI screenshots, no tiny unreadable text, no unsupported fallback claim, no external logos.
```

```text
Create a clean GitHub README quick-start infographic for a developer tool named "ModelPort". Wide 16:9 but simple and readable. Show four numbered steps in a spacious 2x2 grid: 1 Prepare env, 2 Validate config, 3 Start gateway, 4 Connect Claude. Include short command snippets only: cp .env.example .env, BASE_URL=https://w.ciykj.cn/v1, scripts/config-validate.sh, scripts/start.sh, ANTHROPIC_MODEL=mimo-v2.5-pro, ANTHROPIC_BASE_URL=http://127.0.0.1:17878. Use professional light theme, teal and graphite accents, orange highlight for Mimo. Add status chips: health ok, auth ok, Mimo upstream ok, stream ok. Keep every text line short enough to remain readable on GitHub. No fake logos, no clutter, no tiny text.
```

## 官方能力边界

根据 OpenAI 官方模型页，`gpt-image-2` 是图像生成和编辑模型，支持文本和图片输入、图片输出，并可用于 Images generation 与 Images edit 端点。官方模型页同时标明它不支持 streaming、function calling、structured outputs 或 fine-tuning。

官方图像生成指南说明，图像能力可以通过两条路径使用：

- Image API：适合单次 prompt 生成或编辑一张/多张图片。
- Responses API：适合多轮、可编辑、上下文式图片体验。

参考：

- OpenAI GPT Image 2 model page: https://developers.openai.com/api/docs/models/gpt-image-2
- OpenAI image generation guide: https://developers.openai.com/api/docs/guides/image-generation
- OpenAI Images API reference: https://developers.openai.com/api/reference/resources/images

## 对 ModelPort 的建议定位

短期不要让 Claude Code 的 `/v1/messages` 直接承载图片生成，因为：

- Claude Code 当前工作流主要期待 Anthropic Messages 文本/SSE。
- GPT Image 输出通常是 base64 图片，响应体比文本大很多。
- 图像生成没有文本 token 流式体验，不能复用当前 Anthropic SSE 文本转换逻辑。
- 图像编辑涉及 multipart/form-data、文件引用、mask 和更大的请求体限制。

推荐把图像能力设计成独立模块：

```text
src/providers/openai_images.rs
src/routes/images.rs
POST /v1/images/generations
POST /v1/images/edits
```

## 建议配置形态

未来可以增加：

```toml
[providers.openai_images]
display_name = "OpenAI GPT Image"
protocol = "openai-images"
base_url_env = "OPENAI_BASE_URL"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
default_model = "gpt-image-2"
models = ["gpt-image-2", "gpt-image-2-2026-04-21"]
```

环境变量：

```bash
OPENAI_API_KEY=replace-with-openai-api-key
OPENAI_BASE_URL=https://api.openai.com/v1
OPENAI_IMAGE_MODEL=gpt-image-2
MODELPORT_IMAGE_MAX_REQUEST_BODY_BYTES=67108864
MODELPORT_IMAGE_MAX_RESPONSE_BYTES=134217728
```

## API 设计建议

第一阶段优先实现 Image API pass-through：

- `POST /v1/images/generations`
- `POST /v1/images/edits`

保留 OpenAI 原始响应结构，避免过早抽象：

- GPT Image 模型默认返回 base64 图片数据。
- 支持 `output_format=jpeg|png|webp`。
- 延迟敏感场景优先 `jpeg` 或 `webp`。
- `png` 更适合需要透明背景或无损输出的场景。

第二阶段再考虑 Responses API：

- `POST /v1/responses`
- 内置 `image_generation` tool。
- 支持多轮编辑和上下文图片体验。

## 安全和成本

图像能力上线前必须做这些限制：

- 不记录完整 prompt、图片 base64、multipart body 或文件内容。
- 对 request body 和 response body 设置比文本链路更高但明确的上限。
- 对 `n`、`size`、`quality`、`output_format` 做配置化 allowlist。
- 为图像接口单独设置并发限制，避免图片任务挤占 Claude Code 文本任务。
- README 明确说明图像调用会产生额外费用，并可能需要 OpenAI 组织验证。

## 验收清单

- 可以通过 `OPENAI_API_KEY` 调用 `gpt-image-2` 生成图片。
- 图片响应不会进入普通文本日志。
- 大图响应超过限制时返回清晰错误。
- 没有真实 key 或生成图片进入 git。
- Docker、systemd 和本机脚本都能保持文本主路径正常。
- `scripts/check.sh`、`scripts/doctor.sh` 仍然通过。
