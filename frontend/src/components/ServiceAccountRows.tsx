/**
 * Small row-level controls shared by the service-account list and the
 * create-account dialog: "implicit" add rows that sit at the end of a list
 * (styled as a list row rather than a separate boxed form) and the remove
 * affordance used by rows that have not been saved yet.
 */
import { XIcon } from '@primer/octicons-react'
import type { KeyboardEvent } from 'react'
import { useId } from 'react'

import { cx } from '../lib/cx'
import {
  grantDraftIsComplete,
  type GrantDraft,
} from '../lib/serviceAccountGrants'
import { compactInputClasses } from '../lib/ui'
import { Button } from './primitives/Button'

export type GrantDraftRowProps = {
  draft: GrantDraft
  onChange: (draft: GrantDraft) => void
  onAdd: () => void
  pending?: boolean
  addLabel?: string
}

/**
 * The trailing "new grant" row: type, target, push and an Add button that stays
 * disabled until the target is non-empty. Enter in the target field adds too.
 */
export function GrantDraftRow({
  draft,
  onChange,
  onAdd,
  pending,
  addLabel = 'Add',
}: GrantDraftRowProps) {
  const id = useId()
  const typeId = `${id}-type`
  const targetId = `${id}-target`
  const ready = grantDraftIsComplete(draft) && !pending

  // Enter adds the grant when it can; otherwise it falls through to the form's
  // own submit handler (inside the create dialog) instead of doing nothing.
  const submitOnEnter = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key !== 'Enter' || !ready) return
    event.preventDefault()
    onAdd()
  }

  return (
    <li className="flex flex-wrap items-center gap-1.5 px-2 py-1.5">
      <label htmlFor={typeId} className="sr-only">
        Grant type
      </label>
      <span className="w-28 shrink-0">
        <select
          id={typeId}
          value={draft.kind}
          onChange={(event) =>
            onChange({
              ...draft,
              kind:
                event.target.value === 'repository' ? 'repository' : 'namespace',
            })
          }
          className={compactInputClasses('h-7')}
        >
          <option value="namespace">Namespace</option>
          <option value="repository">Repository</option>
        </select>
      </span>
      <label htmlFor={targetId} className="sr-only">
        {draft.kind === 'namespace' ? 'Namespace name' : 'Repository path'}
      </label>
      <span className="min-w-32 flex-1">
        <input
          id={targetId}
          value={draft.target}
          onChange={(event) => onChange({ ...draft, target: event.target.value })}
          onKeyDown={submitOnEnter}
          placeholder={
            draft.kind === 'namespace' ? 'team-name' : 'namespace/image'
          }
          className={compactInputClasses(cx('h-7 font-mono'))}
        />
      </span>
      <label className="inline-flex shrink-0 items-center gap-1 text-xs">
        <input
          type="checkbox"
          checked={draft.canPush}
          onChange={(event) =>
            onChange({ ...draft, canPush: event.target.checked })
          }
        />
        Push
      </label>
      <Button size="sm" disabled={!ready} onClick={onAdd}>
        {pending ? 'Adding…' : addLabel}
      </Button>
    </li>
  )
}

export type IpRangeDraftRowProps = {
  value: string
  onChange: (value: string) => void
  onAdd: () => void
  pending?: boolean
  addLabel?: string
}

/** The trailing "new IP range" row, styled as a list row. */
export function IpRangeDraftRow({
  value,
  onChange,
  onAdd,
  pending,
  addLabel = 'Add',
}: IpRangeDraftRowProps) {
  const id = useId()
  const ready = value.trim().length > 0 && !pending

  return (
    <li className="flex flex-wrap items-center gap-1.5 px-2 py-1.5">
      <label htmlFor={id} className="sr-only">
        IP address or range
      </label>
      <span className="min-w-32 flex-1">
        <input
          id={id}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' || !ready) return
            event.preventDefault()
            onAdd()
          }}
          placeholder="203.0.113.4 or 10.0.0.0/8"
          className={compactInputClasses(cx('h-7 font-mono'))}
        />
      </span>
      <Button size="sm" disabled={!ready} onClick={onAdd}>
        {pending ? 'Adding…' : addLabel}
      </Button>
    </li>
  )
}

export type RowRemoveButtonProps = {
  label: string
  onClick: () => void
  disabled?: boolean
}

/** Quiet icon button for removing a not-yet-saved row. */
export function RowRemoveButton({
  label,
  onClick,
  disabled,
}: RowRemoveButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      title={label}
      className="shrink-0 rounded p-1 text-muted transition-colors hover:bg-neutral-muted hover:text-danger disabled:cursor-not-allowed disabled:opacity-60"
    >
      <XIcon size={12} aria-hidden="true" />
    </button>
  )
}
