/**
 * Byte-level content sniffing for layer file previews.
 *
 * The backend labels a file by guessing from its path, so extension-less files
 * such as `README` arrive as `application/octet-stream` even when they hold
 * plain text. The preview inspects the bytes instead of trusting that label.
 */

const SNIFF_BYTES = 8192
const MAX_UNPRINTABLE_RATIO = 0.1

export function decodeText(bytes: Uint8Array): string {
  return new TextDecoder('utf-8', { fatal: false }).decode(bytes)
}

export function isProbablyBinary(bytes: Uint8Array): boolean {
  if (bytes.length === 0) return false

  const text = decodeText(bytes.subarray(0, SNIFF_BYTES))
  if (text.length === 0) return false
  if (text.includes('\u0000')) return true

  let unprintable = 0
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0
    if (code < 0x20 && code !== 0x09 && code !== 0x0a && code !== 0x0d) {
      unprintable += 1
    }
  }

  return unprintable / text.length > MAX_UNPRINTABLE_RATIO
}
