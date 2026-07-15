export interface GatewayProcessStatus {
  kind: 'online' | 'error' | 'checking'
  label: string
  title: string
}

export function gatewayProcessStatus(
  livenessStatus: string | undefined,
  requestFailed: boolean,
): GatewayProcessStatus {
  if (requestFailed || (livenessStatus !== undefined && livenessStatus !== 'ok')) {
    return {
      kind: 'error',
      label: '无法确认进程',
      title: '无法确认网关进程存活状态',
    }
  }

  if (livenessStatus === 'ok') {
    return {
      kind: 'online',
      label: '网关进程在线',
      title: '存活探针响应正常，仅表示网关进程在线',
    }
  }

  return {
    kind: 'checking',
    label: '正在检查进程',
    title: '正在检查网关进程存活状态',
  }
}
