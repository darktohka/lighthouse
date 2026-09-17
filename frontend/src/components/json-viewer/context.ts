import { createContext, useContext } from 'react'

import type { NodeRow } from './types'

export type JsonViewerContextValue = {
  query: string
  wrap: boolean
  copiedAction: string | null
  activeMatchPath: string | null
  focusTargetPath: string | null
  togglePath: (path: string) => void
  copyNodeValue: (row: NodeRow) => void
  copyNodePath: (path: string) => void
  registerRowRef: (path: string, element: HTMLDivElement | null) => void
  onRowClick: (path: string, element: HTMLDivElement) => void
  onRowFocus: (path: string) => void
}

export const JsonViewerContext = createContext<JsonViewerContextValue | null>(
  null,
)

export function useJsonViewerContext(): JsonViewerContextValue {
  const context = useContext(JsonViewerContext)
  if (!context) {
    throw new Error('useJsonViewerContext must be used within a JsonViewer')
  }
  return context
}
