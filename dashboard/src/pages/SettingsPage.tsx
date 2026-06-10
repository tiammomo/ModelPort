import { useState } from 'react'
import { useSettings, useUpdateSettings, useTestProviderConnection } from '@/hooks'
import { PageHeader } from '@/components/shared/PageHeader'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Switch } from '@/components/ui/switch'
import { Separator } from '@/components/ui/separator'
import { formatBytes } from '@/lib/utils'
import { Save, Plug, Loader2 } from 'lucide-react'
import type { SystemSettings } from '@/types'

export function SettingsPage() {
  const { data: settings, isLoading } = useSettings()

  if (isLoading || !settings) {
    return <div className="flex items-center justify-center h-64 text-muted-foreground">加载中...</div>
  }

  return <SettingsForm initialSettings={settings} />
}

function SettingsForm({ initialSettings }: { initialSettings: SystemSettings }) {
  const updateSettings = useUpdateSettings()
  const testConnection = useTestProviderConnection()

  const [form, setForm] = useState<SystemSettings>(initialSettings)

  const handleSave = () => {
    if (form) updateSettings.mutate(form)
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="系统设置"
        description="配置网关运行参数"
        action={{ label: '保存更改', onClick: handleSave, icon: Save }}
      />

      <Tabs defaultValue="general">
        <TabsList>
          <TabsTrigger value="general">通用</TabsTrigger>
          <TabsTrigger value="auth">认证</TabsTrigger>
          <TabsTrigger value="ratelimits">限流</TabsTrigger>
          <TabsTrigger value="providers">提供商凭证</TabsTrigger>
        </TabsList>

        {/* General Tab */}
        <TabsContent value="general">
          <Card>
            <CardHeader>
              <CardTitle>通用设置</CardTitle>
              <CardDescription>服务器基础配置</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="space-y-2">
                <Label>绑定地址</Label>
                <Input
                  value={form.server.bindAddress}
                  onChange={(e) => setForm({ ...form, server: { ...form.server, bindAddress: e.target.value } })}
                />
                <p className="text-xs text-muted-foreground">格式: host:port，例如 127.0.0.1:17878</p>
              </div>
              <Separator />
              <div className="space-y-2">
                <Label>最大请求体大小</Label>
                <Input
                  type="number"
                  value={form.server.maxRequestBodyBytes}
                  onChange={(e) => setForm({ ...form, server: { ...form.server, maxRequestBodyBytes: Number(e.target.value) } })}
                />
                <p className="text-xs text-muted-foreground">当前值: {formatBytes(form.server.maxRequestBodyBytes)}</p>
              </div>
              <Separator />
              <div className="space-y-2">
                <Label>最大并发请求数</Label>
                <Input
                  type="number"
                  value={form.server.maxConcurrentRequests}
                  onChange={(e) => setForm({ ...form, server: { ...form.server, maxConcurrentRequests: Number(e.target.value) } })}
                />
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        {/* Auth Tab */}
        <TabsContent value="auth">
          <Card>
            <CardHeader>
              <CardTitle>认证设置</CardTitle>
              <CardDescription>配置 API 认证方式</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6">
              <div className="flex items-center justify-between">
                <div className="space-y-0.5">
                  <Label>启用认证</Label>
                  <p className="text-xs text-muted-foreground">要求所有 API 请求携带认证令牌</p>
                </div>
                <Switch
                  checked={form.auth.enabled}
                  onCheckedChange={(checked) => setForm({ ...form, auth: { ...form.auth, enabled: checked } })}
                />
              </div>
              <Separator />
              <div className="flex items-center justify-between">
                <div className="space-y-0.5">
                  <Label>允许无认证访问</Label>
                  <p className="text-xs text-muted-foreground">允许未携带令牌的请求通过（不推荐）</p>
                </div>
                <Switch
                  checked={form.auth.allowNoAuth}
                  onCheckedChange={(checked) => setForm({ ...form, auth: { ...form.auth, allowNoAuth: checked } })}
                />
              </div>
              <Separator />
              <div className="space-y-2">
                <Label>Token 环境变量</Label>
                <Input value={form.auth.tokenEnvVar} readOnly className="bg-muted" />
                <p className="text-xs text-muted-foreground">从此环境变量读取认证令牌</p>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        {/* Rate Limits Tab */}
        <TabsContent value="ratelimits">
          <Card>
            <CardHeader>
              <CardTitle>限流设置</CardTitle>
              <CardDescription>配置请求限流和超时参数</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="grid gap-4 md:grid-cols-2">
                <div className="space-y-2">
                  <Label>最大并发请求数</Label>
                  <Input
                    type="number"
                    value={form.rateLimits.maxConcurrentRequests}
                    onChange={(e) => setForm({ ...form, rateLimits: { ...form.rateLimits, maxConcurrentRequests: Number(e.target.value) } })}
                  />
                </div>
                <div className="space-y-2">
                  <Label>最大请求体大小（字节）</Label>
                  <Input
                    type="number"
                    value={form.rateLimits.maxRequestBodyBytes}
                    onChange={(e) => setForm({ ...form, rateLimits: { ...form.rateLimits, maxRequestBodyBytes: Number(e.target.value) } })}
                  />
                  <p className="text-xs text-muted-foreground">{formatBytes(form.rateLimits.maxRequestBodyBytes)}</p>
                </div>
                <div className="space-y-2">
                  <Label>请求超时（秒）</Label>
                  <Input
                    type="number"
                    value={form.rateLimits.requestTimeoutSecs}
                    onChange={(e) => setForm({ ...form, rateLimits: { ...form.rateLimits, requestTimeoutSecs: Number(e.target.value) } })}
                  />
                </div>
                <div className="space-y-2">
                  <Label>流空闲超时（秒）</Label>
                  <Input
                    type="number"
                    value={form.rateLimits.streamIdleTimeoutSecs}
                    onChange={(e) => setForm({ ...form, rateLimits: { ...form.rateLimits, streamIdleTimeoutSecs: Number(e.target.value) } })}
                  />
                </div>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        {/* Providers Tab */}
        <TabsContent value="providers">
          <Card>
            <CardHeader>
              <CardTitle>提供商凭证</CardTitle>
              <CardDescription>管理各提供商的 API Key 配置状态</CardDescription>
            </CardHeader>
            <CardContent>
              <div className="space-y-3">
                {form.gateway.providerOrder.map((providerId) => (
                  <div key={providerId} className="flex items-center justify-between rounded-lg border p-4">
                    <div className="space-y-0.5">
                      <p className="text-sm font-medium">{providerId}</p>
                      <p className="text-xs text-muted-foreground font-mono">{providerId.toUpperCase()}_API_KEY</p>
                    </div>
                    <div className="flex items-center gap-3">
                      <StatusBadge status={providerId === form.gateway.defaultProvider ? 'active' : 'inactive'} />
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => testConnection.mutate(providerId)}
                        disabled={testConnection.isPending}
                      >
                        {testConnection.isPending ? (
                          <Loader2 className="mr-1 h-3 w-3 animate-spin" />
                        ) : (
                          <Plug className="mr-1 h-3 w-3" />
                        )}
                        测试连接
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  )
}
