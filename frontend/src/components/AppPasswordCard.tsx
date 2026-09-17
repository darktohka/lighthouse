import { KeyIcon } from '@primer/octicons-react'
import { useState } from 'react'

import { isApiError } from '../api/client'
import { appPasswords } from '../api/endpoints'
import type { AppPassword } from '../api/schemas'
import { formatDateTime, formatRelativeTime } from '../lib/format'
import { ConfirmAction } from './ConfirmAction'
import { Box, BoxBody, BoxHeader } from './primitives/Box'
import { Flash } from './primitives/Flash'

export type AppPasswordCardProps = {
  appPassword: AppPassword
  onChanged: () => void
  onToken: (token: string, label: string) => void
}

export function AppPasswordCard({
  appPassword,
  onChanged,
  onToken,
}: AppPasswordCardProps) {
  const [busy, setBusy] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const fail = (error: unknown, fallback: string) => {
    setBusy(null)
    setActionError(isApiError(error) ? error.message : fallback)
  }

  const rotate = () => {
    setActionError(null)
    setBusy('rotate')
    void appPasswords.rotate(appPassword.id).then(
      (response) => {
        setBusy(null)
        onToken(response.token, response.app_password.name)
        onChanged()
      },
      (error: unknown) => fail(error, 'The app password could not be regenerated.'),
    )
  }

  const remove = () => {
    setActionError(null)
    setBusy('delete')
    void appPasswords.remove(appPassword.id).then(
      () => {
        setBusy(null)
        onChanged()
      },
      (error: unknown) => fail(error, 'The app password could not be deleted.'),
    )
  }

  return (
    <Box>
      <BoxHeader>
        <span className="flex items-center gap-2 text-sm font-medium">
          <KeyIcon size={16} aria-hidden="true" />
          {appPassword.name}
        </span>
        <span className="font-mono text-xs text-muted">
          {appPassword.token_prefix}…{appPassword.token_suffix}
        </span>
      </BoxHeader>
      <BoxBody className="space-y-3">
        {actionError ? (
          <Flash variant="danger" onDismiss={() => setActionError(null)}>
            {actionError}
          </Flash>
        ) : null}

        <dl className="grid gap-2 text-xs sm:grid-cols-3">
          <div>
            <dt className="text-muted">Created</dt>
            <dd>{formatDateTime(appPassword.created_at)}</dd>
          </div>
          <div>
            <dt className="text-muted">Last used</dt>
            <dd>
              {appPassword.last_used_at
                ? formatRelativeTime(appPassword.last_used_at)
                : 'Never'}
            </dd>
          </div>
        </dl>

        <div className="flex flex-wrap items-center gap-2 border-t border-border pt-3">
          <ConfirmAction
            label="Regenerate"
            confirmLabel="Regenerate secret"
            resourceName={`the secret for ${appPassword.name}`}
            pending={busy === 'rotate'}
            onConfirm={rotate}
          />
          <ConfirmAction
            label="Delete"
            confirmLabel="Delete app password"
            resourceName={appPassword.name}
            pending={busy === 'delete'}
            onConfirm={remove}
          />
        </div>
      </BoxBody>
    </Box>
  )
}
