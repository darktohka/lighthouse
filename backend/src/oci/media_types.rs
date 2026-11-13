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

// ---- in-toto attestations ---------------------------------------------------

/// `application/vnd.in-toto+json` — an in-toto statement carried as a layer.
pub const IN_TOTO_JSON: &str = "application/vnd.in-toto+json";
/// `application/vnd.docker.attestation.manifest.v1+json` — the `artifactType`
/// BuildKit stamps on the attestation manifest it appends to an image index.
pub const DOCKER_ATTESTATION_MANIFEST: &str =
    "application/vnd.docker.attestation.manifest.v1+json";

/// Index-entry annotation naming the image manifest an attestation manifest
/// was built for (BuildKit's in-index attestation form).
pub const REFERENCE_TYPE_ANNOTATION: &str = "vnd.docker.reference.type";
/// Index-entry annotation carrying the digest of that image manifest.
pub const REFERENCE_DIGEST_ANNOTATION: &str = "vnd.docker.reference.digest";
/// Value of [`REFERENCE_TYPE_ANNOTATION`] for an attestation manifest.
pub const ATTESTATION_MANIFEST_REFERENCE: &str = "attestation-manifest";
/// Layer annotation naming the in-toto predicate a statement carries.
pub const PREDICATE_TYPE_ANNOTATION: &str = "in-toto.io/predicate-type";

/// True for an in-toto statement layer (`application/vnd.in-toto+json`).
///
/// The spec also allows a predicate-specific suffix
/// (`application/vnd.in-toto.<predicate>+json`); both forms are recognised.
pub fn is_in_toto_json(media_type: &str) -> bool {
    let media_type = media_type.trim();
    if media_type.eq_ignore_ascii_case(IN_TOTO_JSON) {
        return true;
    }
    let lower = media_type.to_ascii_lowercase();
    lower.starts_with("application/vnd.in-toto.")
        && (lower.ends_with("+json") || lower.ends_with(".json"))
}

/// True when an index entry is annotated as an attestation manifest.
pub fn is_attestation_reference(annotations: Option<&serde_json::Value>) -> bool {
    annotations
        .and_then(|value| value.get(REFERENCE_TYPE_ANNOTATION))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(ATTESTATION_MANIFEST_REFERENCE))
}

/// The image manifest an attestation entry was built for, taken from the index
/// entry's `vnd.docker.reference.digest` annotation.
pub fn referenced_image_digest<'a>(annotations: Option<&'a serde_json::Value>) -> Option<&'a str> {
    annotations
        .and_then(|value| value.get(REFERENCE_DIGEST_ANNOTATION))
        .and_then(serde_json::Value::as_str)
}

/// True when a manifest body describes an attestation rather than an image:
/// either BuildKit's `artifactType` is set, or it carries in-toto layers.
pub fn is_attestation_manifest(artifact_type: Option<&str>, layer_media_types: &[String]) -> bool {
    if artifact_type.is_some_and(|value| {
        value.eq_ignore_ascii_case(DOCKER_ATTESTATION_MANIFEST)
            || value.eq_ignore_ascii_case(IN_TOTO_JSON)
    }) {
        return true;
    }
    layer_media_types.iter().any(|value| is_in_toto_json(value))
}

/// Short, human-facing label for an in-toto predicate type URI.
///
/// `https://slsa.dev/provenance/v1` becomes `provenance` and
/// `https://spdx.dev/Document` becomes `sbom`; anything else keeps the longest
/// trailing path segment that is not a version marker.
pub fn predicate_label(predicate_type: &str) -> String {
    let raw = predicate_type.trim();
    if raw.is_empty() {
        return "attestation".to_string();
    }
    let lower = raw.to_ascii_lowercase();
    if lower.contains("spdx.dev") || lower.contains("cyclonedx") || lower.contains("sbom") {
        return "sbom".to_string();
    }
    if lower.contains("slsa.dev/provenance") || lower.ends_with("/provenance") {
        return "provenance".to_string();
    }
    let segments: Vec<&str> = raw.trim_end_matches('/').rsplit('/').collect();
    for segment in &segments {
        if segment.is_empty() || is_version_segment(segment) {
            continue;
        }
        return segment.trim_end_matches(".json").to_ascii_lowercase();
    }
    "attestation".to_string()
}

/// True for bare version markers such as `v1`, `v0.2` or `1`.
fn is_version_segment(segment: &str) -> bool {
    let body = segment
        .strip_prefix('v')
        .or_else(|| segment.strip_prefix('V'))
        .unwrap_or(segment);
    !body.is_empty() && body.chars().all(|char| char.is_ascii_digit() || char == '.')
}

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
        let parsed =
            parse_accept("application/vnd.oci.image.manifest.v1+json;q=0.9, application/json, */*");
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
        assert!(negotiate(
            OCI_IMAGE_MANIFEST,
            &["application/*".to_string()]
        ));
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

    #[test]
    fn attestation_constants_are_byte_exact() {
        assert_eq!(IN_TOTO_JSON, "application/vnd.in-toto+json");
        assert_eq!(
            DOCKER_ATTESTATION_MANIFEST,
            "application/vnd.docker.attestation.manifest.v1+json"
        );
        assert_eq!(REFERENCE_TYPE_ANNOTATION, "vnd.docker.reference.type");
        assert_eq!(REFERENCE_DIGEST_ANNOTATION, "vnd.docker.reference.digest");
        assert_eq!(ATTESTATION_MANIFEST_REFERENCE, "attestation-manifest");
        assert_eq!(PREDICATE_TYPE_ANNOTATION, "in-toto.io/predicate-type");
    }

    #[test]
    fn recognises_in_toto_media_types() {
        assert!(is_in_toto_json(IN_TOTO_JSON));
        assert!(is_in_toto_json("application/vnd.in-toto+JSON"));
        assert!(is_in_toto_json(" application/vnd.in-toto+json "));
        assert!(is_in_toto_json(
            "application/vnd.in-toto.spdx+json",
            ),
            "predicate-suffixed form is an in-toto statement"
        );
        assert!(!is_in_toto_json(OCI_IMAGE_LAYER_GZIP));
        assert!(!is_in_toto_json("application/vnd.in-toto.weird"));
    }

    #[test]
    fn detects_attestation_index_entries() {
        let attestation = serde_json::json!({
            "vnd.docker.reference.type": "attestation-manifest",
            "vnd.docker.reference.digest": "sha256:15b657ae",
        });
        assert!(is_attestation_reference(Some(&attestation)));
        assert_eq!(
            referenced_image_digest(Some(&attestation)),
            Some("sha256:15b657ae")
        );

        let image = serde_json::json!({ "org.opencontainers.image.ref.name": "latest" });
        assert!(!is_attestation_reference(Some(&image)));
        assert_eq!(referenced_image_digest(Some(&image)), None);
        assert!(!is_attestation_reference(None));
        assert_eq!(referenced_image_digest(None), None);
    }

    #[test]
    fn detects_attestation_manifests_by_type_or_layers() {
        assert!(is_attestation_manifest(
            Some(DOCKER_ATTESTATION_MANIFEST),
            &[]
        ));
        assert!(is_attestation_manifest(
            Some("application/vnd.docker.attestation.manifest.v1+JSON"),
            &[]
        ));
        assert!(is_attestation_manifest(
            None,
            &[IN_TOTO_JSON.to_string()]
        ));
        // An OCI-empty config alone is not evidence of an attestation.
        assert!(!is_attestation_manifest(None, &[OCI_EMPTY.to_string()]));
        assert!(!is_attestation_manifest(
            Some(OCI_IMAGE_MANIFEST),
            &[OCI_IMAGE_LAYER_GZIP.to_string()]
        ));
        assert!(!is_attestation_manifest(None, &[]));
    }

    #[test]
    fn labels_predicates_for_the_badge() {
        assert_eq!(predicate_label("https://slsa.dev/provenance/v1"), "provenance");
        assert_eq!(
            predicate_label("https://slsa.dev/provenance/v0.2"),
            "provenance"
        );
        assert_eq!(predicate_label("https://spdx.dev/Document"), "sbom");
        assert_eq!(
            predicate_label("https://cyclonedx.org/bom/v1.5"),
            "sbom"
        );
        assert_eq!(
            predicate_label("https://example.com/predicates/vulnerability/v2"),
            "vulnerability"
        );
        assert_eq!(predicate_label(""), "attestation");
        assert_eq!(predicate_label("https://slsa.dev/provenance/v1/"), "provenance");
    }
}
