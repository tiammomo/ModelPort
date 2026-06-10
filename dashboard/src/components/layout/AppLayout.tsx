import { Outlet } from 'react-router-dom'
import { Sidebar } from './Sidebar'
import { Header } from './Header'
import { useAppStore } from '@/stores'
import { cn } from '@/lib/utils'

export function AppLayout() {
  const collapsed = useAppStore((s) => s.sidebarCollapsed)

  return (
    <div className="flex h-screen overflow-hidden">
      <Sidebar />
      <div className={cn("flex flex-1 flex-col overflow-hidden transition-all duration-200", collapsed ? "ml-16" : "ml-64")}>
        <Header />
        <main className="flex-1 overflow-y-auto p-6">
          <Outlet />
        </main>
      </div>
    </div>
  )
}
