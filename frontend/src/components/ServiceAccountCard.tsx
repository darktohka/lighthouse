import { KeyIcon, SyncIcon } from '@primer/octicons-react'
import { useState } from 'react'

import { isApiError } from '../api/client'
import {
  serviceAccountGrants,
  serviceAccounts as serviceAccountsApi,
} from '../api/endpoints'
import type { ServiceAccount } from '../api/schemas'
import { formatDateTime, formatRelativeTime } from '../lib/format'
import { ConfirmAction } from './ConfirmAction'
import { SelectField, SwitchField } from './FormFields'
import { Box, BoxBody, BoxHeader } from './primitives/Box'
import { Button } from './primitives/Button'
import { Flash } from './primitives/Flash'
import { Label } from './primitives/Label'
import { TextInput } from './primitives/TextInput'

export type ServiceAccountCardProps = {
  account: ServiceAccount
  onChanged: () => void
  onToken: (token: string, label: string) => void
}

export function ServiceAccountCard({
  account,
  onChanged,
  onToken,
}: ServiceAccountCardProps) {
  const [grantType, setGrantType] = useState<'namespace' | 'repository'>('namespace')
  const [namespace, setNamespace] = useState('')
  const [repository, setRepository] = useState('')
  const [canPush, setCanPush] = useState(false)
  const [busy, setBusy] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [newRange, setNewRange] = useState('')

  const fail = (error: unknown, fallback: string) => {
    setBusy(null)
    setActionError(isApiError(error) ? error.message : fallback)
  }

  const addGrant = () => {
    setActionError(null)
    if (grantType === 'namespace' && namespace.trim().length === 0) {
      setActionError('Enter a namespace name.')
      return
    }
    if (grantType === 'repository' && repository.trim().length === 0) {
      setActionError('Enter a repository path.')
      return
    }
    setBusy('grant')
    void serviceAccountGrants
      .add(account.id, {
        ...(grantType === 'namespace'
          ? { namespace: namespace.trim() }
          : { repository: repository.trim() }),
        can_push: canPush,
      })
      .then(
        () => {
          setBusy(null)
          setNamespace('')
          setRepository('')
          setCanPush(false)
          onChanged()
        },
        (error: unknown) => fail(error, 'The grant could not be added.'),
      )
  }

  const rotate = () => {
    setActionError(null)
    setBusy('rotate')
    void serviceAccountsApi.rotate(account.id).then(
      (response) => {
        setBusy(null)
        onToken(response.token, account.name)
        onChanged()
      },
      (error: unknown) => fail(error, 'The token could not be rotated.'),
    )
  }

  const remove = () => {
    setActionError(null)
    setBusy('delete')
    void serviceAccountsApi.remove(account.id).then(
      () => {
        setBusy(null)
        onChanged()
      },
      (error: unknown) => fail(error, 'The account could not be deleted.'),
    )
  }

  const removeGrant = (grantId: number) => {
    setActionError(null)
    setBusy(`grant-${grantId}`)
    void serviceAccountsApi.removeGrant(account.id, grantId).then(
      () => {
        setBusy(null)
        onChanged()
      },
      (error: unknown) => fail(error, 'The grant could not be removed.'),
    )
  }

  const addIpRange = () => {
    setActionError(null)
    if (newRange.trim().length === 0) {
      setActionError('Enter an IP address or range.')
      return
    }
    setBusy('ip-range')
    void serviceAccountsApi
      .addIpRange(account.id, { cidr: newRange.trim() })
      .then(
        () => {
          setBusy(null)
          setNewRange('')
          onChanged()
        },
        (error: unknown) => fail(error, 'The IP range could not be added.'),
      )
  }

  const removeIpRange = (rangeId: number) => {
    setActionError(null)
    setBusy(`ip-range-${rangeId}`)
    void serviceAccountsApi.removeIpRange(account.id, rangeId).then(
      () => {
        setBusy(null)
        onChanged()
      },
      (error: unknown) => fail(error, 'The IP range could not be removed.'),
    )
  }

  return (
    <Box>
      <BoxHeader>
        <span className="flex items-center gap-2 text-sm font-medium">
          <KeyIcon size={16} aria-hidden="true" />
          {account.name}
        </span>
        <span className="font-mono text-xs text-muted">
          {account.token_prefix}…{account.token_suffix}
        </span>
      </BoxHeader>
      <BoxBody className="space-y-3">
        {actionError ? (
          <Flash variant="danger" onDismiss={() => setActionError(null)}>
            {actionError}
          </Flash>
        ) : null}

        <dl className="grid gap-2 text-xs sm:grid-cols-3">
          <div>
            <dt className="text-muted">Username</dt>
            <dd className="font-mono">{account.username}</dd>
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

        <div>
          <p className="mb-1 text-sm font-medium">Grants</p>
          {account.grants.length === 0 ? (
            <p className="text-xs text-muted">
              No grants yet — this token can only pull public images.
            </p>
          ) : (
            <ul className="divide-y divide-border rounded-md border border-border">
              {account.grants.map((grant) => (
                <li
                  key={grant.id}
                  className="flex flex-wrap items-center justify-between gap-2 px-3 py-1.5"
                >
                  <span className="flex items-center gap-2 text-xs">
                    <Label variant="muted">
                      {grant.namespace ? 'namespace' : 'repository'}
                    </Label>
                    <span className="font-mono">
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
              ))}
            </ul>
          )}
        </div>

        <div>
          <p className="mb-1 text-sm font-medium">IP allowlist</p>
          {account.ip_ranges.length === 0 ? (
            <p className="text-xs text-muted">
              No IP restriction — this account can authenticate from any address.
            </p>
          ) : (
            <ul className="divide-y divide-border rounded-md border border-border">
              {account.ip_ranges.map((range) => (
                <li
                  key={range.id}
                  className="flex flex-wrap items-center justify-between gap-2 px-3 py-1.5"
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
              ))}
            </ul>
          )}
        </div>

        <div className="grid gap-2 rounded-md border border-border bg-canvas-subtle p-3 sm:grid-cols-2">
          <TextInput
            label="IP address or range"
            value={newRange}
            onChange={(event) => setNewRange(event.target.value)}
            placeholder="203.0.113.4 or 10.0.0.0/8"
          />
          <div className="sm:col-span-2">
            <Button
              variant="primary"
              size="sm"
              disabled={busy === 'ip-range' || newRange.trim().length === 0}
              onClick={addIpRange}
            >
              {busy === 'ip-range' ? 'Adding…' : 'Add range'}
            </Button>
          </div>
          <p className="text-xs text-muted sm:col-span-2">
            Single addresses are treated as /32 (IPv4) or /128 (IPv6). The client
            IP is taken from the reverse proxy&apos;s X-Forwarded-For header.
          </p>
        </div>

        <div className="grid gap-2 rounded-md border border-border bg-canvas-subtle p-3 sm:grid-cols-2">
          <SelectField
            label="Grant type"
            value={grantType}
            onChange={(event) =>
              setGrantType(
                event.target.value === 'repository' ? 'repository' : 'namespace',
              )
            }
            options={[
              { value: 'namespace', label: 'Namespace' },
              { value: 'repository', label: 'Repository' },
            ]}
          />
          {grantType === 'namespace' ? (
            <TextInput
              label="Namespace"
              value={namespace}
              onChange={(event) => setNamespace(event.target.value)}
              placeholder="team-name"
            />
          ) : (
            <TextInput
              label="Repository"
              value={repository}
              onChange={(event) => setRepository(event.target.value)}
              placeholder="namespace/image"
            />
          )}
          <div className="sm:col-span-2">
            <SwitchField
              label="Push access"
              hint="Push implies pull."
              checked={canPush}
              onCheckedChange={setCanPush}
            />
          </div>
          <div className="sm:col-span-2">
            <Button
              variant="primary"
              size="sm"
              disabled={busy === 'grant'}
              onClick={addGrant}
            >
              {busy === 'grant' ? 'Adding…' : 'Add grant'}
            </Button>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2 border-t border-border pt-3">
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
      </BoxBody>
    </Box>
  )
}
