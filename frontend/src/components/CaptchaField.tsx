import { useCaptcha, type CapToken } from 'pow-captcha-react'
import { useEffect, useRef, useState } from 'react'

import { auth } from '../api/endpoints'
import { Spinner } from './primitives/StateViews'

/**
 * Endpoint prefix where `pow-captcha-axum` mounts its `/challenge`, `/redeem`,
 * `/validate` and `/consume` routes (docs/AUTH.md §5).
 */
export const CAPTCHA_ENDPOINT = '/api/auth/captcha/'

export type CaptchaGateProps = {
  /** Receives the redeemed captcha token (or `null` while unsolved). */
  onToken: (token: string | null) => void
  /** Reports whether the proof-of-work solver is running. */
  onSolving: (solving: boolean) => void
  /** Reports whether captcha is enabled server-side. */
  onEnabled: (enabled: boolean) => void
}

/**
 * Probes `GET /api/auth/captcha` and only mounts the solver when the endpoint
 * answers `200`. A `204` means captcha is disabled and nothing is rendered.
 */
export function CaptchaGate({
  onToken,
  onSolving,
  onEnabled,
}: CaptchaGateProps) {
  const [state, setState] = useState<'probing' | 'enabled' | 'disabled' | 'error'>(
    'probing',
  )
  const onEnabledRef = useRef(onEnabled)
  useEffect(() => {
    onEnabledRef.current = onEnabled
  })

  useEffect(() => {
    const controller = new AbortController()
    auth.captchaEnabled({ signal: controller.signal }).then(
      (enabled) => {
        if (controller.signal.aborted) return
        onEnabledRef.current(enabled)
        setState(enabled ? 'enabled' : 'disabled')
      },
      () => {
        if (controller.signal.aborted) return
        onEnabledRef.current(false)
        setState('error')
      },
    )
    return () => controller.abort()
  }, [])

  if (state === 'probing') {
    return (
      <p className="flex items-center gap-2 text-xs text-muted" role="status">
        <Spinner />
        Checking captcha…
      </p>
    )
  }

  if (state === 'disabled') return null

  if (state === 'error') {
    return (
      <p className="text-xs text-danger" role="alert">
        The captcha challenge could not be loaded. Reload the page to try again.
      </p>
    )
  }

  return <CaptchaSolver onToken={onToken} onSolving={onSolving} />
}

function CaptchaSolver({
  onToken,
  onSolving,
}: Pick<CaptchaGateProps, 'onToken' | 'onSolving'>) {
  const onTokenRef = useRef(onToken)
  const onSolvingRef = useRef(onSolving)
  useEffect(() => {
    onTokenRef.current = onToken
    onSolvingRef.current = onSolving
  })

  const captcha = useCaptcha({
    endpoint: CAPTCHA_ENDPOINT,
    localStorageEnabled: true,
    onSolve: (token: CapToken) => {
      onTokenRef.current(token.token)
      onSolvingRef.current(false)
    },
    onError: () => {
      onTokenRef.current(null)
    },
  })

  const { solvingCaptcha, captchaError, captchaProgress, captchaToken } = captcha

  useEffect(() => {
    onSolvingRef.current(solvingCaptcha)
    if (!solvingCaptcha && captchaToken === null) onTokenRef.current(null)
  }, [solvingCaptcha, captchaToken])

  if (captchaError) {
    return (
      <p className="text-xs text-danger" role="alert">
        Captcha failed: {captchaError}
      </p>
    )
  }

  if (captchaToken) {
    return (
      <p className="text-xs text-success" role="status">
        Captcha solved.
      </p>
    )
  }

  return (
    <p className="flex items-center gap-2 text-xs text-muted" role="status">
      <Spinner />
      Solving proof-of-work captcha
      {captchaProgress !== null ? ` (${captchaProgress}%)` : ''}…
    </p>
  )
}
