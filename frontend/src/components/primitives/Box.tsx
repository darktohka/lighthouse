import type { HTMLAttributes } from 'react'

import { cx } from '../../lib/cx'

/** The fundamental Primer layout unit: 1px border, 6px radius, no shadow. */
export function Box({
  className,
  children,
  ...rest
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cx(
        'overflow-hidden rounded-md border border-border bg-canvas-default',
        className,
      )}
      {...rest}
    >
      {children}
    </div>
  )
}

export function BoxHeader({
  className,
  children,
  ...rest
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cx(
        'flex flex-wrap items-center justify-between gap-2 border-b border-border bg-canvas-subtle px-4 py-2',
        className,
      )}
      {...rest}
    >
      {children}
    </div>
  )
}

export function BoxBody({
  className,
  children,
  ...rest
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={cx('p-4', className)} {...rest}>
      {children}
    </div>
  )
}
