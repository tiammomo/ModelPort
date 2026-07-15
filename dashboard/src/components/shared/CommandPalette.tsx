import { useEffect, useState } from 'react'
import { Command } from 'cmdk'
import { useNavigate } from 'react-router-dom'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { DialogTitle } from '@/components/ui/dialog'
import { NAV_SECTIONS, navItemsForRole } from '@/lib/constants'
import { useAuthStore } from '@/stores'
import { Search, LayoutDashboard, KeyRound, Users, Gauge, Boxes, ScrollText, Settings, ShieldCheck } from 'lucide-react'

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

export function CommandPalette() {
  const [open, setOpen] = useState(false)
  const navigate = useNavigate()
  const role = useAuthStore((state) => state.currentUser?.role)
  const navItems = navItemsForRole(role)

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault()
        setOpen((prev) => !prev)
      }
    }
    document.addEventListener('keydown', onKeyDown)
    const openPalette = () => setOpen(true)
    document.addEventListener('modelport:open-command-palette', openPalette)
    return () => {
      document.removeEventListener('keydown', onKeyDown)
      document.removeEventListener('modelport:open-command-palette', openPalette)
    }
  }, [])

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="max-w-lg gap-0 overflow-hidden p-0">
        <DialogTitle className="sr-only">快速导航</DialogTitle>
        <Command className="bg-transparent" shouldFilter>
          <div className="flex items-center border-b border-border/75 px-4">
            <Search className="mr-2.5 h-4 w-4 shrink-0 text-muted-foreground" />
            <Command.Input
              placeholder="搜索页面或功能，例如 Provider、日志、密钥..."
              className="flex h-12 w-full bg-transparent py-3 text-sm outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-50"
            />
          </div>
          <Command.List className="max-h-[320px] overflow-y-auto p-2.5">
            <Command.Empty className="py-6 text-center text-sm text-muted-foreground">
              未找到结果
            </Command.Empty>
            {NAV_SECTIONS.map((section) => (
              <Command.Group key={section} heading={section}>
              {navItems.filter((item) => item.section === section).map((item) => {
                const Icon = iconMap[item.icon]
                return (
                  <Command.Item
                    key={item.path}
                    value={`${item.label} ${item.keywords}`}
                    onSelect={() => {
                      navigate(item.path)
                      setOpen(false)
                    }}
                    className="relative flex cursor-pointer select-none items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm outline-none transition-colors aria-selected:bg-accent/85 aria-selected:text-accent-foreground"
                  >
                    {Icon && <Icon className="h-4 w-4" />}
                    {item.label}
                  </Command.Item>
                )
              })}
              </Command.Group>
            ))}
          </Command.List>
        </Command>
      </DialogContent>
    </Dialog>
  )
}
