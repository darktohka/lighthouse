import { FileIcon, ShieldCheckIcon } from '@primer/octicons-react'
import { useState } from 'react'
import { Link } from 'react-router-dom'

import { repositories as repositoriesApi } from '../api/endpoints'
import type {
  Attestation,
  LayerInfo,
  PlatformDetail,
  TagDetail,
} from '../api/schemas'
import { CopyButton } from '../components/CopyButton'
import { JsonViewer } from '../components/json-viewer'
import { PageHeader } from '../components/PageHeader'
import { PlatformBadges } from '../components/PlatformBadge'
import { Box } from '../components/primitives/Box'
import { Button, LinkButton } from '../components/primitives/Button'
import { Label } from '../components/primitives/Label'
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from '../components/primitives/StateViews'
import { Table, type TableColumn } from '../components/primitives/Table'
import { cx } from '../lib/cx'
import {
  formatBytes,
  formatDateTime,
  formatNumber,
  formatPlatform,
  formatRelativeTime,
  shortDigest,
} from '../lib/format'
import { layerRoute, repoRoute } from '../lib/paths'
import { useAsync } from '../lib/useAsync'

export type TagPageProps = {
  namespace: string
  repo: string
  tag: string
}

type TabId = 'manifest' | 'config' | 'layers' | 'attestations'

const TABS: ReadonlyArray<{ id: TabId; label: string }> = [
  { id: 'layers', label: 'Layers' },
  { id: 'attestations', label: 'Attestations' },
  { id: 'manifest', label: 'Manifest' },
  { id: 'config', label: 'Config' },
]

function defaultPlatformDigest(
  details: readonly PlatformDetail[],
): string | null {
  const preferred = details.find(
    (item) => item.os === 'linux' && item.architecture === 'amd64',
  )
  return (preferred ?? details[0])?.digest ?? null
}

function owningPlatformDigest(
  detail: TagDetail,
  scope: PlatformDetail | null,
  layer: LayerInfo,
): string {
  const matches = detail.platform_details.filter((item) =>
    item.layers.some(
      (edge) => edge.digest === layer.digest && edge.role === layer.role,
    ),
  )
  const scoped =
    scope === null
      ? undefined
      : matches.find((item) => item.digest === scope.digest)
  return (scoped ?? matches[0])?.digest ?? detail.digest
}

/** Short labels for one attestation, falling back to its predicates' kinds. */
function attestationKinds(attestation: Attestation): string[] {
  return attestation.kinds.length > 0
    ? attestation.kinds
    : attestation.predicates.map((predicate) => predicate.kind)
}

/** `linux/arm64` for the attestation's target, or `''` when it is unknown. */
function attestationPlatformLabel(attestation: Attestation): string {
  const target = attestation.target
  if (target === null) return ''
  return formatPlatform(target.os, target.architecture, target.variant)
}

/**
 * Short labels of the attestations that describe one image manifest.
 *
 * Attestation manifests belong to a specific platform, so the platform scope
 * buttons read `linux/arm64 (provenance, sbom)`. An attestation whose target is
 * unknown is left out of every platform.
 */
function attestationKindsFor(
  attestations: readonly Attestation[],
  digest: string,
): string[] {
  const kinds = new Set<string>()
  for (const attestation of attestations) {
    if (attestation.target?.digest !== digest) continue
    for (const label of attestationKinds(attestation)) kinds.add(label)
  }
  return [...kinds]
}

function tabClass(active: boolean): string {
  return cx(
    '-mb-px border-b-2 px-3 py-1.5 text-sm font-medium',
    active
      ? 'border-accent text-foreground'
      : 'border-transparent text-muted hover:text-foreground',
  )
}

export function TagPage({ namespace, repo, tag }: TagPageProps) {
  const [tab, setTab] = useState<TabId>('layers')
  const [platform, setPlatform] = useState<string | null>(null)
  const state = useAsync(
    (signal) => repositoriesApi.tag(namespace, repo, tag, { signal }),
    `tag:${namespace}:${repo}:${tag}`,
  )
  const detail = state.data
  const host = typeof window === 'undefined' ? '' : window.location.host
  const pullCommand = `docker pull ${host}/${namespace}/${repo}:${tag}`

  function browseConfig(digest: string) {
    const owner = detail?.platform_details.find((item) =>
      item.layers.some((edge) => edge.role === 'config' && edge.digest === digest),
    )
    if (owner) setPlatform(owner.digest)
    setTab('config')
  }

  function renderTagDetail(detail: TagDetail) {
    const platformDetails = detail.platform_details
    const scope =
      platform ??
      (platformDetails.length > 1
        ? defaultPlatformDigest(platformDetails)
        : 'all')
    const active =
      scope === 'all'
        ? null
        : (platformDetails.find((p) => p.digest === scope) ?? null)
    const viewManifest = active?.manifest ?? detail.manifest
    const viewConfig = active ? active.config : detail.config
    const viewLayers = active ? active.layers : detail.layers
    const layersCaption = active
      ? `Layers for ${formatPlatform(active.os, active.architecture, active.variant)}`
      : 'Layers referenced by this tag'

    // Attestation labels per platform, used by the platform badges so a tag with
    // attestations reads `linux/arm64 (provenance)` even when it has a single
    // platform (and therefore no scope button row).
    const platformKinds = new Map<string, string[]>()
    for (const item of platformDetails) {
      const kinds = attestationKindsFor(detail.attestations, item.digest)
      if (kinds.length > 0) platformKinds.set(item.digest, kinds)
    }

    // Attestations follow the platform scope the same way layers do: a
    // selected platform lists only the attestations that describe it.
    const viewAttestations =
      active === null
        ? detail.attestations.toSorted((left, right) => {
            // Attestations with an unknown target sort last.
            const leftLabel = attestationPlatformLabel(left)
            const rightLabel = attestationPlatformLabel(right)
            if (leftLabel !== rightLabel) {
              if (leftLabel.length === 0) return 1
              if (rightLabel.length === 0) return -1
              return leftLabel.localeCompare(rightLabel)
            }
            return left.digest.localeCompare(right.digest)
          })
        : detail.attestations.filter(
            (attestation) => attestation.target?.digest === active.digest,
          )
    const activePlatformLabel =
      active === null
        ? null
        : formatPlatform(active.os, active.architecture, active.variant)
    const attestationsCaption =
      activePlatformLabel === null
        ? 'in-toto attestations attached to this tag'
        : `in-toto attestations attached to ${activePlatformLabel}`
    const attestationsEmptyDescription =
      activePlatformLabel === null
        ? 'No in-toto attestation manifests are attached to this tag.'
        : `No in-toto attestation manifests are attached to ${activePlatformLabel}.`

    const attestationColumns: TableColumn<Attestation>[] = [
      {
        key: 'platform',
        header: 'Platform',
        render: (attestation) => {
          const label = attestationPlatformLabel(attestation)
          if (label.length === 0) return <span className="text-muted">-</span>
          return (
            <span className="whitespace-nowrap font-mono text-xs">{label}</span>
          )
        },
      },
      {
        key: 'type',
        header: 'Type',
        render: (attestation) => {
          const kinds = attestationKinds(attestation)
          if (kinds.length === 0) return <span className="text-muted">-</span>
          return (
            <span className="flex flex-wrap items-center gap-1">
              {kinds.map((kind, index) => (
                <Label
                  key={`${kind}:${index}`}
                  variant={index === 0 ? 'accent' : 'muted'}
                >
                  {kind}
                </Label>
              ))}
            </span>
          )
        },
      },
      {
        key: 'predicate',
        header: 'Predicate',
        render: (attestation) => {
          const predicateType =
            attestation.predicates[0]?.predicate_type ?? null
          if (predicateType === null) return <span className="text-muted">-</span>
          return (
            <span
              className="block max-w-[22rem] truncate font-mono text-xs"
              title={predicateType}
            >
              {predicateType}
            </span>
          )
        },
      },
      {
        key: 'digest',
        header: 'Digest',
        render: (attestation) => (
          <span className="font-mono text-xs" title={attestation.digest}>
            {shortDigest(attestation.digest)}
          </span>
        ),
      },
      {
        key: 'size',
        header: 'Size',
        align: 'right',
        render: (attestation) => formatBytes(attestation.size),
      },
      {
        key: 'actions',
        header: <span className="sr-only">Actions</span>,
        align: 'right',
        render: (attestation) => {
          const statementDigest = attestation.predicates[0]?.digest
          // The app has no blob-viewer route; the statement is a blob the
          // registry API can serve, so expose its digest for copy/paste.
          if (statementDigest === undefined) {
            return <span className="text-muted">-</span>
          }
          return (
            <span className="flex items-center justify-end gap-1.5">
              <span
                className="font-mono text-xs text-muted"
                title={statementDigest}
              >
                {shortDigest(statementDigest)}
              </span>
              <CopyButton
                value={statementDigest}
                label="Copy"
                ariaLabel="Copy statement digest"
              />
            </span>
          )
        },
      },
    ]

    const layerColumns: TableColumn<LayerInfo>[] = [
      {
        key: 'role',
        header: 'Role',
        render: (layer) => (
          <Label variant={layer.role === 'config' ? 'accent' : 'muted'}>
            {layer.role}
          </Label>
        ),
      },
      {
        key: 'digest',
        header: 'Digest',
        render: (layer) => (
          <span className="font-mono text-xs" title={layer.digest}>
            {shortDigest(layer.digest)}
          </span>
        ),
      },
      {
        key: 'media_type',
        header: 'Media type',
        render: (layer) => (
          <span className="break-all font-mono text-xs">{layer.media_type}</span>
        ),
      },
      {
        key: 'size',
        header: 'Size',
        align: 'right',
        render: (layer) => formatBytes(layer.size),
      },
      {
        key: 'created',
        header: 'Created',
        render: (layer) =>
          layer.created !== null ? (
            <span
              className="whitespace-nowrap text-xs text-muted"
              title={layer.created}
            >
              {formatDateTime(layer.created)}
            </span>
          ) : (
            <span className="text-muted">-</span>
          ),
      },
      {
        key: 'comment',
        header: 'Comment',
        render: (layer) => {
          const comment = layer.comment?.trim() ?? ''
          if (comment.length === 0) return <span className="text-muted">-</span>
          return (
            <span
              className="block max-w-[20rem] truncate"
              title={layer.comment ?? undefined}
            >
              {comment}
            </span>
          )
        },
      },
      {
        key: 'actions',
        header: <span className="sr-only">Actions</span>,
        align: 'right',
        render: (layer) =>
          layer.role === 'config' ? (
            <Button size="sm" onClick={() => browseConfig(layer.digest)}>
              Browse
            </Button>
          ) : (
            <LinkButton
              size="sm"
              to={layerRoute(namespace, repo, layer.digest, {
                manifest: owningPlatformDigest(detail, active, layer),
              })}
            >
              Browse
            </LinkButton>
          ),
      },
    ]

    return (
      <>
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
          <Box className="p-3">
            <p className="text-xs text-muted">Compressed</p>
            <p className="mt-1 text-lg font-semibold">
              {formatBytes(detail.compressed_size)}
            </p>
          </Box>
          <Box className="p-3">
            <p className="text-xs text-muted">Pulls</p>
            <p className="mt-1 text-lg font-semibold">
              {formatNumber(detail.pull_count)}
            </p>
          </Box>
          <Box className="p-3">
            <p className="text-xs text-muted">Platforms</p>
            <p className="mt-1 text-lg font-semibold">
              {detail.platforms.length}
            </p>
          </Box>
          <Box className="p-3">
            <p className="text-xs text-muted">Attestations</p>
            <p className="mt-1 text-lg font-semibold">
              {detail.attestations.length}
            </p>
          </Box>
        </div>

        {detail.can_pull ? (
          <div className="flex flex-col gap-2 rounded-md border border-border bg-canvas-subtle p-3 sm:flex-row sm:items-center sm:justify-between">
            <code className="break-all font-mono text-xs">{pullCommand}</code>
            <CopyButton value={pullCommand} label="Copy pull command" />
          </div>
        ) : null}

        <Box className="p-3">
          <dl className="grid gap-2 text-xs sm:grid-cols-2">
            <div>
              <dt className="text-muted">Digest</dt>
              <dd className="break-all font-mono">{detail.digest}</dd>
            </div>
            <div>
              <dt className="text-muted">Media type</dt>
              <dd className="break-all font-mono">{detail.media_type}</dd>
            </div>
            <div>
              <dt className="text-muted">Updated</dt>
              <dd>{formatDateTime(detail.updated_at)}</dd>
            </div>
            <div>
              <dt className="text-muted">Platforms</dt>
              <dd className="mt-1">
                <PlatformBadges
                  platforms={detail.platforms}
                  limit={8}
                  kindsByDigest={platformKinds}
                />
              </dd>
            </div>
          </dl>
        </Box>

        <div>
          {platformDetails.length > 1 ? (
            <div
              role="group"
              aria-label="Platform scope"
              className="mb-2 flex flex-wrap items-center gap-1.5"
            >
              <Button
                size="sm"
                variant={scope === 'all' ? 'primary' : 'default'}
                aria-pressed={scope === 'all'}
                title="Every platform and attestation in this tag"
                onClick={() => setPlatform('all')}
              >
                All platforms
              </Button>
              {platformDetails.map((item) => {
                const label = formatPlatform(
                  item.os,
                  item.architecture,
                  item.variant,
                )
                const kinds = attestationKindsFor(
                  detail.attestations,
                  item.digest,
                )
                const selected = scope === item.digest
                const title =
                  kinds.length > 0
                    ? `${label} · ${formatBytes(item.size)} · ${kinds.join(', ')}`
                    : `${label} · ${formatBytes(item.size)}`
                return (
                  <Button
                    key={item.digest}
                    size="sm"
                    variant={selected ? 'primary' : 'default'}
                    aria-pressed={selected}
                    title={title}
                    onClick={() => setPlatform(item.digest)}
                  >
                    <span className="font-mono">{label}</span>
                    {kinds.length > 0 ? (
                      <span
                        className={cx(
                          'text-xs font-normal',
                          selected ? 'opacity-75' : 'text-muted',
                        )}
                      >
                        ({kinds.join(', ')})
                      </span>
                    ) : null}
                  </Button>
                )
              })}
            </div>
          ) : null}

          <div
            role="tablist"
            aria-label="Tag details"
            className="flex gap-1 border-b border-border"
          >
            {TABS.map((item) => (
              <button
                key={item.id}
                type="button"
                role="tab"
                id={`tab-${item.id}`}
                aria-selected={tab === item.id}
                aria-controls={`panel-${item.id}`}
                onClick={() => setTab(item.id)}
                className={tabClass(tab === item.id)}
              >
                {item.label}
              </button>
            ))}
          </div>

          <div
            role="tabpanel"
            id={`panel-${tab}`}
            aria-labelledby={`tab-${tab}`}
            tabIndex={0}
            className="pt-4"
          >
            {tab === 'manifest' ? (
              <JsonViewer data={viewManifest} defaultExpandDepth={3} />
            ) : null}

            {tab === 'config' ? (
              viewConfig ? (
                <JsonViewer data={viewConfig} defaultExpandDepth={2} />
              ) : (
                <EmptyState
                  title="No config blob"
                  description="This manifest does not reference a configuration blob."
                  icon={<FileIcon size={24} aria-hidden="true" />}
                />
              )
            ) : null}

            {tab === 'layers' ? (
              viewLayers.length === 0 ? (
                <EmptyState
                  title="No layers"
                  description="This manifest does not list any layers."
                  icon={<FileIcon size={24} aria-hidden="true" />}
                />
              ) : (
                <Box>
                  <Table
                    columns={layerColumns}
                    rows={viewLayers}
                    rowKey={(layer) => `${layer.role}:${layer.digest}`}
                    caption={layersCaption}
                  />
                </Box>
              )
            ) : null}

            {tab === 'attestations' ? (
              viewAttestations.length === 0 ? (
                <EmptyState
                  title="No attestations"
                  description={attestationsEmptyDescription}
                  icon={<ShieldCheckIcon size={24} aria-hidden="true" />}
                />
              ) : (
                <Box>
                  <Table
                    columns={attestationColumns}
                    rows={viewAttestations}
                    rowKey={(attestation) => attestation.digest}
                    caption={attestationsCaption}
                  />
                </Box>
              )
            ) : null}
          </div>
        </div>
      </>
    )
  }

  return (
    <div className="space-y-4">
      <PageHeader
        breadcrumbs={
          <>
            <Link to="/" className="hover:text-accent hover:underline">
              Explore
            </Link>{' '}
            /{' '}
            <Link
              to={`/${encodeURIComponent(namespace)}`}
              className="hover:text-accent hover:underline"
            >
              {namespace}
            </Link>{' '}
            /{' '}
            <Link
              to={repoRoute(namespace, repo)}
              className="font-mono hover:text-accent hover:underline"
            >
              {repo}
            </Link>{' '}
            / <span className="font-mono">{tag}</span>
          </>
        }
        title={<span className="font-mono">{tag}</span>}
        description={
          detail
            ? `Updated ${formatRelativeTime(detail.updated_at)} · ${formatNumber(detail.pull_count)} pulls`
            : undefined
        }
      />

      {state.loading && !detail ? (
        <LoadingState label="Loading tag…" />
      ) : null}
      {state.error ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}

      {detail ? renderTagDetail(detail) : null}
    </div>
  )
}
