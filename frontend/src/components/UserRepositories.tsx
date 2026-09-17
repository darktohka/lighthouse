import { RepoIcon } from '@primer/octicons-react'
import { useState } from 'react'
import { Link } from 'react-router-dom'

import { repositories as repositoriesApi } from '../api/endpoints'
import type { RepositorySummary } from '../api/schemas'
import { formatBytes, formatRelativeTime } from '../lib/format'
import { repositoryRelativePath, repoRoute } from '../lib/paths'
import {
  nextRepositoryOrder,
  type RepositoryOrder,
  type RepositorySort,
} from '../lib/repositorySort'
import { useAsync } from '../lib/useAsync'
import { Pagination } from './Pagination'
import { Box } from './primitives/Box'
import { EmptyState, ErrorState, LoadingState } from './primitives/StateViews'
import { Table, TableSortHeader, type TableColumn } from './primitives/Table'
import { VisibilityLabel } from './VisibilityLabel'

const PER_PAGE = 25

export type UserRepositoriesProps = {
  namespace: string | null
  reloadToken: number
}

export function UserRepositories({ namespace, reloadToken }: UserRepositoriesProps) {
  const [page, setPage] = useState(1)
  const [sort, setSort] = useState<RepositorySort>('updated')
  const [order, setOrder] = useState<RepositoryOrder>('desc')

  const state = useAsync(
    (signal) =>
      namespace
        ? repositoriesApi.list(
            namespace,
            page,
            PER_PAGE,
            { sort, order },
            { signal },
          )
        : Promise.resolve(null),
    `user-repos:${namespace ?? 'none'}:${page}:${sort}:${order}:${reloadToken}`,
  )

  const changeSort = (next: RepositorySort) => {
    setOrder((previous) => nextRepositoryOrder(sort, previous, next))
    setSort(next)
    setPage(1)
  }

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
      render: (repo) => repo.tag_count,
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
      render: (repo) => formatRelativeTime(repo.updated_at),
    },
  ]

  if (!namespace) {
    return (
      <EmptyState
        title="No repositories"
        description="This account does not have a namespace yet."
        icon={<RepoIcon size={24} aria-hidden="true" />}
      />
    )
  }

  const repositories = state.data?.items ?? []

  return (
    <>
      {state.loading && !state.data ? (
        <LoadingState label="Loading repositories…" />
      ) : null}
      {state.error ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.data ? (
        repositories.length === 0 ? (
          <EmptyState
            title="No repositories"
            description="Repositories the viewer can access will appear here."
            icon={<RepoIcon size={24} aria-hidden="true" />}
          />
        ) : (
          <Box>
            <Table
              columns={columns}
              rows={repositories}
              rowKey={(repo) => repo.id}
              caption={`Repositories in ${namespace}`}
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
