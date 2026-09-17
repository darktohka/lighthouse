import { ChevronDownIcon, ChevronUpIcon } from '@primer/octicons-react'
import { Fragment, type ReactNode } from 'react'

import { cx } from '../../lib/cx'
import { alignClasses } from '../../lib/ui'

export type SortDirection = 'asc' | 'desc'

export type TableColumn<T> = {
  /** Stable identity for React keys. */
  key: string
  header: ReactNode
  render: (row: T) => ReactNode
  align?: 'left' | 'right' | 'center'
  className?: string
  headClassName?: string
  /** When set, announces the column's sort direction via aria-sort. */
  sortDirection?: SortDirection
}

export type TableProps<T> = {
  columns: ReadonlyArray<TableColumn<T>>
  rows: readonly T[]
  rowKey: (row: T, index: number) => string | number
  /** Visually hidden caption describing the table for screen readers. */
  caption?: string
  /** Rendered in a full-width body row when `rows` is empty. */
  empty?: ReactNode
  /** Extra content rendered in a full-width row immediately after `row`. */
  rowDetails?: (row: T, index: number) => ReactNode
  className?: string
}

/**
 * Dense bordered data table. Wrapped in a horizontal scroll container so it
 * stays usable down to ~768px.
 */
export function Table<T>({
  columns,
  rows,
  rowKey,
  caption,
  empty,
  rowDetails,
  className,
}: TableProps<T>) {
  return (
    <div className={cx('w-full overflow-x-auto', className)}>
      <table className="w-full border-collapse text-sm">
        {caption ? <caption className="sr-only">{caption}</caption> : null}
        <thead>
          <tr className="bg-canvas-subtle">
            {columns.map((column) => (
              <th
                key={column.key}
                scope="col"
                aria-sort={
                  column.sortDirection
                    ? column.sortDirection === 'asc'
                      ? 'ascending'
                      : 'descending'
                    : undefined
                }
                className={cx(
                  'whitespace-nowrap border-b border-border px-3 py-2 font-semibold text-muted',
                  alignClasses(column.align),
                  column.headClassName,
                )}
              >
                {column.header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.length === 0 && empty ? (
            <tr className="border-b border-border last:border-b-0">
              <td colSpan={columns.length}>{empty}</td>
            </tr>
          ) : null}
          {rows.map((row, index) => {
            const details = rowDetails?.(row, index)
            return (
              <Fragment key={rowKey(row, index)}>
                <tr className="border-b border-border last:border-b-0">
                  {columns.map((column) => (
                    <td
                      key={column.key}
                      className={cx(
                        'px-3 py-2 align-middle',
                        alignClasses(column.align),
                        column.className,
                      )}
                    >
                      {column.render(row)}
                    </td>
                  ))}
                </tr>
                {details ? (
                  <tr className="border-b border-border last:border-b-0">
                    <td
                      colSpan={columns.length}
                      className="bg-canvas-subtle px-3 py-3 align-top"
                    >
                      {details}
                    </td>
                  </tr>
                ) : null}
              </Fragment>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

export function TableSortHeader({
  label,
  active,
  direction,
  onSort,
}: {
  label: string
  active: boolean
  direction: SortDirection
  onSort: () => void
}) {
  return (
    <button
      type="button"
      onClick={onSort}
      className="inline-flex cursor-pointer items-center gap-1 font-semibold hover:text-foreground"
    >
      {label}
      {active ? (
        direction === 'desc' ? (
          <ChevronDownIcon size={12} aria-hidden="true" />
        ) : (
          <ChevronUpIcon size={12} aria-hidden="true" />
        )
      ) : null}
    </button>
  )
}
