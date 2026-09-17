import { PeopleIcon } from '@primer/octicons-react'
import { useState } from 'react'
import { Link } from 'react-router-dom'

import { users as usersApi } from '../api/endpoints'
import type { Page, UserSummary } from '../api/schemas'
import { formatNumber } from '../lib/format'
import { useAsync } from '../lib/useAsync'
import { Pagination } from './Pagination'
import { Avatar } from './primitives/Avatar'
import { EmptyState, ErrorState, LoadingState } from './primitives/StateViews'
import { TabNav } from './Tabs'

const PER_PAGE = 25

type FollowTab = 'followers' | 'following'

export type FollowPanelProps = {
  username: string
  reloadToken: number
}

export function FollowPanel({ username, reloadToken }: FollowPanelProps) {
  const [tab, setTab] = useState<FollowTab>('followers')
  const [page, setPage] = useState(1)

  const state = useAsync<Page<UserSummary>>(
    (signal) =>
      tab === 'followers'
        ? usersApi.followers(username, page, PER_PAGE, { signal })
        : usersApi.following(username, page, PER_PAGE, { signal }),
    `follows:${username}:${tab}:${page}:${reloadToken}`,
  )

  return (
    <div className="space-y-3">
      <TabNav
        label="Follow graph"
        active={tab}
        onChange={(id) => {
          setTab(id === 'following' ? 'following' : 'followers')
          setPage(1)
        }}
        tabs={[
          { id: 'followers', label: 'Followers' },
          { id: 'following', label: 'Following' },
        ]}
      />

      {state.loading && !state.data ? (
        <LoadingState label="Loading people…" />
      ) : null}
      {state.error ? <ErrorState error={state.error} onRetry={state.reload} /> : null}

      {state.data ? (
        state.data.items.length === 0 ? (
          <EmptyState
            title={tab === 'followers' ? 'No followers yet' : 'Not following anyone'}
            description={
              tab === 'followers'
                ? 'When people follow this account they will appear here.'
                : 'Accounts this user follows will appear here.'
            }
            icon={<PeopleIcon size={24} aria-hidden="true" />}
          />
        ) : (
          <>
            <ul className="grid gap-2 sm:grid-cols-2">
              {state.data.items.map((person) => (
                <li key={person.id}>
                  <Link
                    to={`/users/${encodeURIComponent(person.username)}`}
                    className="flex items-center gap-2 rounded-md border border-border bg-canvas-default p-2 transition-colors hover:border-accent"
                  >
                    <Avatar src={person.avatar_url} name={person.username} size={28} />
                    <span className="min-w-0">
                      <span className="block truncate text-sm font-medium">
                        {person.username}
                      </span>
                      {[person.first_name, person.last_name].filter(Boolean).length >
                      0 ? (
                        <span className="block truncate text-xs text-muted">
                          {[person.first_name, person.last_name]
                            .filter(Boolean)
                            .join(' ')}
                        </span>
                      ) : null}
                    </span>
                  </Link>
                </li>
              ))}
            </ul>
            <Pagination
              page={state.data.page}
              perPage={state.data.per_page}
              total={state.data.total}
              onPageChange={setPage}
            />
          </>
        )
      ) : null}

      {state.data ? (
        <p className="text-xs text-muted">
          {formatNumber(state.data.total)}{' '}
          {tab === 'followers' ? 'followers' : 'following'}
        </p>
      ) : null}
    </div>
  )
}
