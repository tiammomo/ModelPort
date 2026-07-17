import type { FidelityMode, MaxTokensField, ProviderProtocol, ToolUseCapabilities } from '@/types'

export interface ProviderTemplate {
  id: string
  displayName: string
  family: string
  protocol: ProviderProtocol
  baseUrl: string
  baseUrlEnv: string
  apiKeyEnv: string
  apiKeyRequired: boolean
  defaultModel: string
  models: string[]
  modelPrefixes: string[]
  passthroughUnknownModels: boolean
  maxTokensField: MaxTokensField
  deduplicateStreamText?: boolean
  bufferStreamText?: boolean
  fidelityMode?: FidelityMode
  toolUse?: ToolUseCapabilities
  notes: string
}

export const MODEL_FAMILIES = [
  'OpenAI',
  'Claude',
  'DeepSeek',
  'Gemini',
  'Qwen',
  'Kimi',
  'GLM',
  'Grok',
  'Llama',
  'Mistral',
  'Doubao',
  'Mimo',
  'Local',
  'Custom',
] as const

export const PROVIDER_TEMPLATES: ProviderTemplate[] = [
  {
    id: 'mimo',
    displayName: '小米 MiMo',
    family: 'Mimo',
    protocol: 'openai-compat',
    baseUrl: 'https://api.xiaomimimo.com/v1',
    baseUrlEnv: 'MIMO_OPENAI_BASE_URL',
    apiKeyEnv: 'MIMO_OPENAI_API_KEY',
    apiKeyRequired: true,
    defaultModel: 'mimo-v2.5-pro',
    models: ['mimo-v2.5-pro'],
    modelPrefixes: ['mimo-'],
    passthroughUnknownModels: false,
    maxTokensField: 'max_completion_tokens',
    toolUse: {
      supported: true,
      toolChoice: true,
      parallelToolCalls: true,
      streamingArguments: 'delta',
    },
    notes: '小米 Mimo 官方模型渠道；GPT 系列应配置到 OpenAI 或自定义 OpenAI-compatible 渠道。',
  },
  {
    id: 'deepseek',
    displayName: 'DeepSeek 官方 Anthropic',
    family: 'DeepSeek',
    protocol: 'anthropic',
    baseUrl: 'https://api.deepseek.com/anthropic',
    baseUrlEnv: 'DEEPSEEK_ANTHROPIC_BASE_URL',
    apiKeyEnv: 'DEEPSEEK_ANTHROPIC_AUTH_TOKEN',
    apiKeyRequired: true,
    defaultModel: 'deepseek-v4-flash',
    models: ['deepseek-v4-pro', 'deepseek-v4-flash', 'deepseek-chat', 'deepseek-reasoner'],
    modelPrefixes: ['deepseek-'],
    passthroughUnknownModels: false,
    maxTokensField: 'max_tokens',
    toolUse: {
      supported: true,
      toolChoice: true,
      parallelToolCalls: true,
      streamingArguments: 'native',
    },
    notes: 'DeepSeek 官方 Anthropic 协议渠道，适合 Claude Code 直连语义。',
  },
  {
    id: 'deepseek_openai',
    displayName: 'DeepSeek 官方 OpenAI 兼容',
    family: 'DeepSeek',
    protocol: 'openai-compat',
    baseUrl: 'https://api.deepseek.com',
    baseUrlEnv: 'DEEPSEEK_OPENAI_BASE_URL',
    apiKeyEnv: 'DEEPSEEK_OPENAI_API_KEY',
    apiKeyRequired: true,
    defaultModel: 'deepseek-v4-flash',
    models: ['deepseek-v4-pro', 'deepseek-v4-flash', 'deepseek-chat', 'deepseek-reasoner'],
    modelPrefixes: ['deepseek-'],
    passthroughUnknownModels: false,
    maxTokensField: 'max_tokens',
    toolUse: {
      supported: true,
      toolChoice: true,
      parallelToolCalls: true,
      streamingArguments: 'delta',
    },
    notes: '同一 DeepSeek 模型的 OpenAI 兼容渠道，可和 Anthropic 渠道并存。',
  },
  {
    id: 'openai',
    displayName: 'OpenAI 官方上游',
    family: 'OpenAI',
    protocol: 'openai-compat',
    baseUrl: 'https://api.openai.com/v1',
    baseUrlEnv: 'MODELPORT_OPENAI_BASE_URL',
    apiKeyEnv: 'MODELPORT_OPENAI_API_KEY',
    apiKeyRequired: true,
    defaultModel: 'gpt-5.5',
    models: ['gpt-5.5', 'gpt-5.5-pro', 'gpt-5.4', 'gpt-5.4-pro', 'gpt-5.4-mini', 'gpt-5.4-nano', 'gpt-5.3-codex', 'gpt-5.2', 'gpt-5', 'gpt-5-mini', 'gpt-4.1', 'gpt-4.1-mini'],
    modelPrefixes: ['gpt-', 'o1', 'o3', 'o4', 'o5', 'chatgpt-'],
    passthroughUnknownModels: false,
    maxTokensField: 'max_completion_tokens',
    notes: 'ModelPort 服务端的官方 OpenAI 上游；客户端连接网关时仍使用标准 OPENAI_* 变量。',
  },
  {
    id: 'anthropic',
    displayName: 'Anthropic Claude',
    family: 'Claude',
    protocol: 'anthropic',
    baseUrl: 'https://api.anthropic.com',
    baseUrlEnv: 'ANTHROPIC_UPSTREAM_BASE_URL',
    apiKeyEnv: 'ANTHROPIC_API_KEY',
    apiKeyRequired: true,
    defaultModel: 'claude-fable-5',
    models: ['claude-fable-5', 'claude-mythos-5', 'claude-opus-4-8', 'claude-opus-4-7', 'claude-sonnet-4-6', 'claude-sonnet-4-5', 'claude-haiku-4-5', 'claude-opus-4-20250514', 'claude-sonnet-4-20250514', 'claude-3-5-haiku-20241022'],
    modelPrefixes: ['claude-'],
    passthroughUnknownModels: false,
    maxTokensField: 'max_tokens',
    notes: '官方 Anthropic 渠道；Claude 5 与 Claude 4.8 适合高能力任务。',
  },
  {
    id: 'openrouter',
    displayName: 'OpenRouter',
    family: 'Custom',
    protocol: 'openai-compat',
    baseUrl: 'https://openrouter.ai/api/v1',
    baseUrlEnv: 'OPENROUTER_BASE_URL',
    apiKeyEnv: 'OPENROUTER_API_KEY',
    apiKeyRequired: true,
    defaultModel: 'openrouter/auto',
    models: ['openrouter/auto', 'anthropic/claude-fable-5', 'anthropic/claude-opus-4.8', 'anthropic/claude-sonnet-4.6', 'openai/gpt-5.5', 'deepseek/deepseek-v4-flash', 'google/gemini-3.5-flash', 'qwen/qwen-plus'],
    modelPrefixes: ['anthropic/', 'deepseek/', 'google/', 'meta-llama/', 'mistralai/', 'moonshotai/', 'openai/', 'qwen/', 'x-ai/', 'z-ai/'],
    passthroughUnknownModels: true,
    maxTokensField: 'max_completion_tokens',
    notes: '聚合渠道，适合快速验证第三方供应商和跨模型路由。',
  },
  {
    id: 'gemini',
    displayName: 'Google Gemini OpenAI 兼容',
    family: 'Gemini',
    protocol: 'openai-compat',
    baseUrl: 'https://generativelanguage.googleapis.com/v1beta/openai',
    baseUrlEnv: 'GEMINI_OPENAI_BASE_URL',
    apiKeyEnv: 'GEMINI_API_KEY',
    apiKeyRequired: true,
    defaultModel: 'gemini-3.5-flash',
    models: ['gemini-3.5-flash', 'gemini-3.5-pro', 'gemini-2.5-pro', 'gemini-2.5-flash', 'gemini-2.5-flash-lite'],
    modelPrefixes: ['gemini-'],
    passthroughUnknownModels: false,
    maxTokensField: 'max_completion_tokens',
    notes: 'Google Gemini 的 OpenAI 兼容入口，适合多模态与长上下文模型。',
  },
  {
    id: 'dashscope',
    displayName: '阿里云百炼 Qwen',
    family: 'Qwen',
    protocol: 'openai-compat',
    baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    baseUrlEnv: 'DASHSCOPE_BASE_URL',
    apiKeyEnv: 'DASHSCOPE_API_KEY',
    apiKeyRequired: true,
    defaultModel: 'qwen-plus',
    models: ['qwen-plus', 'qwen-max', 'qwen-turbo', 'qwen3-plus', 'qwen3-max', 'qwq-plus', 'qvq-max'],
    modelPrefixes: ['qwen-', 'qwq-', 'qvq-'],
    passthroughUnknownModels: false,
    maxTokensField: 'max_tokens',
    notes: '阿里云百炼 OpenAI 兼容渠道，适合 Qwen / QwQ / QvQ 系列。',
  },
  {
    id: 'kimi',
    displayName: 'Moonshot Kimi',
    family: 'Kimi',
    protocol: 'openai-compat',
    baseUrl: 'https://api.moonshot.cn/v1',
    baseUrlEnv: 'KIMI_BASE_URL',
    apiKeyEnv: 'MOONSHOT_API_KEY',
    apiKeyRequired: true,
    defaultModel: 'kimi-k2.6',
    models: ['kimi-k2.6', 'kimi-k2', 'moonshot-v1-128k', 'moonshot-v1-32k', 'moonshot-v1-8k'],
    modelPrefixes: ['kimi-', 'moonshot-'],
    passthroughUnknownModels: false,
    maxTokensField: 'max_completion_tokens',
    notes: '月之暗面 Kimi 渠道，适合长上下文中文场景。',
  },
  {
    id: 'zhipu',
    displayName: '智谱 GLM',
    family: 'GLM',
    protocol: 'openai-compat',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    baseUrlEnv: 'ZHIPU_BASE_URL',
    apiKeyEnv: 'ZHIPU_API_KEY',
    apiKeyRequired: true,
    defaultModel: 'glm-4.7',
    models: ['glm-4.7', 'glm-4.6', 'glm-4-flash', 'glm-z1-flash'],
    modelPrefixes: ['glm-', 'charglm-', 'codegeex-'],
    passthroughUnknownModels: false,
    maxTokensField: 'max_tokens',
    notes: '智谱 GLM 渠道，适合中文、工具调用和低成本场景。',
  },
  {
    id: 'custom',
    displayName: '自定义 OpenAI 兼容',
    family: 'Custom',
    protocol: 'openai-compat',
    baseUrl: 'http://127.0.0.1:8000/v1',
    baseUrlEnv: 'CUSTOM_OPENAI_BASE_URL',
    apiKeyEnv: 'CUSTOM_OPENAI_API_KEY',
    apiKeyRequired: false,
    defaultModel: 'default',
    models: ['default'],
    modelPrefixes: [],
    passthroughUnknownModels: true,
    maxTokensField: 'max_completion_tokens',
    fidelityMode: 'best_effort',
    notes: '自定义 OpenAI 兼容渠道，适合代理、私有部署或聚合网关。',
  },
  {
    id: 'local_sglang',
    displayName: 'Local SGLang',
    family: 'Local',
    protocol: 'openai-compat',
    baseUrl: 'http://127.0.0.1:30000/v1',
    baseUrlEnv: 'SGLANG_BASE_URL',
    apiKeyEnv: 'SGLANG_API_KEY',
    apiKeyRequired: false,
    defaultModel: 'local-model',
    models: ['local-model'],
    modelPrefixes: [],
    passthroughUnknownModels: true,
    maxTokensField: 'max_tokens',
    fidelityMode: 'best_effort',
    notes: '本地 SGLang OpenAI-compatible server；模型名按启动参数或 /v1/models 返回值填写。',
  },
  {
    id: 'local_vllm',
    displayName: 'Local vLLM',
    family: 'Local',
    protocol: 'openai-compat',
    baseUrl: 'http://127.0.0.1:8000/v1',
    baseUrlEnv: 'VLLM_BASE_URL',
    apiKeyEnv: 'VLLM_API_KEY',
    apiKeyRequired: false,
    defaultModel: 'local-model',
    models: ['local-model'],
    modelPrefixes: [],
    passthroughUnknownModels: true,
    maxTokensField: 'max_tokens',
    fidelityMode: 'best_effort',
    notes: '本地 vLLM OpenAI-compatible server；如果启动时指定 served-model-name，这里填同一个名字。',
  },
  {
    id: 'local_llamacpp',
    displayName: 'Local llama.cpp',
    family: 'Local',
    protocol: 'openai-compat',
    baseUrl: 'http://127.0.0.1:8080/v1',
    baseUrlEnv: 'LLAMACPP_BASE_URL',
    apiKeyEnv: 'LLAMACPP_API_KEY',
    apiKeyRequired: false,
    defaultModel: 'local-model',
    models: ['local-model'],
    modelPrefixes: [],
    passthroughUnknownModels: true,
    maxTokensField: 'max_tokens',
    fidelityMode: 'best_effort',
    notes: '本地 llama.cpp OpenAI-compatible server；适合 GGUF 模型。',
  },
]

export function guessModelFamily(model: string): string {
  const value = model.toLowerCase()
  if (value.includes('claude') || value.startsWith('anthropic/')) return 'Claude'
  if (
    value.startsWith('gpt-')
    || value.startsWith('o1')
    || value.startsWith('o3')
    || value.startsWith('o4')
    || value.startsWith('o5')
    || value.startsWith('chatgpt-')
    || value.startsWith('codex-')
    || value.includes('-codex')
    || value.startsWith('openai/')
  ) return 'OpenAI'
  if (value.includes('deepseek')) return 'DeepSeek'
  if (value.includes('gemini') || value.startsWith('google/')) return 'Gemini'
  if (value.includes('qwen') || value.startsWith('qwq-') || value.startsWith('qvq-')) return 'Qwen'
  if (value.includes('kimi') || value.includes('moonshot')) return 'Kimi'
  if (value.startsWith('glm-') || value.includes('z-ai/')) return 'GLM'
  if (value.includes('grok') || value.includes('x-ai/')) return 'Grok'
  if (value.includes('llama') || value.includes('meta-llama/')) return 'Llama'
  if (value.includes('mistral') || value.includes('codestral')) return 'Mistral'
  if (value.includes('doubao')) return 'Doubao'
  if (value.includes('mimo')) return 'Mimo'
  return 'Custom'
}

export function providerToml(template: ProviderTemplate): string {
  const lines = [
    `[providers.${template.id}]`,
    `display_name = "${template.displayName}"`,
    `protocol = "${template.protocol}"`,
    `base_url_env = "${template.baseUrlEnv}"`,
    `base_url = "${template.baseUrl}"`,
    `api_key_env = "${template.apiKeyEnv}"`,
  ]

  if (!template.apiKeyRequired) {
    lines.push('api_key_required = false')
  }

  lines.push(
    `default_model = "${template.defaultModel}"`,
    `models = [${template.models.map((model) => `"${model}"`).join(', ')}]`,
  )

  if (template.modelPrefixes.length > 0) {
    lines.push(`model_prefixes = [${template.modelPrefixes.map((prefix) => `"${prefix}"`).join(', ')}]`)
  }

  lines.push(
    `passthrough_unknown_models = ${template.passthroughUnknownModels}`,
    `max_tokens_field = "${template.maxTokensField}"`,
  )

  if (template.deduplicateStreamText) {
    lines.push('deduplicate_stream_text = true')
  }

  if (template.bufferStreamText) {
    lines.push('buffer_stream_text = true')
  }

  if (template.fidelityMode) {
    lines.push(`fidelity_mode = "${template.fidelityMode}"`)
  }

  if (template.toolUse) {
    lines.push(
      '',
      `[providers.${template.id}.tool_use]`,
      `supported = ${template.toolUse.supported}`,
      `tool_choice = ${template.toolUse.toolChoice}`,
      `parallel_tool_calls = ${template.toolUse.parallelToolCalls}`,
      `streaming_arguments = "${template.toolUse.streamingArguments}"`,
    )
  }

  return lines.join('\n')
}

export function providerEnv(template: ProviderTemplate): string {
  const keyLine = template.apiKeyRequired
    ? `export ${template.apiKeyEnv}=sk-...`
    : `export ${template.apiKeyEnv}=optional`

  return [
    `export ${template.baseUrlEnv}=${template.baseUrl}`,
    keyLine,
    `export MODELPORT_ENABLE_${template.id.toUpperCase()}=1`,
  ].join('\n')
}
