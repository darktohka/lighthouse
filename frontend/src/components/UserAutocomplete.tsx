import { SearchIcon } from '@primer/octicons-react'
import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'

import { isApiError } from '../api/client'
import { directory } from '../api/endpoints'
import type { UserSummary } from '../api/schemas'
import { cx } from '../lib/cx'
import { inputClasses } from '../lib/ui'
import { Avatar } from './primitives/Avatar'
import { Spinner } from './primitives/StateViews'

export type UserAutocompleteProps = {
  label: string
  value: string
  onChange: (username: string) => void
  onSelect?: (user: UserSummary) => void
  hint?: ReactNode
  error?: string | null
  disabled?: boolean
}

export function UserAutocomplete({
  label,
  value,
  onChange,
  onSelect,
  hint,
  error,
  disabled,
}: UserAutocompleteProps) {
  const [open, setOpen] = useState(false)
  const [results, setResults] = useState<UserSummary[]>([])
  const [loading, setLoading] = useState(false)
  const [searchError, setSearchError] = useState<string | null>(null)
  const [highlight, setHighlight] = useState(-1)

  const generatedId = useId()
  const inputId = `user-search-${generatedId}`
  const listboxId = `${inputId}-listbox`
  const rootRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const term = value.trim()
    const controller = new AbortController()
    const timer = window.setTimeout(
      () => {
        if (term.length === 0) {
          setResults([])
          setSearchError(null)
          setLoading(false)
          return
        }
        setLoading(true)
        directory.searchUsers(term, 8, { signal: controller.signal }).then(
          (users) => {
            setLoading(false)
            setResults(users)
            setHighlight(users.length > 0 ? 0 : -1)
            setOpen(true)
          },
          (reason: unknown) => {
            if (controller.signal.aborted) return
            setLoading(false)
            setResults([])
            setSearchError(
              isApiError(reason) ? reason.message : 'Could not search for users.',
            )
          },
        )
      },
      term.length === 0 ? 0 : 250,
    )
    return () => {
      window.clearTimeout(timer)
      controller.abort()
    }
  }, [value])

  useEffect(() => {
    const onPointerDown = (event: MouseEvent) => {
      if (
        rootRef.current &&
        event.target instanceof Node &&
        !rootRef.current.contains(event.target)
      ) {
        setOpen(false)
      }
    }
    document.addEventListener('mousedown', onPointerDown)
    return () => document.removeEventListener('mousedown', onPointerDown)
  }, [])

  const select = (user: UserSummary) => {
    onChange(user.username)
    onSelect?.(user)
    setOpen(false)
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (!open || results.length === 0) return
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      setHighlight((index) => (index + 1) % results.length)
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      setHighlight((index) => (index - 1 + results.length) % results.length)
    } else if (event.key === 'Enter') {
      if (highlight >= 0) {
        event.preventDefault()
        select(results[highlight])
      }
    } else if (event.key === 'Escape') {
      setOpen(false)
    }
  }

  return (
    <div ref={rootRef} className="relative space-y-1">
      <label htmlFor={inputId} className="block text-sm font-medium">
        {label}
      </label>
      {hint ? <p className="text-xs text-muted">{hint}</p> : null}
      <div className="relative">
        <SearchIcon
          size={16}
          aria-hidden="true"
          className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted"
        />
        <input
          id={inputId}
          role="combobox"
          aria-expanded={open}
          aria-controls={listboxId}
          aria-autocomplete="list"
          aria-activedescendant={
            open && highlight >= 0 ? `${listboxId}-option-${highlight}` : undefined
          }
          aria-invalid={error ? true : undefined}
          disabled={disabled}
          autoComplete="off"
          value={value}
          onChange={(event) => {
            onChange(event.target.value)
            setSearchError(null)
            setResults([])
            setLoading(false)
          }}
          onFocus={() => setOpen(true)}
          onKeyDown={onKeyDown}
          className={inputClasses(cx('pl-8', error ? 'border-danger' : undefined))}
        />
        {loading ? (
          <Spinner className="absolute right-2 top-1/2 -translate-y-1/2 text-muted" />
        ) : null}
      </div>
      {error ? (
        <p className="text-xs text-danger" aria-live="polite">
          {error}
        </p>
      ) : null}
      {searchError ? (
        <p className="text-xs text-danger" aria-live="polite">
          {searchError}
        </p>
      ) : null}
      {open && !loading && value.trim().length > 0 && !searchError ? (
        <ul
          id={listboxId}
          role="listbox"
          aria-label="Matching users"
          className="absolute z-20 mt-1 max-h-60 w-full overflow-auto rounded-md border border-border bg-canvas-overlay py-1 shadow-medium scrollbar-thin"
        >
          {results.length === 0 ? (
            <li className="px-3 py-2 text-xs text-muted">No users found.</li>
          ) : (
            results.map((user, index) => (
              <li
                key={user.id}
                id={`${listboxId}-option-${index}`}
                role="option"
                aria-selected={highlight === index}
                onMouseEnter={() => setHighlight(index)}
                onMouseDown={(event) => {
                  event.preventDefault()
                  select(user)
                }}
                className={cx(
                  'flex cursor-pointer items-center gap-2 px-3 py-1.5 text-sm',
                  highlight === index ? 'bg-accent-subtle' : undefined,
                )}
              >
                <Avatar src={user.avatar_url} name={user.username} size={20} />
                <span className="font-medium">{user.username}</span>
                {[user.first_name, user.last_name].filter(Boolean).length > 0 ? (
                  <span className="truncate text-xs text-muted">
                    {[user.first_name, user.last_name].filter(Boolean).join(' ')}
                  </span>
                ) : null}
              </li>
            ))
          )}
        </ul>
      ) : null}
    </div>
  )
}
