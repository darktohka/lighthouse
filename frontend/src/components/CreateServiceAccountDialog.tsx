import { KeyIcon, PlusIcon } from '@primer/octicons-react'
import { useId, useState, type FormEvent } from 'react'

import { isApiError } from '../api/client'
import { serviceAccounts as serviceAccountsApi } from '../api/endpoints'
import type { ServiceAccountCategory } from '../api/schemas'
import { SelectField } from './FormFields'
import { Dialog } from './primitives/Dialog'
import { Button } from './primitives/Button'
import { Flash } from './primitives/Flash'
import { Label } from './primitives/Label'
import { TextInput } from './primitives/TextInput'
import {
  GrantDraftRow,
  IpRangeDraftRow,
  RowRemoveButton,
} from './ServiceAccountRows'
import {
  EMPTY_GRANT_DRAFT,
  grantDraftIsComplete,
  type GrantDraft,
} from '../lib/serviceAccountGrants'

export type CreateServiceAccountDialogProps = {
  categories: ServiceAccountCategory[]
  /** Defaults to the category the user is currently looking at. */
  initialCategoryId?: number | null
  onClose: () => void
  onCreated: (result: { token: string; label: string }) => void
}

/**
 * One-shot account setup: identity, category, IP allowlist and grants are all
 * queued here and submitted in a single atomic `POST /api/service-accounts`.
 * A grant with no target left in the trailing row is included automatically, so
 * the user never has to press "Add" for the last one.
 */
export function CreateServiceAccountDialog({
  categories,
  initialCategoryId = null,
  onClose,
  onCreated,
}: CreateServiceAccountDialogProps) {
  const formId = useId()
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [categoryId, setCategoryId] = useState(
    initialCategoryId === null ? '' : String(initialCategoryId),
  )
  const [nameError, setNameError] = useState<string | null>(null)
  const [ranges, setRanges] = useState<string[]>([])
  const [rangeDraft, setRangeDraft] = useState('')
  const [grants, setGrants] = useState<GrantDraft[]>([])
  const [grantDraft, setGrantDraft] = useState<GrantDraft>(EMPTY_GRANT_DRAFT)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const addRange = () => {
    const value = rangeDraft.trim()
    if (value.length === 0) return
    if (!ranges.includes(value)) setRanges([...ranges, value])
    setRangeDraft('')
  }

  const addGrant = () => {
    const target = grantDraft.target.trim()
    if (target.length === 0) return
    setGrants([...grants, { ...grantDraft, target }])
    setGrantDraft({ ...EMPTY_GRANT_DRAFT, kind: grantDraft.kind })
  }

  /** Queued rows plus whatever is still typed in the trailing rows. */
  const collectRanges = (): string[] => {
    const pending = rangeDraft.trim()
    if (pending.length === 0 || ranges.includes(pending)) return ranges
    return [...ranges, pending]
  }

  const collectGrants = (): GrantDraft[] => {
    if (!grantDraftIsComplete(grantDraft)) return grants
    return [...grants, { ...grantDraft, target: grantDraft.target.trim() }]
  }

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setError(null)
    setNameError(null)

    const trimmedName = name.trim()
    if (trimmedName.length === 0) {
      setNameError('Give the service account a name.')
      return
    }

    const rangesToSend = collectRanges()
    const grantsToSend = collectGrants()

    setSubmitting(true)
    void serviceAccountsApi
      .create({
        name: trimmedName,
        ...(description.trim() ? { description: description.trim() } : {}),
        ...(categoryId ? { category_id: Number(categoryId) } : {}),
        ...(rangesToSend.length > 0 ? { ip_ranges: rangesToSend } : {}),
        ...(grantsToSend.length > 0
          ? {
              grants: grantsToSend.map((grant) => ({
                ...(grant.kind === 'namespace'
                  ? { namespace: grant.target }
                  : { repository: grant.target }),
                can_push: grant.canPush,
              })),
            }
          : {}),
      })
      .then(
        (response) => {
          setSubmitting(false)
          onCreated({
            token: response.token,
            label: response.account.name,
          })
          onClose()
        },
        (failure: unknown) => {
          setSubmitting(false)
          setError(
            isApiError(failure)
              ? failure.message
              : 'The service account could not be created.',
          )
        },
      )
  }

  return (
    <Dialog
      open
      title="New service account"
      description="Set up the token, its categories, reachable networks and permissions in one go."
      onClose={submitting ? () => undefined : onClose}
      widthClassName="max-w-2xl"
      footer={
        <>
          <Button onClick={onClose} disabled={submitting}>
            Cancel
          </Button>
          <Button
            type="submit"
            form={formId}
            variant="success"
            disabled={submitting}
          >
            {submitting ? 'Creating…' : 'Create service account'}
          </Button>
        </>
      }
    >
      <form id={formId} className="space-y-4" onSubmit={submit} noValidate>
        {error ? (
          <Flash variant="danger" onDismiss={() => setError(null)}>
            {error}
          </Flash>
        ) : null}

        <div className="grid gap-3 sm:grid-cols-2">
          <TextInput
            label="Name"
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="ci-deploy"
            error={nameError}
            required
            autoComplete="off"
          />
          <SelectField
            label="Category"
            value={categoryId}
            onChange={(event) => setCategoryId(event.target.value)}
            options={[
              { value: '', label: 'Uncategorized' },
              ...categories.map((category) => ({
                value: String(category.id),
                label: category.name,
              })),
            ]}
          />
          <div className="sm:col-span-2">
            <TextInput
              label="Description"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              placeholder="Deploys production from GitHub Actions"
              autoComplete="off"
            />
          </div>
        </div>

        <section className="space-y-1">
          <h3 className="text-sm font-medium">IP allowlist</h3>
          <p className="text-xs text-muted">
            Optional. Leave empty to let the account authenticate from any
            address. Single addresses become <code>/32</code> (IPv4) or{' '}
            <code>/128</code> (IPv6).
          </p>
          <ul className="divide-y divide-border rounded-md border border-border">
            {ranges.map((range) => (
              <li
                key={range}
                className="flex items-center justify-between gap-2 px-2 py-1"
              >
                <span className="font-mono text-xs">{range}</span>
                <RowRemoveButton
                  label={`Remove ${range}`}
                  onClick={() =>
                    setRanges(ranges.filter((item) => item !== range))
                  }
                />
              </li>
            ))}
            <IpRangeDraftRow
              value={rangeDraft}
              onChange={setRangeDraft}
              onAdd={addRange}
            />
          </ul>
        </section>

        <section className="space-y-1">
          <h3 className="text-sm font-medium">Grants</h3>
          <p className="text-xs text-muted">
            Each grant is created together with the account. If one is rejected,
            nothing is created.
          </p>
          <ul className="divide-y divide-border rounded-md border border-border">
            {grants.map((grant, index) => (
              <li
                key={`${grant.kind}:${grant.target}:${index}`}
                className="flex items-center justify-between gap-2 px-2 py-1"
              >
                <span className="flex min-w-0 items-center gap-2 text-xs">
                  <Label variant="muted">{grant.kind}</Label>
                  <span className="truncate font-mono">{grant.target}</span>
                  <Label variant={grant.canPush ? 'success' : 'muted'}>
                    {grant.canPush ? 'push + pull' : 'pull'}
                  </Label>
                </span>
                <RowRemoveButton
                  label={`Remove grant on ${grant.target}`}
                  onClick={() =>
                    setGrants(grants.filter((_, position) => position !== index))
                  }
                />
              </li>
            ))}
            <GrantDraftRow
              draft={grantDraft}
              onChange={setGrantDraft}
              onAdd={addGrant}
            />
          </ul>
        </section>

        <p className="flex items-center gap-1.5 text-xs text-muted">
          <KeyIcon size={12} aria-hidden="true" />
          The token is shown once, right after creation.
          <PlusIcon size={12} aria-hidden="true" />
          Everything above is saved atomically.
        </p>
      </form>
    </Dialog>
  )
}
