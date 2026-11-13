import {
  ChevronRightIcon,
  KeyIcon,
  SyncIcon,
} from '@primer/octicons-react'
import { useId, useState } from 'react'

import { isApiError } from '../api/client'
import {
  serviceAccountGrants,
  serviceAccounts as serviceAccountsApi,
} from '../api/endpoints'
import type { ServiceAccount, ServiceAccountCategory } from '../api/schemas'
import { cx } from '../lib/cx'
import { formatDateTime, formatRelativeTime } from '../lib/format'
import { compactInputClasses } from '../lib/ui'
import { ConfirmAction } from './ConfirmAction'
import { EMPTY_GRANT_DRAFT } from '../lib/serviceAccountGrants'
import { GrantDraftRow, IpRangeDraftRow } from './ServiceAccountRows'
import { Flash } from './primitives/Flash'
import { Label } from './primitives/Label'

export type ServiceAccountCardProps = {
  account: ServiceAccount
  categories: ServiceAccountCategory[]
  onChanged: () => void
  onToken: (token: string, label: string) => void
}

function plural(count: number, singular: string, pluralForm = `${singular}s`) {
  return `${count} ${count === 1 ? singular : pluralForm}`
}

/**
 * One account as a single compact line: name, token fingerprint, category,
 * a grant/IP summary and the last time it was used. Everything destructive or
 * detailed lives behind the chevron, so the list stays scannable.
 */
export function ServiceAccountCard({
  account,
  categories,
  onChanged,
  onToken,
}: ServiceAccountCardProps) {
  const [expanded, setExpanded] = useState(false)
  const [grantDraft, setGrantDraft] = useState(EMPTY_GRANT_DRAFT)
  const [newRange, setNewRange] = useState('')
  const [busy, setBusy] = useState<string | null>(null)
  const [grantError, setGrantError] = useState<string | null>(null)
  const [ipError, setIpError] = useState<string | null>(null)
  const [accountError, setAccountError] = useState<string | null>(null)
  const [categoryError, setCategoryError] = useState<string | null>(null)
  const bodyId = useId()
  const categoryId = useId()

  const fail = (
    error: unknown,
    fallback: string,
    setter: (message: string | null) => void,
  ) => {
    setBusy(null)
    setter(isApiError(error) ? error.message : fallback)
  }

  const category = account.category_id
    ? categories.find((entry) => entry.id === account.category_id)
    : undefined

  // A category that has since been deleted would leave the <select> showing a
  // value no option carries, so fall back to "Uncategorized" for display.
  const selectedCategoryId = category ? String(category.id) : ''

  const addGrant = () => {
    setGrantError(null)
    const target = grantDraft.target.trim()
    if (target.length === 0) {
      setGrantError('Enter a namespace or repository name.')
      return
    }
    setBusy('grant')
    void serviceAccountGrants
      .add(account.id, {
        ...(grantDraft.kind === 'namespace'
          ? { namespace: target }
          : { repository: target }),
        can_push: grantDraft.canPush,
      })
      .then(
        () => {
          setBusy(null)
          setGrantDraft(EMPTY_GRANT_DRAFT)
          onChanged()
        },
        (error: unknown) =>
          fail(error, 'The grant could not be added.', setGrantError),
      )
  }

  const addIpRange = () => {
    setIpError(null)
    const value = newRange.trim()
    if (value.length === 0) {
      setIpError('Enter an IP address or range.')
      return
    }
    setBusy('ip-range')
    void serviceAccountsApi.addIpRange(account.id, { cidr: value }).then(
      () => {
        setBusy(null)
        setNewRange('')
        onChanged()
      },
      (error: unknown) =>
        fail(error, 'The IP range could not be added.', setIpError),
    )
  }

  const removeGrant = (grantId: number) => {
    setGrantError(null)
    setBusy(`grant-${grantId}`)
    void serviceAccountsApi.removeGrant(account.id, grantId).then(
      () => {
        setBusy(null)
        onChanged()
      },
      (error: unknown) =>
        fail(error, 'The grant could not be removed.', setGrantError),
    )
  }

  const removeIpRange = (rangeId: number) => {
    setIpError(null)
    setBusy(`ip-range-${rangeId}`)
    void serviceAccountsApi.removeIpRange(account.id, rangeId).then(
      () => {
        setBusy(null)
        onChanged()
      },
      (error: unknown) =>
        fail(error, 'The IP range could not be removed.', setIpError),
    )
  }

  const rotate = () => {
    setAccountError(null)
    setBusy('rotate')
    void serviceAccountsApi.rotate(account.id).then(
      (response) => {
        setBusy(null)
        onToken(response.token, account.name)
        onChanged()
      },
      (error: unknown) =>
        fail(error, 'The token could not be rotated.', setAccountError),
    )
  }

  const remove = () => {
    setAccountError(null)
    setBusy('delete')
    void serviceAccountsApi.remove(account.id).then(
      () => {
        setBusy(null)
        onChanged()
      },
      (error: unknown) =>
        fail(error, 'The account could not be deleted.', setAccountError),
    )
  }

  const moveToCategory = (value: string) => {
    setCategoryError(null)
    setBusy('category')
    void serviceAccountsApi
      .updateCategory(account.id, {
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

  const summary = [
    plural(account.grants.length, 'grant'),
    plural(account.ip_ranges.length, 'IP range'),
  ].join(' · ')

  return (
    <li className="border-b border-border last:border-b-0">
      <div className="flex items-center gap-2 pr-2 hover:bg-canvas-subtle">
        <button
          type="button"
          aria-expanded={expanded}
          aria-controls={expanded ? bodyId : undefined}
          onClick={() => setExpanded((value) => !value)}
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
          <span className="truncate text-sm font-medium">{account.name}</span>
          <span className="hidden shrink-0 font-mono text-xs text-muted sm:inline">
            {account.token_prefix}…{account.token_suffix}
          </span>
          <span className="hidden shrink-0 text-xs text-muted md:inline">
            {summary}
          </span>
          <span className="ml-auto hidden shrink-0 text-xs text-muted sm:inline">
            {account.last_used_at
              ? formatRelativeTime(account.last_used_at)
              : 'Never used'}
          </span>
        </button>

        <label htmlFor={categoryId} className="sr-only">
          Category for {account.name}
        </label>
        <span className="w-28 shrink-0 sm:w-36">
          <select
            id={categoryId}
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
          {accountError ? (
            <Flash variant="danger" onDismiss={() => setAccountError(null)}>
              {accountError}
            </Flash>
          ) : null}

          <dl className="grid gap-2 pl-6 text-xs sm:grid-cols-4">
            <div>
              <dt className="text-muted">Username</dt>
              <dd className="truncate font-mono">{account.username}</dd>
            </div>
            <div>
              <dt className="text-muted">Category</dt>
              <dd>{category ? category.name : 'Uncategorized'}</dd>
            </div>
            <div>
              <dt className="text-muted">Created</dt>
              <dd>{formatDateTime(account.created_at)}</dd>
            </div>
            <div>
              <dt className="text-muted">Last used</dt>
              <dd>
                {account.last_used_at
                  ? formatRelativeTime(account.last_used_at)
                  : 'Never'}
              </dd>
            </div>
          </dl>

          {account.description ? (
            <p className="pl-6 text-xs text-muted">{account.description}</p>
          ) : null}

          <section className="space-y-1 pl-6">
            <h3 className="text-xs font-semibold">
              Grants <Label variant="muted">{account.grants.length}</Label>
            </h3>
            {grantError ? (
              <Flash variant="danger" onDismiss={() => setGrantError(null)}>
                {grantError}
              </Flash>
            ) : null}
            <ul className="divide-y divide-border rounded-md border border-border">
              {account.grants.length === 0 ? (
                <li className="px-2 py-1.5 text-xs text-muted">
                  No grants yet — this token can only pull public images.
                </li>
              ) : (
                account.grants.map((grant) => (
                  <li
                    key={grant.id}
                    className="flex items-center justify-between gap-2 px-2 py-1"
                  >
                    <span className="flex min-w-0 items-center gap-2 text-xs">
                      <Label variant="muted">
                        {grant.namespace ? 'namespace' : 'repository'}
                      </Label>
                      <span className="truncate font-mono">
                        {grant.namespace ?? grant.repository}
                      </span>
                      <Label variant={grant.can_push ? 'success' : 'muted'}>
                        {grant.can_push ? 'push + pull' : 'pull'}
                      </Label>
                    </span>
                    <ConfirmAction
                      label="Remove"
                      confirmLabel="Remove grant"
                      resourceName={`grant on ${grant.namespace ?? grant.repository ?? 'resource'}`}
                      pending={busy === `grant-${grant.id}`}
                      onConfirm={() => removeGrant(grant.id)}
                    />
                  </li>
                ))
              )}
              <GrantDraftRow
                draft={grantDraft}
                onChange={setGrantDraft}
                onAdd={addGrant}
                pending={busy === 'grant'}
              />
            </ul>
          </section>

          <section className="space-y-1 pl-6">
            <h3 className="text-xs font-semibold">
              IP allowlist <Label variant="muted">{account.ip_ranges.length}</Label>
            </h3>
            {ipError ? (
              <Flash variant="danger" onDismiss={() => setIpError(null)}>
                {ipError}
              </Flash>
            ) : null}
            <ul className="divide-y divide-border rounded-md border border-border">
              {account.ip_ranges.length === 0 ? (
                <li className="px-2 py-1.5 text-xs text-muted">
                  No IP restriction — this account can authenticate from any
                  address.
                </li>
              ) : (
                account.ip_ranges.map((range) => (
                  <li
                    key={range.id}
                    className="flex items-center justify-between gap-2 px-2 py-1"
                  >
                    <span className="font-mono text-xs">{range.cidr}</span>
                    <ConfirmAction
                      label="Remove"
                      confirmLabel="Remove range"
                      resourceName={`range ${range.cidr}`}
                      pending={busy === `ip-range-${range.id}`}
                      onConfirm={() => removeIpRange(range.id)}
                    />
                  </li>
                ))
              )}
              <IpRangeDraftRow
                value={newRange}
                onChange={setNewRange}
                onAdd={addIpRange}
                pending={busy === 'ip-range'}
              />
            </ul>
            <p className="text-xs text-muted">
              Single addresses become /32 (IPv4) or /128 (IPv6). The client IP
              comes from the reverse proxy&apos;s X-Forwarded-For header.
            </p>
          </section>

          <div className="flex flex-wrap items-center gap-2 border-t border-border pt-2 pl-6">
            <ConfirmAction
              label="Rotate token"
              confirmLabel="Rotate token"
              resourceName={`the token for ${account.name}`}
              pending={busy === 'rotate'}
              onConfirm={rotate}
            />
            <ConfirmAction
              label="Delete account"
              confirmLabel="Delete account"
              resourceName={account.name}
              pending={busy === 'delete'}
              onConfirm={remove}
            />
            <span className="inline-flex items-center gap-1 text-xs text-muted">
              <SyncIcon size={12} aria-hidden="true" />
              Rotating invalidates the current token immediately.
            </span>
          </div>
        </div>
      ) : null}
    </li>
  )
}
