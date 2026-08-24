export type ClientProfileId = 'claude-code' | 'qwen-code' | 'openai-sdk' | 'codex-cli'
export type SupportedClientProfileId = Exclude<ClientProfileId, 'codex-cli'>

export type ClientProfile = SupportedClientProfile | BlockedClientProfile

export interface SupportedClientProfile {
  id: SupportedClientProfileId
  name: string
  protocol: 'anthropic-messages' | 'openai-chat-completions'
  status: 'supported'
  description: string
  configuration: string
}

export interface BlockedClientProfile {
  id: 'codex-cli'
  name: string
  protocol: 'openai-responses'
  status: 'blocked'
  description: string
  reason: string
  followUp: string
  configuration?: never
}

export interface BuildClientProfilesInput {
  gatewayOrigin: string
  selectedModel?: string
  oneTimeClientKey?: string
}

const CLIENT_KEY_PLACEHOLDER = '<你的 ModelPort API Key>'
const MODEL_PLACEHOLDER = '<先选择可用模型>'

function normalizeOrigin(origin: string) {
  return origin.trim().replace(/\/+$/, '')
}

export function buildClientProfiles({
  gatewayOrigin,
  selectedModel,
  oneTimeClientKey,
}: BuildClientProfilesInput): ClientProfile[] {
  const origin = normalizeOrigin(gatewayOrigin)
  const model = selectedModel || MODEL_PLACEHOLDER
  const clientKey = oneTimeClientKey || CLIENT_KEY_PLACEHOLDER

  return [
    {
      id: 'claude-code',
      name: 'Claude Code / Anthropic SDK',
      protocol: 'anthropic-messages',
      status: 'supported',
      description: 'Anthropic Messages 客户端直接连接 ModelPort。',
      configuration: `ANTHROPIC_BASE_URL=${origin}\nANTHROPIC_AUTH_TOKEN=${clientKey}\nANTHROPIC_MODEL=${model}`,
    },
    {
      id: 'qwen-code',
      name: 'Qwen Code',
      protocol: 'openai-chat-completions',
      status: 'supported',
      description: '使用 Qwen Code 的 OpenAI-compatible 客户端协议；密钥只放在环境变量中，不写入 settings.json。',
      configuration: `# 环境变量\nMODELPORT_API_KEY=${clientKey}\n\n# ~/.qwen/settings.json\n${JSON.stringify({
        modelProviders: {
          openai: [{
            id: model,
            name: model,
            description: 'ModelPort governed route',
            baseUrl: `${origin}/v1`,
            envKey: 'MODELPORT_API_KEY',
          }],
        },
        security: { auth: { selectedType: 'openai' } },
        model: { name: model },
      }, null, 2)}`,
    },
    {
      id: 'openai-sdk',
      name: 'OpenAI SDK',
      protocol: 'openai-chat-completions',
      status: 'supported',
      description: 'OpenAI-compatible Chat Completions 客户端连接 ModelPort。',
      configuration: `OPENAI_BASE_URL=${origin}/v1\nOPENAI_API_KEY=${clientKey}\nOPENAI_MODEL=${model}`,
    },
    {
      id: 'codex-cli',
      name: 'Codex CLI',
      protocol: 'openai-responses',
      status: 'blocked',
      description: '客户端兼容性状态；Codex CLI 不是 ModelPort Provider。',
      reason: 'Codex CLI 自定义 Provider 需要 Responses wire API，而 ModelPort 尚未提供 POST /v1/responses。',
      followUp: '待独立、限定范围的 Responses ingress 实现并验证后再提供配置。',
    },
  ]
}
