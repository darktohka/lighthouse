# Lighthouse — Permissions

Owner: wave 2B (`src/permissions.rs`). This is the frozen interface every OCI and
control-plane module calls to authorize access.

---

## 1. Types

```rust
pub struct Access { pub can_pull: bool, pub can_push: bool }

pub enum Action { Pull, Push, Delete }

pub async fn namespace_access(state, actor, namespace)  -> Result<Access, ApiError>;
pub async fn repository_access(state, actor, repo_name) -> Result<Access, ApiError>;
pub async fn authorize_oci(state, actor, repo_name, action) -> Result<Access, RegistryError>;
pub fn is_unauthenticated(err: &RegistryError) -> bool;
```

- `Access` is `Copy`; `Access::full()` is pull+push, `Access::pull_only()` is
  pull only, `Default` is no access.
- `Access::allows(action)` maps `Pull → can_pull` and `Push | Delete → can_push`.
  **Delete requires push.**
- `repository_access` may be called for a repository that does not exist yet:
  pushing creates it, so a namespace owner can push to a new name.

The default posture is **private**: with no matching rule the actor has no
access.

---

## 2. Resolution algorithm

`repository_access` resolves the namespace (first path segment) and the
repository row, then evaluates rules **first match wins**:

| # | Rule | Result |
|---|---|---|
| 1 | `namespaces.owner_user_id == actor.user_id`, or a `namespace_members` row | **pull + push** |
| 2 | `repository_permissions` grant for the actor on the repository (`subject_type='user'` + `subject_user_id`, or `subject_type='anonymous'` when anonymous), or a repository-scoped `service_account_grants` row | grant flags, with **push implying pull** |
| 3 | `namespace_permissions` grant for the actor on the repository's namespace, or a namespace-scoped `service_account_grants` row | grant flags, with **push implying pull** |
| 4 | `repositories.is_public = 1` **or** `namespaces.is_public = 1` | **pull only** |
| 5 | otherwise | no access |

Notes:

- Grants only *add* access; they never revoke. Ownership (rule 1) therefore beats
  a pull-only repository grant.
- Rule 4 is evaluated even when the repository row is absent, so a public
  namespace exposes names under it for pull.
- Service accounts resolve through `service_account_grants` at rules 2 and 3.
  They do **not** inherit their owner's ownership or membership — explicit
  grants are the whole model.
- A service account outside its IP allowlist is not authenticated at all: the
  credential is rejected before any rule runs, so the rules above never apply.
  An empty allowlist is unrestricted; see `docs/AUTH.md` for enforcement.
- Anonymous actors match only `subject_type='anonymous'` grants and public
  visibility.

`namespace_access` follows the same order without the repository steps:
ownership/membership, then `namespace_permissions`, then namespace-scoped
`service_account_grants`, then `namespaces.is_public` (pull only).

---

## 3. Delegation model

Two grant scopes share one shape:

```text
namespace_permissions(namespace_id, subject_type, subject_user_id, can_pull, can_push)
repository_permissions(repository_id, subject_type, subject_user_id, can_pull, can_push)
service_account_grants(service_account_id, namespace_id, repository_id, can_pull, can_push)
```

- `subject_type` is `'user'` (with a `subject_user_id`) or `'anonymous'`
  (`subject_user_id IS NULL`). The unique index on
  `(scope, subject_type, COALESCE(subject_user_id, 0))` keeps one grant per
  subject per scope.
- A repository grant is the narrowest and is evaluated before a namespace grant,
  so delegating a single image cannot be widened by a namespace-level rule that
  would otherwise match first.
- A namespace grant covers every repository inside that namespace.
- `can_push` without `can_pull` is normalised at read time: push implies pull.
- Service-account grants mirror the same scopes; a repository-scoped grant wins
  over a namespace-scoped grant because it is evaluated in rule 2 before rule 3.

### Roles

`namespace_members.role` (`admin` / `member`) currently grants the same access as
ownership (pull + push). It is stored so the control plane can distinguish the
two later without a schema change.

---

## 4. OCI integration

`authorize_oci` converts the internal error vocabulary to the OCI envelope:

| Situation | `RegistryError` | HTTP |
|---|---|---|
| anonymous and not permitted | `ErrorCode::Unauthorized` | 401 |
| authenticated but not permitted | `ErrorCode::Denied` | 403 |
| permitted | `Ok(Access)` | — |

The OCI layer inspects `is_unauthenticated(&err)` to attach
`WWW-Authenticate: Basic realm="Lighthouse Registry"` (from
`auth::middleware::challenge_headers`) only for genuine authentication failures —
a policy denial is a plain `403 DENIED`. For a cross-repository blob mount
(`?from=`), call `repository_access` on the source repository and additionally
require `can_pull` there.

---

## 5. Test matrix

`src/permissions.rs` tests cover the matrix end to end:

| Case | Expected |
|---|---|
| owner, own namespace | pull + push, including a not-yet-created repo |
| stranger on a private repo | no access |
| anonymous on a public namespace/repo | pull only; push → `401 UNAUTHORIZED` |
| explicit repository grant with `can_push` only | pull + push (push implies pull) |
| namespace grant | applies to every repo under it |
| service-account grant | honoured, resolved as the service actor |
| anonymous denial vs policy denial | `401 is_unauthenticated` vs `403 DENIED` |
