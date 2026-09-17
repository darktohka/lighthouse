import {
  useCallback,
  useMemo,
  useState,
  type ChangeEvent,
  type KeyboardEvent,
} from 'react'

import { collectMatches } from './json-model'
import type { JsonMatch } from './types'

type UseJsonSearchOptions = {
  data: unknown
  setExpanded: (updater: (previous: Set<string>) => Set<string>) => void
  setFocusedPath: (path: string) => void
}

export function useJsonSearch({
  data,
  setExpanded,
  setFocusedPath,
}: UseJsonSearchOptions) {
  const [query, setQuery] = useState('')
  const [activeMatchIndex, setActiveMatchIndex] = useState(0)

  const matches = useMemo(() => {
    const trimmed = query.trim()
    return trimmed.length === 0 ? [] : collectMatches(data, trimmed)
  }, [data, query])

  const activeMatch =
    matches.length > 0
      ? matches[Math.min(activeMatchIndex, matches.length - 1)]
      : null
  const safeActiveIndex =
    matches.length === 0
      ? 0
      : Math.min(activeMatchIndex, matches.length - 1)
  const activeMatchPath = activeMatch !== null ? activeMatch.path : null

  const revealMatch = useCallback(
    (match: JsonMatch | undefined) => {
      if (!match) return
      setExpanded((previous) => {
        let next: Set<string> | null = null
        for (const ancestor of match.ancestors) {
          if (!previous.has(ancestor)) {
            if (next === null) next = new Set(previous)
            next.add(ancestor)
          }
        }
        return next ?? previous
      })
      setFocusedPath(match.path)
    },
    [setExpanded, setFocusedPath],
  )

  const stepMatch = useCallback(
    (delta: number) => {
      if (matches.length === 0) return
      const nextIndex =
        (safeActiveIndex + delta + matches.length) % matches.length
      setActiveMatchIndex(nextIndex)
      revealMatch(matches[nextIndex])
    },
    [matches, safeActiveIndex, revealMatch],
  )

  const handleSearchChange = (event: ChangeEvent<HTMLInputElement>) => {
    const value = event.target.value
    setQuery(value)
    setActiveMatchIndex(0)
    const trimmed = value.trim()
    if (trimmed.length === 0) return
    revealMatch(collectMatches(data, trimmed)[0])
  }

  const clearSearch = useCallback(() => {
    setQuery('')
    setActiveMatchIndex(0)
  }, [])

  const handleSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault()
      stepMatch(event.shiftKey ? -1 : 1)
      return
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      clearSearch()
    }
  }

  const matchLabel =
    query.trim().length === 0
      ? ''
      : matches.length === 0
        ? 'No matches'
        : `${safeActiveIndex + 1} of ${matches.length}`

  return {
    query,
    matches,
    activeMatchIndex,
    activeMatchPath,
    matchLabel,
    stepMatch,
    handleSearchChange,
    handleSearchKeyDown,
    clearSearch,
  }
}
