/**
 * Authentication provider.
 *
 * The access token never touches JS — it is an httpOnly cookie. This provider
 * only mirrors the *identity* returned by `/api/auth/me` and stores the
 * refresh token (the one credential the backend explicitly returns in JSON).
 */
import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react'

import {
  clearRefreshToken,
  getRefreshToken,
  isApiError,
  setRefreshToken,
} from '../api/client'
import { auth } from '../api/endpoints'
import type {
  AuthUser,
  ForgotPasswordRequest,
  LoginRequest,
  LoginResponse,
  Namespace,
  RegisterRequest,
  ResetPasswordRequest,
} from '../api/schemas'
import { AuthContext, type AuthStatus, type LoginOutcome } from './auth-context'
import { useTheme } from './theme-context'

export function AuthProvider({ children }: { children: ReactNode }) {
  const { setTheme } = useTheme()
  const [status, setStatus] = useState<AuthStatus>('loading')
  const [user, setUser] = useState<AuthUser | null>(null)
  const [namespaces, setNamespaces] = useState<Namespace[]>([])

  /** The server-side `theme` preference wins over the local one when present. */
  const applyServerTheme = useCallback(
    (nextUser: AuthUser | null) => {
      if (nextUser) setTheme(nextUser.theme)
    },
    [setTheme],
  )

  const refreshMe = useCallback(async () => {
    const response = await auth.me()
    setUser(response.user)
    setNamespaces(response.namespaces)
    setStatus('authenticated')
    applyServerTheme(response.user)
  }, [applyServerTheme])

  useEffect(() => {
    let active = true
    void (async () => {
      try {
        const response = await auth.me()
        if (!active) return
        setUser(response.user)
        setNamespaces(response.namespaces)
        setStatus('authenticated')
        applyServerTheme(response.user)
      } catch (error) {
        if (!active) return
        // A 401 after the client's refresh attempt means the session is gone.
        if (isApiError(error) && error.status === 401) clearRefreshToken()
        setUser(null)
        setNamespaces([])
        setStatus('anonymous')
      }
    })()
    return () => {
      active = false
    }
  }, [applyServerTheme])

  const finishLogin = useCallback(
    async (response: LoginResponse) => {
      setRefreshToken(response.refresh_token)
      setUser(response.user)
      setStatus('authenticated')
      applyServerTheme(response.user)
      try {
        const me = await auth.me()
        setNamespaces(me.namespaces)
      } catch {
        setNamespaces([])
      }
    },
    [applyServerTheme],
  )

  const login = useCallback(
    async (body: LoginRequest): Promise<LoginOutcome> => {
      const response = await auth.login(body)
      if ('two_factor_required' in response) {
        return { status: 'two_factor_required', mfaToken: response.mfa_token }
      }
      await finishLogin(response)
      return { status: 'ok' }
    },
    [finishLogin],
  )

  const verifyTwoFactor = useCallback(
    async (mfaToken: string, code: string) => {
      const response = await auth.loginTwoFactor(mfaToken, code)
      await finishLogin(response)
    },
    [finishLogin],
  )

  const register = useCallback(async (body: RegisterRequest) => {
    await auth.register(body)
  }, [])

  const verifyEmail = useCallback(async (token: string) => {
    await auth.verifyEmail(token)
  }, [])

  const resendVerification = useCallback(async (email: string) => {
    await auth.resendVerification(email)
  }, [])

  const forgotPassword = useCallback(async (body: ForgotPasswordRequest) => {
    await auth.forgotPassword(body)
  }, [])

  const resetPassword = useCallback(async (body: ResetPasswordRequest) => {
    await auth.resetPassword(body)
  }, [])

  const logout = useCallback(async () => {
    const token = getRefreshToken()
    try {
      await auth.logout(token ?? undefined)
    } catch {
      /* even if the network call fails, drop local identity */
    }
    clearRefreshToken()
    setUser(null)
    setNamespaces([])
    setStatus('anonymous')
  }, [])

  const value = useMemo(
    () => ({
      status,
      user,
      namespaces,
      login,
      verifyTwoFactor,
      register,
      verifyEmail,
      resendVerification,
      forgotPassword,
      resetPassword,
      logout,
      refreshMe,
    }),
    [
      status,
      user,
      namespaces,
      login,
      verifyTwoFactor,
      register,
      verifyEmail,
      resendVerification,
      forgotPassword,
      resetPassword,
      logout,
      refreshMe,
    ],
  )

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>
}
