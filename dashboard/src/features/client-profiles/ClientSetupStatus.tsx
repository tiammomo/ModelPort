import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { setupMessage, type ClientSetupCheck } from './client-setup'

export function ClientSetupStatus({ check, pending, error, hasSelection, isAdmin, onRetry }: {
  check?: ClientSetupCheck
  pending: boolean
  error: unknown
  hasSelection: boolean
  isAdmin: boolean
  onRetry: () => void
}) {
  return (
    <div className="space-y-2 rounded-md border bg-muted/20 p-3 text-sm" role="status" aria-label="客户端接入检查">
      <p className="font-medium">{pending && hasSelection ? '正在检查所选密钥与路由…' : !hasSelection ? '请选择密钥和模型以检查接入' : error ? '接入检查失败，配置复制已禁用。' : setupMessage(check?.code)}</p>
      {check?.project && <p className="break-all text-xs text-muted-foreground">项目：{check.project.organizationId}/{check.project.projectId}/{check.project.environmentId}</p>}
      {check?.status === 'ready' && !error && <p className="text-xs text-muted-foreground">配置检查通过尚不代表调用成功。{check.ipRestricted ? '此密钥有 IP 限制，请从实际客户端验证。' : ''}完整请求仍需通过实时配额、网络和请求内容能力检查；调用后到请求日志核对结果。</p>}
      <div className="flex flex-wrap gap-2">
        {hasSelection && <Button type="button" size="sm" variant="outline" disabled={pending} onClick={onRetry}>重新检查接入</Button>}
        {check?.code === 'local_only' && isAdmin && <Button asChild size="sm" variant="outline"><Link to="/governance">检查项目策略</Link></Button>}
      </div>
    </div>
  )
}
