export const isMockMode = import.meta.env.VITE_MODELPORT_MOCK === '1'

export function cloneMock<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

export function mockDelay<T>(value: T, ms = 120): Promise<T> {
  return new Promise((resolve) => {
    window.setTimeout(() => resolve(cloneMock(value)), ms)
  })
}

export function nextMockId(prefix: string) {
  return `${prefix}_${Date.now().toString(36)}`
}
