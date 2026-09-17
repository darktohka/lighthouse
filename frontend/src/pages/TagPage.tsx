import { FileIcon } from '@primer/octicons-react'
import { useState } from 'react'
import { Link } from 'react-router-dom'

import { repositories as repositoriesApi } from '../api/endpoints'
import type { LayerInfo } from '../api/schemas'
import { CopyButton } from '../components/CopyButton'
import { JsonViewer } from '../components/json-viewer'
import { PageHeader } from '../components/PageHeader'
import { PlatformBadges } from '../components/PlatformBadge'
import { Box } from '../components/primitives/Box'
import { LinkButton } from '../components/primitives/Button'
import { Label } from '../components/primitives/Label'
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from '../components/primitives/StateViews'
import { Table, type TableColumn } from '../components/primitives/Table'
import { cx } from '../lib/cx'
import {
  formatBytes,
  formatDateTime,
  formatNumber,
  formatRelativeTime,
  shortDigest,
} from '../lib/format'
import { layerRoute, repoRoute } from '../lib/paths'
import { useAsync } from '../lib/useAsync'

export type TagPageProps = {
  namespace: string
  repo: string
  tag: string
}

type TabId = 'manifest' | 'config' | 'layers'

const TABS: ReadonlyArray<{ id: TabId; label: string }> = [
  { id: 'manifest', label: 'Manifest' },
  { id: 'config', label: 'Config' },
  { id: 'layers', label: 'Layers' },
]

function tabClass(active: boolean): string {
  return cx(
    '-mb-px border-b-2 px-3 py-1.5 text-sm font-medium',
    active
      ? 'border-accent text-foreground'
      : 'border-transparent text-muted hover:text-foreground',
  )
}

export function TagPage({ namespace, repo, tag }: TagPageProps) {
  const [tab, setTab] = useState<TabId>('manifest')
  const state = useAsync(
    (signal) => repositoriesApi.tag(namespace, repo, tag, { signal }),
    `tag:${namespace}:${repo}:${tag}`,
  )
  const detail = state.data
  const host = typeof window === 'undefined' ? '' : window.location.host
  const pullCommand = `docker pull ${host}/${namespace}/${repo}:${tag}`

  const layerColumns: TableColumn<LayerInfo>[] = [
    {
      key: 'role',
      header: 'Role',
      render: (layer) => (
        <Label variant={layer.role === 'config' ? 'accent' : 'muted'}>
          {layer.role}
        </Label>
      ),
    },
    {
      key: 'digest',
      header: 'Digest',
      render: (layer) => (
        <span className="font-mono text-xs" title={layer.digest}>
          {shortDigest(layer.digest)}
        </span>
      ),
    },
    {
      key: 'media_type',
      header: 'Media type',
      render: (layer) => (
        <span className="break-all font-mono text-xs">{layer.media_type}</span>
      ),
    },
    {
      key: 'size',
      header: 'Size',
      align: 'right',
      render: (layer) => formatBytes(layer.size),
    },
    {
      key: 'uncompressed_size',
      header: 'Uncompressed',
      align: 'right',
      render: (layer) => formatBytes(layer.uncompressed_size),
    },
    {
      key: 'actions',
      header: <span className="sr-only">Actions</span>,
      align: 'right',
      render: (layer) =>
        layer.role === 'layer' ? (
          <LinkButton
            size="sm"
            to={layerRoute(namespace, repo, layer.digest)}
          >
            Browse
          </LinkButton>
        ) : (
          <span className="text-xs text-muted">—</span>
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
            /{' '}
            <Link
              to={repoRoute(namespace, repo)}
              className="font-mono hover:text-accent hover:underline"
            >
              {repo}
            </Link>{' '}
            / <span className="font-mono">{tag}</span>
          </>
        }
        title={<span className="font-mono">{tag}</span>}
        description={
          detail
            ? `Updated ${formatRelativeTime(detail.updated_at)} · ${formatNumber(detail.pull_count)} pulls`
            : undefined
        }
      />

      {state.loading && !detail ? (
        <LoadingState label="Loading tag…" />
      ) : null}
      {state.error ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}

      {detail ? (
        <>
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <Box className="p-3">
              <p className="text-xs text-muted">Compressed</p>
              <p className="mt-1 text-lg font-semibold">
                {formatBytes(detail.compressed_size)}
              </p>
            </Box>
            <Box className="p-3">
              <p className="text-xs text-muted">Uncompressed</p>
              <p className="mt-1 text-lg font-semibold">
                {formatBytes(detail.uncompressed_size)}
              </p>
            </Box>
            <Box className="p-3">
              <p className="text-xs text-muted">Pulls</p>
              <p className="mt-1 text-lg font-semibold">
                {formatNumber(detail.pull_count)}
              </p>
            </Box>
            <Box className="p-3">
              <p className="text-xs text-muted">Platforms</p>
              <p className="mt-1 text-lg font-semibold">
                {detail.platforms.length}
              </p>
            </Box>
          </div>

          {detail.can_pull ? (
            <div className="flex flex-col gap-2 rounded-md border border-border bg-canvas-subtle p-3 sm:flex-row sm:items-center sm:justify-between">
              <code className="break-all font-mono text-xs">{pullCommand}</code>
              <CopyButton value={pullCommand} label="Copy pull command" />
            </div>
          ) : null}

          <Box className="p-3">
            <dl className="grid gap-2 text-xs sm:grid-cols-2">
              <div>
                <dt className="text-muted">Digest</dt>
                <dd className="break-all font-mono">{detail.digest}</dd>
              </div>
              <div>
                <dt className="text-muted">Media type</dt>
                <dd className="break-all font-mono">{detail.media_type}</dd>
              </div>
              <div>
                <dt className="text-muted">Updated</dt>
                <dd>{formatDateTime(detail.updated_at)}</dd>
              </div>
              <div>
                <dt className="text-muted">Platforms</dt>
                <dd className="mt-1">
                  <PlatformBadges platforms={detail.platforms} limit={8} />
                </dd>
              </div>
            </dl>
          </Box>

          <div>
            <div role="tablist" aria-label="Tag details" className="flex gap-1 border-b border-border">
              {TABS.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  role="tab"
                  id={`tab-${item.id}`}
                  aria-selected={tab === item.id}
                  aria-controls={`panel-${item.id}`}
                  onClick={() => setTab(item.id)}
                  className={tabClass(tab === item.id)}
                >
                  {item.label}
                </button>
              ))}
            </div>

            <div
              role="tabpanel"
              id={`panel-${tab}`}
              aria-labelledby={`tab-${tab}`}
              tabIndex={0}
              className="pt-4"
            >
              {tab === 'manifest' ? (
                <JsonViewer data={detail.manifest} defaultExpandDepth={3} />
              ) : null}

              {tab === 'config' ? (
                detail.config ? (
                  <JsonViewer data={detail.config} defaultExpandDepth={2} />
                ) : (
                  <EmptyState
                    title="No config blob"
                    description="This manifest does not reference a configuration blob."
                    icon={<FileIcon size={24} aria-hidden="true" />}
                  />
                )
              ) : null}

              {tab === 'layers' ? (
                detail.layers.length === 0 ? (
                  <EmptyState
                    title="No layers"
                    description="This manifest does not list any layers."
                    icon={<FileIcon size={24} aria-hidden="true" />}
                  />
                ) : (
                  <Box>
                    <Table
                      columns={layerColumns}
                      rows={detail.layers}
                      rowKey={(layer) => `${layer.role}:${layer.digest}`}
                      caption="Layers referenced by this tag"
                    />
                  </Box>
                )
              ) : null}
            </div>
          </div>
        </>
      ) : null}
    </div>
  )
}
