import { useState, type FormEvent } from 'react'
import { Link, useSearchParams } from 'react-router-dom'

import { resetPasswordFormSchema } from '../api/schemas'
import { AuthLayout } from '../components/AuthLayout'
import { Button } from '../components/primitives/Button'
import { Flash } from '../components/primitives/Flash'
import { TextInput } from '../components/primitives/TextInput'
import { useAuth } from '../lib/auth-context'
import { authErrorMessage } from '../lib/auth-errors'
import { validateForm, type FieldErrors } from '../lib/forms'

export function ResetPasswordPage() {
  const [searchParams] = useSearchParams()
  const token = searchParams.get('token')
  const { resetPassword } = useAuth()

  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [errors, setErrors] = useState<FieldErrors>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [done, setDone] = useState(false)

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!token) return
    setFormError(null)

    const validation = validateForm(resetPasswordFormSchema, {
      password,
      confirm,
    })
    if (!validation.ok) {
      setErrors(validation.errors)
      if (validation.formError) setFormError(validation.formError)
      return
    }
    if (validation.value.password !== validation.value.confirm) {
      setErrors({ confirm: 'Passwords do not match.' })
      return
    }
    setErrors({})
    setSubmitting(true)
    void resetPassword({ token, password: validation.value.password }).then(
      () => {
        setSubmitting(false)
        setDone(true)
      },
      (error: unknown) => {
        setFormError(authErrorMessage(error))
        setSubmitting(false)
      },
    )
  }

  if (!token) {
    return (
      <AuthLayout title="Reset your password">
        <Flash variant="danger" title="Missing reset token">
          This page needs the token from your reset e-mail.{' '}
          <Link to="/forgot-password" className="text-accent hover:underline">
            Request a new link
          </Link>
          .
        </Flash>
      </AuthLayout>
    )
  }

  return (
    <AuthLayout
      title="Choose a new password"
      subtitle="All other sessions will be signed out."
      footer={
        <p>
          <Link to="/login" className="text-accent hover:underline">
            Back to sign in
          </Link>
        </p>
      }
    >
      {done ? (
        <Flash variant="success" title="Password updated">
          You can now{' '}
          <Link to="/login" className="text-accent hover:underline">
            sign in
          </Link>{' '}
          with your new password.
        </Flash>
      ) : (
        <form className="space-y-3" onSubmit={onSubmit} noValidate>
          {formError ? <Flash variant="danger">{formError}</Flash> : null}
          <TextInput
            label="New password"
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            error={errors.password}
            autoComplete="new-password"
            autoFocus
            required
          />
          <TextInput
            label="Confirm new password"
            type="password"
            value={confirm}
            onChange={(event) => setConfirm(event.target.value)}
            error={errors.confirm}
            autoComplete="new-password"
            required
          />
          <Button
            type="submit"
            variant="primary"
            className="w-full"
            disabled={submitting}
          >
            {submitting ? 'Updating…' : 'Update password'}
          </Button>
        </form>
      )}
    </AuthLayout>
  )
}
