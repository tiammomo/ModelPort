export const ROUTES = {
  LOGIN: "/login",
  DASHBOARD: "/dashboard",
  GUIDE: "/guide",
  API_KEYS: "/api-keys",
  USERS: "/users",
  QUOTAS: "/quotas",
  MODELS: "/models",
  LOGS: "/logs",
  ENTERPRISE: "/enterprise",
  GOVERNANCE: "/governance",
  OPERATIONS: "/operations",
  SETTINGS: "/settings",
} as const

export const NAV_ITEMS = [
  { path: ROUTES.DASHBOARD, label: "仪表盘", icon: "LayoutDashboard", section: "概览", keywords: "概览 监控 健康", adminOnly: false },
  { path: ROUTES.MODELS, label: "模型目录", icon: "Boxes", section: "模型接入", keywords: "模型 provider 供应商 路由 凭证", adminOnly: false },
  { path: ROUTES.GUIDE, label: "用户使用说明", icon: "BookOpen", section: "模型接入", keywords: "帮助 接入 API Key Claude Code OpenAI", adminOnly: false },
  { path: ROUTES.LOGS, label: "请求日志", icon: "ScrollText", section: "请求与用量", keywords: "请求 错误 trace 延迟 费用", adminOnly: false },
  { path: ROUTES.ENTERPRISE, label: "运行账本", icon: "ShieldCheck", section: "请求与用量", keywords: "账本 ledger 幂等 租约 attempt 预算", adminOnly: true },
  { path: ROUTES.GOVERNANCE, label: "治理与审批", icon: "Scale", section: "团队与策略", keywords: "审批 路由 外发 Provider 策略", adminOnly: true },
  { path: ROUTES.USERS, label: "用户管理", icon: "Users", section: "团队与策略", keywords: "用户 角色 权限 账号", adminOnly: true },
  { path: ROUTES.API_KEYS, label: "API 密钥", icon: "KeyRound", section: "团队与策略", keywords: "api key token 密钥 项目", adminOnly: false },
  { path: ROUTES.QUOTAS, label: "配额管理", icon: "Gauge", section: "团队与策略", keywords: "配额 token 请求 费用 限额", adminOnly: true },
  { path: ROUTES.SETTINGS, label: "系统设置", icon: "Settings", section: "系统", keywords: "配置 运维 安全 备份", adminOnly: true },
  { path: ROUTES.OPERATIONS, label: "运维事件", icon: "Siren", section: "系统", keywords: "运维 agent 事件 incident 告警 恢复", adminOnly: true },
] as const

export const NAV_SECTIONS = ["概览", "模型接入", "请求与用量", "团队与策略", "系统"] as const

export function navGroupsForRole(role?: string) {
  const visible = navItemsForRole(role)
  return NAV_SECTIONS.flatMap((section) => {
    const items = visible.filter((item) => item.section === section)
    if (items.length === 0) return []
    return [{ label: items.length === 1 && role !== 'admin' ? items[0].label : section, path: items[0].path, icon: items[0].icon, items }]
  })
}

export function navItemsForRole(role?: string) {
  return NAV_ITEMS.filter((item) => !item.adminOnly || role === "admin")
}

export const ROLE_LABELS: Record<string, string> = {
  admin: "管理员",
  user: "普通用户",
  viewer: "只读用户",
}

export const STATUS_COLORS: Record<string, string> = {
  active: "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-300",
  disabled: "bg-gray-100 text-gray-800 dark:bg-gray-800 dark:text-gray-300",
  suspended: "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-300",
  success: "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-300",
  error: "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-300",
  timeout: "bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-300",
  healthy: "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-300",
  degraded: "bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-300",
  down: "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-300",
  inactive: "bg-gray-100 text-gray-800 dark:bg-gray-800 dark:text-gray-300",
  error_badge: "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-300",
}

export const PROVIDER_PROTOCOL_LABELS: Record<string, string> = {
  anthropic: "Anthropic",
  "openai-compat": "OpenAI 兼容",
}
