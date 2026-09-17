import {
  ChevronDownIcon,
  ChevronUpIcon,
  GlobeIcon,
  RepoIcon,
} from '@primer/octicons-react'
import { useMemo, useState } from 'react'
import { Link, useSearchParams } from 'react-router-dom'

import {
  namespaces as namespacesApi,
  repositories as repositoriesApi,
} from '../api/endpoints'
import type { Namespace, RepositorySummary } from '../api/schemas'
import { SelectField } from '../components/FormFields'
import { PageHeader } from '../components/PageHeader'
import { Pagination } from '../components/Pagination'
import { Avatar } from '../components/primitives/Avatar'
import { Button } from '../components/primitives/Button'
import { CounterLabel } from '../components/primitives/CounterLabel'
import { Label } from '../components/primitives/Label'
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from '../components/primitives/StateViews'
import { VisibilityLabel } from '../components/VisibilityLabel'
import { formatBytes, formatRelativeTime } from '../lib/format'
import { repoRoute } from '../lib/paths'
import {
  compareRepositories,
  isRepositorySort,
  REPOSITORY_SORT_OPTIONS,
  type RepositoryOrder,
  type RepositorySort,
} from '../lib/repositorySort'
import { useAsync } from '../lib/useAsync'

type ExploreData = {
  namespaces: Namespace[]
  repositories: RepositorySummary[]
}

const MAX_NAMESPACES_SCANNED = 8
const PER_PAGE = 12

export function ExplorePage() {
  const [searchParams] = useSearchParams()
  const query = (searchParams.get('q') ?? '').trim().toLowerCase()
  const [sort, setSort] = useState<RepositorySort>('updated')
  const [order, setOrder] = useState<RepositoryOrder>('desc')
  const listContext = `${query}|${sort}|${order}`
  const [pageState, setPageState] = useState({ context: listContext, page: 1 })
  const page = pageState.context === listContext ? pageState.page : 1

  const changePage = (next: number) => {
    setPageState({ context: listContext, page: next })
  }

  const { data, error, loading, reload } = useAsync<ExploreData>(
    async (signal) => {
      const page = await namespacesApi.list(1, 60, { signal })
      const publicNamespaces = page.items.filter((item) => item.is_public)
      const repoPages = await Promise.all(
        publicNamespaces.slice(0, MAX_NAMESPACES_SCANNED).map(async (item) => {
          try {
            const repos = await repositoriesApi.list(
              item.name,
              1,
              12,
              undefined,
              { signal },
            )
            return repos.items.filter((repo) => repo.is_public)
          } catch {
            return []
          }
        }),
      )
      return { namespaces: page.items, repositories: repoPages.flat() }
    },
    'explore',
  )

  const filtered = useMemo(() => {
    const all: ExploreData = data ?? { namespaces: [], repositories: [] }
    if (query.length === 0) return all
    return {
      namespaces: all.namespaces.filter(
        (item) =>
          item.name.toLowerCase().includes(query) ||
          (item.description ?? '').toLowerCase().includes(query),
      ),
      repositories: all.repositories.filter(
        (item) =>
          item.path.toLowerCase().includes(query) ||
          (item.description ?? '').toLowerCase().includes(query),
      ),
    }
  }, [data, query])

  const sortedRepositories = useMemo(
    () =>
      [...filtered.repositories].sort((a, b) =>
        compareRepositories(a, b, sort, order),
      ),
    [filtered.repositories, sort, order],
  )

  const visibleRepositories = useMemo(
    () => sortedRepositories.slice((page - 1) * PER_PAGE, page * PER_PAGE),
    [sortedRepositories, page],
  )

  const publicNamespaceCount =
    data?.namespaces.filter((item) => item.is_public).length ?? 0

  return (
    <div className="space-y-6">
      <PageHeader
        title="Explore"
        description="Public namespaces and repositories on this registry."
      />

      {loading && !data ? <LoadingState label="Loading registry…" /> : null}
      {error ? <ErrorState error={error} onRetry={reload} /> : null}

      {data ? (
        <>
          {query.length > 0 &&
          filtered.namespaces.length === 0 &&
          filtered.repositories.length === 0 ? (
            <EmptyState
              title={`No matches for “${query}”`}
              description="Try a different name or clear the search."
            />
          ) : null}

          <section aria-labelledby="public-namespaces">
            <div className="mb-3 flex items-center gap-2">
              <h2 id="public-namespaces" className="text-base font-semibold">
                Public namespaces
              </h2>
              <CounterLabel>{filtered.namespaces.length}</CounterLabel>
            </div>
            {filtered.namespaces.length === 0 ? (
              <EmptyState
                title="No public namespaces"
                description="Nothing has been shared publicly yet."
                icon={<GlobeIcon size={24} aria-hidden="true" />}
              />
            ) : (
              <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                {filtered.namespaces.map((item) => (
                  <li key={item.id}>
                    <Link
                      to={
                        item.kind === 'user'
                          ? `/users/${encodeURIComponent(item.name)}`
                          : `/${encodeURIComponent(item.name)}`
                      }
                      className="block h-full rounded-md border border-border bg-canvas-default p-3 transition-colors hover:border-accent"
                    >
                      <div className="flex items-center gap-2">
                        {item.owner ? (
                          <Avatar
                            src={item.owner.avatar_url}
                            name={item.owner.username}
                            size={20}
                          />
                        ) : (
                          <GlobeIcon size={16} aria-hidden="true" />
                        )}
                        <span className="truncate font-semibold">
                          {item.name}
                        </span>
                        <Label variant="muted">{item.kind}</Label>
                      </div>
                      {item.description ? (
                        <p className="mt-1 line-clamp-2 text-xs text-muted">
                          {item.description}
                        </p>
                      ) : null}
                      <p className="mt-2 text-xs text-muted">
                        {item.repository_count} repositories
                      </p>
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section aria-labelledby="public-repositories">
            <div className="mb-3 flex items-center gap-2">
              <h2 id="public-repositories" className="text-base font-semibold">
                Public repositories
              </h2>
              <CounterLabel>{filtered.repositories.length}</CounterLabel>
            </div>
            {publicNamespaceCount > MAX_NAMESPACES_SCANNED ? (
              <p className="mb-2 text-xs text-muted">
                Showing repositories from the first {MAX_NAMESPACES_SCANNED}{' '}
                public namespaces.
              </p>
            ) : null}
            {filtered.repositories.length === 0 ? (
              <EmptyState
                title="No public repositories"
                description="Push an image and make its repository public to see it here."
                icon={<RepoIcon size={24} aria-hidden="true" />}
              />
            ) : (
              <>
                <div className="mb-3 flex flex-wrap items-end gap-2">
                  <div className="w-44">
                    <SelectField
                      label="Sort repositories by"
                      value={sort}
                      onChange={(event) => {
                        const value = event.target.value
                        if (isRepositorySort(value)) setSort(value)
                      }}
                      options={REPOSITORY_SORT_OPTIONS}
                    />
                  </div>
                  <Button
                    size="sm"
                    onClick={() =>
                      setOrder((previous) =>
                        previous === 'desc' ? 'asc' : 'desc',
                      )
                    }
                    leadingIcon={
                      order === 'desc' ? (
                        <ChevronDownIcon size={14} aria-hidden="true" />
                      ) : (
                        <ChevronUpIcon size={14} aria-hidden="true" />
                      )
                    }
                    aria-label={
                      order === 'desc'
                        ? 'Sort descending, activate for ascending'
                        : 'Sort ascending, activate for descending'
                    }
                  >
                    {order === 'asc' ? 'Ascending' : 'Descending'}
                  </Button>
                </div>
                <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                  {visibleRepositories.map((item) => (
                    <li key={item.id}>
                      <Link
                        to={repoRoute(item.namespace, item.name)}
                        className="block h-full rounded-md border border-border bg-canvas-default p-3 transition-colors hover:border-accent"
                      >
                        <div className="flex items-center gap-2">
                          <RepoIcon
                            size={16}
                            aria-hidden="true"
                            className="text-muted"
                          />
                          <span className="truncate font-semibold">
                            {item.path}
                          </span>
                        </div>
                        {item.description ? (
                          <p className="mt-1 line-clamp-2 text-xs text-muted">
                            {item.description}
                          </p>
                        ) : null}
                        <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-muted">
                          <VisibilityLabel isPublic={item.is_public} />
                          <span>{item.tag_count} tags</span>
                          <span>{formatBytes(item.size)}</span>
                          <span>
                            updated {formatRelativeTime(item.updated_at)}
                          </span>
                        </div>
                      </Link>
                    </li>
                  ))}
                </ul>
                <div className="mt-2">
                  <Pagination
                    page={page}
                    perPage={PER_PAGE}
                    total={sortedRepositories.length}
                    onPageChange={changePage}
                  />
                </div>
              </>
            )}
          </section>
        </>
      ) : null}
    </div>
  )
}
