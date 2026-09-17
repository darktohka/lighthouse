/**
 * Sort vocabulary shared by every repository list.
 *
 * The profile (`UserRepositories`) and workspace (`NamespacePage`) tables
 * paginate a single server page, so their sort/order travel to the API. The
 * dashboard and Explore payloads arrive whole, so they reuse the same key types
 * and default-direction rule but sort in memory with `compareRepositories`.
 */
import type { RepositorySummary } from '../api/schemas'

export type RepositorySort = 'name' | 'size' | 'updated'
export type RepositoryOrder = 'asc' | 'desc'

/** Options for the labelled Explore sort `<select>`. */
export const REPOSITORY_SORT_OPTIONS: readonly {
  value: RepositorySort
  label: string
}[] = [
  { value: 'updated', label: 'Last updated' },
  { value: 'name', label: 'Name' },
  { value: 'size', label: 'Size' },
]

/** Narrows a `<select>` value to a sort key without an unchecked cast. */
export function isRepositorySort(value: string): value is RepositorySort {
  return value === 'name' || value === 'size' || value === 'updated'
}

/**
 * Direction to use when `next` becomes the active sort: re-selecting the active
 * key flips the current direction, a newly selected key uses its natural
 * default (`name` ascending, `size`/`updated` descending).
 */
export function nextRepositoryOrder(
  currentSort: RepositorySort,
  currentOrder: RepositoryOrder,
  next: RepositorySort,
): RepositoryOrder {
  if (next === currentSort) {
    return currentOrder === 'desc' ? 'asc' : 'desc'
  }
  return next === 'name' ? 'asc' : 'desc'
}

/** Comparator for in-memory lists; `updated_at` is an ISO-8601 timestamp. */
export function compareRepositories(
  a: RepositorySummary,
  b: RepositorySummary,
  sort: RepositorySort,
  order: RepositoryOrder,
): number {
  const direction = order === 'asc' ? 1 : -1
  switch (sort) {
    case 'name':
      return a.path.toLowerCase().localeCompare(b.path.toLowerCase()) * direction
    case 'size':
      return (a.size - b.size) * direction
    case 'updated':
      return a.updated_at.localeCompare(b.updated_at) * direction
  }
}
