import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { cx } from '../../lib/cx'
import { JsonNodeRow } from './JsonNodeRow'
import { JsonToolbar } from './JsonToolbar'
import { writeClipboard } from './clipboard'
import { COPIED_RESET_MS } from './constants'
import { JsonViewerContext, type JsonViewerContextValue } from './context'
import {
  buildRows,
  clipboardText,
  collectDefaultExpanded,
  collectExpandablePaths,
  displayPath,
  safeStringify,
} from './json-model'
import { useJsonSearch } from './use-json-search'
import { useTreeNavigation } from './use-tree-navigation'
import type { ExpandedState, JsonViewerProps, NodeRow } from './types'

/**
 * Read-only JSON viewer with syntax highlighting, collapsible objects/arrays,
 * search, per-node copy actions and ARIA tree keyboard navigation. Deliberately
 * dependency-free.
 */
export function JsonViewer({
  data,
  defaultExpandDepth = 2,
  className,
  copyValue,
  downloadFileName = 'data.json',
}: JsonViewerProps) {
  const [expandedState, setExpandedState] = useState<ExpandedState>(() => ({
    token: data,
    depth: defaultExpandDepth,
    paths: collectDefaultExpanded(data, defaultExpandDepth),
  }))
  const [copiedAction, setCopiedAction] = useState<string | null>(null)
  const [focusedPath, setFocusedPath] = useState<string | null>(null)
  const [wrap, setWrap] = useState(false)

  const copyTimer = useRef<number | null>(null)
  const rowRefs = useRef(new Map<string, HTMLDivElement>())
  const scrollRef = useRef<HTMLDivElement | null>(null)

  const defaultExpanded = useMemo(
    () => collectDefaultExpanded(data, defaultExpandDepth),
    [data, defaultExpandDepth],
  )

  const allExpandablePaths = useMemo(
    () => collectExpandablePaths(data),
    [data],
  )

  const isCurrentState =
    expandedState.token === data && expandedState.depth === defaultExpandDepth
  const expanded = isCurrentState ? expandedState.paths : defaultExpanded

  const setExpanded = useCallback(
    (updater: (previous: Set<string>) => Set<string>) => {
      setExpandedState((previous) => {
        const base =
          previous.token === data && previous.depth === defaultExpandDepth
            ? previous.paths
            : defaultExpanded
        return {
          token: data,
          depth: defaultExpandDepth,
          paths: updater(base),
        }
      })
    },
    [data, defaultExpandDepth, defaultExpanded],
  )

  const togglePath = useCallback(
    (path: string) => {
      setExpanded((previous) => {
        const next = new Set(previous)
        if (next.has(path)) next.delete(path)
        else next.add(path)
        return next
      })
    },
    [setExpanded],
  )

  const expandAll = useCallback(() => {
    setExpanded(() => new Set(allExpandablePaths))
  }, [setExpanded, allExpandablePaths])

  const collapseAll = useCallback(() => {
    setExpanded(() => new Set())
  }, [setExpanded])

  const rows = useMemo(() => buildRows(data, expanded), [data, expanded])

  const search = useJsonSearch({ data, setExpanded, setFocusedPath })
  const { focusTargetPath, handleTreeKeyDown } = useTreeNavigation({
    rows,
    expanded,
    matches: search.matches,
    activeMatchIndex: search.activeMatchIndex,
    focusedPath,
    setFocusedPath,
    togglePath,
    rowRefs,
    scrollRef,
  })

  useEffect(
    () => () => {
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current)
    },
    [],
  )

  const flashAction = useCallback((action: string) => {
    setCopiedAction(action)
    if (copyTimer.current !== null) window.clearTimeout(copyTimer.current)
    copyTimer.current = window.setTimeout(() => {
      setCopiedAction(null)
      copyTimer.current = null
    }, COPIED_RESET_MS)
  }, [])

  const copyText = useCallback(
    (text: string, action: string) => {
      void writeClipboard(text).then((ok) => {
        if (ok) flashAction(action)
      })
    },
    [flashAction],
  )

  const onCopyFull = () => {
    copyText(copyValue ?? safeStringify(data), 'full')
  }

  const copyNodeValue = (row: NodeRow) => {
    copyText(clipboardText(row.value), `value:${row.path}`)
  }

  const copyNodePath = (path: string) => {
    copyText(displayPath(path), `path:${path}`)
  }

  const onDownload = () => {
    const text = copyValue ?? safeStringify(data)
    const blob = new Blob([text], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = downloadFileName
    document.body.appendChild(anchor)
    anchor.click()
    anchor.remove()
    window.setTimeout(() => URL.revokeObjectURL(url), 0)
  }

  const onToggleWrap = useCallback(() => {
    setWrap((previous) => !previous)
  }, [])

  const onRowFocus = useCallback((path: string) => {
    setFocusedPath(path)
  }, [])

  const onRowClick = useCallback((path: string, element: HTMLDivElement) => {
    setFocusedPath(path)
    element.focus()
  }, [])

  const registerRowRef = useCallback(
    (path: string, element: HTMLDivElement | null) => {
      if (element) rowRefs.current.set(path, element)
      else rowRefs.current.delete(path)
    },
    [],
  )

  const contextValue: JsonViewerContextValue = {
    query: search.query,
    wrap,
    copiedAction,
    activeMatchPath: search.activeMatchPath,
    focusTargetPath,
    togglePath,
    copyNodeValue,
    copyNodePath,
    registerRowRef,
    onRowClick,
    onRowFocus,
  }

  return (
    <JsonViewerContext.Provider value={contextValue}>
      <div
        className={cx(
          'flex flex-col overflow-hidden rounded-md border border-border bg-canvas-inset',
          className,
        )}
      >
        <JsonToolbar
          search={{
            query: search.query,
            matchLabel: search.matchLabel,
            hasMatches: search.matches.length > 0,
            onChange: search.handleSearchChange,
            onKeyDown: search.handleSearchKeyDown,
            onPrevious: () => search.stepMatch(-1),
            onNext: () => search.stepMatch(1),
            onClear: search.clearSearch,
          }}
          wrap={wrap}
          onToggleWrap={onToggleWrap}
          onExpandAll={expandAll}
          onCollapseAll={collapseAll}
          copied={copiedAction === 'full'}
          onCopy={onCopyFull}
          onDownload={onDownload}
        />

        <div
          ref={scrollRef}
          className="relative max-h-[70vh] overflow-auto p-3 font-mono text-xs leading-5 scrollbar-thin"
        >
          <div
            role="tree"
            aria-label="JSON content"
            onKeyDown={handleTreeKeyDown}
            className={wrap ? 'min-w-0' : 'min-w-max'}
          >
            {rows.map((row) => (
              <JsonNodeRow
                key={
                  row.kind === 'close' ? `close:${row.id}` : `node:${row.path}`
                }
                row={row}
              />
            ))}
          </div>
        </div>
      </div>
    </JsonViewerContext.Provider>
  )
}
