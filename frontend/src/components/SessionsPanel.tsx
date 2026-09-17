import { DeviceDesktopIcon } from '@primer/octicons-react'
import { useState } from 'react'

import { isApiError } from '../api/client'
import { authSessions } from '../api/endpoints'
import { formatDateTime, formatRelativeTime } from '../lib/format'
import { useAsync } from '../lib/useAsync'
import { ConfirmAction } from './ConfirmAction'
import { Box, BoxBody, BoxHeader } from './primitives/Box'
import { Flash } from './primitives/Flash'
import { Label } from './primitives/Label'
import { EmptyState, ErrorState, LoadingState } from './primitives/StateViews'

export function SessionsPanel() {
  const state = useAsync(
    (signal) => authSessions.list({ signal }),
    'auth-sessions',
  )
  const [revoking, setRevoking] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const revoke = (id: string, label: string) => {
    setActionError(null)
    setRevoking(id)
    void authSessions.revoke(id).then(
      () => {
        setRevoking(null)
        state.reload()
      },
      (error: unknown) => {
        setRevoking(null)
        setActionError(
          isApiError(error)
            ? error.message
            : `The session ${label} could not be revoked.`,
        )
      },
    )
  }

  return (
    <Box>
      <BoxHeader>
        <span className="flex items-center gap-2 text-sm font-medium">
          <DeviceDesktopIcon size={16} aria-hidden="true" />
          Active sessions
        </span>
      </BoxHeader>
      <BoxBody className="space-y-3">
        {actionError ? (
          <Flash variant="danger" onDismiss={() => setActionError(null)}>
            {actionError}
          </Flash>
        ) : null}

        {state.loading && !state.data ? (
          <LoadingState label="Loading sessions…" />
        ) : null}
        {state.error ? (
          <ErrorState error={state.error} onRetry={state.reload} />
        ) : null}

        {state.data ? (
          state.data.sessions.length === 0 ? (
            <EmptyState
              title="No sessions"
              description="Sign in again to create a new web session."
            />
          ) : (
            <ul className="divide-y divide-border rounded-md border border-border">
              {state.data.sessions.map((session) => {
                const label = session.user_agent ?? session.ip ?? session.id
                return (
                  <li
                    key={session.id}
                    className="flex flex-wrap items-center justify-between gap-2 px-3 py-2"
                  >
                    <div className="min-w-0">
                      <p className="truncate text-sm">
                        {session.user_agent ?? 'Unknown client'}
                      </p>
                      <p className="text-xs text-muted">
                        {session.ip ?? 'Unknown IP'} · started{' '}
                        {formatRelativeTime(session.created_at)} · last seen{' '}
                        {formatRelativeTime(session.last_seen_at)}
                      </p>
                      <p
                        className="text-xs text-muted"
                        title={formatDateTime(session.expires_at)}
                      >
                        expires {formatDateTime(session.expires_at)}
                      </p>
                    </div>
                    <div className="flex items-center gap-2">
                      <Label variant={session.revoked ? 'muted' : 'success'}>
                        {session.revoked ? 'revoked' : 'active'}
                      </Label>
                      {session.revoked ? null : (
                        <ConfirmAction
                          label="Revoke"
                          confirmLabel="Revoke session"
                          resourceName={label}
                          pending={revoking === session.id}
                          onConfirm={() => revoke(session.id, label)}
                        />
                      )}
                    </div>
                  </li>
                )
              })}
            </ul>
          )
        ) : null}

        <p className="text-xs text-muted">
          Revoking a session stops it from refreshing, but an already-issued
          access token stays valid until it expires.
        </p>
      </BoxBody>
    </Box>
  )
}
