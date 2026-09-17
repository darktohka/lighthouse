import type { ReactNode } from 'react'

import { cx } from '../lib/cx'

export type PageHeaderProps = {
  title: ReactNode
  description?: ReactNode
  actions?: ReactNode
  breadcrumbs?: ReactNode
  className?: string
}

export function PageHeader({
  title,
  description,
  actions,
  breadcrumbs,
  className,
}: PageHeaderProps) {
  return (
    <div className={cx('mb-4 space-y-2', className)}>
      {breadcrumbs ? (
        <nav aria-label="Breadcrumb" className="text-xs text-muted">
          {breadcrumbs}
        </nav>
      ) : null}
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h1 className="break-words text-xl font-semibold">{title}</h1>
          {description ? (
            <p className="mt-1 text-sm text-muted">{description}</p>
          ) : null}
        </div>
        {actions ? (
          <div className="flex flex-wrap items-center gap-2">{actions}</div>
        ) : null}
      </div>
    </div>
  )
}
