import {
  ArrowLeftIcon,
  ChevronRightIcon,
  DownloadIcon,
  FileDirectoryFillIcon,
  FileIcon,
  LinkIcon,
  MultiSelectIcon,
} from '@primer/octicons-react'
import { useMemo, useRef, useState } from 'react'
import { Link, useSearchParams } from 'react-router-dom'

import { layers } from '../api/endpoints'
import type { LayerTreeEntry } from '../api/schemas'
import { CopyButton } from '../components/CopyButton'
import { PageHeader } from '../components/PageHeader'
import { AnchorButton, Button } from '../components/primitives/Button'
import { Box } from '../components/primitives/Box'
import { Flash } from '../components/primitives/Flash'
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from '../components/primitives/StateViews'
import {
  Table,
  TableSortHeader,
  type TableColumn,
} from '../components/primitives/Table'
import { decodeText, isProbablyBinary } from '../lib/content'
import { cx } from '../lib/cx'
import { formatBytes, formatMode, formatModeOctal } from '../lib/format'
import { apiUrl, repoRoute } from '../lib/paths'
import { useAsync } from '../lib/useAsync'

const MAX_PREVIEW_CHARS = 200_000

type LayerSortKey = 'name' | 'size'
type LayerSortOrder = 'asc' | 'desc'

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

  const preRef = useRef<HTMLPreElement>(null)

  if (state.loading) return <LoadingState label="Loading file…" />
  if (state.error) {
    return <ErrorState error={state.error} onRetry={state.reload} />
  }
  if (!state.data) return null

  const result = state.data
  const fileName = path.split('/').pop() || path
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
          download={fileName}
          size="sm"
          leadingIcon={<DownloadIcon size={14} aria-hidden="true" />}
        >
          Download file
        </AnchorButton>
      </div>
    )
  }
  const selectAll = () => {
    const node = preRef.current
    if (node === null) return
    const selection = window.getSelection()
    if (selection === null) return
    const range = document.createRange()
    range.selectNodeContents(node)
    selection.removeAllRanges()
    selection.addRange(range)
  }

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          onClick={selectAll}
          leadingIcon={<MultiSelectIcon size={14} aria-hidden="true" />}
        >
          Select all
        </Button>
        <CopyButton value={result.text} />
        <AnchorButton
          href={apiUrl(layers.layerFilePath(namespace, repo, digest, path))}
          download={fileName}
          size="sm"
          leadingIcon={<DownloadIcon size={14} aria-hidden="true" />}
        >
          Download
        </AnchorButton>
      </div>
      {result.truncated ? (
        <Flash variant="warning">
          Preview truncated at {MAX_PREVIEW_CHARS.toLocaleString()} characters.
          Download the file for the full contents.
        </Flash>
      ) : null}
      <pre
        ref={preRef}
        className="max-h-[600px] overflow-auto whitespace-pre-wrap break-all rounded-md border border-border bg-canvas-inset p-3 font-mono text-xs leading-5"
      >
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

function LayerBreadcrumbs({
  segments,
  onNavigate,
}: {
  segments: readonly string[]
  onNavigate: (path: string) => void
}) {
  const chip =
    'inline-flex h-7 max-w-[20rem] cursor-pointer items-center rounded-full border border-border bg-canvas-subtle px-3 font-mono text-xs text-accent transition-colors hover:border-accent hover:bg-neutral-subtle'

  return (
    <nav aria-label="Layer path" className="flex flex-wrap items-center gap-1.5">
      {segments.length === 0 ? (
        <span
          aria-current="page"
          className="inline-flex h-7 items-center rounded-full border border-border bg-neutral-muted px-3 font-mono text-xs font-semibold text-foreground"
        >
          /
        </span>
      ) : (
        <button
          type="button"
          onClick={() => onNavigate('')}
          title="Layer root"
          aria-label="Layer root"
          className={chip}
        >
          /
        </button>
      )}
      {segments.map((segment, index) => {
        const target = segments.slice(0, index + 1).join('/')
        const isCurrent = index === segments.length - 1
        return (
          <span key={target} className="flex items-center gap-1.5">
            <ChevronRightIcon
              size={12}
              aria-hidden="true"
              className="shrink-0 text-muted"
            />
            {isCurrent ? (
              <span
                aria-current="page"
                title={target}
                className="inline-flex h-7 max-w-[20rem] items-center rounded-full border border-border bg-neutral-muted px-3 font-mono text-xs font-semibold text-foreground"
              >
                <span className="min-w-0 truncate">{segment}</span>
              </span>
            ) : (
              <button
                type="button"
                onClick={() => onNavigate(target)}
                title={target}
                className={chip}
              >
                <span className="min-w-0 truncate">{segment}</span>
              </button>
            )}
          </span>
        )
      })}
    </nav>
  )
}

export function LayerBrowserPage({
  namespace,
  repo,
  digest,
}: LayerBrowserPageProps) {
  const [searchParams, setSearchParams] = useSearchParams()
  const path = searchParams.get('path') ?? ''
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [sort, setSort] = useState<{ key: LayerSortKey; order: LayerSortOrder }>({
    key: 'name',
    order: 'asc',
  })

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

  const openLink = (resolved: string, kind: 'file' | 'dir') => {
    if (kind === 'dir') {
      goToPath(resolved)
      return
    }
    const parent = resolved.includes('/')
      ? resolved.slice(0, resolved.lastIndexOf('/'))
      : ''
    if (parent !== path) {
      const params = new URLSearchParams(searchParams)
      if (parent.length > 0) params.set('path', parent)
      else params.delete('path')
      setSearchParams(params)
    }
    setSelectedFile(resolved)
  }

  const segments = path.split('/').filter((segment) => segment.length > 0)

  const parentPath = segments.slice(0, -1).join('/')
  const backLabel =
    parentPath.length > 0 ? `Back to /${parentPath}` : 'Back to layer root'

  const changeSort = (key: LayerSortKey) => {
    setSort((current) =>
      current.key === key
        ? { key, order: current.order === 'asc' ? 'desc' : 'asc' }
        : { key, order: key === 'size' ? 'desc' : 'asc' },
    )
  }

  const rows = useMemo(() => {
    const entries = [...(state.data ?? [])]
    const factor = sort.order === 'asc' ? 1 : -1
    entries.sort((a, b) => {
      if (sort.key === 'size') {
        return (a.size - b.size || a.name.localeCompare(b.name)) * factor
      }
      return (a.name.localeCompare(b.name) || a.size - b.size) * factor
    })
    return entries
  }, [state.data, sort])

  const columns: TableColumn<LayerTreeEntry>[] = [
    {
      key: 'kind',
      header: (
        <>
          {segments.length > 0 ? (
            <button
              type="button"
              onClick={() => goToPath(parentPath)}
              title={backLabel}
              aria-label={backLabel}
              className="inline-flex cursor-pointer items-center justify-center rounded-md p-0.5 text-muted transition-colors hover:bg-neutral-subtle hover:text-foreground"
            >
              <ArrowLeftIcon size={16} aria-hidden="true" />
            </button>
          ) : null}
          <span className="sr-only">Kind</span>
        </>
      ),
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
          const clickable =
            entry.link_resolved !== null && entry.link_kind !== null
          return (
            <LinkIcon
              size={16}
              aria-hidden="true"
              className={clickable ? 'text-accent' : 'text-muted'}
            />
          )
        }
        return <FileIcon size={16} aria-hidden="true" className="text-muted" />
      },
    },
    {
      key: 'name',
      header: (
        <TableSortHeader
          label="Name"
          active={sort.key === 'name'}
          direction={sort.order}
          onSort={() => changeSort('name')}
        />
      ),
      sortDirection: sort.key === 'name' ? sort.order : undefined,
      render: (entry) => {
        if (entry.kind === 'dir') {
          return (
            <button
              type="button"
              onClick={() => goToPath(entry.path)}
              className="cursor-pointer font-mono text-accent hover:underline"
            >
              {entry.name}/
            </button>
          )
        }
        if (entry.kind === 'symlink') {
          const label = `${entry.name} → ${entry.link_target ?? '?'}`
          const resolved = entry.link_resolved
          const targetKind = entry.link_kind
          if (resolved === null || targetKind === null) {
            return <span className="font-mono text-muted">{label}</span>
          }
          return (
            <button
              type="button"
              onClick={() => openLink(resolved, targetKind)}
              title={resolved}
              className="cursor-pointer font-mono text-accent hover:underline"
            >
              {label}
            </button>
          )
        }
        return (
          <button
            type="button"
            onClick={() =>
              setSelectedFile((current) =>
                current === entry.path ? null : entry.path,
              )
            }
            aria-expanded={selectedFile === entry.path}
            className={cx(
              'cursor-pointer font-mono hover:underline',
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
      header: (
        <TableSortHeader
          label="Size"
          active={sort.key === 'size'}
          direction={sort.order}
          onSort={() => changeSort('size')}
        />
      ),
      align: 'right',
      sortDirection: sort.key === 'size' ? sort.order : undefined,
      render: (entry) =>
        entry.kind === 'dir' ? (
          <span className="text-muted" title="Recursive total">
            {formatBytes(entry.size)}
          </span>
        ) : (
          formatBytes(entry.size)
        ),
    },
    {
      key: 'mode',
      header: 'Mode',
      align: 'right',
      render: (entry) => (
        <span className="whitespace-nowrap font-mono text-xs text-muted">
          {formatMode(entry.mode, entry.kind)}{' '}
          <span className="text-subtle">{formatModeOctal(entry.mode)}</span>
        </span>
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

      <LayerBreadcrumbs segments={segments} onNavigate={goToPath} />

      <Box>
        {state.loading && !state.data ? (
          <LoadingState label="Listing layer…" />
        ) : null}
        {state.error ? (
          <ErrorState error={state.error} onRetry={state.reload} />
        ) : null}
        {state.data ? (
          <Table
            columns={columns}
            rows={rows}
            rowKey={(entry) => `${entry.kind}:${entry.path}`}
            caption={`Contents of /${path}`}
            empty={
              <EmptyState
                title="Empty directory"
                description="There is nothing at this path in the layer."
              />
            }
            rowDetails={(entry) =>
              entry.kind === 'file' && entry.path === selectedFile ? (
                <FilePreview
                  namespace={namespace}
                  repo={repo}
                  digest={digest}
                  path={entry.path}
                />
              ) : null
            }
          />
        ) : null}
      </Box>
    </div>
  )
}
