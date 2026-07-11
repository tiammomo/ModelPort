import { Link, useNavigate } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { useAuthStore } from '@/stores'
import { ArrowLeft, FileQuestion, LayoutDashboard, LogIn } from 'lucide-react'

export function NotFoundPage() {
  const navigate = useNavigate()
  const isAuthenticated = useAuthStore((state) => state.isAuthenticated)

  return (
    <main className="flex min-h-dvh flex-col items-center justify-center gap-6 px-6 text-center animate-fade-in">
      <div className="relative">
        <div className="pointer-events-none absolute inset-0 -z-10 mx-auto h-[300px] w-[300px] rounded-full bg-primary/10 blur-[80px]" />
        <p className="bg-gradient-to-br from-primary to-primary/50 bg-clip-text text-[8rem] font-black leading-none tracking-tighter text-transparent">
          404
        </p>
      </div>
      <div className="rounded-full bg-muted p-4">
        <FileQuestion className="h-10 w-10 text-muted-foreground" />
      </div>
      <div className="space-y-2 text-center">
        <h1 className="text-xl font-semibold">页面不存在</h1>
        <p className="text-muted-foreground">地址可能已变更。你可以返回上一页，或回到可用入口继续操作。</p>
      </div>
      <div className="flex flex-wrap items-center justify-center gap-3">
        <Button variant="outline" onClick={() => navigate(-1)}>
          <ArrowLeft className="h-4 w-4" />
          返回上一页
        </Button>
        <Button asChild>
          <Link to={isAuthenticated ? '/dashboard' : '/login'}>
            {isAuthenticated ? <LayoutDashboard className="h-4 w-4" /> : <LogIn className="h-4 w-4" />}
            {isAuthenticated ? '前往仪表盘' : '前往登录'}
          </Link>
        </Button>
      </div>
    </main>
  )
}
