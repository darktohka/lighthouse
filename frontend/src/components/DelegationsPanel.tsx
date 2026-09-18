import { PersonIcon, ShieldLockIcon } from '@primer/octicons-react'
import { useState } from 'react'

import { isApiError } from '../api/client'
import type { CreateGrant, PublicPermission } from '../api/schemas'
import { formatDateTime } from '../lib/format'
import { useAsync } from '../lib/useAsync'
import { ConfirmAction } from './ConfirmAction'
import { SelectField, SwitchField } from './FormFields'
import { LibravatarAvatar } from './LibravatarAvatar'
import { Box, BoxBody, BoxHeader } from './primitives/Box'
import { Button } from './primitives/Button'
import { Flash } from './primitives/Flash'
import { Label } from './primitives/Label'
import { EmptyState, ErrorState, LoadingState } from './primitives/StateViews'
import { UserAutocomplete } from './UserAutocomplete'

export type DelegationsPanelProps = {
  title: string
  resourceName: string
  load: (signal: AbortSignal) => Promise<PublicPermission[]>
  add: (body: CreateGrant) => Promise<PublicPermission>
  remove: (id: number) => Promise<void>
  reloadKey: string
}

export function DelegationsPanel({
  title,
  resourceName,
  load,
  add,
  remove,
  reloadKey,
}: DelegationsPanelProps) {
  const state = useAsync(load, `delegations:${reloadKey}`)
  const [subjectType, setSubjectType] = useState<'user' | 'anonymous'>('user')
  const [subject, setSubject] = useState('')
  const [canPush, setCanPush] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [revoking, setRevoking] = useState<number | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const submit = () => {
    setActionError(null)
    if (subjectType === 'user' && subject.trim().length === 0) {
      setActionError('Choose a user before adding a grant.')
      return
    }
    setSubmitting(true)
    void add({
      subject_type: subjectType,
      ...(subjectType === 'user' ? { subject: subject.trim() } : {}),
      can_push: canPush,
    }).then(
      () => {
        setSubmitting(false)
        setSubject('')
        setCanPush(false)
        state.reload()
      },
      (error: unknown) => {
        setSubmitting(false)
        setActionError(
          isApiError(error) ? error.message : 'The grant could not be added.',
        )
      },
    )
  }

  const revoke = (id: number) => {
    setActionError(null)
    setRevoking(id)
    void remove(id).then(
      () => {
        setRevoking(null)
        state.reload()
      },
      (error: unknown) => {
        setRevoking(null)
        setActionError(
          isApiError(error) ? error.message : 'The grant could not be revoked.',
        )
      },
    )
  }

  return (
    <Box>
      <BoxHeader>
        <span className="flex items-center gap-2 text-sm font-medium">
          <ShieldLockIcon size={16} aria-hidden="true" />
          {title}
        </span>
      </BoxHeader>
      <BoxBody className="space-y-4">
        {actionError ? (
          <Flash variant="danger" onDismiss={() => setActionError(null)}>
            {actionError}
          </Flash>
        ) : null}

        <div className="grid gap-3 rounded-md border border-border bg-canvas-subtle p-3 sm:grid-cols-2">
          <SelectField
            label="Subject"
            value={subjectType}
            onChange={(event) => {
              setSubjectType(event.target.value === 'anonymous' ? 'anonymous' : 'user')
              setSubject('')
            }}
            options={[
              { value: 'user', label: 'User' },
              { value: 'anonymous', label: 'Anonymous' },
            ]}
          />
          {subjectType === 'user' ? (
            <UserAutocomplete
              label="Username"
              value={subject}
              onChange={setSubject}
              hint="Search for a user to delegate access to."
            />
          ) : (
            <div className="text-sm">
              <span className="block font-medium">Anonymous</span>
              <span className="text-xs text-muted">
                Anyone, including unauthenticated clients.
              </span>
            </div>
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
              disabled={submitting}
              onClick={submit}
            >
              {submitting ? 'Adding…' : 'Add grant'}
            </Button>
          </div>
        </div>

        {state.loading && !state.data ? (
          <LoadingState label="Loading grants…" />
        ) : null}
        {state.error ? (
          <ErrorState error={state.error} onRetry={state.reload} />
        ) : null}

        {state.data ? (
          state.data.length === 0 ? (
            <EmptyState
              title="No grants yet"
              description="Only the owner and members can access this resource until you delegate it."
              icon={<PersonIcon size={24} aria-hidden="true" />}
            />
          ) : (
            <ul className="divide-y divide-border rounded-md border border-border">
              {state.data.map((grant) => (
                <li
                  key={grant.id}
                  className="flex flex-wrap items-center justify-between gap-2 px-3 py-2"
                >
                  <div className="flex min-w-0 items-center gap-2">
                    {grant.subject ? (
                      <>
                        <LibravatarAvatar
                          src={grant.subject.avatar_url}
                          hash={grant.subject.avatar_hash}
                          name={grant.subject.username}
                          size={20}
                        />
                        <span className="font-medium">
                          {grant.subject.username}
                        </span>
                      </>
                    ) : (
                      <>
                        <PersonIcon size={16} aria-hidden="true" className="text-muted" />
                        <span className="font-medium">Anonymous</span>
                      </>
                    )}
                    <Label variant={grant.can_push ? 'success' : 'muted'}>
                      {grant.can_push ? 'push + pull' : 'pull'}
                    </Label>
                    <span
                      className="text-xs text-muted"
                      title={formatDateTime(grant.created_at)}
                    >
                      added {formatDateTime(grant.created_at)}
                    </span>
                  </div>
                  <ConfirmAction
                    label="Revoke"
                    confirmLabel="Revoke grant"
                    resourceName={`grant for ${grant.subject ? grant.subject.username : 'anonymous'} on ${resourceName}`}
                    pending={revoking === grant.id}
                    onConfirm={() => revoke(grant.id)}
                  />
                </li>
              ))}
            </ul>
          )
        ) : null}
      </BoxBody>
    </Box>
  )
}
