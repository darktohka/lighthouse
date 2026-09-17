import { isApiError } from '../api/client'

/** Maps server error codes (docs/AUTH.md §4) to human sentences. */
export function authErrorMessage(error: unknown): string {
  if (!isApiError(error)) {
    return error instanceof Error ? error.message : 'Something went wrong.'
  }
  switch (error.code) {
    case 'invalid_credentials':
      return 'Incorrect username/e-mail or password.'
    case 'email_not_verified':
      return 'Your e-mail address is not verified yet. Check your inbox or resend the verification e-mail.'
    case 'username_taken':
      return 'That username is already taken.'
    case 'namespace_taken':
      return 'That name is already in use by another user or workspace.'
    case 'email_taken':
      return 'That e-mail address is already registered.'
    case 'registration_disabled':
      return 'Registration is currently disabled.'
    case 'rate_limited':
      return 'Too many attempts. Please wait a moment and try again.'
    case 'forbidden':
      return 'You are not allowed to perform this action.'
    case 'unauthorized':
      return 'Your session has expired. Please sign in again.'
    case 'captcha_required':
      return 'Please solve the captcha before continuing.'
    default:
      return error.message
  }
}

export function isEmailNotVerified(error: unknown): boolean {
  return isApiError(error) && error.code === 'email_not_verified'
}
