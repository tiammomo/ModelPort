import { describe, expect, it } from 'vitest'

import { gatewayProcessStatus } from './sidebar-status'

describe('gateway process status copy', () => {
  it('describes a successful liveness probe as process availability only', () => {
    expect(gatewayProcessStatus('ok', false)).toEqual({
      kind: 'online',
      label: '网关进程在线',
      title: '存活探针响应正常，仅表示网关进程在线',
    })
  })

  it('distinguishes initial checking from probe failures', () => {
    expect(gatewayProcessStatus(undefined, false).kind).toBe('checking')
    expect(gatewayProcessStatus(undefined, true)).toMatchObject({
      kind: 'error',
      label: '无法确认进程',
    })
    expect(gatewayProcessStatus('unexpected', false).kind).toBe('error')
  })
})
