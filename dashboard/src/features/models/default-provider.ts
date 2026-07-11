import type { Provider, SystemSettings } from '@/types'

export function resolveDefaultProvider(
  configuredDefault: string | undefined,
  providers: readonly Provider[],
): string {
  const authoritative = configuredDefault?.trim()
  if (authoritative) return authoritative
  return providers.find((provider) => provider.status === 'active')?.id
    ?? providers[0]?.id
    ?? ''
}

export function withDefaultProvider(
  settings: SystemSettings,
  providerId: string,
): SystemSettings {
  if (settings.gateway.defaultProvider === providerId) return settings
  return {
    ...settings,
    gateway: {
      ...settings.gateway,
      defaultProvider: providerId,
    },
  }
}
