import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import {
  useUpdateProviderModel
} from '@/hooks'
import type {
CapabilitySupport,
ModelProfileOverride,
Provider,
ProviderModelInventory,
ReasoningDialect,
ReasoningEffort,
ReasoningReplay
} from '@/types'
import {
  Loader2
} from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { Field } from './ModelFormField'

type Inheritable<T extends string> = T | 'inherit'

interface ModelAdaptationForm {
  displayName: string
  family: string
  contextWindow: string
  maxOutputTokens: string
  inputModalities: string
  toolUse: Inheritable<CapabilitySupport>
  toolChoice: Inheritable<CapabilitySupport>
  parallelToolCalls: Inheritable<CapabilitySupport>
  strictToolSchema: Inheritable<CapabilitySupport>
  reasoning: Inheritable<CapabilitySupport>
  reasoningEfforts: string
  defaultReasoningEffort: Inheritable<ReasoningEffort>
  reasoningDialect: Inheritable<ReasoningDialect>
  reasoningReplay: Inheritable<ReasoningReplay>
}

const REASONING_EFFORT_VALUES: ReasoningEffort[] = [
  'off',
  'minimal',
  'low',
  'medium',
  'high',
  'xhigh',
  'max',
]

function parseReasoningEfforts(value: string): ReasoningEffort[] | null {
  const efforts = Array.from(new Set(value
    .split(/[\s,]+/)
    .map((item) => item.trim().toLowerCase())
    .filter(Boolean)))
  if (efforts.some((effort) => !REASONING_EFFORT_VALUES.includes(effort as ReasoningEffort))) return null
  return efforts as ReasoningEffort[]
}

export function ModelAdaptationDialog({ target: editingModelAdaptation, onClose }: {
  target: { provider: Provider; item: ProviderModelInventory }
  onClose: () => void
}) {
  const updateProviderModel = useUpdateProviderModel()
  const [modelAdaptationForm, setModelAdaptationForm] = useState<ModelAdaptationForm>(() => {
    const override = editingModelAdaptation.item.override ?? {}
    return {
      displayName: override.display_name ?? '',
      family: override.family ?? '',
      contextWindow: override.context_window?.toString() ?? '',
      maxOutputTokens: override.max_output_tokens?.toString() ?? '',
      inputModalities: override.input_modalities?.join(', ') ?? '',
      toolUse: override.tool_use ?? 'inherit',
      toolChoice: override.tool_choice ?? 'inherit',
      parallelToolCalls: override.parallel_tool_calls ?? 'inherit',
      strictToolSchema: override.strict_tool_schema ?? 'inherit',
      reasoning: override.reasoning ?? 'inherit',
      reasoningEfforts: override.reasoning_efforts?.join(', ') ?? '',
      defaultReasoningEffort: override.default_reasoning_effort ?? 'inherit',
      reasoningDialect: override.reasoning_dialect ?? 'inherit',
      reasoningReplay: override.reasoning_replay ?? 'inherit',
    }
  })
  const saveModelAdaptation = () => {
    const contextWindow = modelAdaptationForm.contextWindow.trim() ? Number(modelAdaptationForm.contextWindow) : undefined
    const maxOutputTokens = modelAdaptationForm.maxOutputTokens.trim() ? Number(modelAdaptationForm.maxOutputTokens) : undefined
    if ((contextWindow !== undefined && (!Number.isSafeInteger(contextWindow) || contextWindow <= 0))
      || (maxOutputTokens !== undefined && (!Number.isSafeInteger(maxOutputTokens) || maxOutputTokens <= 0))
      || (contextWindow !== undefined && maxOutputTokens !== undefined && maxOutputTokens > contextWindow)) {
      toast.error('上下文和最大输出必须是正整数，且最大输出不能超过上下文')
      return
    }
    const efforts = parseReasoningEfforts(modelAdaptationForm.reasoningEfforts)
    if (!efforts) {
      toast.error('推理档位只能使用 off/minimal/low/medium/high/xhigh/max')
      return
    }
    const modalities = Array.from(new Set(modelAdaptationForm.inputModalities
      .split(/[\s,]+/)
      .map((value) => value.trim().toLowerCase())
      .filter(Boolean)))
    if (modalities.some((value) => !['text', 'image'].includes(value))
      || (modalities.length > 0 && !modalities.includes('text'))) {
      toast.error('输入模态只能使用 text 或 image，且当前协议必须包含 text')
      return
    }
    const profile: ModelProfileOverride = {}
    if (modelAdaptationForm.displayName.trim()) profile.display_name = modelAdaptationForm.displayName.trim()
    if (modelAdaptationForm.family.trim()) profile.family = modelAdaptationForm.family.trim()
    if (contextWindow !== undefined) profile.context_window = contextWindow
    if (maxOutputTokens !== undefined) profile.max_output_tokens = maxOutputTokens
    if (modalities.length > 0) profile.input_modalities = modalities as Array<'text' | 'image'>
    for (const [field, value] of [
      ['tool_use', modelAdaptationForm.toolUse],
      ['tool_choice', modelAdaptationForm.toolChoice],
      ['parallel_tool_calls', modelAdaptationForm.parallelToolCalls],
      ['strict_tool_schema', modelAdaptationForm.strictToolSchema],
      ['reasoning', modelAdaptationForm.reasoning],
    ] as const) {
      if (value !== 'inherit') profile[field] = value
    }
    if (efforts.length > 0) profile.reasoning_efforts = efforts
    if (modelAdaptationForm.defaultReasoningEffort !== 'inherit') profile.default_reasoning_effort = modelAdaptationForm.defaultReasoningEffort
    if (modelAdaptationForm.reasoningDialect !== 'inherit') profile.reasoning_dialect = modelAdaptationForm.reasoningDialect
    if (modelAdaptationForm.reasoningReplay !== 'inherit') profile.reasoning_replay = modelAdaptationForm.reasoningReplay

    updateProviderModel.mutate({
      providerId: editingModelAdaptation.provider.id,
      data: {
        model: editingModelAdaptation.item.model,
        status: editingModelAdaptation.item.status,
        profile,
      },
    }, {
      onSuccess: () => {
        toast.success(`已保存 ${editingModelAdaptation.item.model} 的适配画像`)
        onClose()
      },
      onError: (error) => toast.error(error instanceof Error ? error.message : '保存模型适配画像失败'),
    })
  }

  return (
      <Dialog open={!!editingModelAdaptation} onOpenChange={(open) => { if (!open) onClose() }}>
        <DialogContent className="max-h-[94vh] w-[calc(100vw-2rem)] max-w-3xl overflow-hidden">
          <DialogHeader>
            <DialogTitle>模型适配画像</DialogTitle>
            <DialogDescription>
              {editingModelAdaptation
                ? `${editingModelAdaptation.provider.id}:${editingModelAdaptation.item.model}。留空或选择“继承目录”会回到版本化目录 / Provider 默认值；保存不会把模型标记为已实测。`
                : '配置精确模型的能力与推理方言。'}
            </DialogDescription>
          </DialogHeader>
          {editingModelAdaptation && (
            <ScrollArea className="max-h-[70vh] pr-3">
              <div className="grid gap-4 md:grid-cols-2">
                <div className="rounded-md border bg-muted/30 p-3 text-xs leading-5 text-muted-foreground md:col-span-2">
                  当前有效来源：{editingModelAdaptation.item.source ?? 'provider_default'} · 验证：{editingModelAdaptation.item.verification === 'verified' ? '已实测' : '未实测'} · 目录版本：{editingModelAdaptation.item.catalogVersion ?? '—'}。`unknown` 会对高级能力失败关闭。
                </div>
                <Field label="显示名称" htmlFor="model-profile-display-name" description={`当前：${editingModelAdaptation.item.displayName || '未设置'}`}>
                  <Input id="model-profile-display-name" value={modelAdaptationForm.displayName} onChange={(event) => setModelAdaptationForm({ ...modelAdaptationForm, displayName: event.target.value })} placeholder="留空继承" />
                </Field>
                <Field label="模型家族" htmlFor="model-profile-family" description={`当前：${editingModelAdaptation.item.family || '未设置'}`}>
                  <Input id="model-profile-family" value={modelAdaptationForm.family} onChange={(event) => setModelAdaptationForm({ ...modelAdaptationForm, family: event.target.value })} placeholder="留空继承" />
                </Field>
                <Field label="上下文窗口" htmlFor="model-profile-context" description={`当前：${editingModelAdaptation.item.contextWindow?.toLocaleString() || '未知'}`}>
                  <Input id="model-profile-context" type="number" min="1" value={modelAdaptationForm.contextWindow} onChange={(event) => setModelAdaptationForm({ ...modelAdaptationForm, contextWindow: event.target.value })} placeholder="留空继承" />
                </Field>
                <Field label="最大输出 Token" htmlFor="model-profile-output" description={`当前：${editingModelAdaptation.item.maxOutputTokens?.toLocaleString() || '未知'}`}>
                  <Input id="model-profile-output" type="number" min="1" value={modelAdaptationForm.maxOutputTokens} onChange={(event) => setModelAdaptationForm({ ...modelAdaptationForm, maxOutputTokens: event.target.value })} placeholder="留空继承" />
                </Field>
                <Field label="输入模态" htmlFor="model-profile-modalities" className="md:col-span-2" description={`当前：${editingModelAdaptation.item.inputModalities?.join(', ') || 'text'}。当前 Exchange IR 仍只接受文本；image 仅记录能力，不开放图片请求。`}>
                  <Input id="model-profile-modalities" value={modelAdaptationForm.inputModalities} onChange={(event) => setModelAdaptationForm({ ...modelAdaptationForm, inputModalities: event.target.value })} placeholder="留空继承；可填 text 或 text, image" />
                </Field>
                <CapabilityProfileField label="Tool Use" value={modelAdaptationForm.toolUse} effective={editingModelAdaptation.item.toolUse} onChange={(toolUse) => setModelAdaptationForm({ ...modelAdaptationForm, toolUse })} />
                <CapabilityProfileField label="tool_choice" value={modelAdaptationForm.toolChoice} effective={editingModelAdaptation.item.toolChoice} onChange={(toolChoice) => setModelAdaptationForm({ ...modelAdaptationForm, toolChoice })} />
                <CapabilityProfileField label="并行工具调用" value={modelAdaptationForm.parallelToolCalls} effective={editingModelAdaptation.item.parallelToolCalls} onChange={(parallelToolCalls) => setModelAdaptationForm({ ...modelAdaptationForm, parallelToolCalls })} />
                <CapabilityProfileField label="严格工具 Schema" value={modelAdaptationForm.strictToolSchema} effective={editingModelAdaptation.item.strictToolSchema} onChange={(strictToolSchema) => setModelAdaptationForm({ ...modelAdaptationForm, strictToolSchema })} />
                <CapabilityProfileField label="推理" value={modelAdaptationForm.reasoning} effective={editingModelAdaptation.item.reasoning} onChange={(reasoning) => setModelAdaptationForm({ ...modelAdaptationForm, reasoning })} />
                <Field label="推理方言" description={`当前：${editingModelAdaptation.item.reasoningDialect ?? 'none'}`}>
                  <Select value={modelAdaptationForm.reasoningDialect} onValueChange={(value) => setModelAdaptationForm({ ...modelAdaptationForm, reasoningDialect: value as Inheritable<ReasoningDialect> })}>
                    <SelectTrigger aria-label="推理方言"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {['inherit', 'none', 'native_anthropic', 'openai', 'deepseek', 'openrouter', 'qwen', 'zai', 'string_thinking', 'llama_cpp'].map((value) => <SelectItem key={value} value={value}>{value === 'inherit' ? '继承目录' : value}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </Field>
                <Field label="推理档位" htmlFor="model-profile-efforts" description={`当前：${editingModelAdaptation.item.reasoningEfforts?.join(', ') || '未声明'}`}>
                  <Input id="model-profile-efforts" value={modelAdaptationForm.reasoningEfforts} onChange={(event) => setModelAdaptationForm({ ...modelAdaptationForm, reasoningEfforts: event.target.value })} placeholder="off, low, medium, high" />
                </Field>
                <Field label="默认推理档位" description={`当前：${editingModelAdaptation.item.defaultReasoningEffort ?? '未设置'}`}>
                  <Select value={modelAdaptationForm.defaultReasoningEffort} onValueChange={(value) => setModelAdaptationForm({ ...modelAdaptationForm, defaultReasoningEffort: value as Inheritable<ReasoningEffort> })}>
                    <SelectTrigger aria-label="默认推理档位"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {['inherit', 'off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'].map((value) => <SelectItem key={value} value={value}>{value === 'inherit' ? '继承目录' : value}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </Field>
                <Field label="推理回放" description={`当前：${editingModelAdaptation.item.reasoningReplay ?? 'none'}`}>
                  <Select value={modelAdaptationForm.reasoningReplay} onValueChange={(value) => setModelAdaptationForm({ ...modelAdaptationForm, reasoningReplay: value as Inheritable<ReasoningReplay> })}>
                    <SelectTrigger aria-label="推理回放"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value="inherit">继承目录</SelectItem>
                      <SelectItem value="none">none</SelectItem>
                      <SelectItem value="same_protocol">same_protocol</SelectItem>
                    </SelectContent>
                  </Select>
                </Field>
              </div>
            </ScrollArea>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => onClose()}>取消</Button>
            <Button onClick={saveModelAdaptation} disabled={updateProviderModel.isPending}>
              {updateProviderModel.isPending ? <><Loader2 className="mr-2 h-4 w-4 animate-spin" />保存中</> : '保存适配画像'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

  )
}

function CapabilityProfileField({
  label,
  value,
  effective,
  onChange,
}: {
  label: string
  value: Inheritable<CapabilitySupport>
  effective?: CapabilitySupport
  onChange: (value: Inheritable<CapabilitySupport>) => void
}) {
  return (
    <Field label={label} description={`当前有效值：${effective ?? 'unknown'}`}>
      <Select value={value} onValueChange={(next) => onChange(next as Inheritable<CapabilitySupport>)}>
        <SelectTrigger aria-label={label}><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem value="inherit">继承目录</SelectItem>
          <SelectItem value="supported">supported</SelectItem>
          <SelectItem value="unsupported">unsupported</SelectItem>
          <SelectItem value="unknown">unknown（失败关闭）</SelectItem>
        </SelectContent>
      </Select>
    </Field>
  )
}

