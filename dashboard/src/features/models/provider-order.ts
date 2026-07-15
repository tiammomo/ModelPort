export type ProviderOrderDirection = 'up' | 'down'

/**
 * Keep the persisted order authoritative while making newly-added providers
 * visible and removing stale or duplicate identifiers.
 */
export function normalizeProviderOrder(
  persistedOrder: readonly string[] | undefined,
  providerIds: readonly string[],
): string[] {
  const availableIds = new Set(providerIds)
  const orderedIds: string[] = []
  const seen = new Set<string>()

  for (const providerId of persistedOrder ?? []) {
    if (!availableIds.has(providerId) || seen.has(providerId)) continue
    orderedIds.push(providerId)
    seen.add(providerId)
  }

  for (const providerId of providerIds) {
    if (seen.has(providerId)) continue
    orderedIds.push(providerId)
    seen.add(providerId)
  }

  return orderedIds
}

export function moveProviderInOrder(
  order: readonly string[],
  providerId: string,
  direction: ProviderOrderDirection,
): string[] {
  const currentIndex = order.indexOf(providerId)
  if (currentIndex < 0) return [...order]

  const targetIndex = direction === 'up' ? currentIndex - 1 : currentIndex + 1
  if (targetIndex < 0 || targetIndex >= order.length) return [...order]

  const nextOrder = [...order]
  ;[nextOrder[currentIndex], nextOrder[targetIndex]] = [nextOrder[targetIndex], nextOrder[currentIndex]]
  return nextOrder
}
