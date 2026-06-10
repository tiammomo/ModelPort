import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'

export function NotFoundPage() {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-4">
      <h1 className="text-6xl font-bold text-muted-foreground">404</h1>
      <p className="text-lg text-muted-foreground">页面不存在</p>
      <Button asChild>
        <Link to="/dashboard">返回仪表盘</Link>
      </Button>
    </div>
  )
}
