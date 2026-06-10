import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useAuthStore } from '@/stores'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Zap, Loader2 } from 'lucide-react'

export function LoginPage() {
  const [token, setToken] = useState('')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const login = useAuthStore((s) => s.login)
  const navigate = useNavigate()

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!token.trim()) {
      setError('请输入认证令牌')
      return
    }

    setLoading(true)
    setError('')

    try {
      await login(token.trim())
      navigate('/dashboard')
    } catch {
      setError('认证失败，请检查令牌是否正确')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-background">
      <Card className="w-full max-w-md">
        <CardHeader className="text-center">
          <div className="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
            <Zap className="h-6 w-6 text-primary" />
          </div>
          <CardTitle className="text-2xl">ModelPort 管理后台</CardTitle>
          <CardDescription>请输入您的认证令牌以登录系统</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="token">认证令牌</Label>
              <Input
                id="token"
                type="password"
                placeholder="请输入 MODELPORT_AUTH_TOKEN"
                value={token}
                onChange={(e) => setToken(e.target.value)}
                disabled={loading}
              />
              {error && <p className="text-sm text-destructive">{error}</p>}
            </div>
            <Button type="submit" className="w-full" disabled={loading}>
              {loading && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              登录
            </Button>
            <p className="text-center text-xs text-muted-foreground">
              使用当前后端的 MODELPORT_AUTH_TOKEN 登录
            </p>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}
