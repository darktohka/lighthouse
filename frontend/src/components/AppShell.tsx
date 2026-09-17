import { Outlet } from 'react-router-dom'

import { Header } from './Header'

export function AppShell() {
  return (
    <div className="flex min-h-svh flex-col bg-canvas-default">
      <a
        href="#content"
        className="sr-only focus:not-sr-only focus:absolute focus:left-4 focus:top-4 focus:z-50 focus:rounded-md focus:border focus:border-border focus:bg-canvas-overlay focus:px-3 focus:py-2 focus:text-sm"
      >
        Skip to content
      </a>
      <Header />
      <main id="content" className="mx-auto w-full max-w-7xl flex-1 px-4 py-6">
        <Outlet />
      </main>
      <footer className="border-t border-border py-6">
        <div className="mx-auto max-w-7xl px-4 text-center text-xs text-muted">
          Lighthouse · container registry control plane
        </div>
      </footer>
    </div>
  )
}
