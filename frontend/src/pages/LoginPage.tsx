import { useState, type FormEvent } from 'react'
import { Link, useNavigate } from 'react-router-dom'

import { loginFormSchema, verifyCodeFormSchema } from '../api/schemas'
import { AuthLayout } from '../components/AuthLayout'
import { CaptchaGate } from '../components/CaptchaField'
import { Button } from '../components/primitives/Button'
import { Flash } from '../components/primitives/Flash'
import { TextInput } from '../components/primitives/TextInput'
import { useAuth } from '../lib/auth-context'
import { authErrorMessage, isEmailNotVerified } from '../lib/auth-errors'
import { validateForm, type FieldErrors } from '../lib/forms'

export function LoginPage() {
  const { login, verifyTwoFactor } = useAuth()
  const navigate = useNavigate()

  const [identifier, setIdentifier] = useState('')
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [mfaToken, setMfaToken] = useState<string | null>(null)
  const [errors, setErrors] = useState<FieldErrors>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [notVerified, setNotVerified] = useState(false)
  const [submitting, setSubmitting] = useState(false)

  const [captchaToken, setCaptchaToken] = useState<string | null>(null)
  const [captchaEnabled, setCaptchaEnabled] = useState(false)
  const [captchaSolving, setCaptchaSolving] = useState(false)
  const [captchaKey, setCaptchaKey] = useState(0)

  const resetCaptcha = () => {
    setCaptchaKey((key) => key + 1)
    setCaptchaToken(null)
  }

  const onSubmitCredentials = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setFormError(null)
    setNotVerified(false)

    const validation = validateForm(loginFormSchema, { identifier, password })
    if (!validation.ok) {
      setErrors(validation.errors)
      if (validation.formError) setFormError(validation.formError)
      return
    }
    setErrors({})

    if (captchaEnabled && (captchaSolving || captchaToken === null)) {
      setFormError('Please wait for the captcha to finish solving.')
      return
    }

    setSubmitting(true)
    void login({
      identifier: validation.value.identifier,
      password: validation.value.password,
      ...(captchaToken ? { captcha: captchaToken } : {}),
    }).then(
      (outcome) => {
        if (outcome.status === 'two_factor_required') {
          setMfaToken(outcome.mfaToken)
          resetCaptcha()
          setSubmitting(false)
          return
        }
        navigate('/dashboard')
      },
      (error: unknown) => {
        setFormError(authErrorMessage(error))
        setNotVerified(isEmailNotVerified(error))
        resetCaptcha()
        setSubmitting(false)
      },
    )
  }

  const onSubmitCode = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setFormError(null)
    const validation = validateForm(verifyCodeFormSchema, { code })
    if (!validation.ok) {
      setErrors(validation.errors)
      return
    }
    setErrors({})
    setSubmitting(true)
    void verifyTwoFactor(mfaToken ?? '', validation.value.code).then(
      () => navigate('/dashboard'),
      (error: unknown) => {
        setFormError(authErrorMessage(error))
        setSubmitting(false)
      },
    )
  }

  if (mfaToken) {
    return (
      <AuthLayout
        title="Two-factor authentication"
        subtitle="Enter the 6-digit code from your authenticator app, or one of your backup codes."
        footer={
          <button
            type="button"
            className="text-accent hover:underline"
            onClick={() => {
              setMfaToken(null)
              setCode('')
              setFormError(null)
            }}
          >
            Back to sign in
          </button>
        }
      >
        <form className="space-y-3" onSubmit={onSubmitCode} noValidate>
          {formError ? (
            <Flash variant="danger">
              <p>{formError}</p>
            </Flash>
          ) : null}

          <TextInput
            label="Authentication code"
            value={code}
            onChange={(event) => setCode(event.target.value)}
            error={errors.code}
            inputMode="numeric"
            autoComplete="one-time-code"
            autoFocus
            required
          />

          <Button
            type="submit"
            variant="primary"
            className="w-full"
            disabled={submitting}
          >
            {submitting ? 'Verifying…' : 'Verify and sign in'}
          </Button>
        </form>
      </AuthLayout>
    )
  }

  return (
    <AuthLayout
      title="Sign in to Lighthouse"
      subtitle="Use your username or e-mail address."
      footer={
        <p>
          New to Lighthouse?{' '}
          <Link to="/register" className="text-accent hover:underline">
            Create an account
          </Link>
        </p>
      }
    >
      <form className="space-y-3" onSubmit={onSubmitCredentials} noValidate>
        {formError ? (
          <Flash variant="danger">
            <p>{formError}</p>
            {notVerified ? (
              <Link
                to="/verify-email"
                className="mt-1 inline-block text-accent hover:underline"
              >
                Resend the verification e-mail
              </Link>
            ) : null}
          </Flash>
        ) : null}

        <TextInput
          label="Username or e-mail"
          value={identifier}
          onChange={(event) => setIdentifier(event.target.value)}
          error={errors.identifier}
          autoComplete="username"
          autoFocus
          required
        />
        <TextInput
          label="Password"
          type="password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          error={errors.password}
          autoComplete="current-password"
          required
        />

        <CaptchaGate
          key={captchaKey}
          onToken={setCaptchaToken}
          onSolving={setCaptchaSolving}
          onEnabled={setCaptchaEnabled}
        />

        <Button
          type="submit"
          variant="primary"
          className="w-full"
          disabled={submitting}
        >
          {submitting ? 'Signing in…' : 'Sign in'}
        </Button>
      </form>

      <div className="mt-3 text-center text-sm">
        <Link to="/forgot-password" className="text-accent hover:underline">
          Forgot your password?
        </Link>
      </div>
    </AuthLayout>
  )
}
