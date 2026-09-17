import {
  useCallback,
  useEffect,
  useMemo,
  type KeyboardEvent,
  type RefObject,
} from 'react'

import { isNodeRow } from './json-model'
import type { JsonMatch, Row } from './types'

type UseTreeNavigationOptions = {
  rows: Row[]
  expanded: ReadonlySet<string>
  matches: JsonMatch[]
  activeMatchIndex: number
  focusedPath: string | null
  setFocusedPath: (path: string) => void
  togglePath: (path: string) => void
  rowRefs: RefObject<Map<string, HTMLDivElement>>
  scrollRef: RefObject<HTMLDivElement | null>
}

export function useTreeNavigation({
  rows,
  expanded,
  matches,
  activeMatchIndex,
  focusedPath,
  setFocusedPath,
  togglePath,
  rowRefs,
  scrollRef,
}: UseTreeNavigationOptions) {
  const navigableRows = useMemo(() => rows.filter(isNodeRow), [rows])

  const navIndex = useMemo(() => {
    const index = new Map<string, number>()
    navigableRows.forEach((row, position) => index.set(row.path, position))
    return index
  }, [navigableRows])

  const focusTargetPath =
    focusedPath !== null && navIndex.has(focusedPath)
      ? focusedPath
      : navigableRows.length > 0
        ? navigableRows[0].path
        : null

  useEffect(() => {
    if (matches.length === 0) return
    const match = matches[Math.min(activeMatchIndex, matches.length - 1)]
    if (!match) return
    const container = scrollRef.current
    const element = rowRefs.current.get(match.path)
    if (!container || !element) return
    const top = element.offsetTop
    const bottom = top + element.offsetHeight
    if (top < container.scrollTop) {
      container.scrollTop = top
    } else if (bottom > container.scrollTop + container.clientHeight) {
      container.scrollTop = bottom - container.clientHeight
    }
  }, [activeMatchIndex, matches, expanded, rowRefs, scrollRef])

  const focusRowAt = useCallback(
    (index: number) => {
      if (navigableRows.length === 0) return
      const clamped = Math.max(0, Math.min(index, navigableRows.length - 1))
      const target = navigableRows[clamped]
      if (!target) return
      setFocusedPath(target.path)
      const element = rowRefs.current.get(target.path)
      if (element) element.focus()
    },
    [navigableRows, setFocusedPath, rowRefs],
  )

  const handleTreeKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (navigableRows.length === 0) return
    const currentIndex =
      focusedPath !== null && navIndex.has(focusedPath)
        ? navIndex.get(focusedPath) ?? 0
        : 0
    const row = navigableRows[currentIndex]
    if (!row) return

    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault()
        focusRowAt(currentIndex + 1)
        break
      case 'ArrowUp':
        event.preventDefault()
        focusRowAt(currentIndex - 1)
        break
      case 'ArrowRight':
        event.preventDefault()
        if (row.nodeKind !== 'primitive' && row.childCount > 0) {
          if (row.isExpanded) focusRowAt(currentIndex + 1)
          else togglePath(row.path)
        }
        break
      case 'ArrowLeft':
        event.preventDefault()
        if (row.nodeKind !== 'primitive' && row.isExpanded) {
          togglePath(row.path)
        } else if (row.parentPath !== null && navIndex.has(row.parentPath)) {
          focusRowAt(navIndex.get(row.parentPath) ?? currentIndex)
        }
        break
      case 'Home':
        event.preventDefault()
        focusRowAt(0)
        break
      case 'End':
        event.preventDefault()
        focusRowAt(navigableRows.length - 1)
        break
      case 'Enter':
      case ' ':
        event.preventDefault()
        if (row.nodeKind !== 'primitive') togglePath(row.path)
        break
      default:
        break
    }
  }

  return { focusTargetPath, handleTreeKeyDown }
}
