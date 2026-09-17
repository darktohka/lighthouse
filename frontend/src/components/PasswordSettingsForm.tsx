import { useState, type FormEvent } from 'react'

import { isApiError } from '../api/client'
import { auth } from '../api/endpoints'
import { changePasswordFormSchema } from '../api/schemas'
import { Box, BoxBody, BoxHeader } from './primitives/Box'
import { Button } from './primitives/Button'
import { Flash } from './primitives/Flash'
import { TextInput } from './primitives/TextInput'
import { validateForm, type FieldErrors } from '../lib/forms'

export function PasswordSettingsForm() {
  const [currentPassword, setCurrentPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [errors, setErrors] = useState<FieldErrors>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [saved, setSaved] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setFormError(null)
    setSaved(null)

    const validation = validateForm(changePasswordFormSchema, {
      current_password: currentPassword,
      new_password: newPassword,
      confirm,
    })
    const nextErrors = validation.ok ? {} : validation.errors
    if (!validation.ok && validation.formError) setFormError(validation.formError)
    if (newPassword !== confirm) {
      nextErrors.confirm = 'The new passwords do not match'
    }
    if (Object.keys(nextErrors).length > 0) {
      setErrors(nextErrors)
      return
    }
    setErrors({})

    setSubmitting(true)
    void auth.changePassword(currentPassword, newPassword).then(
      () => {
        setSubmitting(false)
        setCurrentPassword('')
        setNewPassword('')
        setConfirm('')
        setSaved(
          'Your password has been changed. Other sessions have been signed out.',
        )
      },
      (error: unknown) => {
        setSubmitting(false)
        setFormError(
          isApiError(error) ? error.message : 'Your password could not be changed.',
        )
      },
    )
  }

  return (
    <Box>
      <BoxHeader>
        <span className="text-sm font-medium">Change password</span>
      </BoxHeader>
      <BoxBody>
        <form className="space-y-3" onSubmit={onSubmit} noValidate>
          {formError ? (
            <Flash variant="danger" onDismiss={() => setFormError(null)}>
              {formError}
            </Flash>
          ) : null}
          {saved ? (
            <Flash variant="success" onDismiss={() => setSaved(null)}>
              {saved}
            </Flash>
          ) : null}

          <TextInput
            label="Current password"
            type="password"
            value={currentPassword}
            onChange={(event) => setCurrentPassword(event.target.value)}
            error={errors.current_password}
            autoComplete="current-password"
            required
          />
          <TextInput
            label="New password"
            type="password"
            value={newPassword}
            onChange={(event) => setNewPassword(event.target.value)}
            error={errors.new_password}
            hint="At least 8 characters."
            autoComplete="new-password"
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
          <Button type="submit" variant="primary" disabled={submitting}>
            {submitting ? 'Changing…' : 'Change password'}
          </Button>
        </form>
      </BoxBody>
    </Box>
  )
}
