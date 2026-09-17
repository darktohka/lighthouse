import { useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'

import { grants, namespaces as namespacesApi, permissions } from '../api/endpoints'
import { DelegationsPanel } from '../components/DelegationsPanel'
import { MembersPanel } from '../components/MembersPanel'
import { NamespaceGeneralPanel } from '../components/NamespaceGeneralPanel'
import { PageHeader } from '../components/PageHeader'
import { TabNav, TabPanel } from '../components/Tabs'
import { Avatar } from '../components/primitives/Avatar'
import { Label } from '../components/primitives/Label'
import { ErrorState, LoadingState } from '../components/primitives/StateViews'
import { useAuth } from '../lib/auth-context'
import { useAsync } from '../lib/useAsync'
import { NotFoundPage } from './NotFoundPage'

type TabId = 'general' | 'members' | 'delegations'

const TABS: ReadonlyArray<{ id: TabId; label: string }> = [
  { id: 'general', label: 'General' },
  { id: 'members', label: 'Members' },
  { id: 'delegations', label: 'Delegations' },
]

function isTab(value: string): value is TabId {
  return TABS.some((tab) => tab.id === value)
}

export function NamespaceSettingsPage() {
  const params = useParams()
  const name = params.name ?? ''
  const navigate = useNavigate()
  const { user } = useAuth()
  const [tab, setTab] = useState<TabId>('general')

  const state = useAsync(
    (signal) =>
      name ? namespacesApi.detail(name, { signal }) : Promise.resolve(null),
    `namespace-settings:${name}`,
  )

  if (name.length === 0) return <NotFoundPage />

  const detail = state.data
  const canManage = Boolean(
    detail && user && detail.owner?.username === user.username,
  )

  return (
    <div className="space-y-4">
      <PageHeader
        breadcrumbs={
          <>
            <Link
              to={`/${encodeURIComponent(name)}`}
              className="hover:text-accent hover:underline"
            >
              {name}
            </Link>{' '}
            / <span>Settings</span>
          </>
        }
        title={<span className="font-mono">{name} settings</span>}
        description={detail?.description ?? undefined}
        actions={detail ? <Label variant="muted">{detail.kind}</Label> : null}
      />

      {detail ? (
        <div className="flex items-center gap-2 text-sm text-muted">
          {detail.owner ? (
            <>
              <Avatar
                src={detail.owner.avatar_url}
                name={detail.owner.username}
                size={20}
              />
              <span>@{detail.owner.username}</span>
            </>
          ) : null}
          <span>{detail.repository_count} repositories</span>
        </div>
      ) : null}

      {state.loading && !detail ? (
        <LoadingState label="Loading workspace…" />
      ) : null}
      {state.error ? <ErrorState error={state.error} onRetry={state.reload} /> : null}

      {detail ? (
        <>
          <TabNav
            label="Workspace settings sections"
            tabs={TABS.map((item) => ({ id: item.id, label: item.label }))}
            active={tab}
            onChange={(id) => {
              if (isTab(id)) setTab(id)
            }}
          />

          <TabPanel id="general" active={tab}>
            <NamespaceGeneralPanel
              namespace={detail}
              canManage={canManage}
              onChanged={state.reload}
              onDeleted={() => navigate('/dashboard')}
            />
          </TabPanel>

          <TabPanel id="members" active={tab}>
            <MembersPanel namespace={name} canManage={canManage} />
          </TabPanel>

          <TabPanel id="delegations" active={tab}>
            <DelegationsPanel
              title="Namespace permissions"
              resourceName={name}
              reloadKey={`namespace:${name}`}
              load={(signal) => grants.onNamespace(name, { signal })}
              add={(body) => permissions.addToNamespace(name, body)}
              remove={(id) => permissions.revokeOnNamespace(name, id)}
            />
          </TabPanel>
        </>
      ) : null}
    </div>
  )
}
