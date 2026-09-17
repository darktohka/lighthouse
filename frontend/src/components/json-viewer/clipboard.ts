export function writeClipboard(text: string): Promise<boolean> {
  if (typeof navigator === 'undefined' || !navigator.clipboard) {
    return Promise.resolve(false)
  }
  return navigator.clipboard.writeText(text).then(
    () => true,
    () => false,
  )
}
