import type { ReactNode } from 'react'

import { labelClasses, type LabelVariant } from '../../lib/ui'

export type LabelProps = {
  variant?: LabelVariant
  children: ReactNode
  className?: string
  title?: string
}

/** Small rounded status pill (Primer `Label`). */
export function Label({
  variant = 'default',
  children,
  className,
  title,
}: LabelProps) {
  return (
    <span className={labelClasses(variant, className)} title={title}>
      {children}
    </span>
  )
}
