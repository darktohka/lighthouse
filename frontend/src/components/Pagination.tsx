import { ChevronLeftIcon, ChevronRightIcon } from '@primer/octicons-react'

import { formatNumber } from '../lib/format'
import { Button } from './primitives/Button'

export type PaginationProps = {
  page: number
  perPage: number
  total: number
  onPageChange: (page: number) => void
  className?: string
}

export function Pagination({
  page,
  perPage,
  total,
  onPageChange,
  className,
}: PaginationProps) {
  const pageCount = Math.max(1, Math.ceil(total / Math.max(1, perPage)))
  if (pageCount <= 1) return null

  return (
    <nav
      aria-label="Pagination"
      className={`flex flex-wrap items-center justify-between gap-2 py-3 ${className ?? ''}`}
    >
      <p className="text-xs text-muted">
        Page {page} of {pageCount} · {formatNumber(total)} total
      </p>
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          disabled={page <= 1}
          onClick={() => onPageChange(page - 1)}
          leadingIcon={<ChevronLeftIcon size={14} aria-hidden="true" />}
        >
          Previous
        </Button>
        <Button
          size="sm"
          disabled={page >= pageCount}
          onClick={() => onPageChange(page + 1)}
          trailingIcon={<ChevronRightIcon size={14} aria-hidden="true" />}
        >
          Next
        </Button>
      </div>
    </nav>
  )
}
