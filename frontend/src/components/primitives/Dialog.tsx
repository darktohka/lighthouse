import { XIcon } from '@primer/octicons-react'
import { useEffect, useId, useRef, type ReactNode } from 'react'
import { createPortal } from 'react-dom'

import { cx } from '../../lib/cx'
import { Button } from './Button'

export type DialogProps = {
  open: boolean
  /** Rendered as the accessible name via `aria-labelledby`. */
  title: ReactNode
  /** Short line under the title; linked with `aria-describedby`. */
  description?: ReactNode
  onClose: () => void
  children: ReactNode
  /** Action row pinned under the body (usually Cancel + submit). */
  footer?: ReactNode
  /** Tailwind max-width utility for the panel. Defaults to `max-w-2xl`. */
  widthClassName?: string
}

/**
 * Modal dialog (Primer-flavoured, portal-rendered).
 *
 * Behaviour kept deliberately small but correct: Escape closes, a backdrop
 * click closes, focus moves into the panel on open, Tab cycles inside it, and
 * the page behind cannot scroll. Colours come from the `--lh-*` tokens, so it
 * follows the active light/dark theme like every other surface.
 */
export function Dialog({
  open,
  title,
  description,
  onClose,
  children,
  footer,
  widthClassName = 'max-w-2xl',
}: DialogProps) {
  const panelRef = useRef<HTMLDivElement>(null)
  const titleId = useId()
  const descriptionId = useId()
  const onCloseRef = useRef(onClose)
  useEffect(() => {
    onCloseRef.current = onClose
  }, [onClose])

  // Escape to close + Tab containment while the dialog is mounted.
  useEffect(() => {
    if (!open) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        onCloseRef.current()
        return
      }
      if (event.key !== 'Tab') return
      const panel = panelRef.current
      if (!panel) return
      const focusable = panel.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      )
      if (focusable.length === 0) return
      const first = focusable[0]
      const last = focusable[focusable.length - 1]
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault()
        first.focus()
      }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [open])

  // Body scroll lock + initial focus, restoring both on close.
  useEffect(() => {
    if (!open) return
    const { body } = document
    const previousOverflow = body.style.overflow
    const previousPaddingRight = body.style.paddingRight
    const scrollbar = window.innerWidth - document.documentElement.clientWidth
    body.style.overflow = 'hidden'
    if (scrollbar > 0) body.style.paddingRight = `${scrollbar}px`

    const panel = panelRef.current
    const target =
      panel?.querySelector<HTMLElement>(
        'input:not([disabled]), select:not([disabled]), textarea:not([disabled])',
      ) ?? panel
    target?.focus()

    return () => {
      body.style.overflow = previousOverflow
      body.style.paddingRight = previousPaddingRight
    }
  }, [open])

  if (!open) return null

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto p-4 sm:items-center sm:p-6"
      // Backdrop click closes, but only when the click starts and ends outside
      // the panel (a drag that starts inside must not dismiss the dialog).
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose()
      }}
    >
      <div
        aria-hidden="true"
        className="pointer-events-none fixed inset-0 bg-neutral-muted/60 backdrop-blur-[1px]"
      />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description ? descriptionId : undefined}
        tabIndex={-1}
        className={cx(
          'relative my-auto w-full rounded-lg border border-border bg-canvas-overlay shadow-medium outline-none',
          widthClassName,
        )}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-3 border-b border-border px-4 py-3">
          <div className="min-w-0">
            <h2 id={titleId} className="text-base font-semibold">
              {title}
            </h2>
            {description ? (
              <p id={descriptionId} className="mt-0.5 text-xs text-muted">
                {description}
              </p>
            ) : null}
          </div>
          <Button
            size="sm"
            aria-label="Close dialog"
            onClick={onClose}
            className="shrink-0 border-transparent bg-transparent hover:bg-neutral-subtle"
          >
            <XIcon size={16} aria-hidden="true" />
          </Button>
        </div>

        <div className="max-h-[min(72svh,42rem)] overflow-y-auto px-4 py-3 scrollbar-thin">
          {children}
        </div>

        {footer ? (
          <div className="flex flex-wrap items-center justify-end gap-2 border-t border-border bg-canvas-subtle px-4 py-3">
            {footer}
          </div>
        ) : null}
      </div>
    </div>,
    document.body,
  )
}
