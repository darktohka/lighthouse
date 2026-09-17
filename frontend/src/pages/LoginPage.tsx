import { useState, type FormEvent } from 'react'
import { Link, useNavigate } from 'react-router-dom'

import { loginFormSchema } from '../api/schemas'
import { AuthLayout } from '../components/AuthLayout'
import { CaptchaGate } from '../components/CaptchaField'
import { Button } from '../components/primitives/Button'
import { Flash } from '../components/primitives/Flash'
import { TextInput } from '../components/primitives/TextInput'
import { useAuth } from '../lib/auth-context'
import { authErrorMessage, isEmailNotVerified } from '../lib/auth-errors'
import { validateForm, type FieldErrors } from '../lib/forms'

export function LoginPage() {
  const { login } = useAuth()
  const navigate = useNavigate()

  const [identifier, setIdentifier] = useState('')
  const [password, setPassword] = useState('')
  const [errors, setErrors] = useState<FieldErrors>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [notVerified, setNotVerified] = useState(false)
  const [submitting, setSubmitting] = useState(false)

  const [captchaToken, setCaptchaToken] = useState<string | null>(null)
  const [captchaEnabled, setCaptchaEnabled] = useState(false)
  const [captchaSolving, setCaptchaSolving] = useState(false)
  const [captchaKey, setCaptchaKey] = useState(0)

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
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
      () => navigate('/dashboard'),
      (error: unknown) => {
        setFormError(authErrorMessage(error))
        setNotVerified(isEmailNotVerified(error))
        setCaptchaKey((key) => key + 1)
        setCaptchaToken(null)
        setSubmitting(false)
      },
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
      <form className="space-y-3" onSubmit={onSubmit} noValidate>
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
