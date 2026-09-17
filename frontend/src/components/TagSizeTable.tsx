import { ChevronDownIcon, ChevronUpIcon } from '@primer/octicons-react'
import type { ReactNode } from 'react'
import { Link } from 'react-router-dom'

import type { TagSizeEntry } from '../api/schemas'
import { formatBytes, formatDateTime, formatRelativeTime } from '../lib/format'
import { repositoryRelativePath, repoRoute, tagRoute } from '../lib/paths'
import { PlatformBadges } from './PlatformBadge'
import { Table, type TableColumn } from './primitives/Table'
import { tagSelectionKey } from './tagSelection'

function SortHeader({
  label,
  active,
  order,
  onSort,
}: {
  label: string
  active: boolean
  order: 'asc' | 'desc'
  onSort: () => void
}) {
  return (
    <button
      type="button"
      onClick={onSort}
      className="inline-flex items-center gap-1 font-semibold hover:text-foreground"
    >
      {label}
      {active ? (
        order === 'desc' ? (
          <ChevronDownIcon size={12} aria-hidden="true" />
        ) : (
          <ChevronUpIcon size={12} aria-hidden="true" />
        )
      ) : null}
    </button>
  )
}

export type TagSizeTableProps = {
  entries: readonly TagSizeEntry[]
  selected: ReadonlySet<string>
  onToggle: (key: string) => void
  onToggleAll: () => void
  allSelected: boolean
  sort: 'total_size' | 'unique_size'
  order: 'asc' | 'desc'
  onSort: (sort: 'total_size' | 'unique_size') => void
}

export function TagSizeTable({
  entries,
  selected,
  onToggle,
  onToggleAll,
  allSelected,
  sort,
  order,
  onSort,
}: TagSizeTableProps) {
  const columns: TableColumn<TagSizeEntry>[] = [
    {
      key: 'select',
      header: (
        <input
          type="checkbox"
          checked={allSelected}
          onChange={onToggleAll}
          aria-label="Select all tags on this page"
        />
      ),
      render: (entry) => {
        const key = tagSelectionKey(entry)
        return (
          <input
            type="checkbox"
            checked={selected.has(key)}
            onChange={() => onToggle(key)}
            aria-label={`Select ${entry.repository}:${entry.tag}`}
          />
        )
      },
    },
    {
      key: 'repository',
      header: 'Repository',
      render: (entry) => {
        const relative = repositoryRelativePath(entry.namespace, {
          path: entry.repository,
          name: entry.repository,
        })
        return (
          <Link
            to={repoRoute(entry.namespace, relative)}
            className="break-all font-mono text-xs text-accent hover:underline"
          >
            {entry.repository}
          </Link>
        )
      },
    },
    {
      key: 'tag',
      header: 'Tag',
      render: (entry) => {
        const relative = repositoryRelativePath(entry.namespace, {
          path: entry.repository,
          name: entry.repository,
        })
        return (
          <Link
            to={tagRoute(entry.namespace, relative, entry.tag)}
            className="font-mono text-accent hover:underline"
          >
            {entry.tag}
          </Link>
        )
      },
    },
    {
      key: 'platforms',
      header: 'Platforms',
      render: (entry) => <PlatformBadges platforms={entry.platforms} limit={3} />,
    },
    {
      key: 'total_size',
      header: (
        <SortHeader
          label="Total"
          active={sort === 'total_size'}
          order={order}
          onSort={() => onSort('total_size')}
        />
      ),
      align: 'right',
      render: (entry) => formatBytes(entry.total_size),
    },
    {
      key: 'unique_size',
      header: (
        <SortHeader
          label="Unique"
          active={sort === 'unique_size'}
          order={order}
          onSort={() => onSort('unique_size')}
        />
      ),
      align: 'right',
      render: (entry) => formatBytes(entry.unique_size),
    },
    {
      key: 'shared',
      header: 'Shared storage',
      render: (entry): ReactNode => {
        const percentage =
          entry.total_size > 0 ? (entry.shared_size / entry.total_size) * 100 : 0
        return (
          <span className="flex items-center gap-2">
            <span className="h-1.5 w-20 overflow-hidden rounded-full bg-canvas-subtle">
              <span
                className="block h-full rounded-full bg-attention"
                style={{ width: `${percentage}%` }}
              />
            </span>
            <span className="tabular-nums text-xs text-muted">
              {percentage.toFixed(0)}%
            </span>
          </span>
        )
      },
    },
    {
      key: 'updated',
      header: 'Updated',
      align: 'right',
      render: (entry) => (
        <span title={formatDateTime(entry.updated_at)}>
          {formatRelativeTime(entry.updated_at)}
        </span>
      ),
    },
  ]

  return (
    <Table
      columns={columns}
      rows={entries}
      rowKey={(entry) => tagSelectionKey(entry)}
      caption="Visible tags ranked by storage size"
    />
  )
}
