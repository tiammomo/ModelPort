import { Outlet } from 'react-router-dom'
import { Sidebar } from './Sidebar'
import { Header } from './Header'
import { CommandPalette } from '@/components/shared/CommandPalette'
import { cn } from '@/lib/utils'
import { Toaster } from 'sonner'
import { useEffect, useState } from 'react'

function useIsMobile() {
  const [isMobile, setIsMobile] = useState(() => window.matchMedia('(max-width: 768px)').matches)
  useEffect(() => {
    const mq = window.matchMedia('(max-width: 768px)')
    const handler = (e: MediaQueryListEvent) => setIsMobile(e.matches)
    mq.addEventListener('change', handler)
    return () => mq.removeEventListener('change', handler)
  }, [])
  return isMobile
}

export function AppLayout() {
  const isMobile = useIsMobile()
  const [mobileOpen, setMobileOpen] = useState(false)

  useEffect(() => {
    if (!mobileOpen) return
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setMobileOpen(false)
    }
    document.addEventListener('keydown', closeOnEscape)
    return () => document.removeEventListener('keydown', closeOnEscape)
  }, [mobileOpen])

  return (
    <div className="flex h-dvh overflow-hidden bg-background">
      <a
        href="#main-content"
        className="fixed left-3 top-3 z-[100] -translate-y-20 rounded-md bg-primary px-3 py-2 text-sm font-medium text-primary-foreground shadow-lg transition-transform focus:translate-y-0"
      >
        跳到主要内容
      </a>
      {/* Mobile backdrop */}
      {isMobile && mobileOpen && (
        <button
          type="button"
          aria-label="关闭导航菜单"
          className="fixed inset-0 z-50 bg-slate-950/55 backdrop-blur-[2px] animate-fade-in"
          onClick={() => setMobileOpen(false)}
        />
      )}

      {/* Sidebar */}
      {isMobile ? (
        <div
          id="mobile-navigation"
          className={cn(
            'fixed inset-y-0 left-0 z-50 transition-transform duration-300',
            mobileOpen ? 'translate-x-0' : '-translate-x-full',
          )}
        >
          <Sidebar mobile onNavigate={() => setMobileOpen(false)} />
        </div>
      ) : (
        <Sidebar />
      )}

      {/* Main content */}
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <Header
          onMenuClick={() => setMobileOpen(true)}
          isMobile={isMobile}
          mobileMenuOpen={mobileOpen}
        />
        <main id="main-content" tabIndex={-1} className="flex-1 overflow-y-auto bg-muted/15 px-4 py-5 outline-none md:px-6 md:py-6">
          <div className="mx-auto w-full max-w-[1600px]">
            <Outlet />
          </div>
        </main>
      </div>

      <CommandPalette />
      <Toaster
        position="top-right"
        toastOptions={{
          className: 'text-sm',
          duration: 4000,
        }}
        richColors
        closeButton
      />
    </div>
  )
}
