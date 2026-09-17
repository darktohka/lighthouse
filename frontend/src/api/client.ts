/**
 * Typed fetch wrapper for the Lighthouse control plane.
 *
 * - `credentials: 'include'` on every request so the httpOnly access-token
 *   cookie travels with it.
 * - Every response body is parsed through a Valibot schema; malformed payloads
 *   raise an `ApiError` instead of leaking `unknown` values into the app.
 * - A single-flight refresh retries the original request exactly once after a
 *   `401`, using the refresh token kept in `localStorage` (never the access
 *   token — that is cookie-only and invisible to JS).
 */
import * as v from 'valibot'

import { errorEnvelopeSchema, refreshResponseSchema } from './schemas'

export const API_BASE = '/api'
export const REFRESH_TOKEN_KEY = 'lighthouse.refresh_token'

/** A structured API failure carrying the envelope's code and message. */
export class ApiError extends Error {
  readonly status: number
  readonly code: string

  constructor(status: number, code: string, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
  }
}

/** True when `error` is an `ApiError` (safe across module instances). */
export function isApiError(error: unknown): error is ApiError {
  return error instanceof ApiError
}

// ---------------------------------------------------------------------------
// Refresh-token storage (the only credential that may live in JS)
// ---------------------------------------------------------------------------

export function getRefreshToken(): string | null {
  try {
    return window.localStorage.getItem(REFRESH_TOKEN_KEY)
  } catch {
    return null
  }
}

export function setRefreshToken(token: string): void {
  try {
    window.localStorage.setItem(REFRESH_TOKEN_KEY, token)
  } catch {
    /* storage unavailable (private mode) — session simply won't persist */
  }
}

export function clearRefreshToken(): void {
  try {
    window.localStorage.removeItem(REFRESH_TOKEN_KEY)
  } catch {
    /* ignore */
  }
}

// ---------------------------------------------------------------------------
// Single-flight refresh
// ---------------------------------------------------------------------------

let refreshInFlight: Promise<boolean> | null = null

/**
 * Rotates the refresh token, installing a fresh access cookie as a side effect.
 * Returns `true` when a new access token was issued. Concurrent callers share
 * one network request.
 */
export function refreshSession(): Promise<boolean> {
  if (refreshInFlight) return refreshInFlight

  const token = getRefreshToken()
  if (!token) return Promise.resolve(false)

  const promise = (async (): Promise<boolean> => {
    try {
      const response = await fetch(`${API_BASE}/auth/refresh`, {
        method: 'POST',
        credentials: 'include',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ refresh_token: token }),
      })
      if (!response.ok) {
        clearRefreshToken()
        return false
      }
      const parsed = v.safeParse(refreshResponseSchema, await response.json())
      if (!parsed.success) {
        clearRefreshToken()
        return false
      }
      setRefreshToken(parsed.output.refresh_token)
      return true
    } catch {
      return false
    } finally {
      refreshInFlight = null
    }
  })()

  refreshInFlight = promise
  return promise
}

// ---------------------------------------------------------------------------
// Core request plumbing
// ---------------------------------------------------------------------------

export type RequestOptions = {
  /** Abort signal, used by React effects to cancel stale loads. */
  signal?: AbortSignal
  /** Retry once after a successful refresh on 401. Defaults to true. */
  retry?: boolean
}

type SendOptions = RequestOptions & { body?: unknown }

async function toApiError(response: Response): Promise<ApiError> {
  let code = 'http_error'
  let message = `Request failed with status ${response.status}`
  try {
    const payload: unknown = await response.json()
    const parsed = v.safeParse(errorEnvelopeSchema, payload)
    if (parsed.success) {
      code = parsed.output.error.code
      message = parsed.output.error.message
    }
  } catch {
    /* response body was empty or not JSON — keep the generic message */
  }
  return new ApiError(response.status, code, message)
}

async function fetchWithRefresh(
  method: string,
  path: string,
  options: SendOptions,
): Promise<Response> {
  const headers: Record<string, string> = { accept: 'application/json' }
  let body: string | undefined
  if (options.body !== undefined) {
    headers['content-type'] = 'application/json'
    body = JSON.stringify(options.body)
  }

  const init: RequestInit = {
    method,
    credentials: 'include',
    headers,
    signal: options.signal,
  }
  if (body !== undefined) init.body = body

  const url = `${API_BASE}${path}`
  let response = await fetch(url, init)

  if (
    response.status === 401 &&
    options.retry !== false &&
    getRefreshToken() !== null
  ) {
    const refreshed = await refreshSession()
    if (refreshed) {
      response = await fetch(url, init)
    }
  }

  if (!response.ok) throw await toApiError(response)
  return response
}

async function readJson(response: Response): Promise<unknown> {
  if (response.status === 204) return null
  const text = await response.text()
  if (text.length === 0) return null
  try {
    const parsed: unknown = JSON.parse(text)
    return parsed
  } catch {
    throw new ApiError(
      response.status,
      'invalid_response',
      'The server returned a malformed JSON body',
    )
  }
}

async function request<T>(
  method: string,
  path: string,
  schema: v.GenericSchema<unknown, T>,
  options: SendOptions,
): Promise<T> {
  const response = await fetchWithRefresh(method, path, options)
  const payload = await readJson(response)
  const parsed = v.safeParse(schema, payload)
  if (!parsed.success) {
    throw new ApiError(
      response.status,
      'invalid_response',
      `Unexpected response shape for ${method} ${path}`,
    )
  }
  return parsed.output
}

async function send(
  method: string,
  path: string,
  options: SendOptions,
): Promise<void> {
  await fetchWithRefresh(method, path, options)
}

// ---------------------------------------------------------------------------
// Public surface
// ---------------------------------------------------------------------------

export const api = {
  get<T>(
    path: string,
    schema: v.GenericSchema<unknown, T>,
    options: RequestOptions = {},
  ): Promise<T> {
    return request('GET', path, schema, options)
  },

  post<T>(
    path: string,
    schema: v.GenericSchema<unknown, T>,
    body?: unknown,
    options: RequestOptions = {},
  ): Promise<T> {
    return request('POST', path, schema, { ...options, body })
  },

  patch<T>(
    path: string,
    schema: v.GenericSchema<unknown, T>,
    body?: unknown,
    options: RequestOptions = {},
  ): Promise<T> {
    return request('PATCH', path, schema, { ...options, body })
  },

  delete<T>(
    path: string,
    schema: v.GenericSchema<unknown, T>,
    options: RequestOptions = {},
  ): Promise<T> {
    return request('DELETE', path, schema, options)
  },

  /** POST / DELETE for `204 No Content` endpoints. */
  postVoid(
    path: string,
    body?: unknown,
    options: RequestOptions = {},
  ): Promise<void> {
    return send('POST', path, { ...options, body })
  },

  deleteVoid(path: string, options: RequestOptions = {}): Promise<void> {
    return send('DELETE', path, options)
  },

  /**
   * Escape hatch for non-JSON payloads (layer files, blob downloads). Still
   * gets cookie credentials and the refresh-on-401 behaviour.
   */
  async raw(path: string, options: RequestOptions = {}): Promise<Response> {
    return fetchWithRefresh('GET', path, options)
  },
} as const
