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
  const method = options.method || 'GET'
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...(options.headers as Record<string, string>),
  }
  if (!['GET', 'HEAD'].includes(method.toUpperCase())) {
    headers['X-ModelPort-CSRF'] = headers['X-ModelPort-CSRF'] || '1'
  }

  const response = await fetch(`${BASE_URL}${path}`, {
    ...options,
    headers,
    credentials: 'include',
  })

  if (response.status === 401) {
    if (window.location.pathname !== '/login') {
      window.location.href = '/login'
    }
    throw new Error('Unauthorized')
  }

  if (!response.ok) {
    const error = await response.json().catch(() => ({ message: response.statusText }))
    const message = error.error?.message || error.message || `HTTP ${response.status}`
    const hint = error.error?.hint
    throw new ApiError(hint ? `${message} · ${hint}` : message, response.status, error)
  }

  if (response.status === 204) return undefined as T
  const text = await response.text()
  if (!text) return undefined as T
  return JSON.parse(text)
}

export const api = {
  get: <T>(path: string) => request<T>(path),
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'POST', body: body ? JSON.stringify(body) : undefined }),
  put: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'PUT', body: body ? JSON.stringify(body) : undefined }),
  delete: <T = void>(path: string, body?: unknown) =>
    request<T>(path, { method: 'DELETE', body: body ? JSON.stringify(body) : undefined }),
}
