import { useState } from 'react'

import { users as usersApi } from '../api/endpoints'
import { AvatarSettingsPanel } from '../components/AvatarSettingsPanel'
import { LoginHistoryPanel } from '../components/LoginHistoryPanel'
import { PageHeader } from '../components/PageHeader'
import { PasswordSettingsForm } from '../components/PasswordSettingsForm'
import { ProfileSettingsForm } from '../components/ProfileSettingsForm'
import { SessionsPanel } from '../components/SessionsPanel'
import { TabNav, TabPanel } from '../components/Tabs'
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from '../components/primitives/StateViews'
import { useAuth } from '../lib/auth-context'
import { useAsync } from '../lib/useAsync'

type SettingsTab = 'profile' | 'picture' | 'sessions' | 'logins' | 'password'

const TABS: ReadonlyArray<{ id: SettingsTab; label: string }> = [
  { id: 'profile', label: 'Profile' },
  { id: 'picture', label: 'Profile picture' },
  { id: 'sessions', label: 'Sessions' },
  { id: 'logins', label: 'Login history' },
  { id: 'password', label: 'Password' },
]

function isSettingsTab(value: string): value is SettingsTab {
  return TABS.some((item) => item.id === value)
}

export function SettingsPage() {
  const { user } = useAuth()
  const [tab, setTab] = useState<SettingsTab>('profile')

  const profileState = useAsync(
    (signal) =>
      user ? usersApi.profile(user.username, { signal }) : Promise.resolve(null),
    `settings-profile:${user?.username ?? 'anonymous'}`,
  )

  const profile = profileState.data

  return (
    <div className="space-y-4">
      <PageHeader
        title="Settings"
        description="Manage your profile, avatar, sessions, security and password."
      />

      <TabNav
        label="Settings sections"
        tabs={TABS.map((item) => ({ id: item.id, label: item.label }))}
        active={tab}
        onChange={(id) => {
          if (isSettingsTab(id)) setTab(id)
        }}
      />

      <TabPanel id="profile" active={tab}>
        {profileState.loading && !profile ? (
          <LoadingState label="Loading your profile…" />
        ) : null}
        {profileState.error ? (
          <ErrorState error={profileState.error} onRetry={profileState.reload} />
        ) : null}
        {profile && user ? (
          <ProfileSettingsForm profile={profile} />
        ) : !profileState.loading && !profileState.error ? (
          <EmptyState
            title="Sign in to manage your profile"
            description="Your profile settings are only available to signed-in accounts."
          />
        ) : null}
      </TabPanel>

      <TabPanel id="picture" active={tab}>
        {user ? (
          <AvatarSettingsPanel email={user.email} />
        ) : (
          <EmptyState title="Sign in to change your avatar" />
        )}
      </TabPanel>

      <TabPanel id="sessions" active={tab}>
        <SessionsPanel />
      </TabPanel>

      <TabPanel id="logins" active={tab}>
        <LoginHistoryPanel />
      </TabPanel>

      <TabPanel id="password" active={tab}>
        <PasswordSettingsForm />
      </TabPanel>
    </div>
  )
}
