import {
  LinkIcon,
  LocationIcon,
  OrganizationIcon,
} from '@primer/octicons-react'
import { useState } from 'react'
import { Link } from 'react-router-dom'

import { isApiError } from '../api/client'
import { users as usersApi } from '../api/endpoints'
import type { UserProfile } from '../api/schemas'
import { formatDateTime, formatNumber } from '../lib/format'
import { useAuth } from '../lib/auth-context'
import { LibravatarAvatar } from './LibravatarAvatar'
import { Button, LinkButton } from './primitives/Button'
import { Flash } from './primitives/Flash'

export type ProfileHeaderProps = {
  profile: UserProfile
  avatarHash?: string | null
  onFollowChange: () => void
}

function websiteHref(website: string): string {
  return /^https?:\/\//.test(website) ? website : `https://${website}`
}

export function ProfileHeader({ profile, avatarHash, onFollowChange }: ProfileHeaderProps) {
  const { user } = useAuth()
  const [optimistic, setOptimistic] = useState<{
    base: UserProfile
    following: boolean
  } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const following =
    optimistic && optimistic.base === profile
      ? optimistic.following
      : profile.is_following
  const followerDelta =
    following === profile.is_following ? 0 : following ? 1 : -1
  const followers = profile.follower_count + followerDelta

  const displayName =
    [profile.first_name, profile.last_name].filter(Boolean).join(' ') ||
    profile.username

  const toggleFollow = () => {
    const next = !following
    setBusy(true)
    setError(null)
    const action = next
      ? usersApi.follow(profile.username)
      : usersApi.unfollow(profile.username)
    void action.then(
      () => {
        setBusy(false)
        setOptimistic({ base: profile, following: next })
        onFollowChange()
      },
      (reason: unknown) => {
        setBusy(false)
        setError(
          isApiError(reason)
            ? reason.message
            : 'The follow action could not be completed.',
        )
      },
    )
  }

  const stats: Array<{ label: string; value: number }> = [
    { label: 'repositories', value: profile.repository_count },
    { label: 'public repos', value: profile.public_repository_count },
    { label: 'pulls', value: profile.total_pulls },
    { label: 'followers', value: followers },
    { label: 'following', value: profile.following_count },
  ]

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-start gap-4">
        <LibravatarAvatar
          src={profile.avatar_url}
          hash={avatarHash}
          name={displayName}
          size={96}
        />
        <div className="min-w-0 flex-1 space-y-1">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <h1 className="text-xl font-semibold">{displayName}</h1>
              <p className="text-sm text-muted">@{profile.username}</p>
            </div>
            {profile.is_self ? (
              <LinkButton to="/settings" variant="default">
                Edit profile
              </LinkButton>
            ) : user ? (
              <Button
                variant={following ? 'default' : 'primary'}
                disabled={busy}
                onClick={toggleFollow}
              >
                {busy ? 'Working…' : following ? 'Unfollow' : 'Follow'}
              </Button>
            ) : (
              <Link
                to="/login"
                className="text-sm text-accent hover:underline"
              >
                Sign in to follow
              </Link>
            )}
          </div>

          {error ? (
            <Flash variant="danger" onDismiss={() => setError(null)}>
              {error}
            </Flash>
          ) : null}

          {profile.bio ? <p className="text-sm">{profile.bio}</p> : null}

          <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted">
            {profile.company ? (
              <span className="inline-flex items-center gap-1">
                <OrganizationIcon size={14} aria-hidden="true" />
                {profile.company}
              </span>
            ) : null}
            {profile.location ? (
              <span className="inline-flex items-center gap-1">
                <LocationIcon size={14} aria-hidden="true" />
                {profile.location}
              </span>
            ) : null}
            {profile.website ? (
              <a
                href={websiteHref(profile.website)}
                target="_blank"
                rel="noreferrer noopener"
                className="inline-flex items-center gap-1 text-accent hover:underline"
              >
                <LinkIcon size={14} aria-hidden="true" />
                {profile.website}
              </a>
            ) : null}
            <span>Joined {formatDateTime(profile.created_at)}</span>
          </div>

          <div className="flex flex-wrap items-center gap-3 pt-1 text-sm">
            {stats.map((stat) => (
              <span key={stat.label} className="inline-flex items-center gap-1">
                <strong>{formatNumber(stat.value)}</strong>
                <span className="text-muted">{stat.label}</span>
              </span>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}
