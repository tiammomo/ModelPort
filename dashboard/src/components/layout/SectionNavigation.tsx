import { NavLink, useLocation } from 'react-router-dom'
import { navGroupsForRole } from '@/lib/constants'
import { useAuthStore } from '@/stores'
import { cn } from '@/lib/utils'

export function SectionNavigation() {
  const role = useAuthStore((state) => state.currentUser?.role)
  const { pathname } = useLocation()
  const group = navGroupsForRole(role).find((candidate) => candidate.items.some((item) => pathname === item.path || pathname.startsWith(`${item.path}/`)))
  if (!group || group.items.length < 2) return null
  return (
    <nav aria-label={`${group.label}页面`} className="mb-5 flex gap-2 overflow-x-auto border-b pb-3">
      {group.items.map((item) => (
        <NavLink key={item.path} to={item.path} className={({ isActive }) => cn(
          'shrink-0 rounded-md px-3 py-2 text-sm font-medium transition-colors',
          isActive ? 'bg-primary/10 text-primary' : 'text-muted-foreground hover:bg-muted hover:text-foreground',
        )}>{item.label}</NavLink>
      ))}
    </nav>
  )
}
