import { RepoIcon } from '@primer/octicons-react'
import { useState } from 'react'
import { Link } from 'react-router-dom'

import { repositories as repositoriesApi } from '../api/endpoints'
import type { RepositorySummary } from '../api/schemas'
import { formatBytes, formatRelativeTime } from '../lib/format'
import { repositoryRelativePath, repoRoute } from '../lib/paths'
import { useAsync } from '../lib/useAsync'
import { Pagination } from './Pagination'
import { Box } from './primitives/Box'
import { EmptyState, ErrorState, LoadingState } from './primitives/StateViews'
import { Table, type TableColumn } from './primitives/Table'
import { VisibilityLabel } from './VisibilityLabel'

const PER_PAGE = 25

export type UserRepositoriesProps = {
  namespace: string | null
  reloadToken: number
}

export function UserRepositories({ namespace, reloadToken }: UserRepositoriesProps) {
  const [page, setPage] = useState(1)

  const state = useAsync(
    (signal) =>
      namespace
        ? repositoriesApi.list(namespace, page, PER_PAGE, { signal })
        : Promise.resolve(null),
    `user-repos:${namespace ?? 'none'}:${page}:${reloadToken}`,
  )

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
      render: (repo) => repo.tag_count,
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
      render: (repo) => formatRelativeTime(repo.updated_at),
    },
  ]

  if (!namespace) {
    return (
      <EmptyState
        title="No public repositories"
        description="This account does not have a namespace yet."
        icon={<RepoIcon size={24} aria-hidden="true" />}
      />
    )
  }

  const publicRepos = (state.data?.items ?? []).filter((repo) => repo.is_public)

  return (
    <>
      {state.loading && !state.data ? (
        <LoadingState label="Loading repositories…" />
      ) : null}
      {state.error ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.data ? (
        publicRepos.length === 0 ? (
          <EmptyState
            title="No public repositories"
            description="Repositories this user makes public will appear here."
            icon={<RepoIcon size={24} aria-hidden="true" />}
          />
        ) : (
          <Box>
            <Table
              columns={columns}
              rows={publicRepos}
              rowKey={(repo) => repo.id}
              caption={`Public repositories in ${namespace}`}
            />
            <div className="px-4">
              <Pagination
                page={state.data.page}
                perPage={state.data.per_page}
                total={state.data.total}
                onPageChange={setPage}
              />
            </div>
          </Box>
        )
      ) : null}
    </>
  )
}
