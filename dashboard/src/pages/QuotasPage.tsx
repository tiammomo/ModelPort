import { useState } from 'react'
import { useQuotas, useCreateQuota, useDeleteQuota } from '@/hooks'
import { PageHeader } from '@/components/shared/PageHeader'
import { MetricCard } from '@/components/shared/MetricCard'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Progress } from '@/components/ui/progress'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Gauge, Plus, Trash2, AlertTriangle } from 'lucide-react'
import { formatNumber, formatDate } from '@/lib/utils'
import type { QuotaType, QuotaPeriod } from '@/types'

export function QuotasPage() {
  const { data: quotas = [], isLoading } = useQuotas()
  const createQuota = useCreateQuota()
  const deleteQuota = useDeleteQuota()

  const [showCreateDialog, setShowCreateDialog] = useState(false)
  const [form, setForm] = useState({
    userId: '',
    username: '',
    quotaType: 'tokens' as QuotaType,
    limit: 0,
    period: 'monthly' as QuotaPeriod,
  })

  const totalLimit = quotas.reduce((s, q) => s + q.limit, 0)
  const totalUsed = quotas.reduce((s, q) => s + q.used, 0)
  const overQuota = quotas.filter((q) => q.used / q.limit > 0.9).length

  const getUsagePercent = (used: number, limit: number) => Math.min(100, Math.round((used / limit) * 100))

  const getUsageColor = (percent: number) => {
    if (percent >= 90) return 'text-red-600'
    if (percent >= 70) return 'text-yellow-600'
    return 'text-green-600'
  }

  const formatQuotaValue = (value: number, type: QuotaType) => {
    if (type === 'tokens') return formatNumber(value)
    if (type === 'cost') return `$${value.toFixed(2)}`
    return formatNumber(value)
  }

  const periodLabels: Record<string, string> = {
    daily: '每日',
    weekly: '每周',
    monthly: '每月',
  }

  if (isLoading) {
    return <div className="flex items-center justify-center h-64 text-muted-foreground">加载中...</div>
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="配额管理"
        description="管理用户配额和使用量"
        action={{ label: '新建配额', onClick: () => setShowCreateDialog(true), icon: Plus }}
      />

      {/* Summary Cards */}
      <div className="grid gap-4 md:grid-cols-3">
        <MetricCard title="配额总量" value={formatNumber(totalLimit)} icon={Gauge} description="已分配的配额上限" />
        <MetricCard title="已使用" value={formatNumber(totalUsed)} icon={Gauge} description={`${getUsagePercent(totalUsed, totalLimit)}% 使用率`} />
        <MetricCard title="接近上限" value={overQuota} icon={AlertTriangle} description="使用率超过 90% 的用户" />
      </div>

      {/* Quota Table */}
      <Card>
        <CardHeader>
          <CardTitle>用户配额</CardTitle>
        </CardHeader>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>用户</TableHead>
                <TableHead>配额类型</TableHead>
                <TableHead>限额</TableHead>
                <TableHead>已用</TableHead>
                <TableHead className="w-48">使用率</TableHead>
                <TableHead>周期</TableHead>
                <TableHead>重置时间</TableHead>
                <TableHead className="w-12"></TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {quotas.map((quota) => {
                const percent = getUsagePercent(quota.used, quota.limit)
                return (
                  <TableRow key={quota.id}>
                    <TableCell className="font-medium">{quota.username}</TableCell>
                    <TableCell className="text-muted-foreground">
                      {quota.quotaType === 'tokens' ? 'Token 数' : quota.quotaType === 'requests' ? '请求数' : '费用'}
                    </TableCell>
                    <TableCell>{formatQuotaValue(quota.limit, quota.quotaType)}</TableCell>
                    <TableCell>{formatQuotaValue(quota.used, quota.quotaType)}</TableCell>
                    <TableCell>
                      <div className="flex items-center gap-2">
                        <Progress value={percent} className="flex-1" />
                        <span className={`text-sm font-medium ${getUsageColor(percent)}`}>{percent}%</span>
                      </div>
                    </TableCell>
                    <TableCell>{periodLabels[quota.period]}</TableCell>
                    <TableCell className="text-sm text-muted-foreground">{formatDate(quota.resetAt)}</TableCell>
                    <TableCell>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8 text-destructive"
                        onClick={() => deleteQuota.mutate(quota.id)}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </TableCell>
                  </TableRow>
                )
              })}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {/* Create Dialog */}
      <Dialog open={showCreateDialog} onOpenChange={setShowCreateDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>新建配额</DialogTitle>
            <DialogDescription>为用户设置配额限制</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <Label>用户名</Label>
              <Input value={form.username} onChange={(e) => setForm({ ...form, username: e.target.value })} placeholder="输入用户名" />
            </div>
            <div className="space-y-2">
              <Label>配额类型</Label>
              <Select value={form.quotaType} onValueChange={(v) => setForm({ ...form, quotaType: v as QuotaType })}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="tokens">Token 数</SelectItem>
                  <SelectItem value="requests">请求数</SelectItem>
                  <SelectItem value="cost">费用</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label>限额</Label>
              <Input type="number" value={form.limit || ''} onChange={(e) => setForm({ ...form, limit: Number(e.target.value) })} placeholder="输入限额" />
            </div>
            <div className="space-y-2">
              <Label>周期</Label>
              <Select value={form.period} onValueChange={(v) => setForm({ ...form, period: v as QuotaPeriod })}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="daily">每日</SelectItem>
                  <SelectItem value="weekly">每周</SelectItem>
                  <SelectItem value="monthly">每月</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowCreateDialog(false)}>取消</Button>
            <Button onClick={() => {
              createQuota.mutate({
                ...form,
                userId: form.userId || `usr_${Date.now()}`,
                used: 0,
                periodStart: new Date().toISOString(),
                periodEnd: new Date(Date.now() + 30 * 86400000).toISOString(),
                resetAt: new Date(Date.now() + 30 * 86400000).toISOString(),
              }, { onSuccess: () => setShowCreateDialog(false) })
            }} disabled={createQuota.isPending}>创建</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
