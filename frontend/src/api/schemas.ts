/**
 * Valibot schemas for every DTO exposed by the control-plane API.
 *
 * `docs/API.md` and `docs/AUTH.md` are the authoritative contract; these
 * schemas are the runtime enforcement of it. Responses are always parsed
 * through them (`src/api/client.ts`) so TypeScript types are *derived from*
 * verified data rather than asserted on top of raw JSON.
 */
import * as v from 'valibot'

// ---------------------------------------------------------------------------
// Shared building blocks
// ---------------------------------------------------------------------------

/** RFC 3339 UTC timestamp, e.g. `2026-09-17T10:30:00.000Z`. */
export const timestampSchema = v.string()
export type Timestamp = v.InferOutput<typeof timestampSchema>

/** Opaque JSON object (manifest, config, activity metadata). */
export const jsonObjectSchema = v.record(v.string(), v.unknown())
export type JsonObject = v.InferOutput<typeof jsonObjectSchema>

/** Pagination envelope used by every list endpoint. */
export function pageSchema<T>(item: v.GenericSchema<unknown, T>) {
  return v.object({
    items: v.array(item),
    total: v.number(),
    page: v.number(),
    per_page: v.number(),
  })
}
export type Page<T> = { items: T[]; total: number; page: number; per_page: number }

/** `{ "error": { "code", "message" } }` envelope. */
export const errorEnvelopeSchema = v.object({
  error: v.object({
    code: v.string(),
    message: v.string(),
  }),
})
export type ErrorEnvelope = v.InferOutput<typeof errorEnvelopeSchema>

// ---------------------------------------------------------------------------
// Users & social (API.md §1)
// ---------------------------------------------------------------------------

export const userSummarySchema = v.object({
  id: v.number(),
  username: v.string(),
  first_name: v.nullable(v.string()),
  last_name: v.nullable(v.string()),
  avatar_url: v.nullable(v.string()),
  avatar_hash: v.string(),
})
export type UserSummary = v.InferOutput<typeof userSummarySchema>

export const namespaceKindSchema = v.picklist(['user', 'workspace'])
export type NamespaceKind = v.InferOutput<typeof namespaceKindSchema>

export const userProfileSchema = v.object({
  id: v.number(),
  username: v.string(),
  first_name: v.nullable(v.string()),
  last_name: v.nullable(v.string()),
  bio: v.nullable(v.string()),
  company: v.nullable(v.string()),
  location: v.nullable(v.string()),
  website: v.nullable(v.string()),
  avatar_url: v.nullable(v.string()),
  /** Lowercase hex SHA-256 of the normalized profile e-mail (Libravatar key). */
  avatar_hash: v.string(),
  created_at: timestampSchema,
  namespace: v.nullable(v.string()),
  repository_count: v.number(),
  public_repository_count: v.number(),
  total_pulls: v.number(),
  follower_count: v.number(),
  following_count: v.number(),
  is_following: v.boolean(),
  is_self: v.boolean(),
})
export type UserProfile = v.InferOutput<typeof userProfileSchema>

export const updateProfileSchema = v.object({
  first_name: v.optional(v.string()),
  last_name: v.optional(v.string()),
  bio: v.optional(v.string()),
  company: v.optional(v.string()),
  location: v.optional(v.string()),
  website: v.optional(v.string()),
  theme: v.optional(v.picklist(['light', 'dark'])),
})
export type UpdateProfile = v.InferOutput<typeof updateProfileSchema>

export const heatmapDaySchema = v.object({
  date: v.string(),
  count: v.number(),
})
export type HeatmapDay = v.InferOutput<typeof heatmapDaySchema>

export const heatmapSchema = v.object({
  start: v.string(),
  end: v.string(),
  days: v.array(heatmapDaySchema),
  total: v.number(),
})
export type Heatmap = v.InferOutput<typeof heatmapSchema>

// ---------------------------------------------------------------------------
// Namespaces (API.md §2)
// ---------------------------------------------------------------------------

export const namespaceSchema = v.object({
  id: v.number(),
  name: v.string(),
  kind: namespaceKindSchema,
  owner: v.nullable(userSummarySchema),
  description: v.nullable(v.string()),
  is_public: v.boolean(),
  repository_count: v.number(),
  created_at: timestampSchema,
})
export type Namespace = v.InferOutput<typeof namespaceSchema>

export const namespaceMemberRoleSchema = v.picklist(['admin', 'member'])
export type NamespaceMemberRole = v.InferOutput<typeof namespaceMemberRoleSchema>

export const namespaceMemberSchema = v.object({
  user: userSummarySchema,
  role: namespaceMemberRoleSchema,
  created_at: timestampSchema,
})
export type NamespaceMember = v.InferOutput<typeof namespaceMemberSchema>

export const createNamespaceSchema = v.object({
  name: v.string(),
  description: v.optional(v.string()),
  is_public: v.optional(v.boolean()),
})
export type CreateNamespace = v.InferOutput<typeof createNamespaceSchema>

// ---------------------------------------------------------------------------
// Repositories & tags (API.md §3)
// ---------------------------------------------------------------------------

export const repositorySummarySchema = v.object({
  id: v.number(),
  namespace: v.string(),
  path: v.string(),
  name: v.string(),
  description: v.nullable(v.string()),
  is_public: v.boolean(),
  tag_count: v.number(),
  size: v.number(),
  pull_count: v.number(),
  updated_at: timestampSchema,
})
export type RepositorySummary = v.InferOutput<typeof repositorySummarySchema>

export const platformSchema = v.object({
  os: v.string(),
  architecture: v.string(),
  variant: v.nullable(v.string()),
  digest: v.string(),
  size: v.number(),
})
export type Platform = v.InferOutput<typeof platformSchema>

export const publicPermissionSubjectTypeSchema = v.picklist(['user', 'anonymous'])
export type PublicPermissionSubjectType = v.InferOutput<
  typeof publicPermissionSubjectTypeSchema
>

export const publicPermissionSchema = v.object({
  id: v.number(),
  subject_type: publicPermissionSubjectTypeSchema,
  subject: v.nullable(userSummarySchema),
  can_pull: v.boolean(),
  can_push: v.boolean(),
  created_at: timestampSchema,
})
export type PublicPermission = v.InferOutput<typeof publicPermissionSchema>

export const createGrantSchema = v.object({
  subject_type: publicPermissionSubjectTypeSchema,
  subject: v.optional(v.string()),
  can_push: v.boolean(),
})
export type CreateGrant = v.InferOutput<typeof createGrantSchema>

export const repositoryDetailSchema = v.object({
  id: v.number(),
  namespace: v.string(),
  path: v.string(),
  name: v.string(),
  description: v.nullable(v.string()),
  is_public: v.boolean(),
  tag_count: v.number(),
  size: v.number(),
  pull_count: v.number(),
  updated_at: timestampSchema,
  manifest_count: v.number(),
  platform_count: v.number(),
  total_size: v.number(),
  unique_size: v.number(),
  shared_size: v.number(),
  created_at: timestampSchema,
  created_by: v.nullable(userSummarySchema),
  permissions: v.array(publicPermissionSchema),
  can_pull: v.boolean(),
  can_push: v.boolean(),
})
export type RepositoryDetail = v.InferOutput<typeof repositoryDetailSchema>

export const updateRepositorySchema = v.object({
  description: v.optional(v.string()),
  is_public: v.optional(v.boolean()),
})
export type UpdateRepository = v.InferOutput<typeof updateRepositorySchema>

export const createRepositorySchema = v.object({
  name: v.string(),
  description: v.optional(v.string()),
  is_public: v.optional(v.boolean()),
})
export type CreateRepository = v.InferOutput<typeof createRepositorySchema>

export const tagSummarySchema = v.object({
  name: v.string(),
  digest: v.string(),
  media_type: v.string(),
  size: v.number(),
  compressed_size: v.number(),
  platforms: v.array(platformSchema),
  pull_count: v.number(),
  updated_at: timestampSchema,
})
export type TagSummary = v.InferOutput<typeof tagSummarySchema>

export const layerRoleSchema = v.picklist(['config', 'layer'])
export type LayerRole = v.InferOutput<typeof layerRoleSchema>

export const layerInfoSchema = v.object({
  digest: v.string(),
  media_type: v.string(),
  size: v.number(),
  role: layerRoleSchema,
  created: v.nullable(v.string()),
  created_by: v.nullable(v.string()),
  comment: v.nullable(v.string()),
})
export type LayerInfo = v.InferOutput<typeof layerInfoSchema>

export const platformDetailSchema = v.object({
  os: v.string(),
  architecture: v.string(),
  variant: v.nullable(v.string()),
  digest: v.string(),
  media_type: v.string(),
  size: v.number(),
  manifest: jsonObjectSchema,
  config: v.nullable(jsonObjectSchema),
  layers: v.array(layerInfoSchema),
})
export type PlatformDetail = v.InferOutput<typeof platformDetailSchema>

export const tagDetailSchema = v.object({
  name: v.string(),
  digest: v.string(),
  media_type: v.string(),
  size: v.number(),
  compressed_size: v.number(),
  platforms: v.array(platformSchema),
  pull_count: v.number(),
  updated_at: timestampSchema,
  manifest: jsonObjectSchema,
  config: v.nullable(jsonObjectSchema),
  layers: v.array(layerInfoSchema),
  platform_details: v.array(platformDetailSchema),
  can_pull: v.boolean(),
  can_push: v.boolean(),
})
export type TagDetail = v.InferOutput<typeof tagDetailSchema>

export const tagSizeEntrySchema = v.object({
  repository: v.string(),
  namespace: v.string(),
  tag: v.string(),
  total_size: v.number(),
  unique_size: v.number(),
  shared_size: v.number(),
  platforms: v.array(platformSchema),
  updated_at: timestampSchema,
})
export type TagSizeEntry = v.InferOutput<typeof tagSizeEntrySchema>

export const batchDeleteResultSchema = v.object({ deleted: v.number() })
export type BatchDeleteResult = v.InferOutput<typeof batchDeleteResultSchema>

export const batchDeleteTagsRequestSchema = v.object({
  tags: v.array(v.string()),
})
export type BatchDeleteTagsRequest = v.InferOutput<
  typeof batchDeleteTagsRequestSchema
>

export const batchDeleteTagItemsRequestSchema = v.object({
  items: v.array(v.object({ repository: v.string(), tag: v.string() })),
})
export type BatchDeleteTagItemsRequest = v.InferOutput<
  typeof batchDeleteTagItemsRequestSchema
>

// ---------------------------------------------------------------------------
// Permission grants & service accounts (API.md §5–6)
// ---------------------------------------------------------------------------

export const serviceAccountGrantSchema = v.object({
  id: v.number(),
  namespace: v.nullable(v.string()),
  repository: v.nullable(v.string()),
  can_pull: v.boolean(),
  can_push: v.boolean(),
})
export type ServiceAccountGrant = v.InferOutput<typeof serviceAccountGrantSchema>

export const serviceAccountIpRangeSchema = v.object({
  id: v.number(),
  cidr: v.string(),
})
export type ServiceAccountIpRange = v.InferOutput<typeof serviceAccountIpRangeSchema>

export const serviceAccountSchema = v.object({
  id: v.number(),
  name: v.string(),
  username: v.string(),
  description: v.nullable(v.string()),
  token_prefix: v.string(),
  token_suffix: v.string(),
  created_at: timestampSchema,
  last_used_at: v.nullable(timestampSchema),
  grants: v.array(serviceAccountGrantSchema),
  ip_ranges: v.array(serviceAccountIpRangeSchema),
})
export type ServiceAccount = v.InferOutput<typeof serviceAccountSchema>

export const createdServiceAccountSchema = v.object({
  account: serviceAccountSchema,
  token: v.string(),
})
export type CreatedServiceAccount = v.InferOutput<
  typeof createdServiceAccountSchema
>

export const createServiceAccountSchema = v.object({
  name: v.string(),
  description: v.optional(v.string()),
  ip_ranges: v.optional(v.array(v.string())),
})
export type CreateServiceAccount = v.InferOutput<
  typeof createServiceAccountSchema
>

export const createServiceAccountGrantSchema = v.object({
  namespace: v.optional(v.string()),
  repository: v.optional(v.string()),
  can_push: v.boolean(),
})
export type CreateServiceAccountGrant = v.InferOutput<
  typeof createServiceAccountGrantSchema
>

export const createServiceAccountIpRangeSchema = v.object({ cidr: v.string() })
export type CreateServiceAccountIpRange = v.InferOutput<
  typeof createServiceAccountIpRangeSchema
>

// ---------------------------------------------------------------------------
// Analytics (API.md §7)
// ---------------------------------------------------------------------------

export const diskUsageEntrySchema = v.object({
  repository: v.string(),
  size: v.number(),
  unique_size: v.number(),
})
export type DiskUsageEntry = v.InferOutput<typeof diskUsageEntrySchema>

export const pullsOverTimeSchema = v.object({
  date: v.string(),
  pulls: v.number(),
})
export type PullsOverTime = v.InferOutput<typeof pullsOverTimeSchema>

export const topRepositorySchema = v.object({
  repository: v.string(),
  pulls: v.number(),
})
export type TopRepository = v.InferOutput<typeof topRepositorySchema>

export const analyticsOverviewSchema = v.object({
  total_size: v.number(),
  unique_size: v.number(),
  shared_size: v.number(),
  shared_percentage: v.number(),
  blob_count: v.number(),
  manifest_count: v.number(),
  repository_count: v.number(),
  tag_count: v.number(),
  pull_count: v.number(),
  pull_count_30d: v.number(),
  largest_tags: v.array(tagSizeEntrySchema),
  disk_usage_by_repository: v.array(diskUsageEntrySchema),
  pulls_over_time: v.array(pullsOverTimeSchema),
  top_repositories: v.array(topRepositorySchema),
})
export type AnalyticsOverview = v.InferOutput<typeof analyticsOverviewSchema>

// ---------------------------------------------------------------------------
// Activity & dashboard (API.md §8)
// ---------------------------------------------------------------------------

export const activityEntrySchema = v.object({
  id: v.number(),
  kind: v.string(),
  summary: v.string(),
  actor: v.nullable(userSummarySchema),
  namespace: v.nullable(v.string()),
  repository: v.nullable(v.string()),
  metadata: v.unknown(),
  created_at: timestampSchema,
})
export type ActivityEntry = v.InferOutput<typeof activityEntrySchema>

export const dashboardStatsSchema = v.object({
  repository_count: v.number(),
  tag_count: v.number(),
  total_size: v.number(),
  pull_count_30d: v.number(),
})
export type DashboardStats = v.InferOutput<typeof dashboardStatsSchema>

export const dashboardSchema = v.object({
  repositories: v.array(repositorySummarySchema),
  activity: v.array(activityEntrySchema),
  stats: dashboardStatsSchema,
})
export type Dashboard = v.InferOutput<typeof dashboardSchema>

// ---------------------------------------------------------------------------
// Manifest & layer browsing (API.md §4)
// ---------------------------------------------------------------------------

export const layerTreeEntryKindSchema = v.picklist(['file', 'dir', 'symlink'])
export type LayerTreeEntryKind = v.InferOutput<typeof layerTreeEntryKindSchema>

/** Diff marker for an entry in the `diff` / `aggregate-diff` tree modes. */
export const layerChangeSchema = v.picklist(['new', 'modified', 'removed'])
export type LayerChange = v.InferOutput<typeof layerChangeSchema>

export const layerTreeEntrySchema = v.object({
  name: v.string(),
  path: v.string(),
  kind: layerTreeEntryKindSchema,
  /** Bytes for a file/symlink; recursive total of everything below it for a directory. */
  size: v.number(),
  /** Raw tar mode (e.g. `420` for `0644`); the backend serializes it as a number. */
  mode: v.number(),
  link_target: v.nullable(v.string()),
  /** Normalized layer path a symlink points at, after following chains. */
  link_resolved: v.nullable(v.string()),
  /** Kind of the resolved symlink target; null for a dangling/cyclic link. */
  link_kind: v.nullable(v.picklist(['file', 'dir'])),
  /**
   * How this entry differs from the layers below it. Null/absent in `single`
   * and `aggregate` modes; set by `diff` and `aggregate-diff`.
   */
  change: v.optional(v.nullable(layerChangeSchema), null),
  /**
   * Digest of the layer that provides this entry's bytes (the owning layer in
   * aggregate/diff modes). Null/absent in `single` mode.
   */
  source_digest: v.optional(v.nullable(v.string()), null),
})
export type LayerTreeEntry = v.InferOutput<typeof layerTreeEntrySchema>

/** `GET .../layers/{digest}/tree` returns a bare array of one directory level. */
export const layerTreeSchema = v.array(layerTreeEntrySchema)
export type LayerTree = v.InferOutput<typeof layerTreeSchema>

export const layerReferenceSchema = v.object({
  digest: v.string(),
  media_type: v.nullable(v.string()),
  size: v.number(),
  role: layerRoleSchema,
  created: v.nullable(v.string()),
  created_by: v.nullable(v.string()),
  comment: v.nullable(v.string()),
})
export type LayerReference = v.InferOutput<typeof layerReferenceSchema>

// ---------------------------------------------------------------------------
// Auth (AUTH.md §4)
// ---------------------------------------------------------------------------

/**
 * The authenticated user view returned by `/api/auth/*`. Distinct from
 * `UserProfile` (the public social profile in API.md §1).
 */
export const authUserSchema = v.object({
  id: v.number(),
  username: v.string(),
  email: v.string(),
  first_name: v.nullable(v.string()),
  last_name: v.nullable(v.string()),
  email_verified: v.boolean(),
  is_admin: v.boolean(),
  avatar_url: v.nullable(v.string()),
  theme: v.fallback(v.picklist(['light', 'dark']), 'light'),
  created_at: timestampSchema,
})
export type AuthUser = v.InferOutput<typeof authUserSchema>

export const loginResponseSchema = v.object({
  user: authUserSchema,
  refresh_token: v.string(),
})
export type LoginResponse = v.InferOutput<typeof loginResponseSchema>

export const registerResponseSchema = v.object({
  user: authUserSchema,
})
export type RegisterResponse = v.InferOutput<typeof registerResponseSchema>

export const meResponseSchema = v.object({
  user: authUserSchema,
  namespaces: v.array(namespaceSchema),
})
export type MeResponse = v.InferOutput<typeof meResponseSchema>

export const refreshResponseSchema = v.object({
  refresh_token: v.string(),
})
export type RefreshResponse = v.InferOutput<typeof refreshResponseSchema>

export const verifyEmailResponseSchema = v.object({
  verified: v.literal(true),
})
export type VerifyEmailResponse = v.InferOutput<typeof verifyEmailResponseSchema>

export const resetPasswordResponseSchema = v.object({
  reset: v.literal(true),
})
export type ResetPasswordResponse = v.InferOutput<
  typeof resetPasswordResponseSchema
>

export const captchaChallengeSchema = v.object({
  challenge: v.object({
    c: v.number(),
    s: v.number(),
    d: v.number(),
  }),
  token: v.string(),
  expires: v.number(),
})
export type CaptchaChallenge = v.InferOutput<typeof captchaChallengeSchema>

export const sessionSchema = v.object({
  id: v.string(),
  user_agent: v.nullable(v.string()),
  ip: v.nullable(v.string()),
  created_at: timestampSchema,
  last_seen_at: timestampSchema,
  expires_at: timestampSchema,
  revoked: v.boolean(),
})
export type Session = v.InferOutput<typeof sessionSchema>

export const sessionsResponseSchema = v.object({
  sessions: v.array(sessionSchema),
})
export type SessionsResponse = v.InferOutput<typeof sessionsResponseSchema>

export const loginEventSchema = v.object({
  id: v.number(),
  kind: v.string(),
  success: v.boolean(),
  ip: v.nullable(v.string()),
  user_agent: v.nullable(v.string()),
  username_attempted: v.nullable(v.string()),
  created_at: timestampSchema,
})
export type LoginEvent = v.InferOutput<typeof loginEventSchema>

export const loginHistoryResponseSchema = v.object({
  events: v.array(loginEventSchema),
  limit: v.number(),
  offset: v.number(),
})
export type LoginHistoryResponse = v.InferOutput<
  typeof loginHistoryResponseSchema
>

// ---------------------------------------------------------------------------
// Auth request payloads & form schemas
// ---------------------------------------------------------------------------

export const loginRequestSchema = v.object({
  identifier: v.string(),
  password: v.string(),
  captcha: v.optional(v.string()),
  code: v.optional(v.string()),
})
export type LoginRequest = v.InferOutput<typeof loginRequestSchema>

export const registerRequestSchema = v.object({
  email: v.string(),
  username: v.string(),
  first_name: v.optional(v.string()),
  last_name: v.optional(v.string()),
  password: v.string(),
  captcha: v.optional(v.string()),
})
export type RegisterRequest = v.InferOutput<typeof registerRequestSchema>

export const forgotPasswordRequestSchema = v.object({
  email: v.string(),
  captcha: v.optional(v.string()),
})
export type ForgotPasswordRequest = v.InferOutput<
  typeof forgotPasswordRequestSchema
>

export const resetPasswordRequestSchema = v.object({
  token: v.string(),
  password: v.string(),
})
export type ResetPasswordRequest = v.InferOutput<
  typeof resetPasswordRequestSchema
>

export const verifyEmailRequestSchema = v.object({ token: v.string() })
export type VerifyEmailRequest = v.InferOutput<
  typeof verifyEmailRequestSchema
>

// Form schemas add human-friendly validation messages on top of the request
// shapes. `v.flatten` turns their issues into per-field messages.

export const loginFormSchema = v.object({
  identifier: v.pipe(v.string(), v.nonEmpty('Enter your username or e-mail')),
  password: v.pipe(v.string(), v.nonEmpty('Enter your password')),
})
export type LoginForm = v.InferOutput<typeof loginFormSchema>

export const registerFormSchema = v.object({
  email: v.pipe(
    v.string(),
    v.nonEmpty('Enter your e-mail address'),
    v.email('Enter a valid e-mail address'),
  ),
  username: v.pipe(
    v.string(),
    v.nonEmpty('Choose a username'),
    v.minLength(3, 'Usernames are at least 3 characters'),
    v.maxLength(39, 'Usernames are at most 39 characters'),
    v.regex(
      /^[a-zA-Z0-9](?:[a-zA-Z0-9._-]*[a-zA-Z0-9])?$/,
      'Use letters, numbers, dots, dashes or underscores',
    ),
  ),
  first_name: v.optional(v.string()),
  last_name: v.optional(v.string()),
  password: v.pipe(
    v.string(),
    v.nonEmpty('Choose a password'),
    v.minLength(8, 'Passwords are at least 8 characters'),
  ),
})
export type RegisterForm = v.InferOutput<typeof registerFormSchema>

export const forgotPasswordFormSchema = v.object({
  email: v.pipe(
    v.string(),
    v.nonEmpty('Enter your e-mail address'),
    v.email('Enter a valid e-mail address'),
  ),
})
export type ForgotPasswordForm = v.InferOutput<
  typeof forgotPasswordFormSchema
>

export const resetPasswordFormSchema = v.object({
  password: v.pipe(
    v.string(),
    v.nonEmpty('Choose a new password'),
    v.minLength(8, 'Passwords are at least 8 characters'),
  ),
  confirm: v.string(),
})
export type ResetPasswordForm = v.InferOutput<typeof resetPasswordFormSchema>

// ---------------------------------------------------------------------------
// Wave 3 form schemas (append-only)
// ---------------------------------------------------------------------------

export const updateProfileFormSchema = v.object({
  first_name: v.optional(v.string()),
  last_name: v.optional(v.string()),
  bio: v.optional(
    v.pipe(v.string(), v.maxLength(160, 'Keep your bio under 160 characters')),
  ),
  company: v.optional(v.string()),
  location: v.optional(v.string()),
  website: v.optional(
    v.pipe(
      v.string(),
      v.check(
        (value) => value.length === 0 || /^https?:\/\/\S+$/.test(value),
        'Enter a URL starting with http:// or https://',
      ),
    ),
  ),
  theme: v.picklist(['light', 'dark']),
})
export type UpdateProfileForm = v.InferOutput<typeof updateProfileFormSchema>

export const changePasswordFormSchema = v.object({
  current_password: v.pipe(
    v.string(),
    v.nonEmpty('Enter your current password'),
  ),
  new_password: v.pipe(
    v.string(),
    v.nonEmpty('Choose a new password'),
    v.minLength(8, 'Passwords are at least 8 characters'),
  ),
  confirm: v.string(),
})
export type ChangePasswordForm = v.InferOutput<typeof changePasswordFormSchema>

export const createWorkspaceFormSchema = v.object({
  name: v.pipe(
    v.string(),
    v.nonEmpty('Choose a workspace name'),
    v.minLength(2, 'Names are at least 2 characters'),
    v.maxLength(39, 'Names are at most 39 characters'),
    v.regex(
      /^[a-z0-9](?:[a-z0-9._-]*[a-z0-9])?$/,
      'Use lowercase letters, numbers, dots, dashes or underscores',
    ),
  ),
  description: v.optional(v.string()),
  is_public: v.boolean(),
})
export type CreateWorkspaceForm = v.InferOutput<
  typeof createWorkspaceFormSchema
>

export const createRepositoryFormSchema = v.object({
  namespace: v.pipe(v.string(), v.nonEmpty('Choose a namespace')),
  name: v.pipe(
    v.string(),
    v.nonEmpty('Choose a repository name'),
    v.maxLength(200, 'Names are at most 200 characters'),
    v.regex(
      /^[a-z0-9](?:[a-z0-9._/-]*[a-z0-9])?$/,
      'Use lowercase letters, numbers, dots, dashes, underscores or slashes',
    ),
    v.check(
      (value) => !value.includes('//'),
      'Remove repeated slashes',
    ),
  ),
  description: v.optional(v.string()),
  is_public: v.boolean(),
})
export type CreateRepositoryForm = v.InferOutput<
  typeof createRepositoryFormSchema
>

// ---------------------------------------------------------------------------
// Two-factor authentication & app passwords (append-only)
// ---------------------------------------------------------------------------

/**
 * `/api/auth/login` answers either a completed login or a challenge carrying a
 * short-lived `mfa_token` that `/api/auth/login/2fa` exchanges for a session.
 */
export const twoFactorRequiredSchema = v.object({
  two_factor_required: v.literal(true),
  mfa_token: v.string(),
})
export type TwoFactorRequired = v.InferOutput<typeof twoFactorRequiredSchema>

export const loginResultSchema = v.union([
  loginResponseSchema,
  twoFactorRequiredSchema,
])
export type LoginResult = v.InferOutput<typeof loginResultSchema>

export const loginTwoFactorRequestSchema = v.object({
  mfa_token: v.string(),
  code: v.string(),
})
export type LoginTwoFactorRequest = v.InferOutput<
  typeof loginTwoFactorRequestSchema
>

export const twoFactorStatusSchema = v.object({
  enabled: v.boolean(),
  backup_codes_remaining: v.number(),
})
export type TwoFactorStatus = v.InferOutput<typeof twoFactorStatusSchema>

export const twoFactorSetupSchema = v.object({
  secret: v.string(),
  otpauth_uri: v.string(),
  backup_codes: v.array(v.string()),
})
export type TwoFactorSetup = v.InferOutput<typeof twoFactorSetupSchema>

export const twoFactorEnabledSchema = v.object({
  enabled: v.boolean(),
})
export type TwoFactorEnabled = v.InferOutput<typeof twoFactorEnabledSchema>

export const twoFactorVerifiedSchema = v.object({
  verified: v.boolean(),
})
export type TwoFactorVerified = v.InferOutput<typeof twoFactorVerifiedSchema>

export const backupCodesResponseSchema = v.object({
  backup_codes: v.array(v.string()),
})
export type BackupCodesResponse = v.InferOutput<
  typeof backupCodesResponseSchema
>

export const twoFactorCodeRequestSchema = v.object({ code: v.string() })
export type TwoFactorCodeRequest = v.InferOutput<
  typeof twoFactorCodeRequestSchema
>

export const twoFactorDisableRequestSchema = v.object({
  password: v.string(),
  code: v.string(),
})
export type TwoFactorDisableRequest = v.InferOutput<
  typeof twoFactorDisableRequestSchema
>

export const appPasswordSchema = v.object({
  id: v.number(),
  name: v.string(),
  token_prefix: v.string(),
  token_suffix: v.string(),
  created_at: timestampSchema,
  last_used_at: v.nullable(timestampSchema),
})
export type AppPassword = v.InferOutput<typeof appPasswordSchema>

export const createdAppPasswordSchema = v.object({
  app_password: appPasswordSchema,
  token: v.string(),
})
export type CreatedAppPassword = v.InferOutput<
  typeof createdAppPasswordSchema
>

export const createAppPasswordSchema = v.object({
  name: v.pipe(v.string(), v.nonEmpty('Give the app password a name')),
})
export type CreateAppPassword = v.InferOutput<typeof createAppPasswordSchema>

export const verifyCodeFormSchema = v.object({
  code: v.pipe(v.string(), v.nonEmpty('Enter the 6-digit code')),
})
export type VerifyCodeForm = v.InferOutput<typeof verifyCodeFormSchema>

export const appPasswordFormSchema = v.object({
  name: v.pipe(v.string(), v.nonEmpty('Give the app password a name')),
})
export type AppPasswordForm = v.InferOutput<typeof appPasswordFormSchema>

export const twoFactorDisableFormSchema = v.object({
  password: v.pipe(v.string(), v.nonEmpty('Enter your password')),
  code: v.pipe(v.string(), v.nonEmpty('Enter a code from your authenticator')),
})
export type TwoFactorDisableForm = v.InferOutput<
  typeof twoFactorDisableFormSchema
>
