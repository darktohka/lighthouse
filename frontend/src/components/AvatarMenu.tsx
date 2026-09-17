import { ChevronDownIcon, SignOutIcon } from '@primer/octicons-react'
import { useEffect, useRef, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'

import { useAuth } from '../lib/auth-context'
import { cx } from '../lib/cx'
import { Avatar } from './primitives/Avatar'

export function AvatarMenu() {
  const { user, logout } = useAuth()
  const navigate = useNavigate()
  const [open, setOpen] = useState(false)
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onPointerDown = (event: MouseEvent) => {
      const target = event.target
      if (
        containerRef.current &&
        target instanceof Node &&
        !containerRef.current.contains(target)
      ) {
        setOpen(false)
      }
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', onPointerDown)
    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('mousedown', onPointerDown)
      document.removeEventListener('keydown', onKeyDown)
    }
  }, [open])

  if (!user) return null

  const displayName =
    [user.first_name, user.last_name].filter(Boolean).join(' ') || user.username

  const handleLogout = () => {
    setOpen(false)
    void logout().then(() => navigate('/login'))
  }

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((previous) => !previous)}
        className="flex items-center gap-1 rounded-full p-0.5 hover:bg-neutral-subtle"
      >
        <Avatar src={user.avatar_url} name={displayName} size={26} />
        <ChevronDownIcon size={14} aria-hidden="true" className="text-muted" />
        <span className="sr-only">Account menu</span>
      </button>

      {open ? (
        <div
          role="menu"
          className="absolute right-0 z-30 mt-2 w-60 rounded-md border border-border bg-canvas-overlay py-1 shadow-medium"
        >
          <div className="border-b border-border px-3 py-2">
            <p className="truncate text-sm font-semibold">{displayName}</p>
            <p className="truncate text-xs text-muted">@{user.username}</p>
          </div>
          <Link
            role="menuitem"
            to="/dashboard"
            onClick={() => setOpen(false)}
            className={menuItemClass}
          >
            Dashboard
          </Link>
          <Link
            role="menuitem"
            to={`/${encodeURIComponent(user.username)}`}
            onClick={() => setOpen(false)}
            className={menuItemClass}
          >
            Your repositories
          </Link>
          <Link
            role="menuitem"
            to={`/users/${encodeURIComponent(user.username)}`}
            onClick={() => setOpen(false)}
            className={menuItemClass}
          >
            Your profile
          </Link>
          <Link
            role="menuitem"
            to="/new"
            onClick={() => setOpen(false)}
            className={menuItemClass}
          >
            New workspace
          </Link>
          <Link
            role="menuitem"
            to="/service-accounts"
            onClick={() => setOpen(false)}
            className={menuItemClass}
          >
            Service accounts
          </Link>
          <Link
            role="menuitem"
            to="/settings"
            onClick={() => setOpen(false)}
            className={menuItemClass}
          >
            Settings
          </Link>
          <div className="my-1 border-t border-border" />
          <button
            type="button"
            role="menuitem"
            onClick={handleLogout}
            className={cx(menuItemClass, 'w-full text-left text-danger')}
          >
            <SignOutIcon size={14} aria-hidden="true" />
            Sign out
          </button>
        </div>
      ) : null}
    </div>
  )
}

const menuItemClass =
  'flex items-center gap-2 px-3 py-1.5 text-sm text-foreground hover:bg-neutral-subtle'
