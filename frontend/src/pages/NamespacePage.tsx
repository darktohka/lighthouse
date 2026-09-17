import { useState } from 'react'
import { Link, Navigate, useParams } from 'react-router-dom'

import { isApiError } from '../api/client'
import {
  namespaces as namespacesApi,
  repositories as repositoriesApi,
  users as usersApi,
} from '../api/endpoints'
import type { RepositorySummary } from '../api/schemas'
import { NotFoundPage } from './NotFoundPage'
import { PageHeader } from '../components/PageHeader'
import { Pagination } from '../components/Pagination'
import { Avatar } from '../components/primitives/Avatar'
import { Box } from '../components/primitives/Box'
import { LinkButton } from '../components/primitives/Button'
import { Label } from '../components/primitives/Label'
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from '../components/primitives/StateViews'
import { Table, TableSortHeader, type TableColumn } from '../components/primitives/Table'
import { VisibilityLabel } from '../components/VisibilityLabel'
import { useAuth } from '../lib/auth-context'
import { formatBytes, formatDateTime, formatRelativeTime } from '../lib/format'
import { repositoryRelativePath, repoRoute } from '../lib/paths'
import {
  nextRepositoryOrder,
  type RepositoryOrder,
  type RepositorySort,
} from '../lib/repositorySort'
import { useAsync } from '../lib/useAsync'

const PER_PAGE = 25

export function NamespacePage() {
  const params = useParams()
  const namespace = params.namespace ?? ''
  const { user } = useAuth()
  const [page, setPage] = useState(1)
  const [sort, setSort] = useState<RepositorySort>('updated')
  const [order, setOrder] = useState<RepositoryOrder>('desc')

  const namespaceState = useAsync(
    (signal) => namespacesApi.detail(namespace, { signal }),
    `namespace:${namespace}`,
  )

  const reposState = useAsync(
    (signal) =>
      repositoriesApi.list(
        namespace,
        page,
        PER_PAGE,
        { sort, order },
        { signal },
      ),
    `namespace-repos:${namespace}:${page}:${sort}:${order}`,
  )

  const changeSort = (next: RepositorySort) => {
    setOrder((previous) => nextRepositoryOrder(sort, previous, next))
    setSort(next)
    setPage(1)
  }

  const namespaceError = namespaceState.error
  const namespaceMissing =
    isApiError(namespaceError) && namespaceError.status === 404

  const accountState = useAsync(
    (signal) =>
      namespaceMissing && namespace.length > 0
        ? usersApi.profile(namespace, { signal }).then(
            () => true,
            () => false,
          )
        : Promise.resolve(null),
    `namespace-account:${namespace}:${namespaceMissing}`,
  )

  if (namespace.length === 0) return <NotFoundPage />

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
          to={repoRoute(
            namespace,
            repositoryRelativePath(namespace, repo),
          )}
          className="font-mono text-accent hover:underline"
        >
          {repo.name}
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
          label="Last updated"
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

  const detail = namespaceState.data

  if (detail?.kind === 'user' || accountState.data === true) {
    return <Navigate to={`/users/${encodeURIComponent(namespace)}`} replace />
  }
  if (namespaceMissing && accountState.data === false) {
    return <NotFoundPage />
  }

  return (
    <div className="space-y-4">
      <PageHeader
        breadcrumbs={
          <>
            <Link to="/" className="hover:text-accent hover:underline">
              Explore
            </Link>{' '}
            / <span>{namespace}</span>
          </>
        }
        title={<span className="font-mono">{namespace}</span>}
        description={detail?.description ?? undefined}
        actions={
          detail ? (
            <span className="flex items-center gap-2">
              <VisibilityLabel isPublic={detail.is_public} />
              {user ? (
                <LinkButton
                  to={`/namespaces/${encodeURIComponent(namespace)}/settings`}
                  size="sm"
                >
                  Settings
                </LinkButton>
              ) : null}
            </span>
          ) : null
        }
      />

      {detail ? (
        <div className="flex flex-wrap items-center gap-2 text-sm text-muted">
          {detail.owner ? (
            <span className="flex items-center gap-2">
              <Avatar
                src={detail.owner.avatar_url}
                name={detail.owner.username}
                size={20}
              />
              <span>@{detail.owner.username}</span>
            </span>
          ) : null}
          <Label variant="muted">{detail.kind}</Label>
          <span>{detail.repository_count} repositories</span>
        </div>
      ) : null}

      {namespaceState.loading && !detail ? (
        <LoadingState label="Loading namespace…" />
      ) : null}
      {namespaceMissing && accountState.loading ? (
        <LoadingState label="Looking up account…" />
      ) : null}
      {namespaceState.error && !namespaceMissing ? (
        <ErrorState
          error={namespaceState.error}
          onRetry={namespaceState.reload}
        />
      ) : null}

      {detail ? (
        <Box>
          {reposState.loading && !reposState.data ? (
            <LoadingState label="Loading repositories…" />
          ) : null}
          {reposState.error ? (
            <ErrorState error={reposState.error} onRetry={reposState.reload} />
          ) : null}
          {reposState.data ? (
            reposState.data.items.length === 0 ? (
              <EmptyState
                title="No repositories"
                description="This namespace does not contain any repositories you can see."
              />
            ) : (
              <>
                <Table
                  columns={columns}
                  rows={reposState.data.items}
                  rowKey={(repo) => repo.id}
                  caption={`Repositories in ${namespace}`}
                />
                <div className="px-4">
                  <Pagination
                    page={reposState.data.page}
                    perPage={reposState.data.per_page}
                    total={reposState.data.total}
                    onPageChange={setPage}
                  />
                </div>
              </>
            )
          ) : null}
        </Box>
      ) : null}
    </div>
  )
}
