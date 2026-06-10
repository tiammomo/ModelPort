export const ROUTES = {
  LOGIN: "/login",
  DASHBOARD: "/dashboard",
  API_KEYS: "/api-keys",
  USERS: "/users",
  QUOTAS: "/quotas",
  MODELS: "/models",
  LOGS: "/logs",
  SETTINGS: "/settings",
} as const

export const NAV_ITEMS = [
  { path: ROUTES.DASHBOARD, label: "仪表盘", icon: "LayoutDashboard" },
  { path: ROUTES.API_KEYS, label: "API Keys", icon: "KeyRound" },
  { path: ROUTES.USERS, label: "用户管理", icon: "Users" },
  { path: ROUTES.QUOTAS, label: "配额管理", icon: "Gauge" },
  { path: ROUTES.MODELS, label: "模型管理", icon: "Boxes" },
  { path: ROUTES.LOGS, label: "请求日志", icon: "ScrollText" },
  { path: ROUTES.SETTINGS, label: "系统设置", icon: "Settings" },
] as const

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
