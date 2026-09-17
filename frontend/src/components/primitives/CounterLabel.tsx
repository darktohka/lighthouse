import type { ReactNode } from 'react'

import { cx } from '../../lib/cx'

export type CounterLabelProps = {
  children: ReactNode
  className?: string
  title?: string
}

/** Borderless counter pill used beside headings and in tables. */
export function CounterLabel({
  children,
  className,
  title,
}: CounterLabelProps) {
  return (
    <span
      className={cx(
        'inline-flex min-w-5 items-center justify-center rounded-full border border-border bg-canvas-subtle px-2 py-0.5 text-xs font-medium leading-4 text-muted',
        className,
      )}
      title={title}
    >
      {children}
    </span>
  )
}
