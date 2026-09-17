import { useEffect, useRef, useState } from 'react'

import { Button } from './primitives/Button'

export type ConfirmActionProps = {
  label: string
  confirmLabel: string
  resourceName: string
  onConfirm: () => void | Promise<void>
  pending?: boolean
  disabled?: boolean
  size?: 'sm' | 'md'
  className?: string
}

export function ConfirmAction({
  label,
  confirmLabel,
  resourceName,
  onConfirm,
  pending,
  disabled,
  size = 'sm',
  className,
}: ConfirmActionProps) {
  const [confirming, setConfirming] = useState(false)
  const wrapperRef = useRef<HTMLSpanElement>(null)
  const restoreFocus = useRef(false)

  useEffect(() => {
    if (confirming) {
      wrapperRef.current?.querySelector('button')?.focus()
    } else if (restoreFocus.current) {
      restoreFocus.current = false
      wrapperRef.current?.querySelector('button')?.focus()
    }
  }, [confirming])

  return (
    <span ref={wrapperRef} className="inline-flex items-center gap-1">
      {confirming ? (
        <>
          <Button
            size={size}
            variant="danger"
            disabled={pending}
            onClick={() => void onConfirm()}
          >
            {pending ? 'Working…' : confirmLabel}
          </Button>
          <Button
            size={size}
            disabled={pending}
            onClick={() => {
              restoreFocus.current = true
              setConfirming(false)
            }}
          >
            Cancel
          </Button>
        </>
      ) : (
        <Button
          size={size}
          variant="danger"
          disabled={disabled}
          className={className}
          onClick={() => setConfirming(true)}
          aria-label={`${label} ${resourceName}`}
        >
          {label}
        </Button>
      )}
    </span>
  )
}
