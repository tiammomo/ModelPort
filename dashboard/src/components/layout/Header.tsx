import { useAuthStore, useAppStore } from '@/stores'
import { BreadcrumbNav } from '@/components/shared/BreadcrumbNav'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Button } from '@/components/ui/button'
import { isMockMode } from '@/lib/mock-mode'
import { ROLE_LABELS } from '@/lib/constants'
import { Moon, Sun, Monitor, LogOut, Menu, Search } from 'lucide-react'
import { useState } from 'react'

interface HeaderProps {
  onMenuClick?: () => void
  isMobile?: boolean
  mobileMenuOpen?: boolean
}

export function Header({ onMenuClick, isMobile, mobileMenuOpen }: HeaderProps) {
  const currentUser = useAuthStore((s) => s.currentUser)
  const logout = useAuthStore((s) => s.logout)
  const theme = useAppStore((s) => s.theme)
  const setTheme = useAppStore((s) => s.setTheme)

  const themeIcons = { light: Sun, dark: Moon, system: Monitor }
  const ThemeIcon = themeIcons[theme]

  // Keyboard shortcut hint for command palette
  const [isMac] = useState(() => navigator.platform.includes('Mac'))
  const openCommandPalette = () => document.dispatchEvent(new CustomEvent('modelport:open-command-palette'))

  return (
    <header className="flex h-14 items-center justify-between border-b border-border/70 bg-background/86 px-4 backdrop-blur-xl md:px-6">
      <div className="flex items-center gap-3 min-w-0">
        {isMobile && (
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8 shrink-0"
            onClick={onMenuClick}
            aria-expanded={mobileMenuOpen}
            aria-controls="mobile-navigation"
          >
            <Menu className="h-4 w-4" />
            <span className="sr-only">打开导航菜单</span>
          </Button>
        )}
        <div className="min-w-0">
          <BreadcrumbNav />
        </div>
        {isMockMode && (
          <span className="shrink-0 rounded-full border border-amber-200/80 bg-amber-50 px-2 py-0.5 text-[11px] font-medium text-amber-700 dark:border-amber-900/70 dark:bg-amber-950/45 dark:text-amber-300">
            演示数据
          </span>
        )}
      </div>

      <div className="flex items-center gap-1.5">
        {/* Command palette trigger */}
        {!isMobile && (
          <Button
            variant="outline"
            size="sm"
            className="h-8 gap-2 text-xs text-muted-foreground"
            onClick={openCommandPalette}
            aria-label="打开快速导航"
          >
            <Search className="h-3.5 w-3.5" />
            <span>快速跳转</span>
            <kbd className="pointer-events-none ml-1 select-none rounded border border-border/75 bg-muted/70 px-1.5 font-mono text-[10px] font-medium">
              {isMac ? '⌘' : 'Ctrl+'}K
            </kbd>
          </Button>
        )}

        {isMobile && (
          <Button variant="ghost" size="icon" className="h-8 w-8" onClick={openCommandPalette} aria-label="打开快速导航">
            <Search className="h-4 w-4" />
          </Button>
        )}

        {/* Theme toggle */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" className="h-8 w-8" aria-label="切换界面主题">
              <ThemeIcon className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => setTheme('light')}>
              <Sun className="mr-2 h-4 w-4" />
              浅色
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => setTheme('dark')}>
              <Moon className="mr-2 h-4 w-4" />
              深色
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => setTheme('system')}>
              <Monitor className="mr-2 h-4 w-4" />
              跟随系统
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>

        {/* User menu */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" className="relative h-8 w-8 rounded-full" aria-label="打开账户菜单">
              <Avatar className="h-8 w-8 border border-primary/15 ring-2 ring-primary/10">
                <AvatarFallback className="bg-primary/10 text-primary text-xs font-semibold">
                  {currentUser?.username?.charAt(0).toUpperCase() || 'U'}
                </AvatarFallback>
              </Avatar>
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent className="w-56" align="end" forceMount>
            <DropdownMenuLabel className="font-normal">
              <div className="flex flex-col space-y-1">
                <p className="text-sm font-medium leading-none">
                  {currentUser?.username || '用户'}
                </p>
                <p className="text-xs leading-none text-muted-foreground">
                  {currentUser?.email || ''}
                </p>
                <p className="pt-1 text-xs font-medium text-primary">
                  {ROLE_LABELS[currentUser?.role || ''] || currentUser?.role || '未知角色'}
                </p>
              </div>
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={logout} className="text-destructive focus:text-destructive">
              <LogOut className="mr-2 h-4 w-4" />
              退出登录
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </header>
  )
}
