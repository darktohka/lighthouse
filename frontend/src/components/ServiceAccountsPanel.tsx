import { KeyIcon } from '@primer/octicons-react'
import { useState, type FormEvent } from 'react'

import { isApiError } from '../api/client'
import { serviceAccountList, serviceAccounts as serviceAccountsApi } from '../api/endpoints'
import type { ServiceAccount } from '../api/schemas'
import { OneTimeToken } from './OneTimeToken'
import { ServiceAccountCard } from './ServiceAccountCard'
import { Box, BoxBody, BoxHeader } from './primitives/Box'
import { Button } from './primitives/Button'
import { Flash } from './primitives/Flash'
import { TextInput } from './primitives/TextInput'
import { EmptyState, ErrorState, LoadingState } from './primitives/StateViews'
import { useAsync } from '../lib/useAsync'

export function ServiceAccountsPanel() {
  const state = useAsync(
    (signal) => serviceAccountList.list({ signal }),
    'service-accounts',
  )
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [ipRanges, setIpRanges] = useState('')
  const [creating, setCreating] = useState(false)
  const [createError, setCreateError] = useState<string | null>(null)
  const [issued, setIssued] = useState<{ token: string; label: string } | null>(null)

  const create = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setCreateError(null)
    if (name.trim().length === 0) {
      setCreateError('Give the service account a name.')
      return
    }
    setCreating(true)
    const ranges = ipRanges
      .split(/[\s,]+/)
      .map((value) => value.trim())
      .filter((value) => value.length > 0)
    void serviceAccountsApi
      .create({
        name: name.trim(),
        ...(description.trim() ? { description: description.trim() } : {}),
        ...(ranges.length > 0 ? { ip_ranges: ranges } : {}),
      })
      .then(
        (response) => {
          setCreating(false)
          setName('')
          setDescription('')
          setIpRanges('')
          setIssued({ token: response.token, label: response.account.name })
          state.reload()
        },
        (error: unknown) => {
          setCreating(false)
          setCreateError(
            isApiError(error)
              ? error.message
              : 'The service account could not be created.',
          )
        },
      )
  }

  const accounts: ServiceAccount[] = state.data ?? []

  return (
    <div className="space-y-4">
      {issued ? (
        <OneTimeToken
          token={issued.token}
          title={`Token for ${issued.label}`}
          onDismiss={() => setIssued(null)}
        />
      ) : null}

      <Box>
        <BoxHeader>
          <span className="flex items-center gap-2 text-sm font-medium">
            <KeyIcon size={16} aria-hidden="true" />
            New service account
          </span>
        </BoxHeader>
        <BoxBody>
          <form className="space-y-3" onSubmit={create} noValidate>
            {createError ? (
              <Flash variant="danger" onDismiss={() => setCreateError(null)}>
                {createError}
              </Flash>
            ) : null}
            <div className="grid gap-3 sm:grid-cols-2">
              <TextInput
                label="Name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="ci-deploy"
                required
              />
              <TextInput
                label="Description"
                value={description}
                onChange={(event) => setDescription(event.target.value)}
                placeholder="Deploys production from GitHub Actions"
              />
              <div className="sm:col-span-2">
                <TextInput
                  label="Allowed IP ranges"
                  value={ipRanges}
                  onChange={(event) => setIpRanges(event.target.value)}
                  placeholder="203.0.113.4, 10.0.0.0/8"
                  hint="Optional. Comma- or space-separated addresses/CIDRs; leave empty to allow any address."
                />
              </div>
            </div>
            <Button type="submit" variant="primary" disabled={creating}>
              {creating ? 'Creating…' : 'Create service account'}
            </Button>
          </form>
        </BoxBody>
      </Box>

      {state.loading && !state.data ? (
        <LoadingState label="Loading service accounts…" />
      ) : null}
      {state.error ? <ErrorState error={state.error} onRetry={state.reload} /> : null}

      {state.data ? (
        accounts.length === 0 ? (
          <EmptyState
            title="No service accounts"
            description="Create one above to push or pull images from CI without a password."
            icon={<KeyIcon size={24} aria-hidden="true" />}
          />
        ) : (
          <div className="space-y-4">
            {accounts.map((account) => (
              <ServiceAccountCard
                key={account.id}
                account={account}
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
