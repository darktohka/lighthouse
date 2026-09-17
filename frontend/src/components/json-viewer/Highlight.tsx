import type { ReactNode } from 'react'

export function Highlight({ text, query }: { text: string; query: string }) {
  const needle = query.trim().toLowerCase()
  if (needle.length === 0) return <>{text}</>

  const haystack = text.toLowerCase()
  const parts: ReactNode[] = []
  let cursor = 0
  let found = haystack.indexOf(needle, cursor)

  while (found !== -1) {
    if (found > cursor) parts.push(text.slice(cursor, found))
    parts.push(
      <mark
        key={`${found}-${cursor}`}
        className="rounded-sm bg-attention-subtle px-0.5 text-attention"
      >
        {text.slice(found, found + needle.length)}
      </mark>,
    )
    cursor = found + needle.length
    found = haystack.indexOf(needle, cursor)
  }

  if (cursor < text.length) parts.push(text.slice(cursor))
  return <>{parts}</>
}
