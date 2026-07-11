import { clearSessionQueries } from '@/lib/query-client'

const BASE_URL = import.meta.env.VITE_API_BASE_URL || ''

export class ApiError extends Error {
  status: number
  payload: unknown

  constructor(message: string, status: number, payload: unknown) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.payload = payload
  }
}

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const method = (options.method || 'GET').toUpperCase()
  const headers = new Headers(options.headers)
  if (options.body != null && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json')
  }
  if (!['GET', 'HEAD'].includes(method) && !headers.has('X-ModelPort-CSRF')) {
    headers.set('X-ModelPort-CSRF', '1')
  }

  const response = await fetch(`${BASE_URL}${path}`, {
    ...options,
    headers,
    credentials: 'include',
  })

  const text = response.status === 204 ? '' : await response.text()
  const payload = parsePayload(text)

  if (response.status === 401 && typeof window !== 'undefined') {
    if (window.location.pathname !== '/login') {
      clearSessionQueries()
      try {
        window.sessionStorage.setItem('modelport_auth_notice', '会话已过期，请重新登录后继续。')
        window.sessionStorage.setItem(
          'modelport_return_to',
          `${window.location.pathname}${window.location.search}${window.location.hash}`,
        )
      } catch {
        // A hard redirect still isolates in-memory state when storage is unavailable.
      }
      window.location.href = '/login'
    }
  }

  if (!response.ok) {
    const error = isRecord(payload) ? payload : {}
    const detail = isRecord(error.error) ? error.error : {}
    const message = stringValue(detail.message)
      || stringValue(error.message)
      || response.statusText
      || `HTTP ${response.status}`
    const hint = stringValue(detail.hint)
    throw new ApiError(hint ? `${message} · ${hint}` : message, response.status, payload)
  }

  if (!text) return undefined as T
  if (payload === undefined) {
    throw new ApiError('服务器返回了无效的 JSON 响应', response.status, text)
  }
  return payload as T
}

function parsePayload(text: string): unknown {
  if (!text) return undefined
  try {
    return JSON.parse(text)
  } catch {
    return undefined
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function stringValue(value: unknown): string | undefined {
  return typeof value === 'string' && value ? value : undefined
}

export const api = {
  get: <T>(path: string) => request<T>(path),
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'POST', body: body === undefined ? undefined : JSON.stringify(body) }),
  put: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'PUT', body: body === undefined ? undefined : JSON.stringify(body) }),
  delete: <T = void>(path: string, body?: unknown) =>
    request<T>(path, { method: 'DELETE', body: body === undefined ? undefined : JSON.stringify(body) }),
}
