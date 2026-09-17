//! Validation for repository names, tags and references, following the
//! OCI/Docker distribution grammar.

#![allow(dead_code)]

/// First path segments that are reserved for the control plane and the API
/// surface, and therefore may not be used as a namespace.
pub const RESERVED_NAMESPACES: [&str; 16] = [
    "v2",
    "api",
    "admin",
    "static",
    "assets",
    "login",
    "register",
    "settings",
    "explore",
    "analytics",
    "search",
    "new",
    "notifications",
    "account",
    "organizations",
    "libraries",
];

/// Validates a full repository name: lowercase alphanumeric path components
/// separated by `/`, each component optionally containing `.`, `_` or `-`
/// internally, with a total length of at most 255 characters.
pub fn validate_repository_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255 && name.split('/').all(is_valid_path_component)
}

/// Validates a tag: at most 128 characters, starting with an alphanumeric or
/// underscore, followed by alphanumerics, `.`, `_` or `-`.
pub fn validate_tag(tag: &str) -> bool {
    let bytes = tag.as_bytes();
    if bytes.is_empty() || bytes.len() > 128 {
        return false;
    }
    let valid_first = bytes[0].is_ascii_alphanumeric() || bytes[0] == b'_';
    if !valid_first {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// True when `reference` is written as a digest (`algorithm:encoded`) rather
/// than a tag.
pub fn is_digest_reference(reference: &str) -> bool {
    match reference.split_once(':') {
        Some((algorithm, encoded)) => !algorithm.is_empty() && !encoded.is_empty(),
        None => false,
    }
}

/// Splits a repository name into its namespace (first path segment) and the
/// full name, rejecting reserved namespaces and invalid names.
pub fn parse_name_and_namespace(name: &str) -> Option<(String, String)> {
    if !validate_repository_name(name) {
        return None;
    }
    let namespace = name.split('/').next().unwrap_or(name);
    if is_reserved_namespace(namespace) {
        return None;
    }
    Some((namespace.to_string(), name.to_string()))
}

/// True when the first path segment is reserved for the control plane.
pub fn is_reserved_namespace(namespace: &str) -> bool {
    RESERVED_NAMESPACES
        .iter()
        .any(|reserved| namespace.eq_ignore_ascii_case(reserved))
}

fn is_valid_path_component(component: &str) -> bool {
    let bytes = component.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };
    let Some(&last) = rest.last().or(Some(&first)) else {
        return false;
    };
    if !is_lower_alphanumeric(first) || !is_lower_alphanumeric(last) {
        return false;
    }
    bytes.iter().all(|b| {
        is_lower_alphanumeric(*b) || matches!(b, b'.' | b'_' | b'-')
    })
}

fn is_lower_alphanumeric(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_simple_and_nested_names() {
        assert!(validate_repository_name("library/ubuntu"));
        assert!(validate_repository_name("darktohka/more/complicated/project2"));
        assert!(validate_repository_name("a/b_c/d__e/f-g"));
        assert!(validate_repository_name("registry"));
        assert!(validate_repository_name("team.api/sub_name"));
    }

    #[test]
    fn rejects_invalid_names() {
        assert!(!validate_repository_name(""));
        assert!(!validate_repository_name("Library/ubuntu"));
        assert!(!validate_repository_name("/ubuntu"));
        assert!(!validate_repository_name("ubuntu/"));
        assert!(!validate_repository_name("ubuntu//core"));
        assert!(!validate_repository_name("-ubuntu"));
        assert!(!validate_repository_name("ubuntu-"));
        assert!(!validate_repository_name("_ubuntu"));
        assert!(!validate_repository_name("ubuntu/_core"));
        assert!(!validate_repository_name(&"a".repeat(256)));
        assert!(!validate_repository_name("host:5000/image"));
    }

    #[test]
    fn accepts_255_character_name() {
        let mut name = "a".to_string();
        while name.len() < 255 {
            name.push_str("/a");
        }
        assert_eq!(name.len(), 255);
        assert!(validate_repository_name(&name));
    }

    #[test]
    fn validates_tags() {
        assert!(validate_tag("latest"));
        assert!(validate_tag("v1.0.0"));
        assert!(validate_tag("_2024-01-01"));
        assert!(validate_tag("A"));
        assert!(!validate_tag(""));
        assert!(!validate_tag(".hidden"));
        assert!(!validate_tag("-dash"));
        assert!(!validate_tag("has space"));
        assert!(!validate_tag("has/slash"));
        assert!(!validate_tag(&"a".repeat(129)));
        assert!(validate_tag(&"a".repeat(128)));
    }

    #[test]
    fn detects_digest_references() {
        assert!(is_digest_reference(
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
        assert!(!is_digest_reference("latest"));
        assert!(!is_digest_reference("v1.0"));
        assert!(!is_digest_reference("sha256:"));
    }

    #[test]
    fn parses_namespace_and_full_name() {
        let parsed = parse_name_and_namespace("darktohka/more/complicated/project2");
        assert_eq!(
            parsed,
            Some((
                "darktohka".to_string(),
                "darktohka/more/complicated/project2".to_string()
            ))
        );
        assert_eq!(
            parse_name_and_namespace("library"),
            Some(("library".to_string(), "library".to_string()))
        );
    }

    #[test]
    fn rejects_reserved_namespaces() {
        for reserved in RESERVED_NAMESPACES {
            assert!(is_reserved_namespace(reserved));
            assert_eq!(
                parse_name_and_namespace(&format!("{reserved}/project")),
                None
            );
        }
        assert!(!is_reserved_namespace("darktohka"));
        assert_eq!(parse_name_and_namespace("V2/project"), None);
        assert_eq!(parse_name_and_namespace("badName/project"), None);
    }
}
