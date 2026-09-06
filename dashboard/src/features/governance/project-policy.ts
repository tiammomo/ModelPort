import type { DataClassification, HybridMode, ProjectPolicy, Provider } from '@/types'

export interface ProjectPolicyDraft {
  organizationId: string
  projectId: string
  environmentId: string
  maximumMode: HybridMode
  defaultClassification: DataClassification
  allowedProviders: string
  allowedModels: string
  allowedRegions: string
  allowedApiVersions: string
  cloudEnabled: boolean
}

export const EMPTY_PROJECT_POLICY: ProjectPolicyDraft = {
  organizationId: 'org_local',
  projectId: 'prj_default',
  environmentId: 'env_default',
  maximumMode: 'local_strict',
  defaultClassification: 'unknown',
  allowedProviders: '',
  allowedModels: '',
  allowedRegions: 'local',
  allowedApiVersions: 'openai-compatible-v1',
  cloudEnabled: false,
}

export function selectPolicyProvider(draft: ProjectPolicyDraft, provider: Provider): ProjectPolicyDraft {
  return {
    ...draft,
    allowedProviders: provider.id,
    allowedModels: provider.defaultModel,
    allowedRegions: provider.governance?.region ?? '',
    allowedApiVersions: provider.governance?.apiVersion ?? '',
  }
}

function list(value: string) {
  return [...new Set(value.split(/[,\n]/).map((item) => item.trim()).filter(Boolean))]
}

export function buildProjectPolicy(draft: ProjectPolicyDraft): Omit<ProjectPolicy, 'updatedBy' | 'updatedAtMs'> {
  const scope = [draft.organizationId, draft.projectId, draft.environmentId].map((value) => value.trim())
  if (scope.some((value) => !value || value.includes('/'))) throw new Error('组织、项目和环境标识不能为空或包含 /。')
  const allowedProviders = list(draft.allowedProviders)
  const allowedModels = list(draft.allowedModels)
  const allowedRegions = list(draft.allowedRegions)
  const allowedApiVersions = list(draft.allowedApiVersions)
  if ([allowedProviders, allowedModels, allowedRegions, allowedApiVersions].some((values) => values.length === 0)) {
    throw new Error('请明确允许的 Provider、模型、区域和 API 版本；空列表会扩大范围。')
  }
  return {
    organizationId: scope[0], projectId: scope[1], environmentId: scope[2],
    maximumMode: draft.cloudEnabled ? draft.maximumMode : 'local_strict',
    defaultClassification: draft.defaultClassification,
    allowedProviders, allowedModels, allowedRegions, allowedApiVersions,
    cloudEnabled: draft.cloudEnabled,
  }
}
