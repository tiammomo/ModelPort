const OIDC_ERROR_MESSAGES: Record<string, string> = {
  access_denied: '企业单点登录已取消或未获授权，请重试。',
  account_disabled: '当前账户已停用，请联系管理员。',
  account_not_allowed: '当前企业账户无权访问控制台，请联系管理员。',
  invalid_state: '单点登录会话已失效，请重新发起登录。',
  state_mismatch: '单点登录会话已失效，请重新发起登录。',
  oidc_unavailable: '企业单点登录暂不可用，请稍后重试或联系管理员。',
  provider_unavailable: '企业单点登录暂不可用，请稍后重试或联系管理员。',
  account_not_authorized: '当前企业账户无权访问控制台，请联系管理员。',
  invalid_callback: '单点登录回调无效，请重新发起登录。',
  token_exchange_failed: '企业身份验证未能完成，请重试或联系管理员。',
  token_invalid: '企业身份验证无效，请重新发起登录。',
  provider_error: '企业身份提供方返回错误，请重试或联系管理员。',
}

const GENERIC_OIDC_ERROR = '企业单点登录失败，请重试或联系管理员。'

export function safeReturnPath(value: string | null | undefined): string {
  if (!value || !value.startsWith('/') || /^\/[\\/]/.test(value) || value.startsWith('/login')) return ''
  return value
}

export function buildOidcStartUrl(startUrl: string, returnTo: string, origin: string): string {
  const normalizedStartUrl = startUrl.trim()
  if (normalizedStartUrl.startsWith('//')) {
    throw new Error('OIDC start URL must be same-origin')
  }

  const absoluteStartUrl = new URL(normalizedStartUrl, origin)
  if (absoluteStartUrl.protocol !== 'http:' && absoluteStartUrl.protocol !== 'https:') {
    throw new Error('Unsupported OIDC start URL protocol')
  }
  if (absoluteStartUrl.origin !== new URL(origin).origin) {
    throw new Error('OIDC start URL must be same-origin')
  }
  if (absoluteStartUrl.pathname !== '/admin/auth/oidc/start') {
    throw new Error('Unexpected OIDC start URL path')
  }

  absoluteStartUrl.searchParams.set('returnTo', returnTo)

  const startUrlIsAbsolute = /^[a-z][a-z\d+.-]*:/i.test(normalizedStartUrl)
  if (startUrlIsAbsolute) return absoluteStartUrl.toString()
  return `${absoluteStartUrl.pathname}${absoluteStartUrl.search}${absoluteStartUrl.hash}`
}

export function oidcErrorMessage(search: string): string {
  const rawCode = new URLSearchParams(search).get('oidc_error')
  if (rawCode === null) return ''
  const code = rawCode.trim().toLowerCase()
  return OIDC_ERROR_MESSAGES[code] || GENERIC_OIDC_ERROR
}

export function withoutOidcError(search: string): string {
  const params = new URLSearchParams(search)
  params.delete('oidc_error')
  const remaining = params.toString()
  return remaining ? `?${remaining}` : ''
}
