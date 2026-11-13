import { useId, useState, type FormEvent } from 'react'

import { isApiError } from '../api/client'
import { appPasswords } from '../api/endpoints'
import type { AppPasswordCategory } from '../api/schemas'
import { SelectField } from './FormFields'
import { Dialog } from './primitives/Dialog'
import { Button } from './primitives/Button'
import { Flash } from './primitives/Flash'
import { TextInput } from './primitives/TextInput'

export type CreateAppPasswordDialogProps = {
  categories: AppPasswordCategory[]
  /** Defaults to the category the user is currently looking at. */
  initialCategoryId?: number | null
  onClose: () => void
  onCreated: (result: {
    appPasswordId: number
    token: string
    label: string
  }) => void
}

/**
 * One-shot app-password setup: name plus an optional category, submitted as a
 * single `POST /api/app-passwords`. App passwords carry no grants, IP
 * allowlist, description or username — only the name and where it is filed.
 */
export function CreateAppPasswordDialog({
  categories,
  initialCategoryId = null,
  onClose,
  onCreated,
}: CreateAppPasswordDialogProps) {
  const formId = useId()
  const [name, setName] = useState('')
  const [categoryId, setCategoryId] = useState(
    initialCategoryId === null ? '' : String(initialCategoryId),
  )
  const [nameError, setNameError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setError(null)
    setNameError(null)

    const trimmedName = name.trim()
    if (trimmedName.length === 0) {
      setNameError('Give the app password a name.')
      return
    }

    setSubmitting(true)
    void appPasswords
      .create({
        name: trimmedName,
        ...(categoryId ? { category_id: Number(categoryId) } : {}),
      })
      .then(
        (response) => {
          setSubmitting(false)
          onCreated({
            appPasswordId: response.app_password.id,
            token: response.token,
            label: response.app_password.name,
          })
          onClose()
        },
        (failure: unknown) => {
          setSubmitting(false)
          setError(
            isApiError(failure)
              ? failure.message
              : 'The app password could not be created.',
          )
        },
      )
  }

  return (
    <Dialog
      open
      title="New app password"
      description="App passwords are used to log into Docker without your two-factor code."
      onClose={submitting ? () => undefined : onClose}
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
            {submitting ? 'Creating...' : 'Create app password'}
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
            placeholder="laptop"
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
        </div>
      </form>
    </Dialog>
  )
}
