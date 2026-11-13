import { ContainerIcon, ShieldCheckIcon } from '@primer/octicons-react'

import type { Platform } from '../api/schemas'
import { formatBytes, formatPlatform } from '../lib/format'
import { Label } from './primitives/Label'

export type PlatformBadgeProps = {
  platform: Platform
  /**
   * Short in-toto predicate labels attached to this platform, e.g.
   * `['provenance', 'sbom']`. Rendered as a `(provenance, sbom)` suffix so the
   * badge reads `linux/arm64 (provenance)`.
   */
  kinds?: readonly string[]
}

export function PlatformBadge({ platform, kinds = [] }: PlatformBadgeProps) {
  const label = formatPlatform(
    platform.os,
    platform.architecture,
    platform.variant,
  )
  const suffix = kinds.length > 0 ? ` (${kinds.join(', ')})` : ''
  return (
    <Label
      variant="muted"
      title={`${label} · ${formatBytes(platform.size)}${
        suffix.length > 0 ? ` · in-toto ${kinds.join(', ')}` : ''
      }`}
      className="font-mono"
    >
      <ContainerIcon size={12} aria-hidden="true" />
      {label}
      {suffix.length > 0 ? (
        <span className="inline-flex items-center gap-0.5 text-muted">
          <ShieldCheckIcon size={10} aria-hidden="true" />
          {suffix}
        </span>
      ) : null}
    </Label>
  )
}

export type PlatformBadgesProps = {
  platforms: readonly Platform[]
  limit?: number
  className?: string
  /**
   * Attestation labels per platform digest. When a platform has attestations the
   * badge gains a `(provenance)`-style suffix.
   */
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
