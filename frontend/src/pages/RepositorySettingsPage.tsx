import { useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'

import { grants, permissions, repositories as repositoriesApi } from '../api/endpoints'
import { DelegationsPanel } from '../components/DelegationsPanel'
import { PageHeader } from '../components/PageHeader'
import { RepositoryGeneralPanel } from '../components/RepositoryGeneralPanel'
import { TabNav, TabPanel } from '../components/Tabs'
import { ErrorState, LoadingState } from '../components/primitives/StateViews'
import { repoRoute } from '../lib/paths'
import { useAsync } from '../lib/useAsync'

export type RepositorySettingsPageProps = {
  namespace: string
  repo: string
}

type TabId = 'general' | 'delegations'

const TABS: ReadonlyArray<{ id: TabId; label: string }> = [
  { id: 'general', label: 'General' },
  { id: 'delegations', label: 'Delegations' },
]

function isTab(value: string): value is TabId {
  return TABS.some((tab) => tab.id === value)
}

export function RepositorySettingsPage({
  namespace,
  repo,
}: RepositorySettingsPageProps) {
  const navigate = useNavigate()
  const [tab, setTab] = useState<TabId>('general')

  const state = useAsync(
    (signal) => repositoriesApi.detail(namespace, repo, { signal }),
    `repo-settings:${namespace}:${repo}`,
  )

  const detail = state.data

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
            / <span>Settings</span>
          </>
        }
        title={<span className="font-mono">{repo} settings</span>}
        description={detail?.description ?? undefined}
      />

      {state.loading && !detail ? (
        <LoadingState label="Loading repository…" />
      ) : null}
      {state.error ? <ErrorState error={state.error} onRetry={state.reload} /> : null}

      {detail ? (
        <>
          <TabNav
            label="Repository settings sections"
            tabs={TABS.map((item) => ({ id: item.id, label: item.label }))}
            active={tab}
            onChange={(id) => {
              if (isTab(id)) setTab(id)
            }}
          />

          <TabPanel id="general" active={tab}>
            <RepositoryGeneralPanel
              namespace={namespace}
              repo={repo}
              detail={detail}
              onChanged={state.reload}
              onDeleted={() => navigate(`/${encodeURIComponent(namespace)}`)}
            />
          </TabPanel>

          <TabPanel id="delegations" active={tab}>
            <DelegationsPanel
              title="Image permissions"
              resourceName={`${namespace}/${repo}`}
              reloadKey={`repository:${namespace}:${repo}`}
              load={(signal) => grants.onRepository(namespace, repo, { signal })}
              add={(body) => permissions.addToRepository(namespace, repo, body)}
              remove={(id) => permissions.revokeOnRepository(namespace, repo, id)}
            />
          </TabPanel>
        </>
      ) : null}
    </div>
  )
}
