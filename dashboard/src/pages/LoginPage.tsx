import { useEffect, useState } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import { useAuthStore } from '@/stores'
import { authService, type AuthMethods } from '@/services/auth.service'
import {
  buildOidcStartUrl,
  oidcErrorMessage,
  safeReturnPath,
  withoutOidcError,
} from '@/features/auth/login-auth'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import {
  Eye,
  EyeOff,
  Fingerprint,
  KeyRound,
  Loader2,
  ShieldCheck,
  UserRound,
  Zap,
} from 'lucide-react'

function readSessionValue(key: string): string {
  try {
    return window.sessionStorage.getItem(key) || ''
  } catch {
    return ''
  }
}

export function LoginPage() {
  const navigate = useNavigate()
  const location = useLocation()
  const [username, setUsername] = useState(() => window.localStorage.getItem('modelport_last_username') || '')
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [capsLock, setCapsLock] = useState(false)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [oidcError, setOidcError] = useState(() => oidcErrorMessage(location.search))
  const [authMethods, setAuthMethods] = useState<AuthMethods | null>(null)
  const [sessionNotice] = useState(() => readSessionValue('modelport_auth_notice'))
  const [storedReturnTo] = useState(() => readSessionValue('modelport_return_to'))
  const login = useAuthStore((s) => s.login)
  const from = (location.state as { from?: { pathname?: string; search?: string; hash?: string } } | null)?.from
  const locationReturnTo = from?.pathname ? `${from.pathname}${from.search || ''}${from.hash || ''}` : ''
  const returnTo = safeReturnPath(locationReturnTo) || safeReturnPath(storedReturnTo) || '/dashboard'
  const passwordEnabled = authMethods?.passwordEnabled ?? true
  const oidcEnabled = authMethods?.oidc.enabled === true && !!authMethods.oidc.startUrl.trim()

  useEffect(() => {
    try {
      window.sessionStorage.removeItem('modelport_auth_notice')
      window.sessionStorage.removeItem('modelport_return_to')
    } catch {
      // Session storage can be unavailable in hardened browser contexts.
    }
  }, [])

  useEffect(() => {
    let active = true
    authService.getMethods()
      .then((methods) => {
        if (active) setAuthMethods(methods)
      })
      .catch(() => {
        // Keep password login available when capability discovery is temporarily unavailable.
      })
    return () => {
      active = false
    }
  }, [])

  useEffect(() => {
    const message = oidcErrorMessage(location.search)
    if (!message) return

    navigate({
      pathname: location.pathname,
      search: withoutOidcError(location.search),
      hash: location.hash,
    }, { replace: true, state: location.state })
  }, [location.hash, location.pathname, location.search, location.state, navigate])

  const handleOidcLogin = () => {
    if (!oidcEnabled || !authMethods) return
    try {
      window.location.assign(buildOidcStartUrl(authMethods.oidc.startUrl, returnTo, window.location.origin))
    } catch {
      setOidcError('企业单点登录暂不可用，请联系管理员。')
    }
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!username.trim() || !password) {
      setError('请输入用户名和密码')
      return
    }

    setLoading(true)
    setError('')

    try {
      await login(username.trim(), password)
      window.localStorage.setItem('modelport_last_username', username.trim())
      navigate(returnTo, { replace: true })
    } catch {
      setError('登录失败，请检查用户名或密码')
    } finally {
      setLoading(false)
    }
  }

  return (
    <main className="relative flex min-h-dvh items-center justify-center overflow-hidden bg-[#eef3f7] bg-[linear-gradient(to_right,rgba(15,23,42,0.045)_1px,transparent_1px),linear-gradient(to_bottom,rgba(15,23,42,0.04)_1px,transparent_1px)] [background-size:52px_52px] px-5 py-8 sm:px-8">
      <div className="pointer-events-none absolute inset-0 bg-[radial-gradient(circle_at_24%_18%,rgba(34,211,238,0.12),transparent_32%),radial-gradient(circle_at_82%_18%,rgba(14,165,233,0.10),transparent_30%),linear-gradient(180deg,rgba(255,255,255,0.72),rgba(226,232,240,0.72))]" />
      <Card className="relative grid h-auto min-h-[560px] w-full max-w-[1380px] overflow-hidden rounded-2xl border-slate-200/80 bg-[#f8fbfd] shadow-[0_32px_90px_rgba(15,23,42,0.16)] lg:min-h-[730px] lg:grid-cols-[1.42fr_0.88fr]">
        <div className="relative hidden min-h-[730px] overflow-hidden bg-[#f8f4ec] text-slate-950 lg:block">
          <img
            src="/login-gateway-hero-v3.png"
            alt=""
            aria-hidden="true"
            fetchPriority="high"
            decoding="async"
            className="absolute inset-0 h-full w-full object-cover object-center"
          />
          <div className="pointer-events-none absolute inset-0 bg-[linear-gradient(90deg,rgba(255,252,246,0.66)_0%,rgba(255,252,246,0.32)_42%,transparent_63%)]" />
          <div className="pointer-events-none absolute inset-x-0 bottom-0 h-32 bg-gradient-to-t from-[#fbf8f1]/90 to-transparent" />

          <div className="relative flex min-h-[730px] flex-col p-10 xl:p-12">
            <header className="flex items-start justify-between gap-6">
              <div className="flex items-center gap-3">
                <div className="flex h-12 w-12 items-center justify-center rounded-xl border border-teal-700/15 bg-white/75 text-teal-700 shadow-[0_12px_30px_rgba(15,118,110,0.10)] backdrop-blur-md">
                  <Zap className="h-6 w-6" />
                </div>
                <div>
                  <p className="text-lg font-semibold text-slate-950">ModelPort</p>
                  <p className="text-sm text-slate-600">Enterprise AI Gateway</p>
                </div>
              </div>
              <div className="rounded-full border border-teal-700/15 bg-white/65 px-3 py-1.5 text-[11px] font-semibold uppercase tracking-[0.16em] text-teal-800 backdrop-blur-md">
                control plane
              </div>
            </header>

            <section className="mt-11 max-w-[390px]">
              <p className="text-[11px] font-semibold uppercase tracking-[0.22em] text-teal-700">Route · Govern · Observe</p>
              <h2 className="mt-3 text-3xl font-semibold leading-[1.18] tracking-[-0.035em] text-slate-950 xl:text-[2.15rem]">
                模型自由接入，<br />
                <span className="bg-gradient-to-r from-teal-700 via-cyan-700 to-indigo-600 bg-clip-text text-transparent">治理始终统一。</span>
              </h2>
              <p className="mt-4 max-w-[340px] text-sm leading-6 text-slate-600">
                在同一条安全边界内连接客户端、策略与 Provider，让每一次模型调用可控、可查、可结算。
              </p>
            </section>

            <footer className="mt-auto flex flex-wrap items-center gap-x-4 gap-y-2 border-t border-slate-900/10 pt-4 text-[11px] font-medium text-slate-600">
              <span>OpenAI-compatible</span>
              <span className="h-1 w-1 rounded-full bg-teal-500" />
              <span>Anthropic-compatible</span>
              <span className="h-1 w-1 rounded-full bg-violet-500" />
              <span>Provider Governance</span>
            </footer>
          </div>
        </div>

        <div className="relative flex min-h-[560px] items-center justify-center border-l border-slate-200 bg-[#f8fbfd] px-7 py-10 sm:px-10 lg:min-h-[730px]">
          <div className="pointer-events-none absolute inset-0 bg-[linear-gradient(160deg,rgba(255,255,255,0.78),rgba(226,232,240,0.28)_54%,rgba(255,255,255,0.52))]" />
          <div className="relative w-full max-w-[360px] lg:-translate-y-3">
            <CardHeader className="items-center px-0 pb-7 text-center">
              <div className="mb-3.5 flex h-10 w-10 items-center justify-center rounded-xl border border-teal-200/60 bg-teal-50/80 text-teal-600 shadow-[0_8px_20px_rgba(13,148,136,0.08)]">
                <Zap className="h-5 w-5" />
              </div>
              <div className="space-y-1.5">
                <p className="text-[11px] font-semibold uppercase tracking-[0.18em] text-teal-700">ModelPort Console</p>
                <h1 className="text-2xl font-semibold tracking-[-0.025em] text-slate-950">登录控制台</h1>
                <p className="text-sm text-slate-500">使用管理员账户进入企业网关</p>
              </div>
            </CardHeader>

            <CardContent className="px-0">
              {sessionNotice && (
                <div role="status" className="mb-5 border-l-2 border-amber-400 bg-amber-50/65 px-3 py-2.5 text-xs leading-5 text-amber-800">
                  {sessionNotice}
                </div>
              )}
              {oidcError && (
                <div id="oidc-login-error" role="alert" aria-live="polite" className="mb-5 border-l-2 border-rose-400 bg-rose-50/70 px-3 py-2.5 text-xs leading-5 text-rose-700">
                  {oidcError}
                </div>
              )}
              {oidcEnabled && (
                <div className={passwordEnabled ? 'mb-5' : ''}>
                  <Button
                    type="button"
                    size="lg"
                    onClick={handleOidcLogin}
                    className="h-12 w-full border-indigo-600/80 bg-indigo-600 text-sm font-semibold text-white shadow-[0_14px_30px_rgba(79,70,229,0.24),inset_0_1px_0_rgba(255,255,255,0.2)] hover:bg-indigo-500 hover:shadow-[0_18px_38px_rgba(79,70,229,0.28)]"
                  >
                    <Fingerprint className="h-4 w-4" />
                    {authMethods?.oidc.label.trim() || '企业单点登录'}
                  </Button>
                  {passwordEnabled && (
                    <div className="mt-5 flex items-center gap-3 text-[11px] text-slate-400" aria-hidden="true">
                      <div className="h-px flex-1 bg-slate-200" />
                      <span>或使用密码</span>
                      <div className="h-px flex-1 bg-slate-200" />
                    </div>
                  )}
                </div>
              )}
              {passwordEnabled && (
                <form onSubmit={handleSubmit} className="space-y-4">
                  <div className="space-y-2">
                    <Label htmlFor="username" className="text-xs font-medium text-slate-700">用户名</Label>
                    <div className="relative">
                      <UserRound className="absolute left-3.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-400" />
                      <Input
                        id="username"
                        type="text"
                        placeholder="请输入用户名"
                        value={username}
                        onChange={(e) => setUsername(e.target.value)}
                        disabled={loading}
                        autoComplete="username"
                        autoFocus={!username}
                        aria-invalid={!!error}
                        aria-describedby={error ? 'login-error' : undefined}
                        className="h-11 border-slate-200/90 bg-white/85 pl-9 pr-4 text-sm text-slate-950 shadow-[0_1px_2px_rgba(15,23,42,0.04)] placeholder:text-slate-400 hover:border-slate-300 focus-visible:border-teal-500/70 focus-visible:ring-teal-500/12"
                      />
                    </div>
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="password" className="text-xs font-medium text-slate-700">密码</Label>
                    <div className="relative">
                      <KeyRound className="absolute left-3.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-400" />
                      <Input
                        id="password"
                        type={showPassword ? 'text' : 'password'}
                        placeholder="请输入密码"
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                        onKeyDown={(e) => setCapsLock(e.getModifierState('CapsLock'))}
                        onKeyUp={(e) => setCapsLock(e.getModifierState('CapsLock'))}
                        disabled={loading}
                        autoComplete="current-password"
                        autoFocus={!!username}
                        aria-invalid={!!error}
                        aria-describedby={error ? 'login-error' : capsLock ? 'caps-lock-warning' : undefined}
                        className="h-11 border-slate-200/90 bg-white/85 pl-9 pr-10 text-sm text-slate-950 shadow-[0_1px_2px_rgba(15,23,42,0.04)] placeholder:text-slate-400 hover:border-slate-300 focus-visible:border-teal-500/70 focus-visible:ring-teal-500/12"
                      />
                      <button
                        type="button"
                        className="absolute right-3 top-1/2 flex h-7 w-7 -translate-y-1/2 items-center justify-center rounded-md text-slate-400 transition-colors hover:bg-slate-100 hover:text-slate-700 focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-teal-500/20"
                        onClick={() => setShowPassword((current) => !current)}
                        aria-label={showPassword ? '隐藏密码' : '显示密码'}
                      >
                        {showPassword ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                      </button>
                    </div>
                    {capsLock && (
                      <p id="caps-lock-warning" className="text-xs text-amber-700">大写锁定已开启</p>
                    )}
                    {error && (
                      <p id="login-error" role="alert" aria-live="polite" className="border-l-2 border-rose-400 bg-rose-50/70 px-3 py-2.5 text-xs leading-5 text-rose-700">
                        {error}
                      </p>
                    )}
                  </div>
                  <Button
                    type="submit"
                    size="lg"
                    aria-busy={loading}
                    className="mt-2 h-11 w-full border-teal-600/80 bg-teal-600 text-sm font-medium text-white shadow-[0_12px_28px_rgba(13,148,136,0.20),inset_0_1px_0_rgba(255,255,255,0.18)] hover:bg-teal-500 hover:shadow-[0_16px_34px_rgba(13,148,136,0.24)]"
                    disabled={loading}
                  >
                    {loading && <Loader2 className="h-4 w-4 animate-spin" />}
                    {loading ? '正在登录…' : '登录'}
                  </Button>
                  <div className="flex items-center gap-3 pt-4 text-xs text-slate-300">
                    <div className="h-px flex-1 bg-slate-200" />
                    <ShieldCheck className="h-4 w-4 text-slate-400/80" />
                    <div className="h-px flex-1 bg-slate-200" />
                  </div>
                  <p className="text-center text-[11px] text-slate-400">连接当前实例 · 安全会话鉴权</p>
                </form>
              )}
              {!passwordEnabled && !oidcEnabled && (
                <p role="status" className="rounded-lg border border-amber-200/80 bg-amber-50/70 px-3 py-3 text-center text-xs leading-5 text-amber-800">
                  当前实例未启用可用的登录方式，请联系管理员。
                </p>
              )}
              {!passwordEnabled && oidcEnabled && (
                <p className="mt-4 text-center text-[11px] text-slate-400">将通过企业身份提供方完成安全认证</p>
              )}
            </CardContent>
          </div>
        </div>
      </Card>
    </main>
  )
}
