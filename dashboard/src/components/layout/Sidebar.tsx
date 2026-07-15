import { NavLink, useLocation } from 'react-router-dom'
import { useAppStore, useAuthStore } from '@/stores'
import { NAV_SECTIONS, navItemsForRole } from '@/lib/constants'
import { api } from '@/lib/api-client'
import { useQuery } from '@tanstack/react-query'
import { cn } from '@/lib/utils'
import {
  LayoutDashboard,
  KeyRound,
  Users,
  Gauge,
  Boxes,
  ScrollText,
  Settings,
  ShieldCheck,
  ChevronLeft,
  ChevronRight,
  Zap,
  X,
} from 'lucide-react'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Separator } from '@/components/ui/separator'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger, TooltipProvider } from '@/components/ui/tooltip'

const iconMap: Record<string, React.ElementType> = {
  LayoutDashboard,
  KeyRound,
  Users,
  Gauge,
  Boxes,
  ScrollText,
  Settings,
  ShieldCheck,
}

interface SidebarProps {
  onNavigate?: () => void
  mobile?: boolean
}

export function Sidebar({ onNavigate, mobile = false }: SidebarProps) {
  const collapsed = useAppStore((s) => s.sidebarCollapsed)
  const toggle = useAppStore((s) => s.toggleSidebar)
  const role = useAuthStore((s) => s.currentUser?.role)
  const location = useLocation()
  const isCollapsed = mobile ? false : collapsed
  const navItems = navItemsForRole(role)
  const { data: liveness, isError: livenessError } = useQuery({
    queryKey: ['gateway-liveness'],
    queryFn: () => api.get<{ status: string }>('/livez'),
    refetchInterval: 30_000,
    staleTime: 10_000,
    retry: 1,
  })
  const connected = !livenessError && liveness?.status === 'ok'

  return (
    <TooltipProvider delayDuration={0}>
      <aside
        aria-label="主导航"
        className={cn(
          'flex h-screen flex-col border-r border-sidebar-border/75 bg-sidebar/92 backdrop-blur-xl text-sidebar-foreground transition-all duration-200',
          isCollapsed ? 'w-14' : mobile ? 'w-64' : 'w-56',
        )}
      >
        {/* Logo */}
        <div className="flex h-14 items-center gap-2.5 border-b border-sidebar-border/70 px-3">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-primary/60 bg-primary text-primary-foreground shadow-[0_4px_12px_oklch(0.35_0.08_185/0.14)]">
            <Zap className="h-4 w-4" />
          </div>
          {!isCollapsed && (
            <span className="text-lg font-bold tracking-tight">ModelPort</span>
          )}
          {mobile && (
            <Button variant="ghost" size="icon" onClick={onNavigate} className="ml-auto h-8 w-8" aria-label="关闭导航菜单">
              <X className="h-4 w-4" />
            </Button>
          )}
        </div>

        {/* Navigation */}
        <ScrollArea className="flex-1 py-3">
          <nav className="flex flex-col gap-3 px-2">
            {NAV_SECTIONS.map((section) => {
              const sectionItems = navItems.filter((item) => item.section === section)
              if (sectionItems.length === 0) return null
              return (
                <div key={section} className="space-y-1">
                  {!isCollapsed && (
                    <p className="px-2.5 pb-1 text-[10px] font-semibold uppercase tracking-[0.16em] text-sidebar-foreground/40">
                      {section}
                    </p>
                  )}
                  {sectionItems.map((item) => {
              const Icon = iconMap[item.icon]
              const isActive =
                location.pathname === item.path ||
                (item.path !== '/dashboard' && location.pathname.startsWith(item.path))

              const link = (
                <NavLink
                  key={item.path}
                  to={item.path}
                  onClick={onNavigate}
                  className={cn(
                    'group relative flex items-center gap-3 rounded-lg px-2.5 py-2 text-sm font-medium transition-[color,background-color] duration-150 before:absolute before:left-0 before:h-5 before:w-0.5 before:rounded-full before:bg-transparent before:transition-colors',
                    isActive
                      ? 'bg-sidebar-accent/85 text-sidebar-primary before:bg-sidebar-primary'
                      : 'text-sidebar-foreground/70 hover:bg-sidebar-accent/50 hover:text-sidebar-foreground',
                  )}
                >
                  {Icon && (
                    <Icon
                      className={cn(
                        'h-4 w-4 shrink-0 transition-colors',
                        isActive ? 'text-sidebar-primary' : 'text-sidebar-foreground/50 group-hover:text-sidebar-foreground/80',
                      )}
                    />
                  )}
                  {!isCollapsed && <span>{item.label}</span>}
                </NavLink>
              )

              if (isCollapsed) {
                return (
                  <Tooltip key={item.path}>
                    <TooltipTrigger asChild>{link}</TooltipTrigger>
                    <TooltipContent side="right" sideOffset={8}>
                      {item.label}
                    </TooltipContent>
                  </Tooltip>
                )
              }

              return link
                  })}
                </div>
              )
            })}
          </nav>
        </ScrollArea>

        <Separator className="opacity-50" />

        {/* Status indicator + collapse toggle */}
        <div className="flex items-center justify-between p-2">
          {!isCollapsed && (
            <div className="flex min-w-0 items-center gap-2 px-1 text-xs text-muted-foreground" title={connected ? '网关健康检查正常' : '暂时无法确认网关状态'}>
              <span className={cn('h-2 w-2 shrink-0 rounded-full shadow-[0_0_0_3px_currentColor]', connected ? 'bg-emerald-500 text-emerald-500/10' : livenessError ? 'bg-rose-500 text-rose-500/10' : 'bg-amber-500 text-amber-500/10')} />
              <span className="truncate">{connected ? '网关已连接' : livenessError ? '连接异常' : '正在检查'}</span>
            </div>
          )}
          {!mobile && (
            <Button variant="ghost" size="icon" onClick={toggle} className="h-8 w-8 shrink-0" aria-label={isCollapsed ? '展开侧栏' : '收起侧栏'}>
              {isCollapsed ? <ChevronRight className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}
            </Button>
          )}
        </div>
      </aside>
    </TooltipProvider>
  )
}
