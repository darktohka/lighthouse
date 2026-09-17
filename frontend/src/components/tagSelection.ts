import type { TagSizeEntry } from '../api/schemas'

export function tagSelectionKey(entry: TagSizeEntry): string {
  return `${entry.repository}:${entry.tag}`
}
