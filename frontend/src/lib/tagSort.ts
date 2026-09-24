/**
 * Sort vocabulary shared by the repository tag table.
 *
 * The tags endpoint paginates one server page, so the sort/order travel to the
 * API just like the repository list keys. Keys mirror the backend's `sort`
 * parameter (`name`, `digest`, `compressed_size`, `pull_count`, `updated_at`).
 */

export type TagSort =
  | 'name'
  | 'digest'
  | 'compressed_size'
  | 'pull_count'
  | 'updated_at'
export type TagOrder = 'asc' | 'desc'

/** Narrows a value to a tag sort key without an unchecked cast. */
export function isTagSort(value: string): value is TagSort {
  return (
    value === 'name' ||
    value === 'digest' ||
    value === 'compressed_size' ||
    value === 'pull_count' ||
    value === 'updated_at'
  )
}

/**
 * Direction to use when `next` becomes the active sort: re-selecting the active
 * key flips the current direction, a newly selected key uses its natural
 * default (`name`/`digest` ascending, `compressed_size`/`pull_count`/
 * `updated_at` descending).
 */
export function nextTagOrder(
  currentSort: TagSort,
  currentOrder: TagOrder,
  next: TagSort,
): TagOrder {
  if (next === currentSort) {
    return currentOrder === 'desc' ? 'asc' : 'desc'
  }
  return next === 'name' || next === 'digest' ? 'asc' : 'desc'
}
