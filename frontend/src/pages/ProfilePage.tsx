import { useState } from 'react'
import { useParams } from 'react-router-dom'

import { namespaces as namespacesApi, users as usersApi } from '../api/endpoints'
import { FollowPanel } from '../components/FollowPanel'
import { Heatmap } from '../components/Heatmap'
import { NamespaceGeneralPanel } from '../components/NamespaceGeneralPanel'
import { ProfileHeader } from '../components/ProfileHeader'
import { UserRepositories } from '../components/UserRepositories'
import { Box, BoxHeader } from '../components/primitives/Box'
import { ErrorState, LoadingState } from '../components/primitives/StateViews'
import { useAsync } from '../lib/useAsync'
import { NotFoundPage } from './NotFoundPage'

export function ProfilePage() {
  const params = useParams()
  const username = params.username ?? ''
  const todayIso = new Date().toISOString().slice(0, 10)
  const [endDate, setEndDate] = useState(todayIso)
  const [reloadToken, setReloadToken] = useState(0)

  const profileState = useAsync(
    (signal) =>
      username ? usersApi.profile(username, { signal }) : Promise.resolve(null),
    `profile:${username}:${reloadToken}`,
  )
  const heatmapState = useAsync(
    (signal) =>
      username
        ? usersApi.heatmap(username, endDate, { signal })
        : Promise.resolve(null),
    `heatmap:${username}:${endDate}:${reloadToken}`,
  )
  const namespaceState = useAsync(
    (signal) =>
      profileState.data?.is_self && profileState.data.namespace
        ? namespacesApi.detail(profileState.data.namespace, { signal })
        : Promise.resolve(null),
    `profile-namespace:${profileState.data?.namespace ?? 'none'}`,
  )

  if (username.length === 0) return <NotFoundPage />

  const profile = profileState.data

  return (
    <div className="space-y-6">
      {profileState.loading && !profile ? (
        <LoadingState label="Loading profile…" />
      ) : null}
      {profileState.error ? (
        <ErrorState error={profileState.error} onRetry={profileState.reload} />
      ) : null}

      {profile ? (
        <>
          <ProfileHeader
            profile={profile}
            avatarHash={profile.avatar_hash}
            onFollowChange={() => setReloadToken((value) => value + 1)}
          />

          <Box>
            <BoxHeader>
              <span className="text-sm font-medium">Contributions</span>
            </BoxHeader>
            <div className="p-4">
              <Heatmap
                endDate={endDate}
                data={heatmapState.data}
                loading={heatmapState.loading}
                error={heatmapState.error}
                onRetry={heatmapState.reload}
                onWindowChange={setEndDate}
                maxEnd={todayIso}
              />
            </div>
          </Box>

          <section aria-labelledby="profile-repositories">
            <h2
              id="profile-repositories"
              className="mb-2 text-base font-semibold"
            >
              Repositories
            </h2>
            <UserRepositories
              namespace={profile.namespace}
              reloadToken={reloadToken}
            />
          </section>

          <section aria-labelledby="profile-follows">
            <h2 id="profile-follows" className="mb-2 text-base font-semibold">
              Follow graph
            </h2>
            <FollowPanel username={username} reloadToken={reloadToken} />
          </section>

          {profile.is_self && profile.namespace ? (
            <section aria-labelledby="profile-namespace-settings">
              <h2
                id="profile-namespace-settings"
                className="mb-2 text-base font-semibold"
              >
                Namespace settings
              </h2>
              {namespaceState.loading && !namespaceState.data ? (
                <LoadingState label="Loading namespace settings…" />
              ) : null}
              {namespaceState.error ? (
                <ErrorState
                  error={namespaceState.error}
                  onRetry={namespaceState.reload}
                />
              ) : null}
              {namespaceState.data ? (
                <NamespaceGeneralPanel
                  namespace={namespaceState.data}
                  canManage
                  onChanged={namespaceState.reload}
                />
              ) : null}
            </section>
          ) : null}
        </>
      ) : null}
    </div>
  )
}
