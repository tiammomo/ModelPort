import { describe, expect, it } from 'vitest'
import { buildClientProfiles } from './client-profiles'

describe('buildClientProfiles', () => {
  it('normalizes URLs and substitutes the live model and one-time client key', () => {
    const profiles = buildClientProfiles({
      gatewayOrigin: ' https://gateway.example.test/// ',
      selectedModel: 'code-stable',
      oneTimeClientKey: 'sk-mp-one-time',
    })

    const claude = profiles.find((profile) => profile.id === 'claude-code')
    const openai = profiles.find((profile) => profile.id === 'openai-sdk')
    expect(claude?.status).toBe('supported')
    expect(claude?.configuration).toContain('ANTHROPIC_BASE_URL=https://gateway.example.test')
    expect(openai?.configuration).toContain('OPENAI_BASE_URL=https://gateway.example.test/v1')
    expect(openai?.configuration).toContain('OPENAI_MODEL=code-stable')
    expect(openai?.configuration).toContain('OPENAI_API_KEY=sk-mp-one-time')
  })

  it('keeps the Qwen key in the environment and out of settings.json', () => {
    const qwen = buildClientProfiles({
      gatewayOrigin: 'http://localhost:38082/',
      selectedModel: 'qwen3',
      oneTimeClientKey: 'sk-mp-one-time',
    })
      .find((profile) => profile.id === 'qwen-code')
    expect(qwen?.status).toBe('supported')
    if (!qwen || qwen.status !== 'supported') throw new Error('Qwen profile should be supported')
    const [environment, settingsSource] = qwen.configuration.split('# ~/.qwen/settings.json\n')
    expect(environment).toContain('MODELPORT_API_KEY=sk-mp-one-time')
    expect(settingsSource).not.toContain('sk-mp-one-time')

    const settings = JSON.parse(settingsSource) as {
      modelProviders: { openai: Array<{ id: string; baseUrl: string; envKey: string }> }
      security: { auth: { selectedType: string } }
      model: { name: string }
    }
    expect(settings.modelProviders.openai).toEqual([expect.objectContaining({
      id: 'qwen3',
      baseUrl: 'http://localhost:38082/v1',
      envKey: 'MODELPORT_API_KEY',
    })])
    expect(settings.security.auth.selectedType).toBe('openai')
    expect(settings.model.name).toBe('qwen3')
    expect(settings).not.toHaveProperty('modelProviders.modelport')
  })

  it('blocks Codex without exposing copyable configuration', () => {
    const codex = buildClientProfiles({ gatewayOrigin: 'https://gateway.example.test', selectedModel: 'code' })
      .find((profile) => profile.id === 'codex-cli')
    expect(codex).toMatchObject({ status: 'blocked', protocol: 'openai-responses' })
    expect(codex).not.toHaveProperty('configuration')
    if (codex?.status === 'blocked') {
      expect(codex.reason).toContain('POST /v1/responses')
      expect(codex.followUp).toContain('Responses ingress')
    }
  })

  it('uses placeholders when no one-time key or selectable model exists', () => {
    const supported = buildClientProfiles({ gatewayOrigin: 'http://localhost/' })
      .filter((profile) => profile.status === 'supported')
    expect(supported.every((profile) => profile.configuration.includes('<你的 ModelPort API Key>'))).toBe(true)
    expect(supported.every((profile) => profile.configuration.includes('<先选择可用模型>'))).toBe(true)
  })
})
