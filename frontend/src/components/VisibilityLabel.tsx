import { GlobeIcon, LockIcon } from '@primer/octicons-react'

import { Label } from './primitives/Label'

export function VisibilityLabel({ isPublic }: { isPublic: boolean }) {
  if (isPublic) {
    return (
      <Label variant="muted">
        <GlobeIcon size={12} aria-hidden="true" />
        Public
      </Label>
    )
  }
  return (
    <Label variant="muted">
      <LockIcon size={12} aria-hidden="true" />
      Private
    </Label>
  )
}
