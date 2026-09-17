import { ROOT_PATH } from './constants'
import type { JsonMatch, NodeKind, NodeRow, Row } from './types'

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function kindOf(value: unknown): NodeKind {
  if (Array.isArray(value)) return 'array'
  if (isRecord(value)) return 'object'
  return 'primitive'
}

export function isExpandable(value: unknown): boolean {
  return kindOf(value) !== 'primitive'
}

export function entriesOf(value: unknown): Array<[string, unknown]> {
  if (Array.isArray(value)) {
    return value.map((item, index) => [String(index), item])
  }
  if (isRecord(value)) {
    return Object.entries(value)
  }
  return []
}

/** JSON path segment join; arrays use `parent[index]`, objects use `parent.key`. */
export function childPath(
  parent: string,
  key: string,
  isArray: boolean,
): string {
  if (isArray) return `${parent}[${key}]`
  return parent === '' ? key : `${parent}.${key}`
}

export function displayPath(path: string): string {
  return path === ROOT_PATH ? '$' : path
}

export function primitiveText(value: unknown): string {
  if (typeof value === 'string') return value
  if (value === null) return 'null'
  return String(value)
}

export function safeStringify(value: unknown): string {
  try {
    const text: unknown = JSON.stringify(value, null, 2)
    return typeof text === 'string' ? text : String(value)
  } catch {
    return String(value)
  }
}

/** Text placed on the clipboard for a node's "copy value" action. */
export function clipboardText(value: unknown): string {
  if (typeof value === 'string') return value
  return safeStringify(value)
}

export function isNodeRow(row: Row): row is NodeRow {
  return row.kind === 'node'
}

export function collectDefaultExpanded(
  value: unknown,
  maxDepth: number,
): Set<string> {
  const result = new Set<string>()

  const visit = (node: unknown, path: string, depth: number): void => {
    if (!isExpandable(node) || depth >= maxDepth) return
    result.add(path)
    const isArray = Array.isArray(node)
    for (const [key, child] of entriesOf(node)) {
      visit(child, childPath(path, key, isArray), depth + 1)
    }
  }

  visit(value, ROOT_PATH, 0)
  return result
}

export function collectExpandablePaths(value: unknown): string[] {
  const result: string[] = []

  const visit = (node: unknown, path: string): void => {
    if (!isExpandable(node)) return
    result.push(path)
    const isArray = Array.isArray(node)
    for (const [key, child] of entriesOf(node)) {
      visit(child, childPath(path, key, isArray))
    }
  }

  visit(value, ROOT_PATH)
  return result
}

export function collectMatches(value: unknown, rawQuery: string): JsonMatch[] {
  const query = rawQuery.toLowerCase()
  if (query.length === 0) return []

  const result: JsonMatch[] = []

  const visit = (
    node: unknown,
    key: string | null,
    path: string,
    ancestors: string[],
  ): void => {
    const keyMatch = key !== null && key.toLowerCase().includes(query)
    const primitive = !isExpandable(node)
    const valueMatch =
      primitive && primitiveText(node).toLowerCase().includes(query)
    if (keyMatch || valueMatch) {
      result.push({ path, ancestors, keyMatch, valueMatch })
    }
    if (!isExpandable(node)) return

    const isArray = Array.isArray(node)
    const childAncestors = ancestors.concat(path)
    for (const [childKey, child] of entriesOf(node)) {
      visit(
        child,
        childKey,
        childPath(path, childKey, isArray),
        childAncestors,
      )
    }
  }

  visit(value, null, ROOT_PATH, [])
  return result
}

export function buildRows(value: unknown, expanded: ReadonlySet<string>): Row[] {
  const rows: Row[] = []

  const visit = (
    node: unknown,
    rawKey: string | null,
    path: string,
    depth: number,
    parentPath: string | null,
    posInSet: number,
    setSize: number,
  ): void => {
    const nodeKind = kindOf(node)
    const isContainer = nodeKind !== 'primitive'
    const entries = isContainer ? entriesOf(node) : []
    const isExpanded = isContainer && expanded.has(path)

    rows.push({
      kind: 'node',
      path,
      rawKey,
      value: node,
      nodeKind,
      depth,
      childCount: entries.length,
      isExpanded,
      parentPath,
      level: depth + 1,
      posInSet,
      setSize,
    })

    if (!isExpanded) return

    const isArray = nodeKind === 'array'
    entries.forEach(([childKey, child], index) => {
      visit(
        child,
        childKey,
        childPath(path, childKey, isArray),
        depth + 1,
        path,
        index + 1,
        entries.length,
      )
    })
    rows.push({
      kind: 'close',
      id: path,
      depth,
      bracket: isArray ? ']' : '}',
    })
  }

  visit(value, null, ROOT_PATH, 0, null, 1, 1)
  return rows
}

export function collapsedPreview(row: NodeRow): string {
  const isArray = row.nodeKind === 'array'
  const open = isArray ? '[' : '{'
  const close = isArray ? ']' : '}'
  if (row.childCount === 0) return `${open}${close}`
  const noun = isArray
    ? row.childCount === 1
      ? 'item'
      : 'items'
    : row.childCount === 1
      ? 'key'
      : 'keys'
  return `${open} … ${row.childCount} ${noun} ${close}`
}
