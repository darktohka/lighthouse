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

import { layers, manifests, type LayerTreeMode } from '../api/endpoints'
import type {
  LayerChange,
  LayerReference,
  LayerTreeEntry,
} from '../api/schemas'
import { CopyButton } from '../components/CopyButton'
import { PageHeader } from '../components/PageHeader'
import { TabNav, TabPanel } from '../components/Tabs'
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
import {
  formatBytes,
  formatDateTime,
  formatMode,
  formatModeOctal,
  splitCommandLines,
} from '../lib/format'
import { apiUrl, repoRoute } from '../lib/paths'
import { useAsync } from '../lib/useAsync'

const MAX_PREVIEW_CHARS = 200_000

type LayerSortKey = 'name' | 'size'
type LayerSortOrder = 'asc' | 'desc'

type LayerTabId = 'layer' | 'aggregated' | 'diff' | 'aggregated-difference'

const TABS: ReadonlyArray<{ id: LayerTabId; label: string }> = [
  { id: 'layer', label: 'Layer' },
  { id: 'aggregated', label: 'Aggregated' },
  { id: 'diff', label: 'Diff' },
  { id: 'aggregated-difference', label: 'Aggregated Difference' },
]

const TAB_TITLES: Record<LayerTabId, string> = {
  layer: 'Layer browser',
  aggregated: 'Aggregated layer view',
  diff: 'Layer diff',
  'aggregated-difference': 'Aggregated layer difference',
}

const MODE_FOR_TAB: Record<LayerTabId, LayerTreeMode> = {
  layer: 'single',
  aggregated: 'aggregate',
  diff: 'diff',
  'aggregated-difference': 'aggregate-diff',
}

const EMPTY_COPY: Record<LayerTabId, { title: string; description: string }> = {
  layer: {
    title: 'Empty directory',
    description: 'There is nothing at this path in the layer.',
  },
  aggregated: {
    title: 'Nothing at this path',
    description:
      'No file from this layer or its ancestors exists at this path.',
  },
  diff: {
    title: 'No changes in this layer',
    description: 'This layer adds, modifies or removes nothing at this path.',
  },
  'aggregated-difference': {
    title: 'No changes in this layer',
    description: 'This layer adds, modifies or removes nothing at this path.',
  },
}

function isLayerTab(value: string): value is LayerTabId {
  return TABS.some((item) => item.id === value)
}

function isDiffTab(tab: LayerTabId): boolean {
  return tab === 'diff' || tab === 'aggregated-difference'
}

function changeTextColor(change: LayerChange | null): string | null {
  switch (change) {
    case 'new':
      return 'text-success'
    case 'modified':
      return 'text-attention'
    case 'removed':
      return 'text-danger'
    default:
      return null
  }
}

function ChangeLegend() {
  const items: ReadonlyArray<{ label: string; dot: string }> = [
    { label: 'New', dot: 'bg-success' },
    { label: 'Modified', dot: 'bg-attention' },
    { label: 'Removed', dot: 'bg-danger' },
  ]
  return (
    <ul
      aria-label="Change legend"
      className="flex flex-wrap items-center gap-3 text-xs text-muted"
    >
      {items.map((item) => (
        <li key={item.label} className="inline-flex items-center gap-1.5">
          <span
            aria-hidden="true"
            className={cx('h-2.5 w-2.5 rounded-full', item.dot)}
          />
          {item.label}
        </li>
      ))}
    </ul>
  )
}

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

function LayerMetadataPanel({ reference }: { reference: LayerReference }) {
  const created = reference.created
  const createdByLines =
    reference.created_by === null ? [] : splitCommandLines(reference.created_by)
  const comment = reference.comment?.trim() ?? ''

  if (createdByLines.length === 0 && created === null && comment.length === 0) {
    return null
  }

  return (
    <Box className="p-3">
      <dl className="space-y-3 text-xs">
        {createdByLines.length > 0 ? (
          <div>
            <dt className="text-muted">Created by</dt>
            <dd className="mt-1">
              <div className="max-h-[300px] overflow-auto rounded-md border border-border bg-canvas-inset p-3 font-mono text-xs">
                {createdByLines.map((line, index) => (
                  <p
                    key={`${index}:${line}`}
                    className="whitespace-pre-wrap break-all"
                  >
                    {line}
                  </p>
                ))}
              </div>
            </dd>
          </div>
        ) : null}
        {created !== null ? (
          <div>
            <dt className="text-muted">Created</dt>
            <dd className="mt-0.5">{formatDateTime(created)}</dd>
          </div>
        ) : null}
        {comment.length > 0 ? (
          <div>
            <dt className="text-muted">Comment</dt>
            <dd className="mt-0.5 whitespace-pre-wrap break-all">{comment}</dd>
          </div>
        ) : null}
      </dl>
    </Box>
  )
}

export function LayerBrowserPage({
  namespace,
  repo,
  digest,
}: LayerBrowserPageProps) {
  const [searchParams, setSearchParams] = useSearchParams()
  const path = searchParams.get('path') ?? ''
  const manifest = searchParams.get('manifest')
  const requestedTab = searchParams.get('tab')
  const tab: LayerTabId =
    requestedTab && isLayerTab(requestedTab) ? requestedTab : 'layer'
  const mode = MODE_FOR_TAB[tab]
  const manifestMissing = mode !== 'single' && manifest === null
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [sort, setSort] = useState<{ key: LayerSortKey; order: LayerSortOrder }>({
    key: 'name',
    order: 'asc',
  })

  const state = useAsync(
    (signal): Promise<LayerTreeEntry[]> => {
      if (manifestMissing) return Promise.resolve([])
      return layers.tree(
        namespace,
        repo,
        digest,
        {
          path: path.length > 0 ? path : undefined,
          mode,
          manifest: mode === 'single' ? undefined : (manifest ?? undefined),
        },
        { signal },
      )
    },
    `layer-tree:${namespace}:${repo}:${digest}:${mode}:${manifest ?? ''}:${path}`,
  )

  const referenceState = useAsync(
    (signal): Promise<LayerReference[]> => {
      if (manifest === null) return Promise.resolve([])
      return manifests.references(namespace, repo, manifest, { signal })
    },
    `manifest-references:${namespace}:${repo}:${manifest ?? ''}`,
  )

  const reference =
    manifest !== null && !referenceState.loading && !referenceState.error
      ? (referenceState.data?.find((item) => item.digest === digest) ?? null)
      : null

  const changeTab = (id: string) => {
    if (!isLayerTab(id)) return
    const params = new URLSearchParams(searchParams)
    if (id === 'layer') params.delete('tab')
    else params.set('tab', id)
    params.delete('path')
    setSearchParams(params)
    setSelectedFile(null)
  }

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
        const changed = changeTextColor(entry.change)
        if (entry.kind === 'dir') {
          return (
            <FileDirectoryFillIcon
              size={16}
              aria-hidden="true"
              className={changed ?? 'text-accent'}
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
              className={changed ?? (clickable ? 'text-accent' : 'text-muted')}
            />
          )
        }
        return (
          <FileIcon
            size={16}
            aria-hidden="true"
            className={changed ?? 'text-muted'}
          />
        )
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
        const changed = changeTextColor(entry.change)
        if (entry.kind === 'dir') {
          return (
            <button
              type="button"
              onClick={() => goToPath(entry.path)}
              className={cx(
                'cursor-pointer font-mono hover:underline',
                changed ?? 'text-accent',
              )}
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
            return (
              <span className={cx('font-mono', changed ?? 'text-muted')}>
                {label}
              </span>
            )
          }
          return (
            <button
              type="button"
              onClick={() => openLink(resolved, targetKind)}
              title={resolved}
              className={cx(
                'cursor-pointer font-mono hover:underline',
                changed ?? 'text-accent',
              )}
            >
              {label}
            </button>
          )
        }
        if (entry.change === 'removed') {
          return (
            <span
              className="font-mono text-danger line-through"
              title="Removed in this layer — no contents to preview"
            >
              {entry.name}
            </span>
          )
        }
        const selected = selectedFile === entry.path
        return (
          <button
            type="button"
            onClick={() =>
              setSelectedFile((current) =>
                current === entry.path ? null : entry.path,
              )
            }
            aria-expanded={selected}
            className={cx(
              'cursor-pointer font-mono hover:underline',
              selected ? 'font-semibold' : '',
              changed ?? (selected ? 'text-foreground' : 'text-accent'),
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
        title={TAB_TITLES[tab]}
        description={
          <span className="font-mono text-xs break-all">
            {digest}
            {manifest ? ` · manifest ${manifest}` : ''}
          </span>
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

      {reference ? <LayerMetadataPanel reference={reference} /> : null}

      <LayerBreadcrumbs segments={segments} onNavigate={goToPath} />

      <TabNav label="Layer views" tabs={TABS} active={tab} onChange={changeTab} />

      <TabPanel id={tab} active={tab}>
        {manifestMissing ? (
          <Flash variant="warning" title="Image context required">
            The aggregated views overlay a layer on top of its ancestors, so
            they need the digest of the image manifest that owns this layer.
            Open the layer from a tag&rsquo;s Layers tab, or add a{' '}
            <code>manifest</code> query parameter.
          </Flash>
        ) : (
          <div className="space-y-3">
            {isDiffTab(tab) ? <ChangeLegend /> : null}

            <Box>
              {state.loading ? (
                <LoadingState label="Listing layer…" />
              ) : null}
              {!state.loading && state.error ? (
                <ErrorState error={state.error} onRetry={state.reload} />
              ) : null}
              {!state.loading && !state.error && state.data ? (
                <Table
                  columns={columns}
                  rows={rows}
                  rowKey={(entry) => `${entry.kind}:${entry.path}`}
                  caption={`Contents of /${path}`}
                  empty={
                    <EmptyState
                      title={EMPTY_COPY[tab].title}
                      description={EMPTY_COPY[tab].description}
                    />
                  }
                  rowDetails={(entry) =>
                    entry.kind === 'file' &&
                    entry.change !== 'removed' &&
                    entry.path === selectedFile ? (
                      <FilePreview
                        namespace={namespace}
                        repo={repo}
                        digest={entry.source_digest ?? digest}
                        path={entry.path}
                      />
                    ) : null
                  }
                />
              ) : null}
            </Box>
          </div>
        )}
      </TabPanel>
    </div>
  )
}
