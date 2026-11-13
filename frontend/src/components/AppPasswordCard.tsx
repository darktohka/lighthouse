import { ChevronRightIcon, KeyIcon, SyncIcon } from '@primer/octicons-react'
import { useId, useState } from 'react'

import { isApiError } from '../api/client'
import { appPasswords } from '../api/endpoints'
import type { AppPassword, AppPasswordCategory } from '../api/schemas'
import { cx } from '../lib/cx'
import { formatDateTime, formatRelativeTime } from '../lib/format'
import { compactInputClasses } from '../lib/ui'
import { ConfirmAction } from './ConfirmAction'
import { OneTimeToken } from './OneTimeToken'
import { Flash } from './primitives/Flash'

export type AppPasswordCardProps = {
  appPassword: AppPassword
  categories: AppPasswordCategory[]
  /** Expansion is owned by the panel so only one row is open at a time. */
  expanded: boolean
  onToggle: () => void
  /** Token minted for this app password; rendered inside its own row. */
  pendingToken: string | null
  onTokenDismiss: () => void
  onChanged: () => void
  onToken: (token: string, label: string) => void
}

/**
 * One app password as a single compact line: name, token fingerprint, category
 * and the last time it was used. Regenerating and deleting live behind the
 * chevron so the list stays scannable.
 */
export function AppPasswordCard({
  appPassword,
  categories,
  expanded,
  onToggle,
  pendingToken,
  onTokenDismiss,
  onChanged,
  onToken,
}: AppPasswordCardProps) {
  const [busy, setBusy] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [categoryError, setCategoryError] = useState<string | null>(null)
  const bodyId = useId()
  const categoryFieldId = useId()

  const fail = (
    error: unknown,
    fallback: string,
    setter: (message: string | null) => void,
  ) => {
    setBusy(null)
    setter(isApiError(error) ? error.message : fallback)
  }

  const category = appPassword.category_id
    ? categories.find((entry) => entry.id === appPassword.category_id)
    : undefined

  // A category that has since been deleted would leave the <select> showing a
  // value no option carries, so fall back to "Uncategorized" for display.
  const selectedCategoryId = category ? String(category.id) : ''

  const rotate = () => {
    setActionError(null)
    setBusy('rotate')
    void appPasswords.rotate(appPassword.id).then(
      (response) => {
        setBusy(null)
        onToken(response.token, response.app_password.name)
        onChanged()
      },
      (error: unknown) =>
        fail(error, 'The app password could not be regenerated.', setActionError),
    )
  }

  const remove = () => {
    setActionError(null)
    setBusy('delete')
    void appPasswords.remove(appPassword.id).then(
      () => {
        setBusy(null)
        onChanged()
      },
      (error: unknown) =>
        fail(error, 'The app password could not be deleted.', setActionError),
    )
  }

  const moveToCategory = (value: string) => {
    setCategoryError(null)
    setBusy('category')
    void appPasswords
      .updateCategory(appPassword.id, {
        category_id: value === '' ? null : Number(value),
      })
      .then(
        () => {
          setBusy(null)
          onChanged()
        },
        (error: unknown) =>
          fail(error, 'The category could not be changed.', setCategoryError),
      )
  }

  return (
    <li
      className="scroll-mt-24 border-b border-border last:border-b-0"
      data-app-password={appPassword.id}
    >
      <div className="flex items-center gap-2 pr-2 hover:bg-canvas-subtle">
        <button
          type="button"
          aria-expanded={expanded}
          aria-controls={expanded ? bodyId : undefined}
          onClick={onToggle}
          className="flex min-w-0 flex-1 items-center gap-2 py-1.5 pl-3 text-left"
        >
          <ChevronRightIcon
            size={14}
            aria-hidden="true"
            className={cx(
              'shrink-0 text-muted transition-transform',
              expanded && 'rotate-90',
            )}
          />
          <KeyIcon size={14} aria-hidden="true" className="shrink-0 text-muted" />
          <span className="truncate text-sm font-medium">{appPassword.name}</span>
          <span className="hidden shrink-0 font-mono text-xs text-muted sm:inline">
            {appPassword.token_prefix}…{appPassword.token_suffix}
          </span>
          <span className="ml-auto hidden shrink-0 text-xs text-muted sm:inline">
            {appPassword.last_used_at
              ? formatRelativeTime(appPassword.last_used_at)
              : 'Never used'}
          </span>
        </button>

        <label htmlFor={categoryFieldId} className="sr-only">
          Category for {appPassword.name}
        </label>
        <span className="w-28 shrink-0 sm:w-36">
          <select
            id={categoryFieldId}
            value={selectedCategoryId}
            disabled={busy === 'category'}
            onChange={(event) => moveToCategory(event.target.value)}
            className={compactInputClasses(
              'h-6 border-transparent bg-transparent text-muted hover:border-border',
            )}
          >
            <option value="">Uncategorized</option>
            {categories.map((entry) => (
              <option key={entry.id} value={String(entry.id)}>
                {entry.name}
              </option>
            ))}
          </select>
        </span>
      </div>

      {categoryError ? (
        <Flash
          variant="danger"
          className="mx-3 mb-2"
          onDismiss={() => setCategoryError(null)}
        >
          {categoryError}
        </Flash>
      ) : null}

      {expanded ? (
        <div id={bodyId} className="space-y-3 px-3 pb-3">
          {pendingToken ? (
            <div className="pt-2">
              <OneTimeToken
                token={pendingToken}
                title={`App password for ${appPassword.name}`}
                onDismiss={onTokenDismiss}
              />
            </div>
          ) : null}

          {actionError ? (
            <Flash variant="danger" onDismiss={() => setActionError(null)}>
              {actionError}
            </Flash>
          ) : null}

          <dl className="grid gap-2 pl-6 text-xs sm:grid-cols-3">
            <div>
              <dt className="text-muted">Category</dt>
              <dd>{category ? category.name : 'Uncategorized'}</dd>
            </div>
            <div>
              <dt className="text-muted">Created</dt>
              <dd>{formatDateTime(appPassword.created_at)}</dd>
            </div>
            <div>
              <dt className="text-muted">Last used</dt>
              <dd>
                {appPassword.last_used_at
                  ? formatRelativeTime(appPassword.last_used_at)
                  : 'Never'}
              </dd>
            </div>
          </dl>

          <div className="flex flex-wrap items-center gap-2 border-t border-border pt-2 pl-6">
            <ConfirmAction
              label="Regenerate"
              confirmLabel="Regenerate secret"
              resourceName={`the secret for ${appPassword.name}`}
              pending={busy === 'rotate'}
              onConfirm={rotate}
            />
            <ConfirmAction
              label="Delete"
              confirmLabel="Delete app password"
              resourceName={appPassword.name}
              pending={busy === 'delete'}
              onConfirm={remove}
            />
            <span className="inline-flex items-center gap-1 text-xs text-muted">
              <SyncIcon size={12} aria-hidden="true" />
              Regenerating invalidates the current token immediately.
            </span>
          </div>
        </div>
      ) : null}
    </li>
  )
}
