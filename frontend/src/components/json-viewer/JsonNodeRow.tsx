import {
  CheckIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  CopyIcon,
  LinkIcon,
} from '@primer/octicons-react'

import { cx } from '../../lib/cx'
import { CONTENT_OFFSET_PX, INDENT_PX, NODE_ACTION_BUTTON } from './constants'
import { Highlight } from './Highlight'
import { PrimitiveValue } from './PrimitiveValue'
import { useJsonViewerContext } from './context'
import { collapsedPreview, displayPath } from './json-model'
import type { Row } from './types'

export function JsonNodeRow({ row }: { row: Row }) {
  const {
    query,
    wrap,
    copiedAction,
    activeMatchPath,
    focusTargetPath,
    togglePath,
    copyNodeValue,
    copyNodePath,
    registerRowRef,
    onRowClick,
    onRowFocus,
  } = useJsonViewerContext()

  if (row.kind === 'close') {
    return (
      <div
        aria-hidden="true"
        className="py-px text-muted"
        style={{ paddingLeft: row.depth * INDENT_PX + CONTENT_OFFSET_PX }}
      >
        {row.bracket}
      </div>
    )
  }

  const isActive = activeMatchPath !== null && activeMatchPath === row.path
  const isFocusable = row.path === focusTargetPath

  return (
    <div
      role="treeitem"
      aria-level={row.level}
      aria-setsize={row.setSize}
      aria-posinset={row.posInSet}
      aria-expanded={row.nodeKind === 'primitive' ? undefined : row.isExpanded}
      tabIndex={isFocusable ? 0 : -1}
      ref={(element) => {
        registerRowRef(row.path, element)
      }}
      onClick={(event) => {
        onRowClick(row.path, event.currentTarget)
      }}
      onFocus={() => {
        onRowFocus(row.path)
      }}
      className={cx(
        'group/row flex items-start gap-1 rounded-sm py-px pr-1',
        isActive && 'bg-accent-subtle',
      )}
      style={{ paddingLeft: row.depth * INDENT_PX }}
    >
      {row.nodeKind === 'primitive' ? (
        <span className="w-4 shrink-0" aria-hidden="true" />
      ) : (
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation()
            togglePath(row.path)
          }}
          aria-label={`${row.isExpanded ? 'Collapse' : 'Expand'} ${displayPath(row.path)}`}
          className="mt-0.5 inline-flex h-4 w-4 shrink-0 items-center justify-center rounded text-muted hover:bg-neutral-subtle hover:text-foreground"
        >
          {row.isExpanded ? (
            <ChevronDownIcon size={12} aria-hidden="true" />
          ) : (
            <ChevronRightIcon size={12} aria-hidden="true" />
          )}
        </button>
      )}

      <div
        className={cx(
          'min-w-0 flex-1',
          wrap ? 'whitespace-pre-wrap break-all' : 'whitespace-pre',
        )}
      >
        {row.rawKey !== null ? (
          row.nodeKind === 'array' ? (
            <span className="text-muted">
              [
              <span className="text-accent">
                <Highlight text={row.rawKey} query={query} />
              </span>
              ]{' '}
            </span>
          ) : (
            <span className="text-accent">
              <Highlight text={row.rawKey} query={query} />
              <span className="text-muted">: </span>
            </span>
          )
        ) : null}
        {row.nodeKind === 'primitive' ? (
          <PrimitiveValue value={row.value} query={query} />
        ) : row.isExpanded ? (
          <span className="text-muted">
            {row.nodeKind === 'array' ? '[' : '{'}
          </span>
        ) : (
          <span className="text-muted">{collapsedPreview(row)}</span>
        )}
      </div>

      <span className="flex shrink-0 items-center gap-0.5 self-start pl-1 opacity-100 transition-opacity md:opacity-0 md:group-hover/row:opacity-100 md:group-focus-within/row:opacity-100">
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation()
            copyNodeValue(row)
          }}
          aria-label={`Copy value at ${displayPath(row.path)}`}
          title="Copy value"
          className={NODE_ACTION_BUTTON}
        >
          {copiedAction === `value:${row.path}` ? (
            <CheckIcon size={12} aria-hidden="true" />
          ) : (
            <CopyIcon size={12} aria-hidden="true" />
          )}
        </button>
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation()
            copyNodePath(row.path)
          }}
          aria-label={`Copy path ${displayPath(row.path)}`}
          title="Copy path"
          className={NODE_ACTION_BUTTON}
        >
          {copiedAction === `path:${row.path}` ? (
            <CheckIcon size={12} aria-hidden="true" />
          ) : (
            <LinkIcon size={12} aria-hidden="true" />
          )}
        </button>
      </span>
    </div>
  )
}
