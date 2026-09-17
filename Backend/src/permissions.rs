//! Namespace and repository authorization primitives.
//!
//! Resolution for [`repository_access`] is first-match-wins: namespace
//! ownership/​membership, then an explicit repository grant, then an explicit
//! namespace grant, then public visibility (pull only). Service-account grants
//! participate at the repository and namespace grant steps. Push implies pull.

#![allow(dead_code)]

use crate::error::{ApiError, ErrorCode, RegistryError};
use crate::state::{AppState, AuthContext};

/// The permissions a subject holds for a namespace or repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Access {
    pub can_pull: bool,
    pub can_push: bool,
}

impl Access {
    /// Full access: pull and push.
    pub fn full() -> Self {
        Self {
            can_pull: true,
            can_push: true,
        }
    }

    /// Read-only access.
    pub fn pull_only() -> Self {
        Self {
            can_pull: true,
            can_push: false,
        }
    }

    /// True when the access level permits `action`. Deleting requires push.
    pub fn allows(self, action: Action) -> bool {
        match action {
            Action::Pull => self.can_pull,
            Action::Push | Action::Delete => self.can_push,
        }
    }
}

/// The registry action a request maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Pull,
    Push,
    Delete,
}

#[derive(sqlx::FromRow)]
struct Flags {
    can_pull: bool,
    can_push: bool,
}

#[derive(sqlx::FromRow)]
struct NamespaceRow {
    id: i64,
    owner_user_id: Option<i64>,
    is_public: bool,
}

fn access_from(flags: Option<Flags>) -> Option<Access> {
    flags.map(|flags| Access {
        can_pull: flags.can_pull || flags.can_push,
        can_push: flags.can_push,
    })
}

fn is_anonymous(actor: &AuthContext) -> bool {
    !actor.is_authenticated()
}

async fn load_namespace(state: &AppState, name: &str) -> Result<Option<NamespaceRow>, ApiError> {
    let namespace = sqlx::query_as::<_, NamespaceRow>(
        "SELECT id, owner_user_id, is_public FROM namespaces WHERE name = ? COLLATE NOCASE LIMIT 1",
    )
    .bind(name)
    .fetch_optional(&state.db)
    .await?;
    Ok(namespace)
}

async fn is_owner_or_member(
    state: &AppState,
    namespace: &NamespaceRow,
    actor: &AuthContext,
) -> Result<bool, ApiError> {
    let Some(user_id) = actor.user_id else {
        return Ok(false);
    };
    if namespace.owner_user_id == Some(user_id) {
        return Ok(true);
    }
    let member: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM namespace_members WHERE namespace_id = ? AND user_id = ? LIMIT 1",
    )
    .bind(namespace.id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;
    Ok(member.is_some())
}

async fn user_namespace_grant(
    state: &AppState,
    namespace_id: i64,
    actor: &AuthContext,
) -> Result<Option<Access>, ApiError> {
    let flags = if let Some(user_id) = actor.user_id {
        sqlx::query_as::<_, Flags>(
            "SELECT can_pull, can_push FROM namespace_permissions \
             WHERE namespace_id = ? AND subject_type = 'user' AND subject_user_id = ? LIMIT 1",
        )
        .bind(namespace_id)
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
    } else if is_anonymous(actor) {
        sqlx::query_as::<_, Flags>(
            "SELECT can_pull, can_push FROM namespace_permissions \
             WHERE namespace_id = ? AND subject_type = 'anonymous' LIMIT 1",
        )
        .bind(namespace_id)
        .fetch_optional(&state.db)
        .await?
    } else {
        None
    };
    Ok(access_from(flags))
}

async fn user_repository_grant(
    state: &AppState,
    repository_id: i64,
    actor: &AuthContext,
) -> Result<Option<Access>, ApiError> {
    let flags = if let Some(user_id) = actor.user_id {
        sqlx::query_as::<_, Flags>(
            "SELECT can_pull, can_push FROM repository_permissions \
             WHERE repository_id = ? AND subject_type = 'user' AND subject_user_id = ? LIMIT 1",
        )
        .bind(repository_id)
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
    } else if is_anonymous(actor) {
        sqlx::query_as::<_, Flags>(
            "SELECT can_pull, can_push FROM repository_permissions \
             WHERE repository_id = ? AND subject_type = 'anonymous' LIMIT 1",
        )
        .bind(repository_id)
        .fetch_optional(&state.db)
        .await?
    } else {
        None
    };
    Ok(access_from(flags))
}

async fn service_namespace_grant(
    state: &AppState,
    namespace_id: i64,
    actor: &AuthContext,
) -> Result<Option<Access>, ApiError> {
    let Some(service_account_id) = actor.service_account_id else {
        return Ok(None);
    };
    let flags = sqlx::query_as::<_, Flags>(
        "SELECT can_pull, can_push FROM service_account_grants \
         WHERE namespace_id = ? AND service_account_id = ? LIMIT 1",
    )
    .bind(namespace_id)
    .bind(service_account_id)
    .fetch_optional(&state.db)
    .await?;
    Ok(access_from(flags))
}

async fn service_repository_grant(
    state: &AppState,
    repository_id: i64,
    actor: &AuthContext,
) -> Result<Option<Access>, ApiError> {
    let Some(service_account_id) = actor.service_account_id else {
        return Ok(None);
    };
    let flags = sqlx::query_as::<_, Flags>(
        "SELECT can_pull, can_push FROM service_account_grants \
         WHERE repository_id = ? AND service_account_id = ? LIMIT 1",
    )
    .bind(repository_id)
    .bind(service_account_id)
    .fetch_optional(&state.db)
    .await?;
    Ok(access_from(flags))
}

/// True when the actor owns or is a member of the namespace.
pub async fn namespace_access(
    state: &AppState,
    actor: &AuthContext,
    namespace: &str,
) -> Result<Access, ApiError> {
    let Some(namespace) = load_namespace(state, namespace).await? else {
        return Ok(Access::default());
    };
    if is_owner_or_member(state, &namespace, actor).await? {
        return Ok(Access::full());
    }
    if let Some(access) = user_namespace_grant(state, namespace.id, actor).await? {
        return Ok(access);
    }
    if let Some(access) = service_namespace_grant(state, namespace.id, actor).await? {
        return Ok(access);
    }
    if namespace.is_public {
        return Ok(Access::pull_only());
    }
    Ok(Access::default())
}

/// Effective access to a repository name (may not exist yet — push creates it).
pub async fn repository_access(
    state: &AppState,
    actor: &AuthContext,
    repo_name: &str,
) -> Result<Access, ApiError> {
    let namespace_name = repo_name.split('/').next().unwrap_or(repo_name);
    let namespace = load_namespace(state, namespace_name).await?;
    let repository = state
        .registry
        .find_repository(repo_name)
        .await
        .map_err(ApiError::from)?;

    if let Some(namespace) = &namespace
        && is_owner_or_member(state, namespace, actor).await?
    {
        return Ok(Access::full());
    }

    if let Some(repository) = &repository {
        if let Some(access) = user_repository_grant(state, repository.id, actor).await? {
            return Ok(access);
        }
        if let Some(access) = service_repository_grant(state, repository.id, actor).await? {
            return Ok(access);
        }
    }

    if let Some(namespace) = &namespace {
        if let Some(access) = user_namespace_grant(state, namespace.id, actor).await? {
            return Ok(access);
        }
        if let Some(access) = service_namespace_grant(state, namespace.id, actor).await? {
            return Ok(access);
        }
    }

    let public = repository.as_ref().is_some_and(|repo| repo.is_public)
        || namespace
            .as_ref()
            .is_some_and(|namespace| namespace.is_public);
    if public {
        return Ok(Access::pull_only());
    }

    Ok(Access::default())
}

/// Enforces access for the OCI API, producing spec-compliant errors.
pub async fn authorize_oci(
    state: &AppState,
    actor: &AuthContext,
    repo_name: &str,
    action: Action,
) -> Result<Access, RegistryError> {
    let access = repository_access(state, actor, repo_name)
        .await
        .map_err(|err| RegistryError::internal(err.message))?;

    if access.allows(action) {
        return Ok(access);
    }
    if !actor.is_authenticated() {
        Err(RegistryError::unauthorized("authentication required"))
    } else {
        Err(RegistryError::denied(
            "requested access to the resource is denied",
        ))
    }
}

/// True when a [`RegistryError`] signals missing or invalid credentials.
pub fn is_unauthenticated(err: &RegistryError) -> bool {
    err.code == ErrorCode::Unauthorized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::{create_namespace, create_repository, create_user, test_state};
    use crate::models::User;

    fn actor(user: &User) -> AuthContext {
        AuthContext {
            user_id: Some(user.id),
            username: Some(user.username.clone()),
            service_account_id: None,
            is_admin: false,
        }
    }

    fn anonymous() -> AuthContext {
        AuthContext::default()
    }

    async fn grant_repository(
        state: &AppState,
        repository_id: i64,
        subject_type: &str,
        subject_user_id: Option<i64>,
        can_pull: bool,
        can_push: bool,
    ) {
        sqlx::query(
            "INSERT INTO repository_permissions \
             (repository_id, subject_type, subject_user_id, can_pull, can_push, created_by, created_at) \
             VALUES (?, ?, ?, ?, ?, NULL, ?)",
        )
        .bind(repository_id)
        .bind(subject_type)
        .bind(subject_user_id)
        .bind(can_pull)
        .bind(can_push)
        .bind(chrono::Utc::now())
        .execute(&state.db)
        .await
        .expect("grant repository");
    }

    async fn grant_namespace(
        state: &AppState,
        namespace_id: i64,
        subject_type: &str,
        subject_user_id: Option<i64>,
        can_pull: bool,
        can_push: bool,
    ) {
        sqlx::query(
            "INSERT INTO namespace_permissions \
             (namespace_id, subject_type, subject_user_id, can_pull, can_push, created_by, created_at) \
             VALUES (?, ?, ?, ?, ?, NULL, ?)",
        )
        .bind(namespace_id)
        .bind(subject_type)
        .bind(subject_user_id)
        .bind(can_pull)
        .bind(can_push)
        .bind(chrono::Utc::now())
        .execute(&state.db)
        .await
        .expect("grant namespace");
    }

    #[tokio::test]
    async fn owner_has_full_access_to_their_namespace() {
        let (_dir, state) = test_state().await;
        let alice = create_user(&state, "alice").await;
        assert_eq!(
            repository_access(&state, &actor(&alice), "alice/app")
                .await
                .expect("access"),
            Access::full()
        );
        assert_eq!(
            namespace_access(&state, &actor(&alice), "alice")
                .await
                .expect("access"),
            Access::full()
        );
        assert_eq!(
            repository_access(&state, &actor(&alice), "alice")
                .await
                .expect("access"),
            Access::full()
        );
    }

    #[tokio::test]
    async fn private_repository_denies_strangers() {
        let (_dir, state) = test_state().await;
        let alice = create_user(&state, "alice").await;
        let bob = create_user(&state, "bob").await;
        let namespace_id =
            sqlx::query_scalar::<_, i64>("SELECT id FROM namespaces WHERE name = 'alice' LIMIT 1")
                .fetch_one(&state.db)
                .await
                .expect("namespace");
        create_repository(&state, namespace_id, "alice/app", false, Some(alice.id)).await;

        assert_eq!(
            repository_access(&state, &actor(&bob), "alice/app")
                .await
                .expect("access"),
            Access::default()
        );
    }

    #[tokio::test]
    async fn public_namespace_grants_pull_only() {
        let (_dir, state) = test_state().await;
        let namespace_id = create_namespace(&state, "team", None, true).await;
        create_repository(&state, namespace_id, "team/app", false, None).await;

        let access = repository_access(&state, &anonymous(), "team/app")
            .await
            .expect("access");
        assert_eq!(access, Access::pull_only());
        assert!(
            authorize_oci(&state, &anonymous(), "team/app", Action::Pull)
                .await
                .is_ok()
        );
        let denied = authorize_oci(&state, &anonymous(), "team/app", Action::Push)
            .await
            .expect_err("push denied");
        assert_eq!(denied.code, ErrorCode::Unauthorized);
    }

    #[tokio::test]
    async fn explicit_repository_grant_and_push_implies_pull() {
        let (_dir, state) = test_state().await;
        let alice = create_user(&state, "alice").await;
        let bob = create_user(&state, "bob").await;
        let namespace_id =
            sqlx::query_scalar::<_, i64>("SELECT id FROM namespaces WHERE name = 'alice' LIMIT 1")
                .fetch_one(&state.db)
                .await
                .expect("namespace");
        let repository_id =
            create_repository(&state, namespace_id, "alice/app", false, Some(alice.id)).await;

        grant_repository(&state, repository_id, "user", Some(bob.id), false, true).await;
        let access = repository_access(&state, &actor(&bob), "alice/app")
            .await
            .expect("access");
        assert!(access.can_pull, "push implies pull");
        assert!(access.can_push);
        assert!(
            authorize_oci(&state, &actor(&bob), "alice/app", Action::Delete)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn namespace_grant_covers_repositories() {
        let (_dir, state) = test_state().await;
        let alice = create_user(&state, "alice").await;
        let bob = create_user(&state, "bob").await;
        let namespace_id =
            sqlx::query_scalar::<_, i64>("SELECT id FROM namespaces WHERE name = 'alice' LIMIT 1")
                .fetch_one(&state.db)
                .await
                .expect("namespace");
        create_repository(&state, namespace_id, "alice/app", false, Some(alice.id)).await;

        grant_namespace(&state, namespace_id, "user", Some(bob.id), true, false).await;
        let access = repository_access(&state, &actor(&bob), "alice/app")
            .await
            .expect("access");
        assert_eq!(access, Access::pull_only());
    }

    #[tokio::test]
    async fn service_account_grants_are_honoured() {
        let (_dir, state) = test_state().await;
        let alice = create_user(&state, "alice").await;
        let namespace_id =
            sqlx::query_scalar::<_, i64>("SELECT id FROM namespaces WHERE name = 'alice' LIMIT 1")
                .fetch_one(&state.db)
                .await
                .expect("namespace");
        let repository_id =
            create_repository(&state, namespace_id, "alice/app", false, Some(alice.id)).await;

        let (account, token) = crate::auth::service_accounts::create(
            &state,
            crate::auth::service_accounts::NewServiceAccount {
                owner_user_id: alice.id,
                name: "ci",
                username: "alice-ci",
                description: None,
            },
        )
        .await
        .expect("service account");
        sqlx::query(
            "INSERT INTO service_account_grants \
             (service_account_id, namespace_id, repository_id, can_pull, can_push) \
             VALUES (?, NULL, ?, 1, 1)",
        )
        .bind(account.id)
        .bind(repository_id)
        .execute(&state.db)
        .await
        .expect("grant");

        let service_actor = AuthContext {
            user_id: None,
            username: Some(account.username.clone()),
            service_account_id: Some(account.id),
            is_admin: false,
        };
        let access = repository_access(&state, &service_actor, "alice/app")
            .await
            .expect("access");
        assert_eq!(access, Access::full());
        let _ = token;
    }

    #[tokio::test]
    async fn unauthenticated_denial_differs_from_policy_denial() {
        let (_dir, state) = test_state().await;
        let alice = create_user(&state, "alice").await;
        let bob = create_user(&state, "bob").await;
        let namespace_id =
            sqlx::query_scalar::<_, i64>("SELECT id FROM namespaces WHERE name = 'alice' LIMIT 1")
                .fetch_one(&state.db)
                .await
                .expect("namespace");
        create_repository(&state, namespace_id, "alice/app", false, Some(alice.id)).await;

        let anonymous_error = authorize_oci(&state, &anonymous(), "alice/app", Action::Pull)
            .await
            .expect_err("denied");
        assert!(is_unauthenticated(&anonymous_error));

        let forbidden = authorize_oci(&state, &actor(&bob), "alice/app", Action::Pull)
            .await
            .expect_err("denied");
        assert_eq!(forbidden.code, ErrorCode::Denied);
        assert!(!is_unauthenticated(&forbidden));
    }
}
