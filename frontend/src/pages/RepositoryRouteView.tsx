import { useParams } from 'react-router-dom'

import { parseRepositoryRoute } from '../lib/paths'
import { LayerBrowserPage } from './LayerBrowserPage'
import { NotFoundPage } from './NotFoundPage'
import { RepositoryPage } from './RepositoryPage'
import { TagPage } from './TagPage'

/**
 * Dispatches the `/:namespace/*` splat to the repository, tag or layer view.
 * The API path shape is identical for all three, so the split happens here.
 */
export function RepositoryRouteView() {
  const params = useParams()
  const namespace = params.namespace ?? ''
  const splat = params['*'] ?? ''

  if (namespace.length === 0) return <NotFoundPage />

  const parsed = parseRepositoryRoute(splat)
  if (!parsed || parsed.repo.length === 0) return <NotFoundPage />

  if (parsed.view === 'tag') {
    return (
      <TagPage
        key={`${namespace}/${parsed.repo}:${parsed.tag}`}
        namespace={namespace}
        repo={parsed.repo}
        tag={parsed.tag}
      />
    )
  }

  if (parsed.view === 'layer') {
    return (
      <LayerBrowserPage
        namespace={namespace}
        repo={parsed.repo}
        digest={parsed.digest}
      />
    )
  }

  return <RepositoryPage namespace={namespace} repo={parsed.repo} />
}
