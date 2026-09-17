import { useState, type FormEvent } from 'react'
import { Link } from 'react-router-dom'

import { forgotPasswordFormSchema } from '../api/schemas'
import { AuthLayout } from '../components/AuthLayout'
import { CaptchaGate } from '../components/CaptchaField'
import { Button } from '../components/primitives/Button'
import { Flash } from '../components/primitives/Flash'
import { TextInput } from '../components/primitives/TextInput'
import { useAuth } from '../lib/auth-context'
import { authErrorMessage } from '../lib/auth-errors'
import { validateForm, type FieldErrors } from '../lib/forms'

export function ForgotPasswordPage() {
  const { forgotPassword } = useAuth()

  const [email, setEmail] = useState('')
  const [errors, setErrors] = useState<FieldErrors>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [sent, setSent] = useState(false)

  const [captchaToken, setCaptchaToken] = useState<string | null>(null)
  const [captchaEnabled, setCaptchaEnabled] = useState(false)
  const [captchaSolving, setCaptchaSolving] = useState(false)
  const [captchaKey, setCaptchaKey] = useState(0)

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setFormError(null)

    const validation = validateForm(forgotPasswordFormSchema, { email })
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
    void forgotPassword({
      email: validation.value.email,
      ...(captchaToken ? { captcha: captchaToken } : {}),
    }).then(
      () => {
        setSubmitting(false)
        setSent(true)
      },
      (error: unknown) => {
        setFormError(authErrorMessage(error))
        setCaptchaKey((key) => key + 1)
        setCaptchaToken(null)
        setSubmitting(false)
      },
    )
  }

  return (
    <AuthLayout
      title="Reset your password"
      subtitle="We'll e-mail you a link to choose a new one."
      footer={
        <p>
          Remembered it?{' '}
          <Link to="/login" className="text-accent hover:underline">
            Back to sign in
          </Link>
        </p>
      }
    >
      {sent ? (
        <Flash variant="success" title="Check your inbox">
          If an account exists for <strong>{email}</strong>, a reset link is on
          its way.
        </Flash>
      ) : (
        <form className="space-y-3" onSubmit={onSubmit} noValidate>
          {formError ? <Flash variant="danger">{formError}</Flash> : null}
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
            {submitting ? 'Sending…' : 'Send reset link'}
          </Button>
        </form>
      )}
    </AuthLayout>
  )
}
