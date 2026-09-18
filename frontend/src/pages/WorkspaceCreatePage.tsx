import { useState, type FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'

import { isApiError } from '../api/client'
import { namespaces as namespacesApi } from '../api/endpoints'
import { createWorkspaceFormSchema } from '../api/schemas'
import { PageHeader } from '../components/PageHeader'
import { SwitchField, TextArea } from '../components/FormFields'
import { Box, BoxBody, BoxHeader } from '../components/primitives/Box'
import { Button } from '../components/primitives/Button'
import { Flash } from '../components/primitives/Flash'
import { TextInput } from '../components/primitives/TextInput'
import { ErrorState } from '../components/primitives/StateViews'
import { validateForm, type FieldErrors } from '../lib/forms'
import { useAsync } from '../lib/useAsync'

const RESERVED_NAMES = [
  'v2',
  'api',
  'admin',
  'static',
  'assets',
  'login',
  'register',
  'settings',
  'explore',
  'analytics',
  'search',
  'new',
  'notifications',
  'account',
  'organizations',
  'libraries',
]

export function WorkspaceCreatePage() {
  const navigate = useNavigate()
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [isPublic, setIsPublic] = useState(true)
  const [errors, setErrors] = useState<FieldErrors>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  const known = useAsync(
    (signal) => namespacesApi.list(1, 100, { signal }),
    'workspace-create-namespaces',
  )

  const lower = name.trim().toLowerCase()
  const isReserved = lower.length > 0 && RESERVED_NAMES.includes(lower)
  const isTaken =
    lower.length > 0 &&
    (known.data?.items.some((item) => item.name.toLowerCase() === lower) ?? false)
  const liveError = isReserved
    ? 'That name is reserved by the registry.'
    : isTaken
      ? 'That name is already in use by a user or workspace.'
      : null

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setFormError(null)

    const validation = validateForm(createWorkspaceFormSchema, {
      name: name.trim(),
      description,
      is_public: isPublic,
    })
    if (!validation.ok) {
      setErrors(validation.errors)
      if (validation.formError) setFormError(validation.formError)
      return
    }
    if (liveError) {
      setErrors({ name: liveError })
      return
    }
    setErrors({})

    setSubmitting(true)
    void namespacesApi
      .create({
        name: validation.value.name,
        ...(validation.value.description
          ? { description: validation.value.description }
          : {}),
        is_public: validation.value.is_public,
      })
      .then(
        (created) => {
          setSubmitting(false)
          navigate(`/namespaces/${encodeURIComponent(created.name)}/settings`)
        },
        (error: unknown) => {
          setSubmitting(false)
          setFormError(
            isApiError(error)
              ? error.message
              : 'The workspace could not be created.',
          )
        },
      )
  }

  return (
    <div className="space-y-4">
      <PageHeader
        title="New workspace"
        description="Workspaces group repositories and let you delegate access to a team."
      />

      {known.error ? (
        <ErrorState error={known.error} onRetry={known.reload} />
      ) : null}

      <Box>
        <BoxHeader>
          <span className="text-sm font-medium">Workspace details</span>
        </BoxHeader>
        <BoxBody>
          <form className="space-y-3" onSubmit={onSubmit} noValidate>
            {formError ? (
              <Flash variant="danger" onDismiss={() => setFormError(null)}>
                {formError}
              </Flash>
            ) : null}

            <TextInput
              label="Name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              error={errors.name ?? liveError}
              hint={
                liveError
                  ? undefined
                  : 'Lowercase letters, numbers, dots, dashes or underscores.'
              }
              placeholder="team-infra"
              required
            />
            <TextArea
              label="Description"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              hint="Optional. Shown on the workspace page."
            />
            <SwitchField
              label="Public workspace"
              hint="On by default. Public workspaces list their repositories in Explore."
              checked={isPublic}
              onCheckedChange={setIsPublic}
            />
            <Button
              type="submit"
              variant="primary"
              disabled={submitting || liveError !== null}
            >
              {submitting ? 'Creating…' : 'Create workspace'}
            </Button>
          </form>
        </BoxBody>
      </Box>
    </div>
  )
}
