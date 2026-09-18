import { RepoIcon } from '@primer/octicons-react'
import { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'

import { dashboard as dashboardApi } from '../api/endpoints'
import type { Dashboard, RepositorySummary } from '../api/schemas'
import { ActivityTimeline } from '../components/ActivityTimeline'
import { PageHeader } from '../components/PageHeader'
import { Pagination } from '../components/Pagination'
import { Box, BoxHeader } from '../components/primitives/Box'
import { LinkButton } from '../components/primitives/Button'
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from '../components/primitives/StateViews'
import { Table, TableSortHeader, type TableColumn } from '../components/primitives/Table'
import { VisibilityLabel } from '../components/VisibilityLabel'
import { useAuth } from '../lib/auth-context'
import { formatBytes, formatDateTime, formatNumber, formatRelativeTime } from '../lib/format'
import { repositoryRelativePath, repoRoute } from '../lib/paths'
import {
  compareRepositories,
  nextRepositoryOrder,
  type RepositoryOrder,
  type RepositorySort,
} from '../lib/repositorySort'
import { useAsync } from '../lib/useAsync'

const PER_PAGE = 25

function StatCard({ label, value }: { label: string; value: string }) {
  return (
    <Box className="p-3">
      <p className="text-xs text-muted">{label}</p>
      <p className="mt-1 text-xl font-semibold">{value}</p>
    </Box>
  )
}

export function DashboardPage() {
  const { user } = useAuth()
  const [sort, setSort] = useState<RepositorySort>('updated')
  const [order, setOrder] = useState<RepositoryOrder>('desc')
  const [page, setPage] = useState(1)
  const { data, error, loading, reload } = useAsync<Dashboard>(
    (signal) => dashboardApi.load({ signal }),
    'dashboard',
  )

  const sortedRepositories = useMemo(() => {
    const repositories = data?.repositories ?? []
    return [...repositories].sort((a, b) =>
      compareRepositories(a, b, sort, order),
    )
  }, [data, sort, order])

  const visibleRepositories = useMemo(
    () =>
      sortedRepositories.slice((page - 1) * PER_PAGE, page * PER_PAGE),
    [sortedRepositories, page],
  )

  const changeSort = (next: RepositorySort) => {
    setOrder((previous) => nextRepositoryOrder(sort, previous, next))
    setSort(next)
    setPage(1)
  }

  const host = typeof window === 'undefined' ? '' : window.location.host
  const pushExample = user
    ? `docker push ${host}/${user.username}/my-image:v1`
    : `docker push ${host}/<namespace>/my-image:v1`

  const columns: TableColumn<RepositorySummary>[] = [
    {
      key: 'name',
      header: (
        <TableSortHeader
          label="Repository"
          active={sort === 'name'}
          direction={order}
          onSort={() => changeSort('name')}
        />
      ),
      sortDirection: sort === 'name' ? order : undefined,
      render: (repo) => (
        <Link
          to={repoRoute(repo.namespace, repositoryRelativePath(repo.namespace, repo))}
          className="font-mono text-accent hover:underline"
        >
          {repo.path}
        </Link>
      ),
    },
    {
      key: 'visibility',
      header: 'Visibility',
      render: (repo) => <VisibilityLabel isPublic={repo.is_public} />,
    },
    {
      key: 'tags',
      header: 'Tags',
      align: 'right',
      render: (repo) => formatNumber(repo.tag_count),
    },
    {
      key: 'size',
      header: (
        <TableSortHeader
          label="Size"
          active={sort === 'size'}
          direction={order}
          onSort={() => changeSort('size')}
        />
      ),
      align: 'right',
      sortDirection: sort === 'size' ? order : undefined,
      render: (repo) => formatBytes(repo.size),
    },
    {
      key: 'updated',
      header: (
        <TableSortHeader
          label="Updated"
          active={sort === 'updated'}
          direction={order}
          onSort={() => changeSort('updated')}
        />
      ),
      align: 'right',
      sortDirection: sort === 'updated' ? order : undefined,
      render: (repo) => (
        <span title={formatDateTime(repo.updated_at)}>
          {formatRelativeTime(repo.updated_at)}
        </span>
      ),
    },
  ]

  return (
    <div className="space-y-6">
      <PageHeader
        title={user ? `Welcome back, ${user.username}` : 'Dashboard'}
        description="Your repositories and recent registry activity."
        actions={
          <LinkButton to="/new/repository" size="sm">
            New repository
          </LinkButton>
        }
      />

      {loading && !data ? <LoadingState label="Loading dashboard…" /> : null}
      {error ? <ErrorState error={error} onRetry={reload} /> : null}

      {data ? (
        <>
          <section
            aria-label="Registry statistics"
            className="grid grid-cols-2 gap-3 lg:grid-cols-4"
          >
            <StatCard
              label="Repositories"
              value={formatNumber(data.stats.repository_count)}
            />
            <StatCard label="Tags" value={formatNumber(data.stats.tag_count)} />
            <StatCard
              label="Total size"
              value={formatBytes(data.stats.total_size)}
            />
            <StatCard
              label="Pulls (30d)"
              value={formatNumber(data.stats.pull_count_30d)}
            />
          </section>

          <section aria-labelledby="your-repositories">
            <h2 id="your-repositories" className="mb-2 text-base font-semibold">
              Your repositories
            </h2>
            <Box>
              {data.repositories.length === 0 ? (
                <EmptyState
                  title="No repositories yet"
                  description={
                    <span>
                      Push your first image to get started:
                      <code className="mt-2 block rounded-md border border-border bg-canvas-inset px-2 py-1 font-mono text-xs">
                        {pushExample}
                      </code>
                    </span>
                  }
                  icon={<RepoIcon size={24} aria-hidden="true" />}
                />
              ) : (
                <>
                  <Table
                    columns={columns}
                    rows={visibleRepositories}
                    rowKey={(repo) => repo.id}
                    caption="Your repositories"
                  />
                  <div className="px-4">
                    <Pagination
                      page={page}
                      perPage={PER_PAGE}
                      total={sortedRepositories.length}
                      onPageChange={setPage}
                    />
                  </div>
                </>
              )}
            </Box>
          </section>

          <section aria-labelledby="activity">
            <h2 id="activity" className="mb-2 text-base font-semibold">
              Activity
            </h2>
            <Box>
              <BoxHeader>
                <span className="text-sm font-medium">Recent events</span>
              </BoxHeader>
              <div className="p-4">
                <ActivityTimeline
                  entries={data.activity}
                  emptyDescription="Push an image, create a workspace or change permissions to populate your timeline."
                />
              </div>
            </Box>
          </section>
        </>
      ) : null}
    </div>
  )
}
