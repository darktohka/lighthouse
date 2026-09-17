import { ContainerIcon } from '@primer/octicons-react'

import type { Platform } from '../api/schemas'
import { formatBytes, formatPlatform } from '../lib/format'
import { Label } from './primitives/Label'

export type PlatformBadgeProps = {
  platform: Platform
}

export function PlatformBadge({ platform }: PlatformBadgeProps) {
  const label = formatPlatform(
    platform.os,
    platform.architecture,
    platform.variant,
  )
  return (
    <Label
      variant="muted"
      title={`${label} · ${formatBytes(platform.size)}`}
      className="font-mono"
    >
      <ContainerIcon size={12} aria-hidden="true" />
      {label}
    </Label>
  )
}

export type PlatformBadgesProps = {
  platforms: readonly Platform[]
  limit?: number
  className?: string
}

export function PlatformBadges({
  platforms,
  limit = 4,
  className,
}: PlatformBadgesProps) {
  if (platforms.length === 0) {
    return <span className="text-xs text-muted">—</span>
  }
  const visible = platforms.slice(0, limit)
  const hidden = platforms.length - visible.length
  return (
    <span className={className}>
      <span className="flex flex-wrap items-center gap-1">
        {visible.map((platform) => (
          <PlatformBadge
            key={`${platform.os}/${platform.architecture}/${platform.variant ?? ''}/${platform.digest}`}
            platform={platform}
          />
        ))}
        {hidden > 0 ? (
          <Label variant="muted" title={`${hidden} more platforms`}>
            +{hidden}
          </Label>
        ) : null}
      </span>
    </span>
  )
}
