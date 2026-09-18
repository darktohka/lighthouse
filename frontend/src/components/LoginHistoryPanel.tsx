import { HistoryIcon } from '@primer/octicons-react'
import { useState } from 'react'

import { authSessions } from '../api/endpoints'
import type { LoginEvent } from '../api/schemas'
import { formatDateTime, formatRelativeTime } from '../lib/format'
import { useAsync } from '../lib/useAsync'
import { Box, BoxHeader } from './primitives/Box'
import { Button } from './primitives/Button'
import { Label } from './primitives/Label'
import { EmptyState, ErrorState, LoadingState } from './primitives/StateViews'
import { Table, type TableColumn } from './primitives/Table'

const LIMIT = 25

export function LoginHistoryPanel() {
  const [offset, setOffset] = useState(0)
  const state = useAsync(
    (signal) => authSessions.loginHistory(LIMIT, offset, { signal }),
    `login-history:${offset}`,
  )

  const events = state.data?.events ?? []

  const columns: TableColumn<LoginEvent>[] = [
    {
      key: 'time',
      header: 'Time',
      render: (event) => (
        <span title={formatDateTime(event.created_at)}>
          {formatRelativeTime(event.created_at)}
        </span>
      ),
    },
    {
      key: 'kind',
      header: 'Method',
      render: (event) => <Label variant="muted">{event.kind}</Label>,
    },
    {
      key: 'success',
      header: 'Result',
      render: (event) => (
        <Label variant={event.success ? 'success' : 'danger'}>
          {event.success ? 'success' : 'failed'}
        </Label>
      ),
    },
    {
      key: 'ip',
      header: 'IP address',
      render: (event) => (
        <span className="font-mono text-xs">{event.ip ?? '-'}</span>
      ),
    },
    {
      key: 'username',
      header: 'Account',
      render: (event) => event.username_attempted ?? '-',
    },
    {
      key: 'agent',
      header: 'User agent',
      render: (event) => (
        <span className="block max-w-xs truncate text-xs text-muted" title={event.user_agent ?? ''}>
          {event.user_agent ?? '-'}
        </span>
      ),
    },
  ]

  return (
    <Box>
      <BoxHeader>
        <span className="flex items-center gap-2 text-sm font-medium">
          <HistoryIcon size={16} aria-hidden="true" />
          Login history
        </span>
      </BoxHeader>

      {state.loading && !state.data ? (
        <LoadingState label="Loading login history…" />
      ) : null}
      {state.error ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}

      {state.data ? (
        events.length === 0 ? (
          <EmptyState
            title="No login attempts"
            description="Successful and failed sign-in attempts will appear here."
          />
        ) : (
          <>
            <Table
              columns={columns}
              rows={events}
              rowKey={(event) => event.id}
              caption="Recent login attempts"
            />
            <div className="flex items-center justify-between gap-2 px-4 py-3">
              <p className="text-xs text-muted">
                Showing {offset + 1}–{offset + events.length}
              </p>
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  disabled={offset === 0}
                  onClick={() => setOffset(Math.max(0, offset - LIMIT))}
                >
                  Previous
                </Button>
                <Button
                  size="sm"
                  disabled={events.length < LIMIT}
                  onClick={() => setOffset(offset + LIMIT)}
                >
                  Next
                </Button>
              </div>
            </div>
          </>
        )
      ) : null}
    </Box>
  )
}
