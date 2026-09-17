import { RepoIcon } from '@primer/octicons-react'
import { Link } from 'react-router-dom'

import { dashboard as dashboardApi } from '../api/endpoints'
import type { Dashboard, RepositorySummary } from '../api/schemas'
import { ActivityTimeline } from '../components/ActivityTimeline'
import { PageHeader } from '../components/PageHeader'
import { Box, BoxHeader } from '../components/primitives/Box'
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from '../components/primitives/StateViews'
import { Table, type TableColumn } from '../components/primitives/Table'
import { VisibilityLabel } from '../components/VisibilityLabel'
import { useAuth } from '../lib/auth-context'
import { formatBytes, formatDateTime, formatNumber, formatRelativeTime } from '../lib/format'
import { repositoryRelativePath, repoRoute } from '../lib/paths'
import { useAsync } from '../lib/useAsync'

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
  const { data, error, loading, reload } = useAsync<Dashboard>(
    (signal) => dashboardApi.load({ signal }),
    'dashboard',
  )

  const host = typeof window === 'undefined' ? '' : window.location.host
  const pushExample = user
    ? `docker push ${host}/${user.username}/my-image:v1`
    : `docker push ${host}/<namespace>/my-image:v1`

  const columns: TableColumn<RepositorySummary>[] = [
    {
      key: 'name',
      header: 'Repository',
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
      header: 'Size',
      align: 'right',
      render: (repo) => formatBytes(repo.size),
    },
    {
      key: 'updated',
      header: 'Updated',
      align: 'right',
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
                <Table
                  columns={columns}
                  rows={data.repositories}
                  rowKey={(repo) => repo.id}
                  caption="Your repositories"
                />
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
