export const LIBRAVATAR_HOME = 'https://www.libravatar.org/'

const LIBRAVATAR_BASE_URL = String(
  import.meta.env.VITE_LIBRAVATAR_BASE_URL ?? 'https://seccdn.libravatar.org',
)

/**
 * Builds a Libravatar URL from a pre-computed lowercase hex SHA-256 hash,
 * letting public profile pages render an avatar without the private e-mail.
 */
export function libravatarUrlFromHash(hash: string, size: number): string {
  return `${LIBRAVATAR_BASE_URL}/avatar/${hash}?s=${size}&d=retro`
}
