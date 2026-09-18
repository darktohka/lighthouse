import { useEffect, useState } from 'react'

import { libravatarUrl, libravatarUrlFromHash } from './libravatar'
import { type AvatarProps, Avatar } from './primitives/Avatar'

export type LibravatarAvatarProps = Omit<AvatarProps, 'src'> & {
  src?: string | null
  hash?: string | null
  email?: string | null
}

export function LibravatarAvatar({
  src,
  hash,
  email,
  name,
  size = 20,
  className,
}: LibravatarAvatarProps) {
  const [derived, setDerived] = useState<{ email: string; url: string } | null>(
    null,
  )

  useEffect(() => {
    if (src || hash || !email) return
    let active = true
    void libravatarUrl(email, Math.max(40, size * 2)).then(
      (url) => {
        if (active) setDerived({ email, url })
      },
      () => undefined,
    )
    return () => {
      active = false
    }
  }, [src, hash, email, size])

  const resolved =
    src ??
    (hash ? libravatarUrlFromHash(hash, Math.max(40, size * 2)) : null) ??
    (email && derived && derived.email === email ? derived.url : null)

  return <Avatar src={resolved} name={name} size={size} className={className} />
}
