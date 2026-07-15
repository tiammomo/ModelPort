import { createBrowserRouter, RouterProvider, Navigate } from 'react-router-dom'
import { QueryClientProvider } from '@tanstack/react-query'
import { lazy, Suspense, useEffect, Component, type ReactNode, type ErrorInfo } from 'react'
import { useAuthStore } from '@/stores'
import { queryClient } from '@/lib/query-client'
import { AppLayout } from '@/components/layout/AppLayout'
import { ProtectedRoute, RoleRoute } from '@/components/layout/ProtectedRoute'
import { LoadingPage } from '@/components/shared/LoadingPage'
import { ErrorState } from '@/components/shared/ErrorState'

const LoginPage = lazy(() => import('@/pages/LoginPage').then((module) => ({ default: module.LoginPage })))
const DashboardPage = lazy(() => import('@/pages/DashboardPage').then((module) => ({ default: module.DashboardPage })))
const ApiKeysPage = lazy(() => import('@/pages/ApiKeysPage').then((module) => ({ default: module.ApiKeysPage })))
const UsersPage = lazy(() => import('@/pages/UsersPage').then((module) => ({ default: module.UsersPage })))
const QuotasPage = lazy(() => import('@/pages/QuotasPage').then((module) => ({ default: module.QuotasPage })))
const ModelsPage = lazy(() => import('@/pages/ModelsPage').then((module) => ({ default: module.ModelsPage })))
const LogsPage = lazy(() => import('@/pages/LogsPage').then((module) => ({ default: module.LogsPage })))
const EnterprisePage = lazy(() => import('@/pages/EnterprisePage').then((module) => ({ default: module.EnterprisePage })))
const SettingsPage = lazy(() => import('@/pages/SettingsPage').then((module) => ({ default: module.SettingsPage })))
const NotFoundPage = lazy(() => import('@/pages/NotFoundPage').then((module) => ({ default: module.NotFoundPage })))

function PageLoader() {
  return <LoadingPage />
}

interface ErrorBoundaryState {
  hasError: boolean
  error: Error | null
}

class ErrorBoundary extends Component<{ children: ReactNode }, ErrorBoundaryState> {
  state: ErrorBoundaryState = { hasError: false, error: null }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('Route error:', error, info)
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="flex h-64 items-center justify-center">
          <ErrorState
            title="页面加载出错"
            message={this.state.error?.message || '发生了未知错误'}
            onRetry={() => this.setState({ hasError: false, error: null })}
          />
        </div>
      )
    }
    return this.props.children
  }
}

function lazyPage(page: ReactNode) {
  return (
    <Suspense fallback={<PageLoader />}>
      <ErrorBoundary>{page}</ErrorBoundary>
    </Suspense>
  )
}

const router = createBrowserRouter([
  {
    path: '/login',
    element: lazyPage(<LoginPage />),
  },
  {
    path: '/',
    element: (
      <ProtectedRoute>
        <AppLayout />
      </ProtectedRoute>
    ),
    children: [
      { index: true, element: <Navigate to="/dashboard" replace /> },
      { path: 'dashboard', element: lazyPage(<DashboardPage />) },
      { path: 'api-keys', element: lazyPage(<ApiKeysPage />) },
      { path: 'users', element: <RoleRoute roles={['admin']}>{lazyPage(<UsersPage />)}</RoleRoute> },
      { path: 'quotas', element: <RoleRoute roles={['admin']}>{lazyPage(<QuotasPage />)}</RoleRoute> },
      { path: 'models', element: <RoleRoute roles={['admin']}>{lazyPage(<ModelsPage />)}</RoleRoute> },
      { path: 'logs', element: lazyPage(<LogsPage />) },
      { path: 'enterprise', element: <RoleRoute roles={['admin']}>{lazyPage(<EnterprisePage />)}</RoleRoute> },
      { path: 'settings', element: <RoleRoute roles={['admin']}>{lazyPage(<SettingsPage />)}</RoleRoute> },
    ],
  },
  { path: '*', element: lazyPage(<NotFoundPage />) },
])

function App() {
  const initialize = useAuthStore((s) => s.initialize)

  useEffect(() => {
    initialize()
  }, [initialize])

  return (
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  )
}

export default App
