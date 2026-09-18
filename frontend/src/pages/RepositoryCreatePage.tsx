import { useState, type FormEvent } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'

import { isApiError } from '../api/client'
import { repositories as repositoriesApi } from '../api/endpoints'
import { createRepositoryFormSchema } from '../api/schemas'
import { PageHeader } from '../components/PageHeader'
import { SelectField, SwitchField, TextArea } from '../components/FormFields'
import { Box, BoxBody, BoxHeader } from '../components/primitives/Box'
import { Button, LinkButton } from '../components/primitives/Button'
import { Flash } from '../components/primitives/Flash'
import { EmptyState } from '../components/primitives/StateViews'
import { TextInput } from '../components/primitives/TextInput'
import { useAuth } from '../lib/auth-context'
import { validateForm, type FieldErrors } from '../lib/forms'
import { repoRoute } from '../lib/paths'

export function RepositoryCreatePage() {
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const { namespaces } = useAuth()

  const requested = searchParams.get('namespace')
  const initial =
    namespaces.find((item) => item.name === requested) ?? namespaces[0]

  const [namespace, setNamespace] = useState(initial?.name ?? '')
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [isPublic, setIsPublic] = useState(initial?.is_public ?? false)
  const [errors, setErrors] = useState<FieldErrors>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  const options = namespaces.map((item) => ({
    value: item.name,
    label: item.name,
  }))

  const onNamespaceChange = (next: string) => {
    setNamespace(next)
    const match = namespaces.find((item) => item.name === next)
    if (match) setIsPublic(match.is_public)
  }

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setFormError(null)

    const validation = validateForm(createRepositoryFormSchema, {
      namespace,
      name: name.trim(),
      description,
      is_public: isPublic,
    })
    if (!validation.ok) {
      setErrors(validation.errors)
      if (validation.formError) setFormError(validation.formError)
      return
    }
    setErrors({})

    const target = validation.value
    setSubmitting(true)
    void repositoriesApi
      .create(target.namespace, {
        name: target.name,
        ...(target.description ? { description: target.description } : {}),
        is_public: target.is_public,
      })
      .then(
        () => {
          setSubmitting(false)
          navigate(repoRoute(target.namespace, target.name))
        },
        (error: unknown) => {
          setSubmitting(false)
          setFormError(
            isApiError(error)
              ? error.message
              : 'The repository could not be created.',
          )
        },
      )
  }

  if (namespaces.length === 0) {
    return (
      <div className="space-y-4">
        <PageHeader
          title="New repository"
          description="Create a repository before pushing its first image."
        />
        <Box>
          <EmptyState
            title="No workspaces yet"
            description="Repositories live inside a namespace. Create a workspace first, then add repositories to it."
            action={
              <LinkButton to="/new" variant="primary">
                New workspace
              </LinkButton>
            }
          />
        </Box>
      </div>
    )
  }

  const preview = name.trim().length > 0 ? `${namespace}/${name.trim()}` : null

  return (
    <div className="space-y-4">
      <PageHeader
        title="New repository"
        description="Create a repository before pushing its first image."
      />

      <Box>
        <BoxHeader>
          <span className="text-sm font-medium">Repository details</span>
        </BoxHeader>
        <BoxBody>
          <form className="space-y-3" onSubmit={onSubmit} noValidate>
            {formError ? (
              <Flash variant="danger" onDismiss={() => setFormError(null)}>
                {formError}
              </Flash>
            ) : null}

            <SelectField
              label="Namespace"
              hint="Repositories are created inside a namespace you can push to."
              value={namespace}
              onChange={(event) => onNamespaceChange(event.target.value)}
              options={options}
              error={errors.namespace}
              required
            />
            <TextInput
              label="Name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              error={errors.name}
              hint={
                preview
                  ? `New repository: ${preview}`
                  : 'The path inside the namespace. Nested paths are allowed.'
              }
              placeholder="my-image"
              required
            />
            <TextArea
              label="Description"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              hint="Optional. Shown on the repository page and in Explore."
            />
            <SwitchField
              label="Public repository"
              hint="Public repositories are readable without authentication."
              checked={isPublic}
              onCheckedChange={setIsPublic}
            />
            <Button type="submit" variant="primary" disabled={submitting}>
              {submitting ? 'Creating…' : 'Create repository'}
            </Button>
          </form>
        </BoxBody>
      </Box>
    </div>
  )
}
