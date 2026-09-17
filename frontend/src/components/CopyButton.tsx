import { CheckIcon, CopyIcon } from '@primer/octicons-react'
import { useState } from 'react'

import { Button } from './primitives/Button'

export type CopyButtonProps = {
  value: string
  label?: string
  ariaLabel?: string
  className?: string
}

export function CopyButton({
  value,
  label = 'Copy',
  ariaLabel,
  className,
}: CopyButtonProps) {
  const [copied, setCopied] = useState(false)

  const onCopy = () => {
    if (typeof navigator === 'undefined' || !navigator.clipboard) return
    void navigator.clipboard.writeText(value).then(
      () => {
        setCopied(true)
        window.setTimeout(() => setCopied(false), 1500)
      },
      () => setCopied(false),
    )
  }

  return (
    <Button
      size="sm"
      onClick={onCopy}
      className={className}
      aria-label={ariaLabel ?? label}
      leadingIcon={
        copied ? (
          <CheckIcon size={14} aria-hidden="true" />
        ) : (
          <CopyIcon size={14} aria-hidden="true" />
        )
      }
    >
      {copied ? 'Copied' : label}
    </Button>
  )
}
