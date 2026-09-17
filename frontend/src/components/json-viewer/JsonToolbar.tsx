import {
  ArrowDownIcon,
  ArrowUpIcon,
  CheckIcon,
  CopyIcon,
  DownloadIcon,
  FoldIcon,
  SearchIcon,
  UnfoldIcon,
  WrapIcon,
  XIcon,
} from '@primer/octicons-react'
import type { ChangeEvent, KeyboardEvent } from 'react'

import { cx } from '../../lib/cx'
import { Button } from '../primitives/Button'
import { TOOLBAR_ICON_BUTTON } from './constants'

export type JsonToolbarSearch = {
  query: string
  matchLabel: string
  hasMatches: boolean
  onChange: (event: ChangeEvent<HTMLInputElement>) => void
  onKeyDown: (event: KeyboardEvent<HTMLInputElement>) => void
  onPrevious: () => void
  onNext: () => void
  onClear: () => void
}

export type JsonToolbarProps = {
  search: JsonToolbarSearch
  wrap: boolean
  onToggleWrap: () => void
  onExpandAll: () => void
  onCollapseAll: () => void
  copied: boolean
  onCopy: () => void
  onDownload: () => void
}

export function JsonToolbar({
  search,
  wrap,
  onToggleWrap,
  onExpandAll,
  onCollapseAll,
  copied,
  onCopy,
  onDownload,
}: JsonToolbarProps) {
  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-border bg-canvas-subtle px-2 py-1.5">
      <div className="flex items-center gap-1">
        <Button
          size="sm"
          onClick={onExpandAll}
          leadingIcon={<UnfoldIcon size={14} aria-hidden="true" />}
        >
          Expand all
        </Button>
        <Button
          size="sm"
          onClick={onCollapseAll}
          leadingIcon={<FoldIcon size={14} aria-hidden="true" />}
        >
          Collapse all
        </Button>
        <button
          type="button"
          aria-pressed={wrap}
          onClick={onToggleWrap}
          title="Toggle line wrapping"
          className={cx(
            'inline-flex h-7 items-center gap-1.5 rounded-md border px-2.5 text-xs font-medium transition-colors',
            wrap
              ? 'border-accent bg-accent-subtle text-accent'
              : 'border-border bg-canvas-subtle text-foreground hover:bg-neutral-subtle',
          )}
        >
          <WrapIcon size={14} aria-hidden="true" />
          Wrap
        </button>
      </div>

      <div className="flex min-w-[12rem] flex-1 items-center gap-1">
        <div className="relative min-w-0 flex-1">
          <SearchIcon
            size={14}
            className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted"
            aria-hidden="true"
          />
          <input
            type="search"
            value={search.query}
            onChange={search.onChange}
            onKeyDown={search.onKeyDown}
            placeholder="Search keys and values"
            aria-label="Search JSON keys and values"
            className="h-7 w-full rounded-md border border-border bg-canvas-default pl-7 pr-2 text-xs text-foreground placeholder:text-muted focus:border-accent focus:outline-none"
          />
        </div>
        <span
          className="min-w-[4.5rem] shrink-0 text-center text-[11px] text-muted"
          aria-live="polite"
        >
          {search.matchLabel}
        </span>
        <button
          type="button"
          className={TOOLBAR_ICON_BUTTON}
          onClick={search.onPrevious}
          disabled={!search.hasMatches}
          aria-label="Previous match"
          title="Previous match (Shift+Enter)"
        >
          <ArrowUpIcon size={14} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={TOOLBAR_ICON_BUTTON}
          onClick={search.onNext}
          disabled={!search.hasMatches}
          aria-label="Next match"
          title="Next match (Enter)"
        >
          <ArrowDownIcon size={14} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={TOOLBAR_ICON_BUTTON}
          onClick={search.onClear}
          disabled={search.query.length === 0}
          aria-label="Clear search"
          title="Clear search"
        >
          <XIcon size={14} aria-hidden="true" />
        </button>
      </div>

      <div className="flex items-center gap-1">
        <Button
          size="sm"
          onClick={onCopy}
          leadingIcon={
            copied ? (
              <CheckIcon size={14} aria-hidden="true" />
            ) : (
              <CopyIcon size={14} aria-hidden="true" />
            )
          }
        >
          {copied ? 'Copied' : 'Copy JSON'}
        </Button>
        <Button
          size="sm"
          onClick={onDownload}
          leadingIcon={<DownloadIcon size={14} aria-hidden="true" />}
        >
          Download
        </Button>
      </div>
    </div>
  )
}
