import { useParams } from 'react-router-dom'

import { NotFoundPage } from './NotFoundPage'
import { RepositorySettingsPage } from './RepositorySettingsPage'

export function RepositorySettingsRouteView() {
  const params = useParams()
  const namespace = params.namespace ?? ''
  const splat = params['*'] ?? ''

  if (namespace.length === 0) return <NotFoundPage />

  const segments = splat.split('/').filter((segment) => segment.length > 0)
  if (segments.length < 2 || segments[segments.length - 1] !== 'settings') {
    return <NotFoundPage />
  }

  const repo = segments.slice(0, -1).join('/')
  if (repo.length === 0) return <NotFoundPage />

  return <RepositorySettingsPage namespace={namespace} repo={repo} />
}
