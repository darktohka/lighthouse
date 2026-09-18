import { EyeClosedIcon, GlobeIcon, LockIcon } from '@primer/octicons-react'

import { Label } from './primitives/Label'

export function VisibilityLabel({
  isPublic,
  isHidden = false,
}: {
  isPublic: boolean
  isHidden?: boolean
}) {
  if (isHidden) {
    return (
      <Label variant="muted">
        <EyeClosedIcon size={12} aria-hidden="true" />
        Hidden
      </Label>
    )
  }
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
