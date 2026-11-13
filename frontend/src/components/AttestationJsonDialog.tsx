import { useMemo, useState } from 'react'

import { blobs, manifests } from '../api/endpoints'
import type { Attestation } from '../api/schemas'
import { JsonViewer } from './json-viewer'
import { Dialog } from './primitives/Dialog'
import { ErrorState, LoadingState } from './primitives/StateViews'
import { cx } from '../lib/cx'
import { formatBytes, formatPlatform, shortDigest } from '../lib/format'
import { useAsync } from '../lib/useAsync'

/** Which document the dialog is showing. */
type DocumentId = 'manifest' | 'statement'

export type AttestationJsonDialogProps = {
  /** The attestation to display; `null` closes the dialog. */
  attestation: Attestation | null
  namespace: string
  repo: string
  onClose: () => void
}

/**
 * Full JSON for one attestation.
 *
 * BuildKit attestations are two levels deep: the attestation manifest (the
 * wrapper with `artifactType`/`subject`) and one in-toto statement blob per
 * predicate. Both are shown, each in the same {@link JsonViewer} the Manifest
 * and Config tabs use, so expanding, searching, copying and downloading behave
 * identically. Documents are fetched on demand and only when their tab is
 * selected.
 */
export function AttestationJsonDialog({
  attestation,
  namespace,
  repo,
  onClose,
}: AttestationJsonDialogProps) {
  const [document, setDocument] = useState<DocumentId>('manifest')

  if (attestation === null) return null

  return (
    <Dialog
      open
      onClose={onClose}
      title="Attestation"
      description={describe(attestation)}
      widthClassName="max-w-4xl"
    >
      <AttestationJsonBody
        attestation={attestation}
        namespace={namespace}
        repo={repo}
        document={document}
        onDocumentChange={setDocument}
      />
    </Dialog>
  )
}

/** One-line summary: platform, predicate kinds and digest. */
function describe(attestation: Attestation): string {
  const parts: string[] = []
  if (attestation.target !== null) {
    parts.push(
      formatPlatform(
        attestation.target.os,
        attestation.target.architecture,
        attestation.target.variant,
      ),
    )
  }
  if (attestation.kinds.length > 0) parts.push(attestation.kinds.join(', '))
  parts.push(shortDigest(attestation.digest))
  return parts.join(' · ')
}

type BodyProps = {
  attestation: Attestation
  namespace: string
  repo: string
  document: DocumentId
  onDocumentChange: (document: DocumentId) => void
}

/**
 * The dialog body, split out so the hooks below only run while an attestation
 * is actually selected (`AttestationJsonDialog` returns early otherwise).
 */
function AttestationJsonBody({
  attestation,
  namespace,
  repo,
  document,
  onDocumentChange,
}: BodyProps) {
  const statementDigest = attestation.predicates[0]?.digest ?? null
  const documents = useMemo(
    () =>
      [
        { id: 'manifest' as const, label: 'Attestation manifest', digest: attestation.digest },
        ...(statementDigest === null
          ? []
          : [
              {
                id: 'statement' as const,
                label: 'in-toto statement',
                digest: statementDigest,
              },
            ]),
      ] satisfies ReadonlyArray<{
        id: DocumentId
        label: string
        digest: string
      }>,
    [attestation.digest, statementDigest],
  )

  const active = documents.find((entry) => entry.id === document) ?? documents[0]

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <div
          role="group"
          aria-label="Attestation document"
          className="flex flex-wrap gap-1"
        >
          {documents.map((entry) => {
            const selected = entry.id === active.id
            return (
              <button
                key={entry.id}
                type="button"
                aria-pressed={selected}
                onClick={() => onDocumentChange(entry.id)}
                className={cx(
                  'rounded-md border px-2 py-1 text-xs font-medium',
                  selected
                    ? 'border-accent bg-accent-subtle text-foreground'
                    : 'border-border text-muted hover:text-foreground',
                )}
              >
                {entry.label}
              </button>
            )
          })}
        </div>
        <span
          className="font-mono text-xs text-muted"
          title={active.digest}
        >
          {shortDigest(active.digest)}
        </span>
        <span className="text-xs text-muted">
          {formatBytes(
            active.id === 'manifest'
              ? attestation.size
              : (attestation.predicates[0]?.size ?? 0),
          )}
        </span>
      </div>

      <DocumentView
        namespace={namespace}
        repo={repo}
        document={active.id}
        digest={active.digest}
      />
    </div>
  )
}

type DocumentViewProps = {
  namespace: string
  repo: string
  document: DocumentId
  digest: string
}

/**
 * Loads and renders one document.
 *
 * The attestation manifest is read through the repository-scoped control-plane
 * endpoint so an attestation is only readable by callers who may pull the
 * repository; the statement is an ordinary blob.
 */
function DocumentView({
  namespace,
  repo,
  document,
  digest,
}: DocumentViewProps) {
  const state = useAsync(
    (signal) =>
      document === 'manifest'
        ? manifests.raw(namespace, repo, digest, { signal })
        : blobs.json(digest, { signal }),
    `${document}:${namespace}:${repo}:${digest}`,
  )

  if (state.loading && state.data === null) {
    return <LoadingState label="Loading JSON…" />
  }
  if (state.error) {
    return <ErrorState error={state.error} onRetry={state.reload} />
  }
  if (state.data === null) {
    return (
      <p className="py-8 text-center text-sm text-muted">
        This document could not be loaded.
      </p>
    )
  }
  return (
    <JsonViewer
      data={state.data}
      defaultExpandDepth={document === 'manifest' ? 3 : 2}
      downloadFileName={`${document}-${digest.replace(/[^a-z0-9]/gi, '-')}.json`}
    />
  )
}
