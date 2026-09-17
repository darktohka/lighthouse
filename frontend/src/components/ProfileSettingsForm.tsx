import { useState, type FormEvent } from 'react'

import { isApiError } from '../api/client'
import { users as usersApi } from '../api/endpoints'
import { updateProfileFormSchema, type UserProfile } from '../api/schemas'
import { Box, BoxBody, BoxHeader } from './primitives/Box'
import { Button } from './primitives/Button'
import { Flash } from './primitives/Flash'
import { TextInput } from './primitives/TextInput'
import { SelectField, TextArea } from './FormFields'
import { useAuth } from '../lib/auth-context'
import { validateForm, type FieldErrors } from '../lib/forms'
import { useTheme } from '../lib/theme-context'

export function ProfileSettingsForm({ profile }: { profile: UserProfile }) {
  const { refreshMe } = useAuth()
  const { theme, setTheme } = useTheme()
  const [firstName, setFirstName] = useState(profile.first_name ?? '')
  const [lastName, setLastName] = useState(profile.last_name ?? '')
  const [bio, setBio] = useState(profile.bio ?? '')
  const [company, setCompany] = useState(profile.company ?? '')
  const [location, setLocation] = useState(profile.location ?? '')
  const [website, setWebsite] = useState(profile.website ?? '')
  const [formTheme, setFormTheme] = useState<'light' | 'dark'>(theme)
  const [errors, setErrors] = useState<FieldErrors>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [saved, setSaved] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setFormError(null)
    setSaved(null)

    const validation = validateForm(updateProfileFormSchema, {
      first_name: firstName,
      last_name: lastName,
      bio,
      company,
      location,
      website,
      theme: formTheme,
    })
    if (!validation.ok) {
      setErrors(validation.errors)
      if (validation.formError) setFormError(validation.formError)
      return
    }
    setErrors({})

    setSubmitting(true)
    void usersApi
      .updateMe({
        first_name: firstName.trim(),
        last_name: lastName.trim(),
        bio: bio.trim(),
        company: company.trim(),
        location: location.trim(),
        website: website.trim(),
        theme: formTheme,
      })
      .then(
        () => {
          setTheme(formTheme)
          setSubmitting(false)
          setSaved('Your profile has been updated.')
          return refreshMe()
        },
        (error: unknown) => {
          setSubmitting(false)
          setFormError(
            isApiError(error) ? error.message : 'Your profile could not be saved.',
          )
        },
      )
  }

  return (
    <Box>
      <BoxHeader>
        <span className="text-sm font-medium">Public profile</span>
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

          <div className="grid gap-3 sm:grid-cols-2">
            <TextInput
              label="First name"
              value={firstName}
              onChange={(event) => setFirstName(event.target.value)}
              error={errors.first_name}
              autoComplete="given-name"
            />
            <TextInput
              label="Last name"
              value={lastName}
              onChange={(event) => setLastName(event.target.value)}
              error={errors.last_name}
              autoComplete="family-name"
            />
          </div>
          <TextArea
            label="Bio"
            value={bio}
            onChange={(event) => setBio(event.target.value)}
            error={errors.bio}
            hint="A short description shown on your profile."
          />
          <div className="grid gap-3 sm:grid-cols-2">
            <TextInput
              label="Company"
              value={company}
              onChange={(event) => setCompany(event.target.value)}
              error={errors.company}
              autoComplete="organization"
            />
            <TextInput
              label="Location"
              value={location}
              onChange={(event) => setLocation(event.target.value)}
              error={errors.location}
              autoComplete="address-level2"
            />
          </div>
          <TextInput
            label="Website"
            value={website}
            onChange={(event) => setWebsite(event.target.value)}
            error={errors.website}
            placeholder="https://example.com"
            autoComplete="url"
          />
          <SelectField
            label="Theme"
            value={formTheme}
            onChange={(event) =>
              setFormTheme(event.target.value === 'dark' ? 'dark' : 'light')
            }
            options={[
              { value: 'light', label: 'Light' },
              { value: 'dark', label: 'Dark' },
            ]}
          />
          <Button type="submit" variant="primary" disabled={submitting}>
            {submitting ? 'Saving…' : 'Save profile'}
          </Button>
        </form>
      </BoxBody>
    </Box>
  )
}
