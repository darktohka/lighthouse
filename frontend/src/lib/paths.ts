/**
 * URL / route-path helpers.
 *
 * Repository names may be nested (`alice/more/complicated/app`), so every path
 * segment is encoded individually and rejoined. A single `encodeURIComponent`
 * over the whole path would wrongly escape the separators the API expects.
 */
import { API_BASE } from '../api/client'

/** Encodes one path segment (digests contain `:`, tags may contain `+`). */
export function encodeSegment(value: string): string {
  return encodeURIComponent(value)
}

/** Encodes a variable-length repository path, preserving `/` separators. */
export function encodeRepoPath(repo: string): string {
  return repo
    .split('/')
    .filter((segment) => segment.length > 0)
    .map(encodeSegment)
    .join('/')
}

/** `/namespaces/{name}` */
export function namespaceApiPath(name: string): string {
  return `/namespaces/${encodeSegment(name)}`
}

/** `/repositories/{namespace}/{*repo}` */
export function repositoryApiPath(namespace: string, repo: string): string {
  return `/repositories/${encodeSegment(namespace)}/${encodeRepoPath(repo)}`
}

/** `/repositories/{namespace}/{*repo}/tags/{tag}` */
export function tagApiPath(
  namespace: string,
  repo: string,
  tag: string,
): string {
  return `${repositoryApiPath(namespace, repo)}/tags/${encodeSegment(tag)}`
}

/** `/blobs/{digest}` and friends. */
export function blobApiPath(digest: string, suffix = ''): string {
  return `/blobs/${encodeSegment(digest)}${suffix}`
}

export type QueryValue = string | number | boolean | null | undefined

/** Builds `?a=1&b=2`, skipping null/undefined/empty values. */
export function buildQuery(
  params: Record<string, QueryValue>,
): string {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === '') continue
    search.set(key, String(value))
  }
  const query = search.toString()
  return query.length > 0 ? `?${query}` : ''
}

/** Absolute app path (with `/api` base) for `<a download>` links. */
export function apiUrl(path: string): string {
  return `${API_BASE}${path}`
}

/**
 * Strips the namespace prefix from a `RepositorySummary.path` so it can be
 * appended to the `/{namespace}/` route or API path. Tolerates payloads where
 * `path` is already relative or merely the last segment.
 */
export function repositoryRelativePath(
  namespace: string,
  repository: { path: string; name: string },
): string {
  const prefix = `${namespace}/`
  if (repository.path.startsWith(prefix)) {
    return repository.path.slice(prefix.length)
  }
  return repository.path.length > 0 ? repository.path : repository.name
}

// ---------------------------------------------------------------------------
// Front-end route parsing
// ---------------------------------------------------------------------------

export type RepositoryRoute =
  | { view: 'repository'; repo: string }
  | { view: 'tag'; repo: string; tag: string }
  | { view: 'layer'; repo: string; digest: string }

/**
 * Splits the `/:namespace/*` splat into a repository, tag or layer view.
 *
 * The last `tags`/`layers` segment is treated as the view delimiter, matching
 * how links are generated. A repository literally named `tags` under a deeper
 * path is an accepted, documented ambiguity (see docs/FRONTEND.md).
 */
export function parseRepositoryRoute(splat: string): RepositoryRoute | null {
  const segments = splat.split('/').filter((segment) => segment.length > 0)
  if (segments.length === 0) return null

  if (segments.length >= 3 && segments[segments.length - 2] === 'tags') {
    return {
      view: 'tag',
      repo: segments.slice(0, -2).join('/'),
      tag: segments[segments.length - 1],
    }
  }

  if (segments.length >= 3 && segments[segments.length - 2] === 'layers') {
    return {
      view: 'layer',
      repo: segments.slice(0, -2).join('/'),
      digest: segments[segments.length - 1],
    }
  }

  return { view: 'repository', repo: segments.join('/') }
}

/** Front-end link helpers. */
export function repoRoute(namespace: string, repo: string): string {
  return `/${encodeSegment(namespace)}/${encodeRepoPath(repo)}`
}

export function tagRoute(
  namespace: string,
  repo: string,
  tag: string,
): string {
  return `${repoRoute(namespace, repo)}/tags/${encodeSegment(tag)}`
}

export type LayerRouteOptions = {
  path?: string
  /** Image manifest digest; the aggregate/diff views require it. */
  manifest?: string
  /** Active layer-browser tab. */
  tab?: string
}

export function layerRoute(
  namespace: string,
  repo: string,
  digest: string,
  options?: LayerRouteOptions,
): string {
  const base = `${repoRoute(namespace, repo)}/layers/${encodeSegment(digest)}`
  return `${base}${buildQuery({
    path: options?.path,
    manifest: options?.manifest,
    tab: options?.tab,
  })}`
}
