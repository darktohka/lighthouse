import type { ReactNode } from 'react'

import { cx } from '../../lib/cx'
import { alignClasses } from '../../lib/ui'

export type TableColumn<T> = {
  /** Stable identity for React keys. */
  key: string
  header: ReactNode
  render: (row: T) => ReactNode
  align?: 'left' | 'right' | 'center'
  className?: string
  headClassName?: string
}

export type TableProps<T> = {
  columns: ReadonlyArray<TableColumn<T>>
  rows: readonly T[]
  rowKey: (row: T, index: number) => string | number
  /** Visually hidden caption describing the table for screen readers. */
  caption?: string
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
          {rows.map((row, index) => (
            <tr
              key={rowKey(row, index)}
              className="border-b border-border last:border-b-0"
            >
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
          ))}
        </tbody>
      </table>
    </div>
  )
}
