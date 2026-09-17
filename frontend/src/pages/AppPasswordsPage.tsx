import { KeyIcon } from '@primer/octicons-react'
import { useState, type FormEvent } from 'react'

import { isApiError } from '../api/client'
import { appPasswords } from '../api/endpoints'
import { createAppPasswordSchema, type AppPassword } from '../api/schemas'
import { AppPasswordCard } from '../components/AppPasswordCard'
import { OneTimeToken } from '../components/OneTimeToken'
import { PageHeader } from '../components/PageHeader'
import { Box, BoxBody, BoxHeader } from '../components/primitives/Box'
import { Button } from '../components/primitives/Button'
import { Flash } from '../components/primitives/Flash'
import { TextInput } from '../components/primitives/TextInput'
import { EmptyState, ErrorState, LoadingState } from '../components/primitives/StateViews'
import { validateForm } from '../lib/forms'
import { useAsync } from '../lib/useAsync'

export function AppPasswordsPage() {
  const state = useAsync(
    (signal) => appPasswords.list({ signal }),
    'app-passwords',
  )
  const [name, setName] = useState('')
  const [nameError, setNameError] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [createError, setCreateError] = useState<string | null>(null)
  const [issued, setIssued] = useState<{ token: string; label: string } | null>(null)

  const create = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setCreateError(null)
    setNameError(null)
    const validation = validateForm(createAppPasswordSchema, { name: name.trim() })
    if (!validation.ok) {
      setNameError(validation.errors.name ?? validation.formError)
      return
    }
    setCreating(true)
    void appPasswords.create({ name: validation.value.name }).then(
      (response) => {
        setCreating(false)
        setName('')
        setIssued({ token: response.token, label: response.app_password.name })
        state.reload()
      },
      (error: unknown) => {
        setCreating(false)
        setCreateError(
          isApiError(error)
            ? error.message
            : 'The app password could not be created.',
        )
      },
    )
  }

  const passwords: AppPassword[] = state.data ?? []

  return (
    <div className="space-y-4">
      <PageHeader
        title="App passwords"
        description="Registry credentials for the Docker CLI and CI. Each one signs in without your password or two-factor code."
      />

      {issued ? (
        <OneTimeToken
          token={issued.token}
          title={`App password for ${issued.label}`}
          onDismiss={() => setIssued(null)}
        />
      ) : null}

      <Box>
        <BoxHeader>
          <span className="flex items-center gap-2 text-sm font-medium">
            <KeyIcon size={16} aria-hidden="true" />
            New app password
          </span>
        </BoxHeader>
        <BoxBody>
          <form className="space-y-3" onSubmit={create} noValidate>
            {createError ? (
              <Flash variant="danger" onDismiss={() => setCreateError(null)}>
                {createError}
              </Flash>
            ) : null}
            <TextInput
              label="Name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="laptop"
              error={nameError}
              required
            />
            <Button type="submit" variant="primary" disabled={creating}>
              {creating ? 'Creating…' : 'Create app password'}
            </Button>
          </form>
        </BoxBody>
      </Box>

      {state.loading && !state.data ? (
        <LoadingState label="Loading app passwords…" />
      ) : null}
      {state.error ? <ErrorState error={state.error} onRetry={state.reload} /> : null}

      {state.data ? (
        passwords.length === 0 ? (
          <EmptyState
            title="No app passwords"
            description="Create one above to run docker login without your account password or two-factor code."
            icon={<KeyIcon size={24} aria-hidden="true" />}
          />
        ) : (
          <div className="space-y-4">
            {passwords.map((appPassword) => (
              <AppPasswordCard
                key={appPassword.id}
                appPassword={appPassword}
                onChanged={state.reload}
                onToken={(token, label) => setIssued({ token, label })}
              />
            ))}
          </div>
        )
      ) : null}
    </div>
  )
}
