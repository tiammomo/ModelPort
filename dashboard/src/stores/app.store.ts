import { create } from 'zustand'

type Theme = 'light' | 'dark' | 'system'

interface AppState {
  sidebarCollapsed: boolean
  toggleSidebar: () => void
  setSidebarCollapsed: (collapsed: boolean) => void
  theme: Theme
  setTheme: (theme: Theme) => void
}

function getSystemTheme(): 'light' | 'dark' {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

function applyTheme(theme: Theme) {
  const resolved = theme === 'system' ? getSystemTheme() : theme
  document.documentElement.classList.toggle('dark', resolved === 'dark')
}

export const useAppStore = create<AppState>((set) => ({
  sidebarCollapsed: false,
  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  setSidebarCollapsed: (collapsed) => set({ sidebarCollapsed: collapsed }),
  theme: (localStorage.getItem('modelport_theme') as Theme) || 'system',
  setTheme: (theme) => {
    localStorage.setItem('modelport_theme', theme)
    applyTheme(theme)
    set({ theme })
  },
}))

// Apply theme on load
const savedTheme = (localStorage.getItem('modelport_theme') as Theme) || 'system'
applyTheme(savedTheme)

// Listen for system theme changes
window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
  const theme = useAppStore.getState().theme
  if (theme === 'system') applyTheme('system')
})
