import { NavLink, useLocation } from 'react-router-dom'
import { useAppStore } from '@/stores'
import { NAV_ITEMS } from '@/lib/constants'
import { cn } from '@/lib/utils'
import {
  LayoutDashboard,
  Users,
  Gauge,
  Boxes,
  ScrollText,
  Settings,
  ChevronLeft,
  ChevronRight,
  Zap,
} from 'lucide-react'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Separator } from '@/components/ui/separator'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger, TooltipProvider } from '@/components/ui/tooltip'

const iconMap: Record<string, React.ElementType> = {
  LayoutDashboard,
  Users,
  Gauge,
  Boxes,
  ScrollText,
  Settings,
}

export function Sidebar() {
  const collapsed = useAppStore((s) => s.sidebarCollapsed)
  const toggle = useAppStore((s) => s.toggleSidebar)
  const location = useLocation()

  return (
    <TooltipProvider delayDuration={0}>
      <aside
        className={cn(
          "fixed left-0 top-0 z-40 flex h-screen flex-col border-r bg-sidebar text-sidebar-foreground transition-all duration-200",
          collapsed ? "w-16" : "w-64"
        )}
      >
        {/* Logo */}
        <div className="flex h-14 items-center gap-2 border-b px-4">
          <Zap className="h-6 w-6 shrink-0 text-primary" />
          {!collapsed && (
            <span className="text-lg font-bold tracking-tight">ModelPort</span>
          )}
        </div>

        {/* Navigation */}
        <ScrollArea className="flex-1 py-2">
          <nav className="flex flex-col gap-1 px-2">
            {NAV_ITEMS.map((item) => {
              const Icon = iconMap[item.icon]
              const isActive = location.pathname === item.path

              const link = (
                <NavLink
                  key={item.path}
                  to={item.path}
                  className={cn(
                    "flex items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors",
                    isActive
                      ? "bg-sidebar-accent text-sidebar-primary"
                      : "text-sidebar-foreground/70 hover:bg-sidebar-accent/50 hover:text-sidebar-foreground"
                  )}
                >
                  {Icon && <Icon className="h-4 w-4 shrink-0" />}
                  {!collapsed && <span>{item.label}</span>}
                </NavLink>
              )

              if (collapsed) {
                return (
                  <Tooltip key={item.path}>
                    <TooltipTrigger asChild>{link}</TooltipTrigger>
                    <TooltipContent side="right">{item.label}</TooltipContent>
                  </Tooltip>
                )
              }

              return link
            })}
          </nav>
        </ScrollArea>

        <Separator />

        {/* Collapse toggle */}
        <div className="flex items-center justify-center p-2">
          <Button variant="ghost" size="icon" onClick={toggle} className="h-8 w-8">
            {collapsed ? <ChevronRight className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}
          </Button>
        </div>
      </aside>
    </TooltipProvider>
  )
}
