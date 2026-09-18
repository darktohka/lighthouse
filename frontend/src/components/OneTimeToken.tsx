import { CheckIcon, KeyIcon } from '@primer/octicons-react'

import { CopyButton } from './CopyButton'
import { Button } from './primitives/Button'
import { Flash } from './primitives/Flash'

export type OneTimeTokenProps = {
  token: string
  title?: string
  onDismiss: () => void
}

export function OneTimeToken({
  token,
  title = 'Copy your new token now',
  onDismiss,
}: OneTimeTokenProps) {
  return (
    <Flash variant="warning" title={title}>
      <div className="space-y-2">
        <p>
          This token is shown <strong>once</strong>. Store it somewhere safe - it
          cannot be retrieved again.
        </p>
        <div className="flex flex-col gap-2 rounded-md border border-attention bg-canvas-default p-2 sm:flex-row sm:items-center sm:justify-between">
          <code className="flex items-center gap-2 break-all font-mono text-xs">
            <KeyIcon size={14} aria-hidden="true" />
            {token}
          </code>
          <CopyButton value={token} label="Copy token" />
        </div>
        <Button
          size="sm"
          variant="primary"
          leadingIcon={<CheckIcon size={14} aria-hidden="true" />}
          onClick={onDismiss}
        >
          I have saved it
        </Button>
      </div>
    </Flash>
  )
}
