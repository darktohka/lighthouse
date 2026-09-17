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
 * The backend already resolves `avatar_url` (Libravatar or an explicit
 * override), so the frontend renders it directly and degrades to initials when
 * it is absent or fails to load.
 */
export function Avatar({ src, name, size = 20, className }: AvatarProps) {
  const [failed, setFailed] = useState(false)
  const showImage = typeof src === 'string' && src.length > 0 && !failed

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
          onError={() => setFailed(true)}
        />
      ) : (
        <span aria-hidden="true">{initials(name)}</span>
      )}
      <span className="sr-only">{name}</span>
    </span>
  )
}
