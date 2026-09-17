import {
  AlertFillIcon,
  CheckCircleFillIcon,
  InfoIcon,
  XCircleFillIcon,
  XIcon,
} from '@primer/octicons-react'
import type { ReactNode } from 'react'

import { cx } from '../../lib/cx'

export type FlashVariant = 'default' | 'success' | 'warning' | 'danger'

export type FlashProps = {
  variant?: FlashVariant
  title?: ReactNode
  children?: ReactNode
  onDismiss?: () => void
  className?: string
}

const FLASH_STYLES: Record<FlashVariant, string> = {
  default: 'border-accent bg-accent-subtle text-foreground',
  success: 'border-success bg-success-subtle text-foreground',
  warning: 'border-attention bg-attention-subtle text-foreground',
  danger: 'border-danger bg-danger-subtle text-foreground',
}

const FLASH_ICONS: Record<FlashVariant, typeof InfoIcon> = {
  default: InfoIcon,
  success: CheckCircleFillIcon,
  warning: AlertFillIcon,
  danger: XCircleFillIcon,
}

const FLASH_ICON_COLORS: Record<FlashVariant, string> = {
  default: 'text-accent',
  success: 'text-success',
  warning: 'text-attention',
  danger: 'text-danger',
}

/** Primer flash / alert banner. */
export function Flash({
  variant = 'default',
  title,
  children,
  onDismiss,
  className,
}: FlashProps) {
  const Icon = FLASH_ICONS[variant]
  return (
    <div
      role={variant === 'danger' ? 'alert' : 'status'}
      className={cx(
        'flex items-start gap-2 rounded-md border px-3 py-2 text-sm',
        FLASH_STYLES[variant],
        className,
      )}
    >
      <Icon
        size={16}
        aria-hidden="true"
        className={cx('mt-0.5 shrink-0', FLASH_ICON_COLORS[variant])}
      />
      <div className="min-w-0 flex-1">
        {title ? <p className="font-semibold">{title}</p> : null}
        {children ? <div className="break-words">{children}</div> : null}
      </div>
      {onDismiss ? (
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss"
          className="shrink-0 rounded p-0.5 text-muted hover:bg-neutral-subtle hover:text-foreground"
        >
          <XIcon size={16} aria-hidden="true" />
        </button>
      ) : null}
    </div>
  )
}
