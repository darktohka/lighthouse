import { useEffect, useRef, useState, type FormEvent } from 'react'
import { Link, useSearchParams } from 'react-router-dom'

import { AuthLayout } from '../components/AuthLayout'
import { Button } from '../components/primitives/Button'
import { Flash } from '../components/primitives/Flash'
import { Spinner } from '../components/primitives/StateViews'
import { TextInput } from '../components/primitives/TextInput'
import { useAuth } from '../lib/auth-context'
import { authErrorMessage } from '../lib/auth-errors'

type VerificationState = 'idle' | 'verifying' | 'verified' | 'error'

export function VerifyEmailPage() {
  const [searchParams] = useSearchParams()
  const token = searchParams.get('token')
  const { verifyEmail, resendVerification } = useAuth()

  const [state, setState] = useState<VerificationState>(token ? 'verifying' : 'idle')
  const [errorMessage, setErrorMessage] = useState<string | null>(null)

  const [email, setEmail] = useState('')
  const [resendStatus, setResendStatus] = useState<'idle' | 'sending' | 'sent' | 'error'>('idle')

  const attemptedToken = useRef<string | null>(null)

  useEffect(() => {
    if (!token || attemptedToken.current === token) return
    attemptedToken.current = token
    let active = true
    void verifyEmail(token).then(
      () => {
        if (active) setState('verified')
      },
      (error: unknown) => {
        if (!active) return
        setErrorMessage(authErrorMessage(error))
        setState('error')
      },
    )
    return () => {
      active = false
    }
  }, [token, verifyEmail])

  const onResend = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setResendStatus('sending')
    void resendVerification(email.trim()).then(
      () => setResendStatus('sent'),
      () => setResendStatus('error'),
    )
  }

  return (
    <AuthLayout
      title="Verify your e-mail"
      subtitle="Confirm your address to activate your account."
      footer={
        <p>
          <Link to="/login" className="text-accent hover:underline">
            Back to sign in
          </Link>
        </p>
      }
    >
      <div className="space-y-4">
        {state === 'verifying' ? (
          <p className="flex items-center gap-2 text-sm text-muted" role="status">
            <Spinner />
            Verifying your e-mail…
          </p>
        ) : null}

        {state === 'verified' ? (
          <Flash variant="success" title="E-mail verified">
            Your account is active.{' '}
            <Link to="/login" className="text-accent hover:underline">
              Sign in
            </Link>
          </Flash>
        ) : null}

        {state === 'error' ? (
          <Flash variant="danger" title="Verification failed">
            {errorMessage ?? 'This verification link is invalid or has expired.'}
          </Flash>
        ) : null}

        {state === 'idle' ? (
          <Flash variant="default" title="Check your inbox">
            Open the verification link we e-mailed you. If it did not arrive,
            request a new one below.
          </Flash>
        ) : null}

        <form className="space-y-3" onSubmit={onResend} noValidate>
          <TextInput
            label="E-mail address"
            type="email"
            value={email}
            onChange={(event) => setEmail(event.target.value)}
            hint="We only send a message when the account exists."
            autoComplete="email"
          />
          <Button type="submit" disabled={resendStatus === 'sending'}>
            {resendStatus === 'sending'
              ? 'Sending…'
              : 'Resend verification e-mail'}
          </Button>
        </form>

        {resendStatus === 'sent' ? (
          <Flash variant="success">
            If that address belongs to an unverified account, a new link is on its
            way.
          </Flash>
        ) : null}
        {resendStatus === 'error' ? (
          <Flash variant="danger">
            We could not resend the message. Please try again later.
          </Flash>
        ) : null}
      </div>
    </AuthLayout>
  )
}
