import { PeopleIcon, PersonAddIcon } from '@primer/octicons-react'
import { useState } from 'react'

import { isApiError } from '../api/client'
import { namespaceMembers, namespaces as namespacesApi } from '../api/endpoints'
import type { NamespaceMember, NamespaceMemberRole } from '../api/schemas'
import { formatRelativeTime } from '../lib/format'
import { useAsync } from '../lib/useAsync'
import { ConfirmAction } from './ConfirmAction'
import { SelectField } from './FormFields'
import { Avatar } from './primitives/Avatar'
import { Box, BoxBody, BoxHeader } from './primitives/Box'
import { Button } from './primitives/Button'
import { Flash } from './primitives/Flash'
import { Label } from './primitives/Label'
import { EmptyState, ErrorState, LoadingState } from './primitives/StateViews'
import { UserAutocomplete } from './UserAutocomplete'

export type MembersPanelProps = {
  namespace: string
  canManage: boolean
}

export function MembersPanel({ namespace, canManage }: MembersPanelProps) {
  const state = useAsync(
    (signal) => namespaceMembers.list(namespace, { signal }),
    `members:${namespace}`,
  )
  const [username, setUsername] = useState('')
  const [role, setRole] = useState<NamespaceMemberRole>('member')
  const [busy, setBusy] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const addMember = () => {
    setActionError(null)
    if (username.trim().length === 0) {
      setActionError('Choose a user to add.')
      return
    }
    setBusy('add')
    void namespacesApi.addMember(namespace, username.trim(), role).then(
      () => {
        setBusy(null)
        setUsername('')
        state.reload()
      },
      (error: unknown) => {
        setBusy(null)
        setActionError(
          isApiError(error) ? error.message : 'The member could not be added.',
        )
      },
    )
  }

  const removeMember = (member: NamespaceMember) => {
    setActionError(null)
    setBusy(`remove-${member.user.id}`)
    void namespacesApi.removeMember(namespace, member.user.username).then(
      () => {
        setBusy(null)
        state.reload()
      },
      (error: unknown) => {
        setBusy(null)
        setActionError(
          isApiError(error) ? error.message : 'The member could not be removed.',
        )
      },
    )
  }

  const members: NamespaceMember[] = state.data ?? []

  return (
    <Box>
      <BoxHeader>
        <span className="flex items-center gap-2 text-sm font-medium">
          <PeopleIcon size={16} aria-hidden="true" />
          Members
        </span>
      </BoxHeader>
      <BoxBody className="space-y-3">
        {actionError ? (
          <Flash variant="danger" onDismiss={() => setActionError(null)}>
            {actionError}
          </Flash>
        ) : null}

        {canManage ? (
          <div className="grid gap-3 rounded-md border border-border bg-canvas-subtle p-3 sm:grid-cols-2">
            <UserAutocomplete
              label="Add member"
              value={username}
              onChange={setUsername}
              hint="Search for a user by name."
            />
            <SelectField
              label="Role"
              value={role}
              onChange={(event) =>
                setRole(event.target.value === 'admin' ? 'admin' : 'member')
              }
              options={[
                { value: 'member', label: 'Member' },
                { value: 'admin', label: 'Admin' },
              ]}
            />
            <div className="sm:col-span-2">
              <Button
                size="sm"
                variant="primary"
                disabled={busy === 'add'}
                onClick={addMember}
                leadingIcon={<PersonAddIcon size={14} aria-hidden="true" />}
              >
                {busy === 'add' ? 'Adding…' : 'Add member'}
              </Button>
            </div>
          </div>
        ) : null}

        {state.loading && !state.data ? (
          <LoadingState label="Loading members…" />
        ) : null}
        {state.error ? (
          <ErrorState error={state.error} onRetry={state.reload} />
        ) : null}

        {state.data ? (
          members.length === 0 ? (
            <EmptyState
              title="No members"
              description="Add teammates to give them access to this workspace."
              icon={<PeopleIcon size={24} aria-hidden="true" />}
            />
          ) : (
            <ul className="divide-y divide-border rounded-md border border-border">
              {members.map((member) => (
                <li
                  key={member.user.id}
                  className="flex flex-wrap items-center justify-between gap-2 px-3 py-2"
                >
                  <span className="flex items-center gap-2">
                    <Avatar
                      src={member.user.avatar_url}
                      name={member.user.username}
                      size={20}
                    />
                    <span className="text-sm font-medium">
                      {member.user.username}
                    </span>
                    <Label variant={member.role === 'admin' ? 'accent' : 'muted'}>
                      {member.role}
                    </Label>
                    <span className="text-xs text-muted">
                      joined {formatRelativeTime(member.created_at)}
                    </span>
                  </span>
                  {canManage ? (
                    <ConfirmAction
                      label="Remove"
                      confirmLabel="Remove member"
                      resourceName={member.user.username}
                      pending={busy === `remove-${member.user.id}`}
                      onConfirm={() => removeMember(member)}
                    />
                  ) : null}
                </li>
              ))}
            </ul>
          )
        ) : null}
      </BoxBody>
    </Box>
  )
}
