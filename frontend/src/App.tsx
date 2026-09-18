import { lazy, Suspense } from 'react'
import { BrowserRouter, Route, Routes } from 'react-router-dom'

import { AppShell } from './components/AppShell'
import { RequireAuth } from './components/RequireAuth'
import { LoadingState } from './components/primitives/StateViews'
import { DashboardPage } from './pages/DashboardPage'
import { ExplorePage } from './pages/ExplorePage'
import { ForgotPasswordPage } from './pages/ForgotPasswordPage'
import { LoginPage } from './pages/LoginPage'
import { NamespacePage } from './pages/NamespacePage'
import { NamespaceSettingsPage } from './pages/NamespaceSettingsPage'
import { NotFoundPage } from './pages/NotFoundPage'
import { ProfilePage } from './pages/ProfilePage'
import { RegisterPage } from './pages/RegisterPage'
import { RepositoryCreatePage } from './pages/RepositoryCreatePage'
import { RepositoryRouteView } from './pages/RepositoryRouteView'
import { RepositorySettingsRouteView } from './pages/RepositorySettingsRouteView'
import { ResetPasswordPage } from './pages/ResetPasswordPage'
import { SettingsPage } from './pages/SettingsPage'
import { TagCleanupPage } from './pages/TagCleanupPage'
import { VerifyEmailPage } from './pages/VerifyEmailPage'
import { WorkspaceCreatePage } from './pages/WorkspaceCreatePage'

const AnalyticsPage = lazy(() =>
  import('./pages/AnalyticsPage').then((module) => ({
    default: module.AnalyticsPage,
  })),
)

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<AppShell />}>
          <Route index element={<ExplorePage />} />
          <Route path="explore" element={<ExplorePage />} />
          <Route path="login" element={<LoginPage />} />
          <Route path="register" element={<RegisterPage />} />
          <Route path="verify-email" element={<VerifyEmailPage />} />
          <Route path="forgot-password" element={<ForgotPasswordPage />} />
          <Route path="reset-password" element={<ResetPasswordPage />} />
          <Route
            path="dashboard"
            element={
              <RequireAuth>
                <DashboardPage />
              </RequireAuth>
            }
          />
          <Route
            path="analytics"
            element={
              <Suspense fallback={<LoadingState label="Loading analytics…" />}>
                <AnalyticsPage />
              </Suspense>
            }
          />
          <Route
            path="tags"
            element={
              <RequireAuth>
                <TagCleanupPage />
              </RequireAuth>
            }
          />
          <Route path="users/:username" element={<ProfilePage />} />
          <Route
            path="settings"
            element={
              <RequireAuth>
                <SettingsPage />
              </RequireAuth>
            }
          />
          <Route
            path="new"
            element={
              <RequireAuth>
                <WorkspaceCreatePage />
              </RequireAuth>
            }
          />
          <Route
            path="new/repository"
            element={
              <RequireAuth>
                <RepositoryCreatePage />
              </RequireAuth>
            }
          />
          <Route
            path="namespaces/:name/settings"
            element={
              <RequireAuth>
                <NamespaceSettingsPage />
              </RequireAuth>
            }
          />
          <Route
            path="repositories/:namespace/*"
            element={
              <RequireAuth>
                <RepositorySettingsRouteView />
              </RequireAuth>
            }
          />
          <Route path=":namespace" element={<NamespacePage />} />
          <Route path=":namespace/*" element={<RepositoryRouteView />} />
          <Route path="*" element={<NotFoundPage />} />
        </Route>
      </Routes>
    </BrowserRouter>
  )
}
