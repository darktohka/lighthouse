import { RepoIcon, TrashIcon } from '@primer/octicons-react'
import { useState } from 'react'
import { Link } from 'react-router-dom'

import { repositories as repositoriesApi } from '../api/endpoints'
import type { TagSummary } from '../api/schemas'
import { CopyButton } from '../components/CopyButton'
import { PageHeader } from '../components/PageHeader'
import { Pagination } from '../components/Pagination'
import { PlatformBadges } from '../components/PlatformBadge'
import { Box } from '../components/primitives/Box'
import { Button, LinkButton } from '../components/primitives/Button'
import { Flash } from '../components/primitives/Flash'
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from '../components/primitives/StateViews'
import { Table, type TableColumn } from '../components/primitives/Table'
import { VisibilityLabel } from '../components/VisibilityLabel'
import { isApiError } from '../api/client'
import {
  formatBytes,
  formatDateTime,
  formatNumber,
  formatRelativeTime,
  shortDigest,
} from '../lib/format'
import { tagRoute } from '../lib/paths'
import { useAuth } from '../lib/auth-context'
import { useAsync } from '../lib/useAsync'

const PER_PAGE = 25

export type RepositoryPageProps = {
  namespace: string
  repo: string
}

export function RepositoryPage({ namespace, repo }: RepositoryPageProps) {
  const { user } = useAuth()
  const [page, setPage] = useState(1)
  const [confirming, setConfirming] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const detailState = useAsync(
    (signal) => repositoriesApi.detail(namespace, repo, { signal }),
    `repo:${namespace}:${repo}`,
  )

  const tagsState = useAsync(
    (signal) => repositoriesApi.tags(namespace, repo, page, PER_PAGE, { signal }),
    `repo-tags:${namespace}:${repo}:${page}`,
  )

  const detail = detailState.data
  const host = typeof window === 'undefined' ? '' : window.location.host
  const firstTag = tagsState.data?.items[0]?.name
  const pullCommand = `docker pull ${host}/${namespace}/${repo}:${firstTag ?? '<tag>'}`
  const pullCommandFor = (tag: string) =>
    `docker pull ${host}/${namespace}/${repo}:${tag}`

  const deleteTag = (tag: string) => {
    setActionError(null)
    setDeleting(tag)
    void repositoriesApi.deleteTag(namespace, repo, tag).then(
      () => {
        setDeleting(null)
        setConfirming(null)
        tagsState.reload()
        detailState.reload()
      },
      (error: unknown) => {
        setDeleting(null)
        setConfirming(null)
        setActionError(
          isApiError(error) ? error.message : 'The tag could not be deleted.',
        )
      },
    )
  }

  const columns: TableColumn<TagSummary>[] = [
    {
      key: 'tag',
      header: 'Tag',
      render: (tag) => (
        <Link
          to={tagRoute(namespace, repo, tag.name)}
          className="font-mono text-accent hover:underline"
        >
          {tag.name}
        </Link>
      ),
    },
    {
      key: 'digest',
      header: 'Digest',
      render: (tag) => (
        <span className="font-mono text-xs text-muted" title={tag.digest}>
          {shortDigest(tag.digest)}
        </span>
      ),
    },
    {
      key: 'platforms',
      header: 'Platforms',
      render: (tag) => <PlatformBadges platforms={tag.platforms} />,
    },
    {
      key: 'size',
      header: 'Compressed',
      align: 'right',
      render: (tag) => formatBytes(tag.compressed_size),
    },
    {
      key: 'pulls',
      header: 'Pulls',
      align: 'right',
      render: (tag) => formatNumber(tag.pull_count),
    },
    {
      key: 'updated',
      header: 'Updated',
      align: 'right',
      render: (tag) => (
        <span title={formatDateTime(tag.updated_at)}>
          {formatRelativeTime(tag.updated_at)}
        </span>
      ),
    },
    {
      key: 'actions',
      header: <span className="sr-only">Actions</span>,
      align: 'right',
      render: (tag) =>
        confirming === tag.name ? (
          <span className="flex items-center justify-end gap-1">
            <Button
              size="sm"
              variant="danger"
              disabled={deleting === tag.name}
              onClick={() => deleteTag(tag.name)}
            >
              {deleting === tag.name ? 'Deleting…' : 'Confirm'}
            </Button>
            <Button size="sm" onClick={() => setConfirming(null)}>
              Cancel
            </Button>
          </span>
        ) : (
          <span className="flex items-center justify-end gap-1">
            {detail?.can_pull ? (
              <CopyButton
                value={pullCommandFor(tag.name)}
                label="Copy pull"
                ariaLabel={`Copy pull command for ${tag.name}`}
              />
            ) : null}
            {detail?.can_push ? (
              <Button
                size="sm"
                variant="danger"
                onClick={() => setConfirming(tag.name)}
                aria-label={`Delete tag ${tag.name}`}
                leadingIcon={<TrashIcon size={14} aria-hidden="true" />}
              >
                Delete
              </Button>
            ) : null}
          </span>
        ),
    },
  ]

  return (
    <div className="space-y-4">
      <PageHeader
        breadcrumbs={
          <>
            <Link to="/" className="hover:text-accent hover:underline">
              Explore
            </Link>{' '}
            /{' '}
            <Link
              to={`/${encodeURIComponent(namespace)}`}
              className="hover:text-accent hover:underline"
            >
              {namespace}
            </Link>{' '}
            / <span className="font-mono">{repo}</span>
          </>
        }
        title={<span className="font-mono">{repo}</span>}
        description={detail?.description ?? undefined}
        actions={
          detail ? (
            <span className="flex items-center gap-2">
              <VisibilityLabel isPublic={detail.is_public} />
              {user ? (
                <LinkButton
                  to={`/repositories/${encodeURIComponent(namespace)}/${repo
                    .split('/')
                    .map((segment) => encodeURIComponent(segment))
                    .join('/')}/settings`}
                  size="sm"
                >
                  Settings
                </LinkButton>
              ) : null}
            </span>
          ) : null
        }
      />

      {detailState.loading && !detail ? (
        <LoadingState label="Loading repository…" />
      ) : null}
      {detailState.error ? (
        <ErrorState error={detailState.error} onRetry={detailState.reload} />
      ) : null}

      {detail ? (
        <>
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <Box className="p-3">
              <p className="text-xs text-muted">Tags</p>
              <p className="mt-1 text-lg font-semibold">
                {formatNumber(detail.tag_count)}
              </p>
            </Box>
            <Box className="p-3">
              <p className="text-xs text-muted">Total size</p>
              <p className="mt-1 text-lg font-semibold">
                {formatBytes(detail.total_size)}
              </p>
            </Box>
            <Box className="p-3">
              <p className="text-xs text-muted">Unique size</p>
              <p className="mt-1 text-lg font-semibold">
                {formatBytes(detail.unique_size)}
              </p>
            </Box>
            <Box className="p-3">
              <p className="text-xs text-muted">Pulls</p>
              <p className="mt-1 text-lg font-semibold">
                {formatNumber(detail.pull_count)}
              </p>
            </Box>
          </div>

          <div className="flex flex-col gap-2 rounded-md border border-border bg-canvas-subtle p-3 sm:flex-row sm:items-center sm:justify-between">
            <code className="break-all font-mono text-xs">{pullCommand}</code>
            <CopyButton value={pullCommand} label="Copy pull command" />
          </div>

          {actionError ? <Flash variant="danger">{actionError}</Flash> : null}

          <Box>
            {tagsState.loading && !tagsState.data ? (
              <LoadingState label="Loading tags…" />
            ) : null}
            {tagsState.error ? (
              <ErrorState error={tagsState.error} onRetry={tagsState.reload} />
            ) : null}
            {tagsState.data ? (
              tagsState.data.items.length === 0 ? (
                <EmptyState
                  title="No tags"
                  description="Push an image to this repository to create a tag."
                  icon={<RepoIcon size={24} aria-hidden="true" />}
                />
              ) : (
                <>
                  <Table
                    columns={columns}
                    rows={tagsState.data.items}
                    rowKey={(tag) => tag.name}
                    caption={`Tags for ${namespace}/${repo}`}
                  />
                  <div className="px-4">
                    <Pagination
                      page={tagsState.data.page}
                      perPage={tagsState.data.per_page}
                      total={tagsState.data.total}
                      onPageChange={setPage}
                    />
                  </div>
                </>
              )
            ) : null}
          </Box>
        </>
      ) : null}
    </div>
  )
}
