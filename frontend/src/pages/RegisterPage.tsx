import { useState, type FormEvent } from 'react'
import { Link } from 'react-router-dom'

import { registerFormSchema, type RegisterRequest } from '../api/schemas'
import { AuthLayout } from '../components/AuthLayout'
import { CaptchaGate } from '../components/CaptchaField'
import { Button } from '../components/primitives/Button'
import { Flash } from '../components/primitives/Flash'
import { TextInput } from '../components/primitives/TextInput'
import { useAuth } from '../lib/auth-context'
import { authErrorMessage } from '../lib/auth-errors'
import { validateForm, type FieldErrors } from '../lib/forms'

export function RegisterPage() {
  const { register } = useAuth()

  const [email, setEmail] = useState('')
  const [username, setUsername] = useState('')
  const [firstName, setFirstName] = useState('')
  const [lastName, setLastName] = useState('')
  const [password, setPassword] = useState('')

  const [errors, setErrors] = useState<FieldErrors>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [registered, setRegistered] = useState(false)

  const [captchaToken, setCaptchaToken] = useState<string | null>(null)
  const [captchaEnabled, setCaptchaEnabled] = useState(false)
  const [captchaSolving, setCaptchaSolving] = useState(false)
  const [captchaKey, setCaptchaKey] = useState(0)

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setFormError(null)

    const validation = validateForm(registerFormSchema, {
      email,
      username,
      first_name: firstName,
      last_name: lastName,
      password,
    })
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

    const value = validation.value
    const body: RegisterRequest = {
      email: value.email,
      username: value.username,
      password: value.password,
      ...(value.first_name && value.first_name.trim().length > 0
        ? { first_name: value.first_name.trim() }
        : {}),
      ...(value.last_name && value.last_name.trim().length > 0
        ? { last_name: value.last_name.trim() }
        : {}),
      ...(captchaToken ? { captcha: captchaToken } : {}),
    }

    setSubmitting(true)
    void register(body).then(
      () => {
        setSubmitting(false)
        setRegistered(true)
      },
      (error: unknown) => {
        setFormError(authErrorMessage(error))
        setCaptchaKey((key) => key + 1)
        setCaptchaToken(null)
        setSubmitting(false)
      },
    )
  }

  if (registered) {
    return (
      <AuthLayout
        title="Check your inbox"
        subtitle="One more step before you can sign in."
        footer={
          <p>
            Already verified?{' '}
            <Link to="/login" className="text-accent hover:underline">
              Sign in
            </Link>
          </p>
        }
      >
        <Flash variant="success" title="Account created">
          <p>
            We sent a verification link to <strong>{email}</strong>. Follow it to
            activate your account.
          </p>
          <p className="mt-2">
            Didn&apos;t get it?{' '}
            <Link to="/verify-email" className="text-accent hover:underline">
              Resend the verification e-mail
            </Link>
          </p>
        </Flash>
      </AuthLayout>
    )
  }

  return (
    <AuthLayout
      title="Create your account"
      subtitle="A personal namespace is created for you automatically."
      footer={
        <p>
          Already have an account?{' '}
          <Link to="/login" className="text-accent hover:underline">
            Sign in
          </Link>
        </p>
      }
    >
      <form className="space-y-3" onSubmit={onSubmit} noValidate>
        {formError ? (
          <Flash variant="danger">{formError}</Flash>
        ) : null}

        <TextInput
          label="E-mail address"
          type="email"
          value={email}
          onChange={(event) => setEmail(event.target.value)}
          error={errors.email}
          autoComplete="email"
          autoFocus
          required
        />
        <TextInput
          label="Username"
          value={username}
          onChange={(event) => setUsername(event.target.value)}
          error={errors.username}
          hint="This becomes your personal namespace."
          autoComplete="username"
          required
        />
        <div className="grid gap-3 sm:grid-cols-2">
          <TextInput
            label="First name"
            value={firstName}
            onChange={(event) => setFirstName(event.target.value)}
            autoComplete="given-name"
          />
          <TextInput
            label="Last name"
            value={lastName}
            onChange={(event) => setLastName(event.target.value)}
            autoComplete="family-name"
          />
        </div>
        <TextInput
          label="Password"
          type="password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          error={errors.password}
          autoComplete="new-password"
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
          {submitting ? 'Creating account…' : 'Create account'}
        </Button>
      </form>
    </AuthLayout>
  )
}
