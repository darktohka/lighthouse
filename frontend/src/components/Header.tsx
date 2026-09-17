import { LightBulbIcon } from '@primer/octicons-react'
import { Link, NavLink } from 'react-router-dom'

import { useAuth } from '../lib/auth-context'
import { cx } from '../lib/cx'
import { AvatarMenu } from './AvatarMenu'
import { LinkButton } from './primitives/Button'
import { Spinner } from './primitives/StateViews'
import { SearchBox } from './SearchBox'
import { ThemeToggle } from './ThemeToggle'

function navLinkClass({ isActive }: { isActive: boolean }): string {
  return cx(
    'rounded-md px-2 py-1 text-sm font-medium hover:bg-neutral-subtle',
    isActive ? 'bg-neutral-subtle text-foreground' : 'text-muted',
  )
}

export function Header() {
  const { status, user } = useAuth()

  return (
    <header className="sticky top-0 z-20 border-b border-border bg-canvas-default">
      <div className="mx-auto flex w-full max-w-7xl items-center gap-3 px-4 py-3">
        <Link
          to="/"
          className="flex shrink-0 items-center gap-2 text-base font-semibold"
          aria-label="Lighthouse home"
        >
          <LightBulbIcon size={22} aria-hidden="true" className="text-accent" />
          <span className="hidden sm:inline">Lighthouse</span>
        </Link>

        <nav aria-label="Main" className="flex items-center gap-1">
          <NavLink to="/" end className={navLinkClass}>
            Explore
          </NavLink>
          <NavLink to="/tags" className={navLinkClass}>
            Tags
          </NavLink>
          {user ? (
            <>
              <NavLink to="/dashboard" className={navLinkClass}>
                Dashboard
              </NavLink>
              <NavLink to="/analytics" className={navLinkClass}>
                Analytics
              </NavLink>
            </>
          ) : null}
        </nav>

        <div className="ml-auto flex flex-1 items-center justify-end gap-3">
          <div className="hidden w-full max-w-xs sm:block">
            <SearchBox />
          </div>
          <ThemeToggle />
          {status === 'loading' ? (
            <Spinner className="text-muted" />
          ) : user ? (
            <AvatarMenu />
          ) : (
            <div className="flex items-center gap-2">
              <LinkButton to="/login" size="sm">
                Sign in
              </LinkButton>
              <LinkButton to="/register" size="sm" variant="primary">
                Sign up
              </LinkButton>
            </div>
          )}
        </div>
      </div>

      <div className="border-t border-border px-4 py-2 sm:hidden">
        <SearchBox />
      </div>
    </header>
  )
}
