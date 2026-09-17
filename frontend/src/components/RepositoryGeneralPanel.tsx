import { AlertIcon } from '@primer/octicons-react'
import { useState } from 'react'

import { isApiError } from '../api/client'
import { repositories as repositoriesApi } from '../api/endpoints'
import type { RepositoryDetail } from '../api/schemas'
import { ConfirmAction } from './ConfirmAction'
import { SwitchField, TextArea } from './FormFields'
import { Box, BoxBody, BoxHeader } from './primitives/Box'
import { Button } from './primitives/Button'
import { Flash } from './primitives/Flash'

export type RepositoryGeneralPanelProps = {
  namespace: string
  repo: string
  detail: RepositoryDetail
  onChanged: () => void
  onDeleted: () => void
}

export function RepositoryGeneralPanel({
  namespace,
  repo,
  detail,
  onChanged,
  onDeleted,
}: RepositoryGeneralPanelProps) {
  const [description, setDescription] = useState(detail.description ?? '')
  const [isPublic, setIsPublic] = useState(detail.is_public)
  const [saving, setSaving] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [saved, setSaved] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const path = detail.path.length > 0 ? detail.path : repo

  const save = () => {
    setActionError(null)
    setSaved(null)
    setSaving(true)
    void repositoriesApi
      .update(namespace, repo, { description: description.trim(), is_public: isPublic })
      .then(
        () => {
          setSaving(false)
          setSaved('Repository settings saved.')
          onChanged()
        },
        (error: unknown) => {
          setSaving(false)
          setActionError(
            isApiError(error)
              ? error.message
              : 'The repository could not be saved.',
          )
        },
      )
  }

  const remove = () => {
    setActionError(null)
    setDeleting(true)
    void repositoriesApi.remove(namespace, repo).then(
      () => {
        setDeleting(false)
        onDeleted()
      },
      (error: unknown) => {
        setDeleting(false)
        setActionError(
          isApiError(error)
            ? error.message
            : 'The image could not be deleted.',
        )
      },
    )
  }

  return (
    <div className="space-y-4">
      <Box>
        <BoxHeader>
          <span className="text-sm font-medium">General</span>
        </BoxHeader>
        <BoxBody className="space-y-3">
          {actionError ? (
            <Flash variant="danger" onDismiss={() => setActionError(null)}>
              {actionError}
            </Flash>
          ) : null}
          {saved ? (
            <Flash variant="success" onDismiss={() => setSaved(null)}>
              {saved}
            </Flash>
          ) : null}
          <TextArea
            label="Description"
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            hint="Shown on the repository page and in Explore."
          />
          <SwitchField
            label="Public repository"
            hint="Public repositories are readable without authentication."
            checked={isPublic}
            onCheckedChange={setIsPublic}
          />
          <Button variant="primary" disabled={saving} onClick={save}>
            {saving ? 'Saving…' : 'Save changes'}
          </Button>
        </BoxBody>
      </Box>

      <Box className="border-danger">
        <BoxHeader className="border-danger bg-danger-subtle">
          <span className="flex items-center gap-2 text-sm font-medium text-danger">
            <AlertIcon size={16} aria-hidden="true" />
            Danger zone
          </span>
        </BoxHeader>
        <BoxBody className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <p className="text-sm font-medium">Delete this image</p>
            <p className="text-xs text-muted">
              Deleting <span className="font-mono">{path}</span> removes every tag
              and reclaims unreferenced blobs. This cannot be undone.
            </p>
          </div>
          <ConfirmAction
            label="Delete image"
            confirmLabel="Delete image"
            resourceName={path}
            pending={deleting}
            size="md"
            onConfirm={remove}
          />
        </BoxBody>
      </Box>
    </div>
  )
}
