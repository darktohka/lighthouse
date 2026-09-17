/**
 * Typed bindings for every endpoint in `docs/API.md` and `docs/AUTH.md`.
 *
 * Pages call these instead of hand-writing paths, so encoding and schema
 * parsing stay in one place. Endpoints whose response shape is not described
 * by the contract are typed as opaque JSON and flagged in the doc comment.
 */
import * as v from 'valibot'

import { api, refreshSession, type RequestOptions } from './client'
import {
  activityEntrySchema,
  analyticsOverviewSchema,
  appPasswordSchema,
  backupCodesResponseSchema,
  batchDeleteResultSchema,
  captchaChallengeSchema,
  createdAppPasswordSchema,
  createdServiceAccountSchema,
  dashboardSchema,
  heatmapSchema,
  jsonObjectSchema,
  layerReferenceSchema,
  layerTreeSchema,
  loginResponseSchema,
  loginResultSchema,
  meResponseSchema,
  namespaceMemberSchema,
  namespaceSchema,
  pageSchema,
  publicPermissionSchema,
  registerResponseSchema,
  repositoryDetailSchema,
  repositorySummarySchema,
  resetPasswordResponseSchema,
  serviceAccountIpRangeSchema,
  serviceAccountSchema,
  tagDetailSchema,
  tagSizeEntrySchema,
  tagSummarySchema,
  twoFactorEnabledSchema,
  twoFactorSetupSchema,
  twoFactorStatusSchema,
  twoFactorVerifiedSchema,
  userProfileSchema,
  userSummarySchema,
  verifyEmailResponseSchema,
  type CreateAppPassword,
  type CreateGrant,
  type CreateNamespace,
  type CreateServiceAccount,
  type CreateServiceAccountGrant,
  type CreateServiceAccountIpRange,
  type ForgotPasswordRequest,
  type LoginRequest,
  type RegisterRequest,
  type ResetPasswordRequest,
  type UpdateProfile,
  type UpdateRepository,
} from './schemas'
import {
  blobApiPath,
  buildQuery,
  namespaceApiPath,
  repositoryApiPath,
  tagApiPath,
} from '../lib/paths'
import type { RepositoryOrder, RepositorySort } from '../lib/repositorySort'

const namespacePageSchema = pageSchema(namespaceSchema)
const repositoryPageSchema = pageSchema(repositorySummarySchema)
const tagPageSchema = pageSchema(tagSummarySchema)
const tagSizePageSchema = pageSchema(tagSizeEntrySchema)
const activityPageSchema = pageSchema(activityEntrySchema)
const userPageSchema = pageSchema(userSummarySchema)
const memberPageSchema = pageSchema(namespaceMemberSchema)
const permissionPageSchema = pageSchema(publicPermissionSchema)

// ---------------------------------------------------------------------------
// Auth (AUTH.md §4)
// ---------------------------------------------------------------------------

export const auth = {
  login(body: LoginRequest, options?: RequestOptions) {
    return api.post('/auth/login', loginResultSchema, body, {
      ...options,
      retry: false,
    })
  },

  loginTwoFactor(mfaToken: string, code: string, options?: RequestOptions) {
    return api.post(
      '/auth/login/2fa',
      loginResponseSchema,
      { mfa_token: mfaToken, code },
      { ...options, retry: false },
    )
  },

  register(body: RegisterRequest, options?: RequestOptions) {
    return api.post('/auth/register', registerResponseSchema, body, {
      ...options,
      retry: false,
    })
  },

  logout(refreshToken?: string) {
    return api.postVoid(
      '/auth/logout',
      refreshToken ? { refresh_token: refreshToken } : {},
    )
  },

  refresh: refreshSession,

  me(options?: RequestOptions) {
    return api.get('/auth/me', meResponseSchema, options)
  },

  /**
   * Probes whether the proof-of-work captcha is enabled. `204` means disabled
   * (see `docs/AUTH.md` §5); anything else means the widget must solve a
   * challenge via `pow-captcha-react` before form submission.
   */
  async captchaEnabled(options?: RequestOptions): Promise<boolean> {
    const response = await api.raw('/auth/captcha', options)
    if (response.status === 204) return false
    // Parse the descriptor defensively; a malformed body still means "enabled".
    const parsed = v.safeParse(captchaChallengeSchema, await response.json())
    return Boolean(parsed)
  },

  verifyEmail(token: string) {
    return api.post(
      '/auth/verify-email',
      verifyEmailResponseSchema,
      { token },
      { retry: false },
    )
  },

  resendVerification(email: string) {
    return api.postVoid('/auth/resend-verification', { email }, { retry: false })
  },

  forgotPassword(body: ForgotPasswordRequest) {
    return api.postVoid('/auth/forgot-password', body, { retry: false })
  },

  resetPassword(body: ResetPasswordRequest) {
    return api.post(
      '/auth/reset-password',
      resetPasswordResponseSchema,
      body,
      { retry: false },
    )
  },

  changePassword(currentPassword: string, newPassword: string) {
    return api.postVoid('/auth/change-password', {
      current_password: currentPassword,
      new_password: newPassword,
    })
  },

  twoFactorStatus(options?: RequestOptions) {
    return api.get('/auth/2fa', twoFactorStatusSchema, options)
  },

  twoFactorSetup() {
    return api.post('/auth/2fa/setup', twoFactorSetupSchema)
  },

  twoFactorVerify(code: string) {
    return api.post('/auth/2fa/verify', twoFactorVerifiedSchema, { code })
  },

  twoFactorEnable() {
    return api.post('/auth/2fa/enable', twoFactorEnabledSchema)
  },

  twoFactorDisable(password: string, code: string) {
    return api.post(
      '/auth/2fa/disable',
      twoFactorEnabledSchema,
      { password, code },
      { retry: false },
    )
  },

  twoFactorBackupCodes(code: string) {
    return api.post(
      '/auth/2fa/backup-codes',
      backupCodesResponseSchema,
      { code },
      { retry: false },
    )
  },
}

// ---------------------------------------------------------------------------
// App passwords (AUTH.md §4)
// ---------------------------------------------------------------------------

export const appPasswords = {
  list(options?: RequestOptions) {
    return api.get('/app-passwords', v.array(appPasswordSchema), options)
  },

  create(body: CreateAppPassword) {
    return api.post('/app-passwords', createdAppPasswordSchema, body)
  },

  rotate(id: number) {
    return api.post(`/app-passwords/${id}/token`, createdAppPasswordSchema)
  },

  remove(id: number) {
    return api.deleteVoid(`/app-passwords/${id}`)
  },
}

// ---------------------------------------------------------------------------
// Users & social (API.md §1)
// ---------------------------------------------------------------------------

export const users = {
  profile(username: string, options?: RequestOptions) {
    return api.get(`/users/${encodeURIComponent(username)}`, userProfileSchema, options)
  },

  updateMe(body: UpdateProfile) {
    return api.patch('/users/me', userProfileSchema, body)
  },

  heatmap(username: string, year?: number, options?: RequestOptions) {
    return api.get(
      `/users/${encodeURIComponent(username)}/heatmap${buildQuery({ year })}`,
      heatmapSchema,
      options,
    )
  },

  followers(username: string, page = 1, perPage = 25, options?: RequestOptions) {
    return api.get(
      `/users/${encodeURIComponent(username)}/followers${buildQuery({ page, per_page: perPage })}`,
      userPageSchema,
      options,
    )
  },

  following(username: string, page = 1, perPage = 25, options?: RequestOptions) {
    return api.get(
      `/users/${encodeURIComponent(username)}/following${buildQuery({ page, per_page: perPage })}`,
      userPageSchema,
      options,
    )
  },

  follow(username: string) {
    return api.postVoid(`/users/${encodeURIComponent(username)}/follow`)
  },

  unfollow(username: string) {
    return api.deleteVoid(`/users/${encodeURIComponent(username)}/follow`)
  },

  search(query: string, limit = 8, options?: RequestOptions) {
    return api.get(
      `/users/search${buildQuery({ q: query, limit })}`,
      userPageSchema,
      options,
    )
  },
}

// ---------------------------------------------------------------------------
// Namespaces (API.md §2)
// ---------------------------------------------------------------------------

export const namespaces = {
  list(page = 1, perPage = 25, options?: RequestOptions) {
    return api.get(
      `/namespaces${buildQuery({ page, per_page: perPage })}`,
      namespacePageSchema,
      options,
    )
  },

  create(body: CreateNamespace) {
    return api.post('/namespaces', namespaceSchema, body)
  },

  detail(name: string, options?: RequestOptions) {
    return api.get(namespaceApiPath(name), namespaceSchema, options)
  },

  update(name: string, body: { description?: string; is_public?: boolean }) {
    return api.patch(namespaceApiPath(name), namespaceSchema, body)
  },

  remove(name: string) {
    return api.deleteVoid(namespaceApiPath(name))
  },

  members(name: string, page = 1, perPage = 25, options?: RequestOptions) {
    return api.get(
      `${namespaceApiPath(name)}/members${buildQuery({ page, per_page: perPage })}`,
      memberPageSchema,
      options,
    )
  },

  addMember(name: string, username: string, role: 'admin' | 'member') {
    return api.post(`${namespaceApiPath(name)}/members`, namespaceMemberSchema, {
      username,
      role,
    })
  },

  removeMember(name: string, username: string) {
    return api.deleteVoid(
      `${namespaceApiPath(name)}/members/${encodeURIComponent(username)}`,
    )
  },
}

// ---------------------------------------------------------------------------
// Repositories & tags (API.md §3)
// ---------------------------------------------------------------------------

export const repositories = {
  list(
    namespace: string,
    page = 1,
    perPage = 25,
    params?: { sort?: RepositorySort; order?: RepositoryOrder },
    options?: RequestOptions,
  ) {
    return api.get(
      `${namespaceApiPath(namespace)}/repositories${buildQuery({
        page,
        per_page: perPage,
        sort: params?.sort,
        order: params?.order,
      })}`,
      repositoryPageSchema,
      options,
    )
  },

  detail(namespace: string, repo: string, options?: RequestOptions) {
    return api.get(repositoryApiPath(namespace, repo), repositoryDetailSchema, options)
  },

  update(namespace: string, repo: string, body: UpdateRepository) {
    return api.patch(repositoryApiPath(namespace, repo), repositoryDetailSchema, body)
  },

  remove(namespace: string, repo: string) {
    return api.deleteVoid(repositoryApiPath(namespace, repo))
  },

  tags(
    namespace: string,
    repo: string,
    page = 1,
    perPage = 25,
    options?: RequestOptions,
  ) {
    return api.get(
      `${repositoryApiPath(namespace, repo)}/tags${buildQuery({ page, per_page: perPage })}`,
      tagPageSchema,
      options,
    )
  },

  tag(namespace: string, repo: string, tag: string, options?: RequestOptions) {
    return api.get(tagApiPath(namespace, repo, tag), tagDetailSchema, options)
  },

  deleteTag(namespace: string, repo: string, tag: string) {
    return api.deleteVoid(tagApiPath(namespace, repo, tag))
  },

  batchDeleteTags(namespace: string, repo: string, tags: string[]) {
    return api.post(
      `${repositoryApiPath(namespace, repo)}/tags/batch-delete`,
      batchDeleteResultSchema,
      { tags },
    )
  },

  /**
   * `GET /repositories/{namespace}/{*repo}/pulls?days=` — API.md describes the
   * endpoint but not its response body, so it is bound as opaque JSON until the
   * contract is firmed up. The pull-statistics UI is deferred to the next wave.
   */
  pulls(namespace: string, repo: string, days = 30, options?: RequestOptions) {
    return api.get(
      `${repositoryApiPath(namespace, repo)}/pulls${buildQuery({ days })}`,
      jsonObjectSchema,
      options,
    )
  },
}

export const tags = {
  ranked(
    sort: 'total_size' | 'unique_size',
    order: 'asc' | 'desc',
    namespace: string | undefined,
    page = 1,
    perPage = 25,
    options?: RequestOptions,
  ) {
    return api.get(
      `/tags${buildQuery({ sort, order, namespace, page, per_page: perPage })}`,
      tagSizePageSchema,
      options,
    )
  },

  batchDelete(items: Array<{ repository: string; tag: string }>) {
    return api.post('/tags/batch-delete', batchDeleteResultSchema, { items })
  },
}

// ---------------------------------------------------------------------------
// Manifest & layer browsing (API.md §4)
// ---------------------------------------------------------------------------

export const manifests = {
  raw(namespace: string, repo: string, digest: string, options?: RequestOptions) {
    return api.get(
      `${repositoryApiPath(namespace, repo)}/manifests/${encodeURIComponent(digest)}`,
      jsonObjectSchema,
      options,
    )
  },

  references(namespace: string, repo: string, digest: string, options?: RequestOptions) {
    return api.get(
      `${repositoryApiPath(namespace, repo)}/manifests/${encodeURIComponent(digest)}/references`,
      v.array(layerReferenceSchema),
      options,
    )
  },
}

export const blobs = {
  json(digest: string, options?: RequestOptions) {
    return api.get(blobApiPath(digest, '/json'), jsonObjectSchema, options)
  },

  /** Raw bytes as a `Response` (used for text previews and downloads). */
  raw(digest: string, options?: RequestOptions) {
    return api.raw(blobApiPath(digest), options)
  },
}

export const layers = {
  tree(
    namespace: string,
    repo: string,
    digest: string,
    path: string | undefined,
    options?: RequestOptions,
  ) {
    return api.get(
      `${repositoryApiPath(namespace, repo)}/layers/${encodeURIComponent(digest)}/tree${buildQuery({ path })}`,
      layerTreeSchema,
      options,
    )
  },

  layerFilePath(namespace: string, repo: string, digest: string, path: string) {
    return `${repositoryApiPath(namespace, repo)}/layers/${encodeURIComponent(digest)}/file${buildQuery({ path })}`
  },

  layerDownloadPath(namespace: string, repo: string, digest: string) {
    return `${repositoryApiPath(namespace, repo)}/layers/${encodeURIComponent(digest)}/download`
  },

  file(
    namespace: string,
    repo: string,
    digest: string,
    path: string,
    options?: RequestOptions,
  ) {
    return api.raw(layers.layerFilePath(namespace, repo, digest, path), options)
  },
}

// ---------------------------------------------------------------------------
// Permissions (API.md §5)
// ---------------------------------------------------------------------------

export const permissions = {
  onNamespace(name: string, options?: RequestOptions) {
    return api.get(
      `${namespaceApiPath(name)}/permissions`,
      permissionPageSchema,
      options,
    )
  },

  addToNamespace(name: string, body: CreateGrant) {
    return api.post(
      `${namespaceApiPath(name)}/permissions`,
      publicPermissionSchema,
      body,
    )
  },

  revokeOnNamespace(name: string, id: number) {
    return api.deleteVoid(`${namespaceApiPath(name)}/permissions/${id}`)
  },

  onRepository(namespace: string, repo: string, options?: RequestOptions) {
    return api.get(
      `${repositoryApiPath(namespace, repo)}/permissions`,
      permissionPageSchema,
      options,
    )
  },

  addToRepository(namespace: string, repo: string, body: CreateGrant) {
    return api.post(
      `${repositoryApiPath(namespace, repo)}/permissions`,
      publicPermissionSchema,
      body,
    )
  },

  revokeOnRepository(namespace: string, repo: string, id: number) {
    return api.deleteVoid(`${repositoryApiPath(namespace, repo)}/permissions/${id}`)
  },
}

// ---------------------------------------------------------------------------
// Service accounts (API.md §6)
// ---------------------------------------------------------------------------

export const serviceAccounts = {
  list(options?: RequestOptions) {
    return api.get('/service-accounts', pageSchema(serviceAccountSchema), options)
  },

  create(body: CreateServiceAccount) {
    return api.post('/service-accounts', createdServiceAccountSchema, body)
  },

  detail(id: number, options?: RequestOptions) {
    return api.get(`/service-accounts/${id}`, serviceAccountSchema, options)
  },

  remove(id: number) {
    return api.deleteVoid(`/service-accounts/${id}`)
  },

  rotate(id: number) {
    return api.post(`/service-accounts/${id}/token`, createdServiceAccountSchema)
  },

  addGrant(id: number, body: CreateServiceAccountGrant) {
    return api.post(`/service-accounts/${id}/grants`, serviceAccountSchema, body)
  },

  removeGrant(id: number, grantId: number) {
    return api.deleteVoid(`/service-accounts/${id}/grants/${grantId}`)
  },

  addIpRange(id: number, body: CreateServiceAccountIpRange) {
    return api.post(`/service-accounts/${id}/ip-ranges`, serviceAccountIpRangeSchema, body)
  },

  removeIpRange(id: number, rangeId: number) {
    return api.deleteVoid(`/service-accounts/${id}/ip-ranges/${rangeId}`)
  },
}

// ---------------------------------------------------------------------------
// Analytics, activity & dashboard (API.md §7–8)
// ---------------------------------------------------------------------------

export const analytics = {
  overview(namespace?: string, options?: RequestOptions) {
    return api.get(
      `/analytics/overview${buildQuery({ namespace })}`,
      analyticsOverviewSchema,
      options,
    )
  },
}

export const activity = {
  feed(page = 1, perPage = 25, options?: RequestOptions) {
    return api.get(
      `/activity${buildQuery({ page, per_page: perPage })}`,
      activityPageSchema,
      options,
    )
  },

  mine(page = 1, perPage = 25, options?: RequestOptions) {
    return api.get(
      `/activity/me${buildQuery({ page, per_page: perPage })}`,
      activityPageSchema,
      options,
    )
  },
}

export const dashboard = {
  load(options?: RequestOptions) {
    return api.get('/dashboard', dashboardSchema, options)
  },
}

// ---------------------------------------------------------------------------
// Wave 3 bindings
//
// Several list endpoints return a bare JSON array in the shipped backend even
// though `docs/API.md` documents the paginated envelope (`/users/search`,
// namespace members, namespace/repository permissions, service accounts).
// These wrappers accept either shape and normalize to a plain array so pages
// depend on one verified contract regardless of which form the server sends.
// They only ever *append* to the frozen endpoint layer.
// ---------------------------------------------------------------------------

import {
  loginHistoryResponseSchema,
  serviceAccountGrantSchema,
  sessionsResponseSchema,
} from './schemas'

const userSearchResponseSchema = v.union([
  v.array(userSummarySchema),
  pageSchema(userSummarySchema),
])
const memberListResponseSchema = v.union([
  v.array(namespaceMemberSchema),
  pageSchema(namespaceMemberSchema),
])
const permissionListResponseSchema = v.union([
  v.array(publicPermissionSchema),
  pageSchema(publicPermissionSchema),
])
const serviceAccountListResponseSchema = v.union([
  v.array(serviceAccountSchema),
  pageSchema(serviceAccountSchema),
])

/** Username autocomplete for delegation forms. */
export const directory = {
  searchUsers(query: string, limit = 8, options?: RequestOptions) {
    return api
      .get(
        `/users/search${buildQuery({ q: query, limit })}`,
        userSearchResponseSchema,
        options,
      )
      .then((value) => (Array.isArray(value) ? value : value.items))
  },
}

/** Namespace membership with a tolerant list shape. */
export const namespaceMembers = {
  list(name: string, options?: RequestOptions) {
    return api
      .get(`${namespaceApiPath(name)}/members`, memberListResponseSchema, options)
      .then((value) => (Array.isArray(value) ? value : value.items))
  },
}

/** Permission grants with a tolerant list shape. */
export const grants = {
  onNamespace(name: string, options?: RequestOptions) {
    return api
      .get(
        `${namespaceApiPath(name)}/permissions`,
        permissionListResponseSchema,
        options,
      )
      .then((value) => (Array.isArray(value) ? value : value.items))
  },

  onRepository(namespace: string, repo: string, options?: RequestOptions) {
    return api
      .get(
        `${repositoryApiPath(namespace, repo)}/permissions`,
        permissionListResponseSchema,
        options,
      )
      .then((value) => (Array.isArray(value) ? value : value.items))
  },
}

/** Service accounts with a tolerant list shape. */
export const serviceAccountList = {
  list(options?: RequestOptions) {
    return api
      .get('/service-accounts', serviceAccountListResponseSchema, options)
      .then((value) => (Array.isArray(value) ? value : value.items))
  },
}

/** Adding a grant returns the new grant, not the whole account. */
export const serviceAccountGrants = {
  add(id: number, body: CreateServiceAccountGrant) {
    return api.post(
      `/service-accounts/${id}/grants`,
      serviceAccountGrantSchema,
      body,
    )
  },
}

/** Web sessions and login history (docs/AUTH.md §4). */
export const authSessions = {
  list(options?: RequestOptions) {
    return api.get('/auth/sessions', sessionsResponseSchema, options)
  },

  revoke(id: string) {
    return api.deleteVoid(`/auth/sessions/${encodeURIComponent(id)}`)
  },

  loginHistory(limit = 50, offset = 0, options?: RequestOptions) {
    return api.get(
      `/auth/login-history${buildQuery({ limit, offset })}`,
      loginHistoryResponseSchema,
      options,
    )
  },
}
