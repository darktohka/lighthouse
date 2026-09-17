import { useEffect, useState } from 'react'

import { libravatarUrl } from './libravatar'
import { type AvatarProps, Avatar } from './primitives/Avatar'

export type LibravatarAvatarProps = Omit<AvatarProps, 'src'> & {
  src?: string | null
  email?: string | null
}

export function LibravatarAvatar({
  src,
  email,
  name,
  size = 20,
  className,
}: LibravatarAvatarProps) {
  const [derived, setDerived] = useState<{ email: string; url: string } | null>(
    null,
  )

  useEffect(() => {
    if (src || !email) return
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
  }, [src, email, size])

  const resolved =
    src ?? (email && derived && derived.email === email ? derived.url : null)

  return <Avatar src={resolved} name={name} size={size} className={className} />
}
