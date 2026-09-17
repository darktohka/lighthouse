/** Human-readable formatters shared by tables, timelines and badges. */

/** Formats a byte count using decimal units (KB/MB/GB), as registries do. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—'
  if (bytes < 1000) return `${bytes} B`
  const units = ['KB', 'MB', 'GB', 'TB', 'PB']
  let value = bytes / 1000
  let index = 0
  while (value >= 1000 && index < units.length - 1) {
    value /= 1000
    index += 1
  }
  const digits = value >= 100 ? 0 : 1
  return `${value.toFixed(digits)} ${units[index]}`
}

export type ModeEntryKind = 'file' | 'dir' | 'symlink'

const MODE_TYPE: Record<ModeEntryKind, string> = {
  dir: 'd',
  symlink: 'l',
  file: '-',
}

/** Parses POSIX mode bits into the symbolic `ls -l` form, e.g. `drwxr-xr-x`. */
export function formatMode(mode: number, kind: ModeEntryKind): string {
  const bits = Math.trunc(mode) & 0o7777
  const shifts = [6, 3, 0] as const
  const specialBits = [0o4000, 0o2000, 0o1000] as const
  const specialChars = ['s', 's', 't'] as const

  let result = MODE_TYPE[kind]
  shifts.forEach((shift, index) => {
    const permission = (bits >> shift) & 0o7
    const executable = (permission & 0o1) !== 0
    result += permission & 0o4 ? 'r' : '-'
    result += permission & 0o2 ? 'w' : '-'
    if (bits & specialBits[index]) {
      result += executable
        ? specialChars[index]
        : specialChars[index].toUpperCase()
    } else {
      result += executable ? 'x' : '-'
    }
  })
  return result
}

/** Four-digit octal mode for `chmod`, e.g. `0644` or `4755`. */
export function formatModeOctal(mode: number): string {
  return (Math.trunc(mode) & 0o7777).toString(8).padStart(4, '0')
}

export function formatNumber(value: number): string {
  if (!Number.isFinite(value)) return '—'
  return value.toLocaleString()
}

export function formatDateTime(value: string): string {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return date.toLocaleString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

const RELATIVE_UNITS: ReadonlyArray<[Intl.RelativeTimeFormatUnit, number]> = [
  ['year', 31_536_000],
  ['month', 2_592_000],
  ['week', 604_800],
  ['day', 86_400],
  ['hour', 3_600],
  ['minute', 60],
  ['second', 1],
]

export function formatRelativeTime(value: string): string {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  const diffSeconds = Math.round((date.getTime() - Date.now()) / 1000)
  const magnitude = Math.abs(diffSeconds)
  const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' })
  for (const [unit, seconds] of RELATIVE_UNITS) {
    if (magnitude >= seconds || unit === 'second') {
      return formatter.format(Math.round(diffSeconds / seconds), unit)
    }
  }
  return value
}

/** `sha256:0123456789ab…` — enough to identify a digest without the noise. */
export function shortDigest(digest: string): string {
  const separator = digest.indexOf(':')
  if (separator < 0) return digest
  const algorithm = digest.slice(0, separator)
  const hex = digest.slice(separator + 1)
  return `${algorithm}:${hex.slice(0, 12)}`
}

export function formatPlatform(
  os: string,
  architecture: string,
  variant?: string | null,
): string {
  return variant ? `${os}/${architecture}/${variant}` : `${os}/${architecture}`
}

/** Fallback initials for the avatar when no image URL is available. */
export function initials(name: string): string {
  const cleaned = name.trim()
  if (cleaned.length === 0) return '?'
  const parts = cleaned.split(/[\s-_]+/).filter(Boolean)
  if (parts.length >= 2) {
    return `${parts[0][0]}${parts[1][0]}`.toUpperCase()
  }
  return cleaned.slice(0, 2).toUpperCase()
}
