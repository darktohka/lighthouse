import { SearchIcon, XIcon } from '@primer/octicons-react'
import { useId } from 'react'

import { cx } from '../lib/cx'
import { compactInputClasses, labelClasses } from '../lib/ui'

/** A ready-made filter for the repository tag list. */
type TagFilterPreset = {
  id: string
  label: string
  pattern: string
  exclude?: boolean
}

const PRESETS: TagFilterPreset[] = [
  { id: 'sha1', label: 'SHA-1 digests', pattern: '^[a-f0-9]{40}$' },
  { id: 'short-sha', label: 'Short SHAs', pattern: '^[a-f0-9]{7,12}$' },
  {
    id: 'semver',
    label: 'Semver',
    pattern: '^v?\\d+\\.\\d+\\.\\d+(?:-[0-9A-Za-z.-]+)?$',
  },
  {
    id: 'aliases',
    label: 'Release aliases',
    pattern: '^(?:latest|main|master)$',
  },
  {
    id: 'aliases-not',
    label: 'Exclude release aliases',
    pattern: '^(?:latest|main|master)$',
    exclude: true,
  },
]

export type TagFilterBarProps = {
  /** Current input text, kept in sync with the parent's debounced query. */
  value: string
  exclude: boolean
  /** Called on every keystroke; the parent also clears the exclusion flag. */
  onChange: (value: string) => void
  /** Called when a preset chip is clicked. */
  onSelectPreset: (pattern: string, exclude: boolean) => void
  /** Clears both the pattern and the exclusion flag. */
  onClear: () => void
  /** Total tags matching the committed pattern, or null while unknown. */
  matchCount?: number | null
  /** Show the match count (only meaningful while a filter is active). */
  showCount?: boolean
  /** Inline validation message, e.g. a rejected regex from the API. */
  error?: string | null
}

/**
 * Search/preset toolbar that sits above the repository tag table. The input is
 * monospace because the value is a regular expression; presets fill it for the
 * common tag naming schemes.
 */
export function TagFilterBar({
  value,
  exclude,
  onChange,
  onSelectPreset,
  onClear,
  matchCount,
  showCount = false,
  error,
}: TagFilterBarProps) {
  const id = useId()
  const inputId = `${id}-input`
  const errorId = `${id}-error`

  const trimmed = value.trim()
  const activePreset = (preset: TagFilterPreset) =>
    trimmed === preset.pattern && exclude === Boolean(preset.exclude)

  return (
    <div className="border-b border-border px-3 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <label htmlFor={inputId} className="text-xs font-medium text-muted">
          Filter tags
        </label>
        <div className="relative w-60 sm:w-72">
          <SearchIcon
            size={14}
            aria-hidden="true"
            className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted"
          />
          <input
            id={inputId}
            type="text"
            value={value}
            onChange={(event) => onChange(event.target.value)}
            placeholder={'e.g. ^v\\d+\\.\\d+\\.\\d+$'}
            spellCheck={false}
            autoComplete="off"
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? errorId : undefined}
            className={compactInputClasses(
              cx('h-7 pl-7 font-mono', value ? 'pr-7' : 'pr-2'),
            )}
          />
          {value ? (
            <button
              type="button"
              onClick={onClear}
              aria-label="Clear filter"
              className="absolute right-1 top-1/2 -translate-y-1/2 cursor-pointer rounded p-0.5 text-muted hover:bg-neutral-subtle hover:text-foreground"
            >
              <XIcon size={14} aria-hidden="true" />
            </button>
          ) : null}
        </div>
        {exclude ? (
          <span
            className={labelClasses('attention')}
            title="Only tags that do not match the pattern are shown"
          >
            Not
          </span>
        ) : null}
        {showCount && matchCount !== null && matchCount !== undefined ? (
          <p
            className="text-xs text-muted sm:ml-auto"
            aria-live="polite"
          >
            {matchCount} {matchCount === 1 ? 'tag matches' : 'tags match'}
          </p>
        ) : null}
      </div>

      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        <span className="text-xs text-muted">Presets</span>
        {PRESETS.map((preset) => {
          const active = activePreset(preset)
          return (
            <button
              key={preset.id}
              type="button"
              onClick={() => onSelectPreset(preset.pattern, Boolean(preset.exclude))}
              aria-pressed={active}
              title={
                preset.exclude
                  ? 'Show tags that do not match this pattern'
                  : `Filter with ${preset.pattern}`
              }
              className={cx(
                'inline-flex cursor-pointer items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium transition-colors',
                active
                  ? 'border-accent bg-accent-subtle text-accent'
                  : 'border-border bg-canvas-subtle text-muted hover:border-accent hover:text-accent',
              )}
            >
              {preset.exclude ? (
                <span className="rounded-full border border-attention px-1 text-[10px] font-semibold uppercase leading-4 text-attention">
                  Not
                </span>
              ) : null}
              {preset.label}
            </button>
          )
        })}
      </div>

      {error ? (
        <p id={errorId} className="mt-1.5 text-xs text-danger">
          {error}
        </p>
      ) : null}
    </div>
  )
}
