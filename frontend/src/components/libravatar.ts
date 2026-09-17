export const LIBRAVATAR_HOME = 'https://www.libravatar.org/'

const LIBRAVATAR_BASE_URL = String(
  import.meta.env.VITE_LIBRAVATAR_BASE_URL ?? 'https://seccdn.libravatar.org',
)

async function hashEmail(email: string): Promise<string> {
  const data = new TextEncoder().encode(email.trim().toLowerCase())
  const digest = await crypto.subtle.digest('SHA-256', data)
  return Array.from(new Uint8Array(digest))
    .map((byte) => byte.toString(16).padStart(2, '0'))
    .join('')
}

export async function libravatarUrl(email: string, size: number): Promise<string> {
  const hash = await hashEmail(email)
  return `${LIBRAVATAR_BASE_URL}/avatar/${hash}?s=${size}&d=retro`
}
