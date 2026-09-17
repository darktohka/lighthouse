import { cx } from './cx'

export type ButtonVariant = 'default' | 'primary' | 'danger'
export type ButtonSize = 'sm' | 'md'
export type LabelVariant =
  | 'default'
  | 'accent'
  | 'success'
  | 'attention'
  | 'danger'
  | 'muted'

const BUTTON_BASE =
  'inline-flex select-none items-center justify-center gap-1.5 whitespace-nowrap rounded-md border font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-60'

const BUTTON_SIZES: Record<ButtonSize, string> = {
  sm: 'h-7 px-2.5 text-xs',
  md: 'h-8 px-3 text-sm',
}

const BUTTON_VARIANTS: Record<ButtonVariant, string> = {
  default:
    'border-border bg-canvas-subtle text-foreground hover:bg-neutral-subtle active:bg-neutral-muted',
  primary:
    'border-accent-emphasis bg-accent-emphasis text-on-emphasis hover:border-accent hover:bg-accent',
  danger:
    'border-border bg-canvas-subtle text-danger hover:border-danger-emphasis hover:bg-danger-emphasis hover:text-on-emphasis',
}

export function buttonClasses(
  variant: ButtonVariant = 'default',
  size: ButtonSize = 'md',
  extra?: string,
): string {
  return cx(BUTTON_BASE, BUTTON_SIZES[size], BUTTON_VARIANTS[variant], extra)
}

export function inputClasses(extra?: string): string {
  return cx(
    'block w-full rounded-md border border-border bg-canvas-default px-2.5 py-1.5 text-sm text-foreground transition-colors placeholder:text-muted focus:border-accent focus:outline-none disabled:cursor-not-allowed disabled:opacity-60',
    extra,
  )
}

const LABEL_VARIANTS: Record<LabelVariant, string> = {
  default: 'border-border bg-canvas-subtle text-foreground',
  accent: 'border-accent bg-accent-subtle text-accent',
  success: 'border-success bg-success-subtle text-success',
  attention: 'border-attention bg-attention-subtle text-attention',
  danger: 'border-danger bg-danger-subtle text-danger',
  muted: 'border-border bg-canvas-subtle text-muted',
}

export function labelClasses(
  variant: LabelVariant = 'default',
  extra?: string,
): string {
  return cx(
    'inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium leading-4',
    LABEL_VARIANTS[variant],
    extra,
  )
}

export function alignClasses(
  align: 'left' | 'right' | 'center' | undefined,
): string {
  if (align === 'right') return 'text-right'
  if (align === 'center') return 'text-center'
  return 'text-left'
}
