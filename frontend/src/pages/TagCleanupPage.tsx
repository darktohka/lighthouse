import { TagIcon } from '@primer/octicons-react'
import { useState } from 'react'

import { isApiError } from '../api/client'
import { tags as tagsApi } from '../api/endpoints'
import { ConfirmAction } from '../components/ConfirmAction'
import { SelectField } from '../components/FormFields'
import { PageHeader } from '../components/PageHeader'
import { Pagination } from '../components/Pagination'
import { Box } from '../components/primitives/Box'
import { Flash } from '../components/primitives/Flash'
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from '../components/primitives/StateViews'
import { TagSizeTable } from '../components/TagSizeTable'
import { tagSelectionKey } from '../components/tagSelection'
import { useAuth } from '../lib/auth-context'
import { formatNumber } from '../lib/format'
import { useAsync } from '../lib/useAsync'

const PER_PAGE = 25

type SortKey = 'total_size' | 'unique_size'
type SortOrder = 'asc' | 'desc'

function toItems(keys: ReadonlySet<string>) {
  const items: Array<{ repository: string; tag: string }> = []
  for (const key of keys) {
    const separator = key.indexOf(':')
    if (separator <= 0) continue
    items.push({ repository: key.slice(0, separator), tag: key.slice(separator + 1) })
  }
  return items
}

export function TagCleanupPage() {
  const { user, namespaces } = useAuth()
  const [sort, setSort] = useState<SortKey>('total_size')
  const [order, setOrder] = useState<SortOrder>('desc')
  const [namespace, setNamespace] = useState('')
  const [page, setPage] = useState(1)
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set())
  const [deleting, setDeleting] = useState(false)
  const [result, setResult] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const state = useAsync(
    (signal) =>
      tagsApi.ranked(sort, order, namespace || undefined, page, PER_PAGE, { signal }),
    `tags:${sort}:${order}:${namespace}:${page}`,
  )

  const items = state.data?.items ?? []
  const allSelected =
    items.length > 0 && items.every((entry) => selected.has(tagSelectionKey(entry)))

  const changeSort = (next: SortKey) => {
    if (next === sort) {
      setOrder((previous) => (previous === 'desc' ? 'asc' : 'desc'))
    } else {
      setSort(next)
      setOrder('desc')
    }
    setPage(1)
  }

  const toggle = (key: string) => {
    setSelected((previous) => {
      const next = new Set(previous)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }

  const toggleAll = () => {
    setSelected((previous) => {
      const next = new Set(previous)
      if (allSelected) {
        for (const entry of items) next.delete(tagSelectionKey(entry))
      } else {
        for (const entry of items) next.add(tagSelectionKey(entry))
      }
      return next
    })
  }

  const deleteSelected = () => {
    setActionError(null)
    setResult(null)
    setDeleting(true)
    void tagsApi.batchDelete(toItems(selected)).then(
      (response) => {
        setDeleting(false)
        setSelected(new Set())
        setResult(
          `Deleted ${response.deleted} tag${response.deleted === 1 ? '' : 's'}.`,
        )
        state.reload()
      },
      (error: unknown) => {
        setDeleting(false)
        setActionError(
          isApiError(error) ? error.message : 'The tags could not be deleted.',
        )
      },
    )
  }

  return (
    <div className="space-y-4">
      <PageHeader title="Tags by storage" />

      {namespaces.length > 1 ? (
        <div className="flex flex-wrap items-end gap-3">
          <div className="w-56">
            <SelectField
              label="Namespace"
              value={namespace}
              onChange={(event) => {
                setNamespace(event.target.value)
                setPage(1)
              }}
              options={[
                { value: '', label: 'All namespaces' },
                ...namespaces.map((item) => ({ value: item.name, label: item.name })),
              ]}
            />
          </div>
        </div>
      ) : null}

      {result ? (
        <Flash variant="success" onDismiss={() => setResult(null)}>
          {result}
        </Flash>
      ) : null}
      {actionError ? (
        <Flash variant="danger" onDismiss={() => setActionError(null)}>
          {actionError}
        </Flash>
      ) : null}

      {selected.size > 0 ? (
        <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-border bg-canvas-subtle px-3 py-2">
          <span className="text-sm" aria-live="polite">
            {formatNumber(selected.size)} tag
            {selected.size === 1 ? '' : 's'} selected
          </span>
          <span className="flex items-center gap-2">
            <button
              type="button"
              className="text-xs text-accent hover:underline"
              onClick={() => setSelected(new Set())}
            >
              Clear selection
            </button>
            <ConfirmAction
              label="Delete selected"
              confirmLabel={`Delete ${selected.size} tag${selected.size === 1 ? '' : 's'}`}
              resourceName={`${selected.size} selected tags`}
              pending={deleting}
              disabled={!user}
              onConfirm={deleteSelected}
            />
          </span>
        </div>
      ) : null}

      <Box>
        {state.loading && !state.data ? (
          <LoadingState label="Loading tags…" />
        ) : null}
        {state.error ? (
          <ErrorState error={state.error} onRetry={state.reload} />
        ) : null}
        {state.data ? (
          items.length === 0 ? (
            <EmptyState
              title="No tags found"
              description="Push an image or widen the namespace filter to see tags here."
              icon={<TagIcon size={24} aria-hidden="true" />}
            />
          ) : (
            <>
              <TagSizeTable
                entries={items}
                selected={selected}
                onToggle={toggle}
                onToggleAll={toggleAll}
                allSelected={allSelected}
                sort={sort}
                order={order}
                onSort={changeSort}
              />
              <div className="px-4">
                <Pagination
                  page={state.data.page}
                  perPage={state.data.per_page}
                  total={state.data.total}
                  onPageChange={setPage}
                />
              </div>
            </>
          )
        ) : null}
      </Box>
    </div>
  )
}
