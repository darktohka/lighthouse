import {
  DownloadIcon,
  FileDirectoryFillIcon,
  FileIcon,
  LinkIcon,
} from '@primer/octicons-react'
import { useState } from 'react'
import { Link, useSearchParams } from 'react-router-dom'

import { layers } from '../api/endpoints'
import type { LayerTreeEntry } from '../api/schemas'
import { PageHeader } from '../components/PageHeader'
import { AnchorButton } from '../components/primitives/Button'
import { Box } from '../components/primitives/Box'
import { Flash } from '../components/primitives/Flash'
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from '../components/primitives/StateViews'
import { Table, type TableColumn } from '../components/primitives/Table'
import { decodeText, isProbablyBinary } from '../lib/content'
import { cx } from '../lib/cx'
import { formatBytes } from '../lib/format'
import { apiUrl, repoRoute } from '../lib/paths'
import { useAsync } from '../lib/useAsync'

const MAX_PREVIEW_CHARS = 200_000

type PreviewResult =
  | { kind: 'text'; text: string; contentType: string; truncated: boolean }
  | { kind: 'binary'; contentType: string }
  | { kind: 'empty' }

function isTextContentType(contentType: string): boolean {
  if (contentType.length === 0) return true
  const type = contentType.split(';')[0].trim().toLowerCase()
  if (type.startsWith('text/')) return true
  if (type.endsWith('+json') || type.endsWith('+xml')) return true
  return [
    'application/json',
    'application/xml',
    'application/javascript',
    'application/x-yaml',
    'application/yaml',
    'image/svg+xml',
    'application/x-sh',
    'application/x-httpd-php',
  ].includes(type)
}

function FilePreview({
  namespace,
  repo,
  digest,
  path,
}: {
  namespace: string
  repo: string
  digest: string
  path: string
}) {
  const state = useAsync<PreviewResult>(
    async (signal) => {
      const response = await layers.file(namespace, repo, digest, path, {
        signal,
      })
      const contentType = response.headers.get('content-type') ?? ''
      if (response.status === 204) return { kind: 'empty' }
      const bytes = new Uint8Array(await response.arrayBuffer())
      if (bytes.length === 0) return { kind: 'empty' }
      if (!isTextContentType(contentType) && isProbablyBinary(bytes)) {
        return { kind: 'binary', contentType }
      }
      const text = decodeText(bytes)
      if (text.length > MAX_PREVIEW_CHARS) {
        return {
          kind: 'text',
          text: text.slice(0, MAX_PREVIEW_CHARS),
          contentType,
          truncated: true,
        }
      }
      return { kind: 'text', text, contentType, truncated: false }
    },
    `layer-file:${namespace}:${repo}:${digest}:${path}`,
  )

  if (state.loading) return <LoadingState label="Loading file…" />
  if (state.error) {
    return <ErrorState error={state.error} onRetry={state.reload} />
  }
  if (!state.data) return null

  const result = state.data
  if (result.kind === 'empty') {
    return <p className="text-sm text-muted">This file is empty.</p>
  }
  if (result.kind === 'binary') {
    return (
      <div className="space-y-2">
        <p className="text-sm text-muted">
          Binary file ({result.contentType || 'unknown type'}) — preview
          unavailable.
        </p>
        <AnchorButton
          href={apiUrl(layers.layerFilePath(namespace, repo, digest, path))}
          download
          size="sm"
          leadingIcon={<DownloadIcon size={14} aria-hidden="true" />}
        >
          Download file
        </AnchorButton>
      </div>
    )
  }
  return (
    <div className="space-y-2">
      {result.truncated ? (
        <Flash variant="warning">
          Preview truncated at {MAX_PREVIEW_CHARS.toLocaleString()} characters.
          Download the file for the full contents.
        </Flash>
      ) : null}
      <pre className="max-h-[600px] overflow-auto whitespace-pre-wrap break-all rounded-md border border-border bg-canvas-inset p-3 font-mono text-xs leading-5">
        {result.text}
      </pre>
    </div>
  )
}

export type LayerBrowserPageProps = {
  namespace: string
  repo: string
  digest: string
}

export function LayerBrowserPage({
  namespace,
  repo,
  digest,
}: LayerBrowserPageProps) {
  const [searchParams, setSearchParams] = useSearchParams()
  const path = searchParams.get('path') ?? ''
  const [selectedFile, setSelectedFile] = useState<string | null>(null)

  const state = useAsync(
    (signal) =>
      layers.tree(namespace, repo, digest, path || undefined, { signal }),
    `layer-tree:${namespace}:${repo}:${digest}:${path}`,
  )

  const goToPath = (next: string) => {
    const params = new URLSearchParams(searchParams)
    if (next.length > 0) params.set('path', next)
    else params.delete('path')
    setSearchParams(params)
    setSelectedFile(null)
  }

  const segments = path.split('/').filter((segment) => segment.length > 0)

  const columns: TableColumn<LayerTreeEntry>[] = [
    {
      key: 'kind',
      header: <span className="sr-only">Kind</span>,
      className: 'w-6',
      render: (entry) => {
        if (entry.kind === 'dir') {
          return (
            <FileDirectoryFillIcon
              size={16}
              aria-hidden="true"
              className="text-accent"
            />
          )
        }
        if (entry.kind === 'symlink') {
          return (
            <LinkIcon size={16} aria-hidden="true" className="text-muted" />
          )
        }
        return <FileIcon size={16} aria-hidden="true" className="text-muted" />
      },
    },
    {
      key: 'name',
      header: 'Name',
      render: (entry) => {
        if (entry.kind === 'dir') {
          return (
            <button
              type="button"
              onClick={() => goToPath(entry.path)}
              className="font-mono text-accent hover:underline"
            >
              {entry.name}/
            </button>
          )
        }
        if (entry.kind === 'symlink') {
          return (
            <span className="font-mono text-muted">
              {entry.name} → {entry.link_target ?? '?'}
            </span>
          )
        }
        return (
          <button
            type="button"
            onClick={() => setSelectedFile(entry.path)}
            className={cx(
              'font-mono hover:underline',
              selectedFile === entry.path
                ? 'font-semibold text-foreground'
                : 'text-accent',
            )}
          >
            {entry.name}
          </button>
        )
      },
    },
    {
      key: 'size',
      header: 'Size',
      align: 'right',
      render: (entry) =>
        entry.kind === 'dir' ? '—' : formatBytes(entry.size),
    },
    {
      key: 'mode',
      header: 'Mode',
      align: 'right',
      render: (entry) => (
        <span className="font-mono text-xs text-muted">{entry.mode}</span>
      ),
    },
  ]

  const downloadPath = layers.layerDownloadPath(namespace, repo, digest)

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
            / <span className="font-mono">{digest.slice(0, 18)}…</span>
          </>
        }
        title="Layer browser"
        description={
          <span className="font-mono text-xs break-all">{digest}</span>
        }
        actions={
          <AnchorButton
            href={apiUrl(downloadPath)}
            download
            size="sm"
            leadingIcon={<DownloadIcon size={14} aria-hidden="true" />}
          >
            Download layer
          </AnchorButton>
        }
      />

      <nav aria-label="Layer path" className="flex flex-wrap items-center gap-1 text-xs">
        <button
          type="button"
          onClick={() => goToPath('')}
          className="font-mono text-accent hover:underline"
        >
          /
        </button>
        {segments.map((segment, index) => {
          const target = segments.slice(0, index + 1).join('/')
          return (
            <span key={target} className="flex items-center gap-1">
              <span className="text-muted">/</span>
              <button
                type="button"
                onClick={() => goToPath(target)}
                className="font-mono text-accent hover:underline"
              >
                {segment}
              </button>
            </span>
          )
        })}
      </nav>

      <Box>
        {state.loading && !state.data ? (
          <LoadingState label="Listing layer…" />
        ) : null}
        {state.error ? (
          <ErrorState error={state.error} onRetry={state.reload} />
        ) : null}
        {state.data ? (
          state.data.length === 0 ? (
            <EmptyState
              title="Empty directory"
              description="There is nothing at this path in the layer."
            />
          ) : (
            <Table
              columns={columns}
              rows={state.data}
              rowKey={(entry) => `${entry.kind}:${entry.path}`}
              caption={`Contents of /${path}`}
            />
          )
        ) : null}
      </Box>

      {selectedFile ? (
        <section aria-labelledby="file-preview">
          <h2 id="file-preview" className="mb-2 text-base font-semibold">
            <span className="font-mono">{selectedFile}</span>
          </h2>
          <FilePreview
            namespace={namespace}
            repo={repo}
            digest={digest}
            path={selectedFile}
          />
        </section>
      ) : null}
    </div>
  )
}
