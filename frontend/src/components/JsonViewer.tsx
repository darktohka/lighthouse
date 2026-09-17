import {
  ChevronDownIcon,
  ChevronRightIcon,
  CheckIcon,
  CopyIcon,
} from '@primer/octicons-react'
import { useState, type ReactNode } from 'react'

import { cx } from '../lib/cx'

type JsonValue = unknown

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function isExpandable(value: unknown): boolean {
  return Array.isArray(value) || isRecord(value)
}

function entriesOf(value: unknown): Array<[string, unknown]> {
  if (Array.isArray(value)) {
    return value.map((item, index) => [String(index), item])
  }
  if (isRecord(value)) {
    return Object.entries(value)
  }
  return []
}

function PrimitiveValue({ value }: { value: JsonValue }) {
  if (typeof value === 'string') {
    return <span className="break-all text-success">&quot;{value}&quot;</span>
  }
  if (typeof value === 'number') {
    return <span className="text-attention">{String(value)}</span>
  }
  if (typeof value === 'boolean') {
    return <span className="text-danger">{String(value)}</span>
  }
  if (value === null) {
    return <span className="text-muted">null</span>
  }
  return <span className="text-muted">{String(value)}</span>
}

type JsonNodeProps = {
  name: string | null
  value: JsonValue
  depth: number
  defaultExpandDepth: number
}

function JsonNode({ name, value, depth, defaultExpandDepth }: JsonNodeProps) {
  const [open, setOpen] = useState(depth < defaultExpandDepth)

  if (!isExpandable(value)) {
    return (
      <div className="flex flex-wrap gap-x-1">
        {name !== null ? <span className="text-accent">{name}:</span> : null}
        <PrimitiveValue value={value} />
      </div>
    )
  }

  const isArray = Array.isArray(value)
  const entries = entriesOf(value)
  const openBracket = isArray ? '[' : '{'
  const closeBracket = isArray ? ']' : '}'
  const noun = isArray ? 'items' : 'keys'

  return (
    <div>
      <div className="flex items-start gap-1">
        <button
          type="button"
          onClick={() => setOpen((previous) => !previous)}
          aria-expanded={open}
          className="mt-0.5 shrink-0 rounded p-0.5 text-muted hover:bg-neutral-subtle hover:text-foreground"
        >
          {open ? (
            <ChevronDownIcon size={12} aria-hidden="true" />
          ) : (
            <ChevronRightIcon size={12} aria-hidden="true" />
          )}
          <span className="sr-only">
            {open ? 'Collapse' : 'Expand'} {name ?? 'value'}
          </span>
        </button>
        <div className="min-w-0 flex-1">
          <div>
            {name !== null ? <span className="text-accent">{name}</span> : null}
            <span className="text-muted">
              {open
                ? openBracket
                : `${openBracket} ${entries.length} ${noun} ${closeBracket}`}
            </span>
          </div>
          {open ? (
            <div className="border-l border-border pl-3">
              {entries.map(([key, child]) => (
                <JsonNode
                  key={key}
                  name={key}
                  value={child}
                  depth={depth + 1}
                  defaultExpandDepth={defaultExpandDepth}
                />
              ))}
              <div className="text-muted">{closeBracket}</div>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  )
}

export type JsonViewerProps = {
  data: JsonValue
  defaultExpandDepth?: number
  className?: string
  /** Text used by the copy button; defaults to pretty-printed JSON. */
  copyValue?: string
}

/**
 * Small recursive JSON viewer with syntax highlighting and collapsible
 * objects/arrays. Deliberately dependency-free.
 */
export function JsonViewer({
  data,
  defaultExpandDepth = 2,
  className,
  copyValue,
}: JsonViewerProps) {
  const [copied, setCopied] = useState(false)

  const onCopy = () => {
    const text = copyValue ?? safeStringify(data)
    if (typeof navigator !== 'undefined' && navigator.clipboard) {
      void navigator.clipboard.writeText(text).then(
        () => setCopied(true),
        () => setCopied(false),
      )
    }
  }

  return (
    <div
      className={cx(
        'relative rounded-md border border-border bg-canvas-inset p-3',
        className,
      )}
    >
      <button
        type="button"
        onClick={onCopy}
        className="absolute right-2 top-2 rounded border border-border bg-canvas-default px-2 py-1 text-xs text-muted hover:text-foreground"
      >
        <span className="flex items-center gap-1">
          {copied ? (
            <CheckIcon size={12} aria-hidden="true" />
          ) : (
            <CopyIcon size={12} aria-hidden="true" />
          )}
          {copied ? 'Copied' : 'Copy'}
        </span>
      </button>
      <div className="overflow-x-auto pt-6 font-mono text-xs leading-5">
        <JsonNode
          name={null}
          value={data}
          depth={0}
          defaultExpandDepth={defaultExpandDepth}
        />
      </div>
    </div>
  )
}

function safeStringify(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

/** Convenience wrapper for callers that want to embed the copy button. */
export type { JsonValue }
export type JsonViewerNode = ReactNode
