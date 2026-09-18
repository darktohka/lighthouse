import { useState } from 'react'

import { cx } from '../../lib/cx'
import { initials } from '../../lib/format'

export type AvatarProps = {
  /** Resolved avatar URL from the API (`avatar_url`). */
  src?: string | null
  /** Display name used for the fallback initials and accessible label. */
  name: string
  /** Pixel size for both width and height. */
  size?: number
  className?: string
}

/**
 * Circular avatar with an initials fallback.
 *
 * Callers pass an already-resolved URL (an explicit `avatar_url` override or a
 * Libravatar URL built from `avatar_hash`); initials are shown only when no URL
 * is available, or as the last resort when the image fails to load.
 */
export function Avatar({ src, name, size = 20, className }: AvatarProps) {
  const [failedSrc, setFailedSrc] = useState<string | null>(null)
  const showImage =
    typeof src === 'string' && src.length > 0 && failedSrc !== src

  return (
    <span
      className={cx(
        'inline-flex shrink-0 items-center justify-center overflow-hidden rounded-full border border-border bg-accent-subtle font-medium text-accent',
        className,
      )}
      style={{ width: size, height: size, fontSize: Math.max(9, size * 0.4) }}
      title={name}
    >
      {showImage ? (
        <img
          src={src}
          alt=""
          width={size}
          height={size}
          loading="lazy"
          className="h-full w-full object-cover"
          onError={() => setFailedSrc(src ?? null)}
        />
      ) : (
        <span aria-hidden="true">{initials(name)}</span>
      )}
      <span className="sr-only">{name}</span>
    </span>
  )
}
