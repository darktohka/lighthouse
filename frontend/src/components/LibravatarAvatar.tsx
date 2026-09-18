import { libravatarUrlFromHash } from './libravatar'
import { type AvatarProps, Avatar } from './primitives/Avatar'

export type LibravatarAvatarProps = Omit<AvatarProps, 'src'> & {
  src?: string | null
  hash?: string | null
}

export function LibravatarAvatar({
  src,
  hash,
  name,
  size = 20,
  className,
}: LibravatarAvatarProps) {
  const resolved =
    src ?? (hash ? libravatarUrlFromHash(hash, Math.max(40, size * 2)) : null)

  return <Avatar src={resolved} name={name} size={size} className={className} />
}
