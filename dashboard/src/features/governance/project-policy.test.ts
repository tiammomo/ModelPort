import { describe, expect, it } from 'vitest'
import { EMPTY_PROJECT_POLICY, buildProjectPolicy, selectPolicyProvider } from './project-policy'
import type { Provider } from '@/types'

const cloudProvider = {
  id: 'approved-cloud', defaultModel: 'code-model',
  governance: { boundary: 'cloud', region: 'eu', apiVersion: 'anthropic-v1' },
} as Provider

describe('project policy form', () => {
  it('selecting a cloud provider never implicitly permits egress or classifies data', () => {
    const policy = buildProjectPolicy(selectPolicyProvider(EMPTY_PROJECT_POLICY, cloudProvider))
    expect(policy).toMatchObject({ cloudEnabled: false, maximumMode: 'local_strict', defaultClassification: 'unknown', allowedRegions: ['eu'], allowedApiVersions: ['anthropic-v1'] })
  })

  it('requires explicit allowlists instead of turning blank form fields into unrestricted access', () => {
    const selected = selectPolicyProvider(EMPTY_PROJECT_POLICY, cloudProvider)
    for (const field of ['allowedModels', 'allowedProviders', 'allowedRegions', 'allowedApiVersions'] as const) {
      expect(() => buildProjectPolicy({ ...selected, [field]: ' , \n ' })).toThrow('空列表会扩大范围')
    }
    expect(() => buildProjectPolicy({ ...selected, projectId: 'other/project' })).toThrow('标识')
  })

  it('preserves unknown classification when cloud is explicitly enabled and normalizes exact scope', () => {
    const policy = buildProjectPolicy({
      ...selectPolicyProvider(EMPTY_PROJECT_POLICY, cloudProvider),
      projectId: ' approved ', cloudEnabled: true, maximumMode: 'cloud_first', allowedModels: 'a, b\na',
    })
    expect(policy).toMatchObject({ projectId: 'approved', cloudEnabled: true, defaultClassification: 'unknown', allowedModels: ['a', 'b'] })
  })
})
