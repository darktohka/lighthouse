//! OCI and Docker media types plus `Accept` header negotiation.

#![allow(dead_code)]

// ---- OCI image specification ------------------------------------------------

/// `application/vnd.oci.image.manifest.v1+json`
pub const OCI_IMAGE_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
/// `application/vnd.oci.image.index.v1+json`
pub const OCI_IMAGE_INDEX: &str = "application/vnd.oci.image.index.v1+json";
/// `application/vnd.oci.image.config.v1+json`
pub const OCI_IMAGE_CONFIG: &str = "application/vnd.oci.image.config.v1+json";
/// `application/vnd.oci.image.layer.v1.tar`
pub const OCI_IMAGE_LAYER: &str = "application/vnd.oci.image.layer.v1.tar";
/// `application/vnd.oci.image.layer.v1.tar+gzip`
pub const OCI_IMAGE_LAYER_GZIP: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
/// `application/vnd.oci.image.layer.v1.tar+zstd`
pub const OCI_IMAGE_LAYER_ZSTD: &str = "application/vnd.oci.image.layer.v1.tar+zstd";
/// `application/vnd.oci.empty.v1+json`
pub const OCI_EMPTY: &str = "application/vnd.oci.empty.v1+json";
/// `application/vnd.oci.descriptor.v1+json`
pub const OCI_DESCRIPTOR: &str = "application/vnd.oci.descriptor.v1+json";

// ---- Docker distribution (schema 2) -----------------------------------------

/// `application/vnd.docker.distribution.manifest.v2+json`
pub const DOCKER_MANIFEST_V2: &str = "application/vnd.docker.distribution.manifest.v2+json";
/// `application/vnd.docker.distribution.manifest.list.v2+json`
pub const DOCKER_MANIFEST_LIST_V2: &str =
    "application/vnd.docker.distribution.manifest.list.v2+json";
/// `application/vnd.docker.container.image.v1+json`
pub const DOCKER_CONFIG_V1: &str = "application/vnd.docker.container.image.v1+json";
/// `application/vnd.docker.image.rootfs.diff.tar.gzip`
pub const DOCKER_LAYER_GZIP: &str = "application/vnd.docker.image.rootfs.diff.tar.gzip";
/// `application/vnd.docker.image.rootfs.foreign.diff.tar.gzip`
pub const DOCKER_FOREIGN_LAYER_GZIP: &str =
    "application/vnd.docker.image.rootfs.foreign.diff.tar.gzip";

/// Every media type that carries an image manifest or an index.
pub const MANIFEST_TYPES: [&str; 4] = [
    OCI_IMAGE_MANIFEST,
    OCI_IMAGE_INDEX,
    DOCKER_MANIFEST_V2,
    DOCKER_MANIFEST_LIST_V2,
];

/// True for single-image manifest media types (OCI image manifest, Docker schema 2).
pub fn is_manifest_type(media_type: &str) -> bool {
    media_type == OCI_IMAGE_MANIFEST || media_type == DOCKER_MANIFEST_V2
}

/// True for multi-platform index media types (OCI index, Docker manifest list).
pub fn is_index_type(media_type: &str) -> bool {
    media_type == OCI_IMAGE_INDEX || media_type == DOCKER_MANIFEST_LIST_V2
}

/// True for either a single-image manifest or a multi-platform index.
pub fn is_manifest_or_index(media_type: &str) -> bool {
    is_manifest_type(media_type) || is_index_type(media_type)
}

/// The concrete image-manifest media type a client should be served when it has
/// not asked for an index: indexes map to their matching single-image schema.
pub fn default_manifest_type_for(media_type: &str) -> Option<&'static str> {
    match media_type {
        OCI_IMAGE_MANIFEST => Some(OCI_IMAGE_MANIFEST),
        OCI_IMAGE_INDEX => Some(OCI_IMAGE_MANIFEST),
        DOCKER_MANIFEST_V2 => Some(DOCKER_MANIFEST_V2),
        DOCKER_MANIFEST_LIST_V2 => Some(DOCKER_MANIFEST_V2),
        _ => None,
    }
}

/// True when `candidate` is acceptable for a single `Accept` entry.
///
/// Handles `*/*`, `type/*` and exact matches, all case-insensitively.
pub fn media_type_matches(pattern: &str, candidate: &str) -> bool {
    if pattern == "*/*" {
        return true;
    }
    match pattern.split_once('/') {
        Some((type_, "*")) => candidate
            .split_once('/')
            .is_some_and(|(candidate_type, _)| candidate_type.eq_ignore_ascii_case(type_)),
        _ => pattern.eq_ignore_ascii_case(candidate),
    }
}

/// Parses a single `Accept` header value into its media types, preserving order
/// and dropping any entry whose quality factor is zero.
pub fn parse_accept(header: &str) -> Vec<String> {
    header
        .split(',')
        .filter_map(|entry| {
            let mut parts = entry.split(';');
            let media_type = parts.next()?.trim();
            if media_type.is_empty() {
                return None;
            }
            let quality = parts
                .filter_map(|param| param.trim().strip_prefix("q="))
                .filter_map(|value| value.trim().parse::<f32>().ok())
                .next()
                .unwrap_or(1.0);
            if quality <= 0.0 {
                None
            } else {
                Some(media_type.to_ascii_lowercase())
            }
        })
        .collect()
}

/// Parses every value of a multi-valued `Accept` header in wire order.
pub fn parse_accept_values<'a, I>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    values.into_iter().flat_map(parse_accept).collect()
}

/// Decides whether a stored manifest of media type `stored` may be served to a
/// client that sent `accept`.
///
/// An empty `Accept` header is treated as the legacy Docker client: it accepts
/// the stored Docker schema 2 manifest but not OCI media types. Wildcards
/// (including `application/*`) accept everything.
pub fn negotiate(stored: &str, accept: &[String]) -> bool {
    if accept.is_empty() {
        return stored == DOCKER_MANIFEST_V2;
    }
    accept
        .iter()
        .any(|pattern| media_type_matches(pattern, stored))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_are_byte_exact() {
        assert_eq!(
            OCI_IMAGE_MANIFEST,
            "application/vnd.oci.image.manifest.v1+json"
        );
        assert_eq!(OCI_IMAGE_INDEX, "application/vnd.oci.image.index.v1+json");
        assert_eq!(OCI_IMAGE_CONFIG, "application/vnd.oci.image.config.v1+json");
        assert_eq!(OCI_IMAGE_LAYER, "application/vnd.oci.image.layer.v1.tar");
        assert_eq!(
            OCI_IMAGE_LAYER_GZIP,
            "application/vnd.oci.image.layer.v1.tar+gzip"
        );
        assert_eq!(
            OCI_IMAGE_LAYER_ZSTD,
            "application/vnd.oci.image.layer.v1.tar+zstd"
        );
        assert_eq!(OCI_EMPTY, "application/vnd.oci.empty.v1+json");
        assert_eq!(OCI_DESCRIPTOR, "application/vnd.oci.descriptor.v1+json");
        assert_eq!(
            DOCKER_MANIFEST_V2,
            "application/vnd.docker.distribution.manifest.v2+json"
        );
        assert_eq!(
            DOCKER_MANIFEST_LIST_V2,
            "application/vnd.docker.distribution.manifest.list.v2+json"
        );
        assert_eq!(
            DOCKER_CONFIG_V1,
            "application/vnd.docker.container.image.v1+json"
        );
        assert_eq!(
            DOCKER_LAYER_GZIP,
            "application/vnd.docker.image.rootfs.diff.tar.gzip"
        );
        assert_eq!(
            DOCKER_FOREIGN_LAYER_GZIP,
            "application/vnd.docker.image.rootfs.foreign.diff.tar.gzip"
        );
    }

    #[test]
    fn classifies_manifest_and_index_types() {
        assert!(is_manifest_type(OCI_IMAGE_MANIFEST));
        assert!(is_manifest_type(DOCKER_MANIFEST_V2));
        assert!(!is_manifest_type(OCI_IMAGE_INDEX));
        assert!(is_index_type(OCI_IMAGE_INDEX));
        assert!(is_index_type(DOCKER_MANIFEST_LIST_V2));
        assert!(!is_index_type(DOCKER_MANIFEST_V2));
        assert!(is_manifest_or_index(OCI_IMAGE_INDEX));
        assert!(!is_manifest_or_index(OCI_IMAGE_LAYER));
    }

    #[test]
    fn maps_default_manifest_types() {
        assert_eq!(
            default_manifest_type_for(OCI_IMAGE_INDEX),
            Some(OCI_IMAGE_MANIFEST)
        );
        assert_eq!(
            default_manifest_type_for(DOCKER_MANIFEST_LIST_V2),
            Some(DOCKER_MANIFEST_V2)
        );
        assert_eq!(
            default_manifest_type_for(OCI_IMAGE_MANIFEST),
            Some(OCI_IMAGE_MANIFEST)
        );
        assert_eq!(default_manifest_type_for(OCI_IMAGE_LAYER), None);
    }

    #[test]
    fn parses_accept_preserving_order_and_stripping_parameters() {
        let parsed = parse_accept(
            "application/vnd.oci.image.manifest.v1+json;q=0.9, application/json, */*",
        );
        assert_eq!(
            parsed,
            vec![
                "application/vnd.oci.image.manifest.v1+json".to_string(),
                "application/json".to_string(),
                "*/*".to_string(),
            ]
        );
    }

    #[test]
    fn parse_accept_drops_zero_quality_entries() {
        let parsed = parse_accept("application/json;q=0, */*;q=1.0");
        assert_eq!(parsed, vec!["*/*".to_string()]);
    }

    #[test]
    fn negotiate_empty_accept_only_serves_docker_schema2() {
        assert!(negotiate(DOCKER_MANIFEST_V2, &[]));
        assert!(!negotiate(OCI_IMAGE_MANIFEST, &[]));
        assert!(!negotiate(OCI_IMAGE_INDEX, &[]));
    }

    #[test]
    fn negotiate_wildcards_accept_everything() {
        assert!(negotiate(OCI_IMAGE_INDEX, &["*/*".to_string()]));
        assert!(negotiate(OCI_IMAGE_MANIFEST, &["application/*".to_string()]));
        assert!(!negotiate(OCI_EMPTY, &["text/*".to_string()]));
    }

    #[test]
    fn negotiate_exact_and_case_insensitive() {
        assert!(negotiate(
            OCI_IMAGE_MANIFEST,
            &[OCI_IMAGE_MANIFEST.to_ascii_uppercase()]
        ));
        assert!(!negotiate(
            OCI_IMAGE_MANIFEST,
            &[DOCKER_MANIFEST_V2.to_string()]
        ));
    }
}
