/**
 * Runs an async loader tied to a dependency key and exposes explicit
 * loading/empty/error state to the page.
 *
 * The loader always receives an `AbortSignal`; the previous request is aborted
 * when the key changes or the component unmounts, so late responses can't
 * clobber newer state.
 *
 * `enabled` (default `true`) lets a caller hold the loader back until a
 * prerequisite is ready — most importantly until the session behind an
 * auth-dependent request has resolved. While disabled no request is made and
 * the hook reports `loading: true`; enabling it (or changing `key`) starts the
 * load. This prevents a public page from fetching as an anonymous viewer and
 * then rendering that stale, empty result after the user's session appears.
 */
import { useCallback, useEffect, useRef, useState } from 'react'

export type AsyncState<T> = {
  data: T | null
  error: Error | null
  loading: boolean
}

export type AsyncResult<T> = AsyncState<T> & { reload: () => void }

type InternalState<T> = {
  key: string
  data: T | null
  error: Error | null
  settled: boolean
}

function toError(value: unknown): Error {
  return value instanceof Error ? value : new Error(String(value))
}

export function useAsync<T>(
  loader: (signal: AbortSignal) => Promise<T>,
  key: string,
  enabled = true,
): AsyncResult<T> {
  const [state, setState] = useState<InternalState<T>>({
    key,
    data: null,
    error: null,
    settled: false,
  })
  const [nonce, setNonce] = useState(0)

  const loaderRef = useRef(loader)
  useEffect(() => {
    loaderRef.current = loader
  })

  useEffect(() => {
    if (!enabled) return
    const controller = new AbortController()
    let active = true

    loaderRef.current(controller.signal).then(
      (data) => {
        if (active) setState({ key, data, error: null, settled: true })
      },
      (error: unknown) => {
        if (!active || controller.signal.aborted) return
        setState({ key, data: null, error: toError(error), settled: true })
      },
    )

    return () => {
      active = false
      controller.abort()
    }
  }, [key, nonce, enabled])

  const reload = useCallback(() => {
    setState((previous) => ({ ...previous, settled: false }))
    setNonce((value) => value + 1)
  }, [])

  const isCurrent = state.key === key

  return {
    data: state.data,
    error: isCurrent ? state.error : null,
    loading: !enabled || !isCurrent || !state.settled,
    reload,
  }
}
