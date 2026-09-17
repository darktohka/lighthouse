import { createContext, useContext } from 'react'

import type {
  AuthUser,
  ForgotPasswordRequest,
  LoginRequest,
  Namespace,
  RegisterRequest,
  ResetPasswordRequest,
} from '../api/schemas'

export type AuthStatus = 'loading' | 'authenticated' | 'anonymous'

/**
 * A login either completes immediately or pauses for a two-factor code, in
 * which case `mfaToken` must be exchanged via `verifyTwoFactor`.
 */
export type LoginOutcome =
  | { status: 'ok' }
  | { status: 'two_factor_required'; mfaToken: string }

export type AuthContextValue = {
  status: AuthStatus
  user: AuthUser | null
  namespaces: Namespace[]
  login: (body: LoginRequest) => Promise<LoginOutcome>
  verifyTwoFactor: (mfaToken: string, code: string) => Promise<void>
  register: (body: RegisterRequest) => Promise<void>
  verifyEmail: (token: string) => Promise<void>
  resendVerification: (email: string) => Promise<void>
  forgotPassword: (body: ForgotPasswordRequest) => Promise<void>
  resetPassword: (body: ResetPasswordRequest) => Promise<void>
  logout: () => Promise<void>
  refreshMe: () => Promise<void>
}

export const AuthContext = createContext<AuthContextValue | null>(null)

export function useAuth(): AuthContextValue {
  const context = useContext(AuthContext)
  if (!context) {
    throw new Error('useAuth must be used within an AuthProvider')
  }
  return context
}
