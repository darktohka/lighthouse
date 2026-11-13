import { ContainerIcon, ShieldCheckIcon } from '@primer/octicons-react'

import type { Platform } from '../api/schemas'
import { formatBytes, formatPlatform } from '../lib/format'
import { Label } from './primitives/Label'

export type PlatformBadgeProps = {
  platform: Platform
  /**
   * Short in-toto predicate labels attached to this platform, e.g.
   * `['provenance', 'sbom']`. They are never rendered as text: the badge grows a
   * shield icon, and the labels are exposed through the tooltip and to assistive
   * technology.
   */
  kinds?: readonly string[]
}

export function PlatformBadge({ platform, kinds = [] }: PlatformBadgeProps) {
  const label = formatPlatform(
    platform.os,
    platform.architecture,
    platform.variant,
  )
  const attested = kinds.length > 0
  return (
    <Label
      variant="muted"
      title={`${label} · ${formatBytes(platform.size)}${
        attested ? ` · ${kinds.join(', ')}` : ''
      }`}
      className="font-mono"
    >
      <ContainerIcon size={12} aria-hidden="true" />
      {label}
      {attested ? (
        <>
          <ShieldCheckIcon size={12} aria-hidden="true" className="text-success" />
          <span className="sr-only">{`, ${kinds.join(', ')}`}</span>
        </>
      ) : null}
    </Label>
  )
}

export type PlatformBadgesProps = {
  platforms: readonly Platform[]
  limit?: number
  className?: string
  /** Attestation labels per platform digest; the badge shows a shield when set. */
  kindsByDigest?: ReadonlyMap<string, readonly string[]>
}

export function PlatformBadges({
  platforms,
  limit = 4,
  className,
  kindsByDigest,
}: PlatformBadgesProps) {
  if (platforms.length === 0) {
    return <span className="text-xs text-muted">-</span>
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
            kinds={kindsByDigest?.get(platform.digest)}
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
