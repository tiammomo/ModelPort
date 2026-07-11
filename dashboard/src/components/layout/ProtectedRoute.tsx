import { Link, Navigate, useLocation } from 'react-router-dom'
import { useAuthStore } from '@/stores'
import type { UserRole } from '@/types'
import { LoadingPage } from '@/components/shared/LoadingPage'
import { Button } from '@/components/ui/button'
import { ShieldX } from 'lucide-react'
import { ROLE_LABELS } from '@/lib/constants'

export function ProtectedRoute({ children }: { children: React.ReactNode }) {
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated)
  const isInitializing = useAuthStore((s) => s.isInitializing)
  const location = useLocation()

  if (isInitializing) {
    return <main className="mx-auto min-h-screen max-w-6xl p-6"><LoadingPage /></main>
  }

  if (!isAuthenticated) {
    return <Navigate to="/login" state={{ from: location }} replace />
  }

  return <>{children}</>
}

export function RoleRoute({
  children,
  roles,
}: {
  children: React.ReactNode
  roles: UserRole[]
}) {
  const currentUser = useAuthStore((state) => state.currentUser)
  if (!currentUser || !roles.includes(currentUser.role)) {
    return (
      <div className="flex min-h-[60vh] flex-col items-center justify-center px-4 text-center">
        <div className="mb-4 rounded-full bg-amber-500/10 p-4 text-amber-600">
          <ShieldX className="h-8 w-8" />
        </div>
        <h1 className="text-xl font-semibold">无权访问此页面</h1>
        <p className="mt-2 max-w-md text-sm leading-6 text-muted-foreground">
          当前账号角色为“{ROLE_LABELS[currentUser?.role || ''] || currentUser?.role || '未知'}”，此页面需要
          {roles.map((role) => `“${ROLE_LABELS[role] || role}”`).join('或')}权限。
        </p>
        <Button asChild className="mt-5">
          <Link to="/dashboard">返回仪表盘</Link>
        </Button>
      </div>
    )
  }
  return <>{children}</>
}
