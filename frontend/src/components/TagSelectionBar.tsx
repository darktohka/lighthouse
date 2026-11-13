import { ConfirmAction } from './ConfirmAction'
import { Button } from './primitives/Button'

export type TagSelectionBarProps = {
  /** Number of tags currently selected. */
  count: number
  /** True while the batch delete request is in flight. */
  pending?: boolean
  onClear: () => void
  onDelete: () => void | Promise<void>
}

/**
 * Bordered bar shown above the tag table once at least one tag is selected.
 * Deletion asks for confirmation through {@link ConfirmAction}.
 */
export function TagSelectionBar({
  count,
  pending = false,
  onClear,
  onDelete,
}: TagSelectionBarProps) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-border bg-canvas-subtle px-3 py-2">
      <p className="text-sm" aria-live="polite">
        {count} {count === 1 ? 'tag' : 'tags'} selected
      </p>
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          className="border-transparent bg-transparent"
          disabled={pending}
          onClick={onClear}
        >
          Clear selection
        </Button>
        <ConfirmAction
          label="Delete"
          confirmLabel={`Delete ${count} ${count === 1 ? 'tag' : 'tags'}`}
          resourceName={`${count} selected tags`}
          pending={pending}
          onConfirm={onDelete}
        />
      </div>
    </div>
  )
}
