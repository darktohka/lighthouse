import { AlertIcon } from '@primer/octicons-react'
import { useState } from 'react'

import { isApiError } from '../api/client'
import { namespaces as namespacesApi } from '../api/endpoints'
import type { Namespace } from '../api/schemas'
import { ConfirmAction } from './ConfirmAction'
import { SwitchField, TextArea } from './FormFields'
import { Box, BoxBody, BoxHeader } from './primitives/Box'
import { Button } from './primitives/Button'
import { Flash } from './primitives/Flash'

export type NamespaceGeneralPanelProps = {
  namespace: Namespace
  canManage: boolean
  onChanged: () => void
  onDeleted?: () => void
}

export function NamespaceGeneralPanel({
  namespace,
  canManage,
  onChanged,
  onDeleted,
}: NamespaceGeneralPanelProps) {
  const [description, setDescription] = useState(namespace.description ?? '')
  const [isPublic, setIsPublic] = useState(namespace.is_public)
  const [saving, setSaving] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [saved, setSaved] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const isWorkspace = namespace.kind === 'workspace'
  const subject = isWorkspace ? 'Workspace' : 'Namespace'
  const subjectLower = isWorkspace ? 'workspace' : 'namespace'

  const save = () => {
    setActionError(null)
    setSaved(null)
    setSaving(true)
    void namespacesApi
      .update(namespace.name, { description: description.trim(), is_public: isPublic })
      .then(
        () => {
          setSaving(false)
          setSaved(`${subject} settings saved.`)
          onChanged()
        },
        (error: unknown) => {
          setSaving(false)
          setActionError(
            isApiError(error) ? error.message : `The ${subjectLower} could not be saved.`,
          )
        },
      )
  }

  const remove = () => {
    setActionError(null)
    setDeleting(true)
    void namespacesApi.remove(namespace.name).then(
      () => {
        setDeleting(false)
        onDeleted?.()
      },
      (error: unknown) => {
        setDeleting(false)
        setActionError(
          isApiError(error) ? error.message : `The ${subjectLower} could not be deleted.`,
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

          {canManage ? (
            <>
              <TextArea
                label="Description"
                value={description}
                onChange={(event) => setDescription(event.target.value)}
                hint={
                  isWorkspace
                    ? 'Shown on the workspace page and in Explore.'
                    : 'Shown in Explore.'
                }
              />
              <SwitchField
                label={`Public ${subjectLower}`}
                hint={
                  isWorkspace
                    ? 'Public workspaces list their repositories in Explore.'
                    : 'Public namespaces list their repositories in Explore.'
                }
                checked={isPublic}
                onCheckedChange={setIsPublic}
              />
              <Button variant="primary" disabled={saving} onClick={save}>
                {saving ? 'Saving…' : 'Save changes'}
              </Button>
            </>
          ) : (
            <p className="text-sm text-muted">
              Only the {subjectLower} owner can change these settings.
            </p>
          )}
        </BoxBody>
      </Box>

      {namespace.kind === 'workspace' && canManage ? (
        <Box className="border-danger">
          <BoxHeader className="border-danger bg-danger-subtle">
            <span className="flex items-center gap-2 text-sm font-medium text-danger">
              <AlertIcon size={16} aria-hidden="true" />
              Danger zone
            </span>
          </BoxHeader>
          <BoxBody className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <p className="text-sm font-medium">Delete this workspace</p>
              <p className="text-xs text-muted">
                Deleting <span className="font-mono">{namespace.name}</span> removes
                its repositories, tags and grants. This cannot be undone.
              </p>
            </div>
            <ConfirmAction
              label="Delete workspace"
              confirmLabel="Delete workspace"
              resourceName={namespace.name}
              pending={deleting}
              size="md"
              onConfirm={remove}
            />
          </BoxBody>
        </Box>
      ) : null}
    </div>
  )
}
