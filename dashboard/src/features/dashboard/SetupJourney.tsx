import { Link, useLocation, useSearchParams } from 'react-router-dom'
import { useDashboard } from '@/hooks/use-dashboard'
import { useProviders } from '@/hooks/use-models'
import { useSettings } from '@/hooks/use-settings'
import { useAuthStore } from '@/stores'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { buildOnboardingState, setupJourneySteps } from './onboarding'

export function SetupJourney() {
  const isAdmin = useAuthStore((state) => state.currentUser?.role === 'admin')
  const [params] = useSearchParams()
  return isAdmin && params.get('setup') === '1' ? <JourneyProgress /> : null
}

function JourneyProgress() {
  const { pathname } = useLocation()
  const [, setParams] = useSearchParams()
  const providers = useProviders()
  const settings = useSettings()
  const stats = useDashboard()
  const queries = [providers, settings, stats]
  const retry = () => { for (const query of queries) void query.refetch() }
  const failed = queries.some((query) => query.isError)
  const state = !failed && providers.data && settings.data && stats.data
    ? buildOnboardingState({ providers: providers.data, settings: settings.data, stats: stats.data })
    : undefined
  const steps = state ? setupJourneySteps(state) : []
  const next = steps.find((step) => !step.complete)

  return (
    <section aria-label="首次接入引导" className="mb-5 space-y-3 rounded-lg border border-primary/25 bg-card p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <p className="text-sm font-semibold">完成首次受治理请求</p>
          <p className="mt-1 text-xs text-muted-foreground">已保存的配置和请求记录会自动恢复进度；连接测试与完整调用分别检查。</p>
        </div>
        <Button size="sm" variant="ghost" onClick={() => setParams((current) => { current.delete('setup'); return current })}>退出引导</Button>
      </div>
      {failed ? <p role="alert" className="text-sm text-destructive">接入进度读取失败。<button type="button" className="ml-2 underline" onClick={retry}>重试</button></p>
        : !state ? <p role="status" className="text-sm text-muted-foreground">正在读取已保存的接入进度…</p>
          : <>
            <nav aria-label="接入步骤" className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              {steps.map((step, index) => <Link key={step.id} to={`${step.to}?setup=1`} aria-current={pathname === step.to ? 'step' : undefined} className={cn('rounded-md border px-3 py-2 text-sm', pathname === step.to && 'border-primary bg-primary/5', step.complete && 'text-emerald-700 dark:text-emerald-400')}>
                {step.complete ? '✓' : index + 1} · {step.title}
              </Link>)}
            </nav>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p className="text-xs text-muted-foreground">{next?.detail ?? '接入检查和成功请求证据已具备，可继续核对团队的实际调用。'}</p>
              {next && pathname !== next.to && <Button asChild size="sm"><Link to={`${next.to}?setup=1`}>继续：{next.title}</Link></Button>}
            </div>
          </>}
    </section>
  )
}
