import type { ReactNode } from 'react'

export type JsonValue = unknown

export type JsonViewerNode = ReactNode

export type NodeKind = 'object' | 'array' | 'primitive'

export type NodeRow = {
  kind: 'node'
  path: string
  rawKey: string | null
  value: unknown
  nodeKind: NodeKind
  depth: number
  childCount: number
  isExpanded: boolean
  parentPath: string | null
  level: number
  posInSet: number
  setSize: number
}

export type CloseRow = {
  kind: 'close'
  id: string
  depth: number
  bracket: string
}

export type Row = NodeRow | CloseRow

export type JsonMatch = {
  path: string
  ancestors: string[]
  keyMatch: boolean
  valueMatch: boolean
}

export type ExpandedState = {
  token: unknown
  depth: number
  paths: Set<string>
}

export type JsonViewerProps = {
  data: JsonValue
  defaultExpandDepth?: number
  className?: string
  /** Text used by the copy/download buttons; defaults to pretty-printed JSON. */
  copyValue?: string
  /** Filename offered by the download button. */
  downloadFileName?: string
}
