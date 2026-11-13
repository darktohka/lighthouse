/**
 * Draft state for grants that have not been saved yet.
 *
 * Kept out of the row components so those files only export components (React
 * Fast Refresh needs that) and so the create dialog and the account row can
 * share one shape.
 */

export type GrantKind = 'namespace' | 'repository'

export type GrantDraft = {
  kind: GrantKind
  target: string
  canPush: boolean
}

export const EMPTY_GRANT_DRAFT: GrantDraft = {
  kind: 'namespace',
  target: '',
  canPush: false,
}

/** True once the draft has a usable target. */
export function grantDraftIsComplete(draft: GrantDraft): boolean {
  return draft.target.trim().length > 0
}
