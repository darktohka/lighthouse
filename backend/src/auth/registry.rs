//! Registry bearer-token plumbing: the `WWW-Authenticate` challenge, the scope
//! grammar (distribution spec `docs/spec/auth/scope.md`) and the mapping from a
//! requested scope to the actions an actor actually holds.

use axum::http::{HeaderName, HeaderValue, header};

use crate::config::{Config, RegistryAuthChallenge};
use crate::permissions::{self, Access};
use crate::state::{AppState, AuthContext};

/// A parsed `repository:<name>:<actions>` / `registry:<name>:<actions>` scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub kind: String,
    pub name: String,
    pub actions: Vec<String>,
}

impl Scope {
    /// Actions that include `wanted` (or the `*` wildcard).
    pub fn requests(&self, wanted: &str) -> bool {
        self.actions
            .iter()
            .any(|action| action == wanted || action == "*")
    }
}

/// Parses a single scope token. The name may itself contain `:`, so the action
/// list is split from the last colon and the type from the first.
pub fn parse_scope(raw: &str) -> Option<Scope> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (rest, actions) = raw.rsplit_once(':')?;
    let (kind, name) = rest.split_once(':')?;
    if kind.is_empty() || name.is_empty() {
        return None;
    }
    let actions: Vec<String> = actions
        .split(',')
        .map(str::trim)
        .filter(|action| !action.is_empty())
        .map(str::to_string)
        .collect();
    if actions.is_empty() {
        return None;
    }
    Some(Scope {
        kind: kind.to_string(),
        name: name.to_string(),
        actions,
    })
}

/// Parses every scope from the repeated/space-joined `scope` parameters.
pub fn parse_scopes(parameters: &[String]) -> Vec<Scope> {
    parameters
        .iter()
        .flat_map(|value| value.split(' '))
        .filter_map(parse_scope)
        .collect()
}

/// Resolves the actions the actor actually holds for `scope`. An empty result
/// means the scope is not granted; per the spec the caller still receives a
/// token, and the registry denies the later request with `DENIED`.
pub async fn granted_actions(
    state: &AppState,
    actor: &AuthContext,
    scope: &Scope,
) -> Vec<String> {
    if scope.kind == "registry" {
        if scope.name == "catalog" && actor.is_authenticated() {
            return vec!["*".to_string()];
        }
        return Vec::new();
    }
    if scope.kind != "repository" {
        return Vec::new();
    }

    let access = match permissions::repository_access(state, actor, &scope.name).await {
        Ok(access) => access,
        Err(_) => return Vec::new(),
    };
    let mut granted = Vec::new();
    if scope.requests("pull") && access.can_pull {
        granted.push("pull".to_string());
    }
    if scope.requests("push") && access.can_push {
        granted.push("push".to_string());
    }
    granted
}

/// True when the granted actions cover every action the scope asked for.
pub fn access_covers(access: &Access, scope: &Scope) -> bool {
    let mut covered = true;
    for action in &scope.actions {
        covered &= match action.as_str() {
            "pull" => access.can_pull,
            "push" | "*" => access.can_push,
            _ => true,
        };
    }
    covered
}

/// Builds the `WWW-Authenticate: Bearer realm=...,service=...,scope=...`
/// challenge. `scope` is omitted for the `/v2/` ping.
pub fn bearer_challenge(config: &Config, scope: Option<&str>) -> (HeaderName, HeaderValue) {
    let realm = format!("{}/api/auth/token", config.base_url.trim_end_matches('/'));
    let mut value = format!(
        "Bearer realm=\"{realm}\",service=\"{}\"",
        config.public_host
    );
    if let Some(scope) = scope {
        value.push_str(&format!(",scope=\"{scope}\""));
    }
    let header = HeaderValue::from_str(&value)
        .unwrap_or_else(|_| HeaderValue::from_static("Bearer realm=\"invalid\""));
    (header::WWW_AUTHENTICATE, header)
}

/// The Basic challenge returned by the token endpoint when a client supplies
/// credentials that do not authenticate.
pub fn basic_challenge() -> (HeaderName, HeaderValue) {
    (
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Basic realm=\"Lighthouse Registry\""),
    )
}

/// The `WWW-Authenticate` challenge(s) for a `401`, honouring the configured
/// [`RegistryAuthChallenge`].
///
/// The two schemes are returned as separate header values rather than one
/// comma-joined value: Bearer's own parameters are comma-separated, so joining
/// them would be ambiguous for a client to parse.
pub fn challenge(config: &Config, scope: Option<&str>) -> Vec<(HeaderName, HeaderValue)> {
    match config.registry_auth_challenge {
        RegistryAuthChallenge::Bearer => vec![bearer_challenge(config, scope)],
        RegistryAuthChallenge::Basic => vec![basic_challenge()],
        RegistryAuthChallenge::Both => {
            vec![bearer_challenge(config, scope), basic_challenge()]
        }
    }
}

/// Renders the `repository:<name>:pull[,push]` scope for a `/v2` request.
pub fn repository_scope(name: &str, push: bool) -> String {
    if push {
        format!("repository:{name}:pull,push")
    } else {
        format!("repository:{name}:pull")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repository_scopes() {
        let scope = parse_scope("repository:alice/app:pull").expect("scope");
        assert_eq!(scope.kind, "repository");
        assert_eq!(scope.name, "alice/app");
        assert_eq!(scope.actions, vec!["pull"]);
        assert!(scope.requests("pull"));
        assert!(!scope.requests("push"));

        let scope = parse_scope("repository:host:5000/a/b:pull,push").expect("scope");
        assert_eq!(scope.name, "host:5000/a/b");
        assert_eq!(scope.actions, vec!["pull", "push"]);
        assert!(scope.requests("push"));
    }

    #[test]
    fn parses_catalog_and_wildcards() {
        let scope = parse_scope("registry:catalog:*").expect("scope");
        assert_eq!(scope.kind, "registry");
        assert_eq!(scope.name, "catalog");
        assert!(scope.requests("pull"));
        assert!(scope.requests("anything"));
    }

    #[test]
    fn rejects_malformed_scopes() {
        assert!(parse_scope("").is_none());
        assert!(parse_scope("repository").is_none());
        assert!(parse_scope("repository:name").is_none());
        assert!(parse_scope("repository::pull").is_none());
        assert!(parse_scope("repository:name:").is_none());
    }

    #[test]
    fn splits_repeated_and_space_joined_scopes() {
        let scopes = parse_scopes(&[
            "repository:a/app:pull".to_string(),
            "repository:b/app:pull registry:catalog:*".to_string(),
        ]);
        assert_eq!(scopes.len(), 3);
        assert_eq!(scopes[2].kind, "registry");
    }
}
