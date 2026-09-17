import type { ReactNode } from 'react'

import { cx } from '../../lib/cx'

export type TimelineProps = {
  children: ReactNode
  className?: string
}

/** Sequential activity feed with a continuous vertical rail. */
export function Timeline({ children, className }: TimelineProps) {
  return (
    <div className={cx('relative', className)}>
      <span
        aria-hidden="true"
        className="absolute bottom-2 left-[15px] top-2 w-px bg-border"
      />
      <ol className="space-y-4">{children}</ol>
    </div>
  )
}

export type TimelineItemProps = {
  children: ReactNode
  avatar?: ReactNode
  icon?: ReactNode
  className?: string
}

export function TimelineItem({
  children,
  avatar,
  icon,
  className,
}: TimelineItemProps) {
  return (
    <li className={cx('relative flex gap-3', className)}>
      <span className="relative z-10 flex h-8 w-8 shrink-0 items-center justify-center overflow-hidden rounded-full border border-border bg-canvas-subtle text-muted">
        {avatar ?? icon}
      </span>
      <div className="min-w-0 flex-1 pt-1">{children}</div>
    </li>
  )
}
