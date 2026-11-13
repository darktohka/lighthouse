//! in-toto attestation discovery.
//!
//! An attestation reaches a registry in one of two shapes, and both are
//! supported here:
//!
//! 1. **In-index (BuildKit default).** The image index carries extra entries
//!    whose platform is `unknown/unknown` and whose annotations are
//!    `vnd.docker.reference.type: attestation-manifest` plus
//!    `vnd.docker.reference.digest: <image manifest digest>`. Each such entry is
//!    an image manifest whose `artifactType` is
//!    `application/vnd.docker.attestation.manifest.v1+json` and whose layers are
//!    `application/vnd.in-toto+json` statements, one per predicate (provenance,
//!    SBOM, …).
//! 2. **Referrers (OCI 1.1).** The attestation is a separate manifest whose
//!    `subject` points at the image manifest. It is discovered through the
//!    referrers relationship rather than the index.
//!
//! Either way the interesting artifact is the in-toto statement: its
//! `predicateType` names what the attestation asserts. [`predicate_label`]
//! reduces that URI to the short label the UI badges show, so a `linux/arm64`
//! image built with provenance and an SBOM reads as
//! `linux/arm64 (provenance, sbom)`.
//!
//! [`predicate_label`]: crate::oci::media_types::predicate_label

use serde::Serialize;
use serde_json::Value;

use crate::oci::media_types;

/// One in-toto statement layer of an attestation manifest.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AttestationPredicate {
    /// `in-toto.io/predicate-type` annotation, when the layer carries one.
    pub predicate_type: Option<String>,
    /// Short label derived from [`predicate_type`](Self::predicate_type).
    pub kind: String,
    /// Digest of the statement blob.
    pub digest: String,
    /// Media type of the statement blob (`application/vnd.in-toto+json`).
    pub media_type: String,
    /// Size of the statement blob in bytes.
    pub size: i64,
}

impl AttestationPredicate {
    /// Builds a predicate from a layer descriptor plus the statement body, when
    /// the body has already been read. The body is only used to recover a
    /// `predicateType` that the descriptor annotation omits.
    pub fn from_descriptor(descriptor: &Value, statement: Option<&Value>) -> Option<Self> {
        let media_type = descriptor
            .get("mediaType")
            .and_then(Value::as_str)?
            .to_string();
        if !media_types::is_in_toto_json(&media_type) {
            return None;
        }
        let digest = descriptor
            .get("digest")
            .and_then(Value::as_str)?
            .to_string();
        let size = descriptor.get("size").and_then(Value::as_i64).unwrap_or(0);
        let annotated = descriptor
            .get("annotations")
            .and_then(|annotations| annotations.get(media_types::PREDICATE_TYPE_ANNOTATION))
            .and_then(Value::as_str);
        let from_statement = statement
            .and_then(|value| value.get("predicateType"))
            .and_then(Value::as_str);
        let predicate_type = annotated.or(from_statement).map(str::to_string);
        Some(Self {
            kind: predicate_type
                .as_deref()
                .map(media_types::predicate_label)
                .unwrap_or_else(|| "attestation".to_string()),
            predicate_type,
            digest,
            media_type,
            size,
        })
    }
}

/// An attestation manifest: the wrapper that carries one or more statements.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Attestation {
    /// Digest of the attestation manifest itself.
    pub digest: String,
    /// Media type of the attestation manifest.
    pub media_type: String,
    /// `artifactType` of the attestation manifest, when it declares one.
    pub artifact_type: Option<String>,
    /// Size of the attestation manifest in bytes.
    pub size: i64,
    /// Deduplicated short labels of every predicate, in statement order.
    pub kinds: Vec<String>,
    /// The in-toto statements, in the manifest's declared order.
    pub predicates: Vec<AttestationPredicate>,
    /// `subject.digest` when the attestation manifest declares one.
    pub subject_digest: Option<String>,
    /// `vnd.docker.reference.digest` from the parent index entry, when present.
    pub referenced_digest: Option<String>,
}

impl Attestation {
    /// Builds an attestation from an attestation manifest's body plus the
    /// index-entry annotations that pointed at it.
    pub fn from_manifest(
        digest: &str,
        media_type: &str,
        size: i64,
        manifest: &Value,
        entry_annotations: Option<&Value>,
    ) -> Self {
        let artifact_type = manifest
            .get("artifactType")
            .and_then(Value::as_str)
            .map(str::to_string);
        let subject_digest = manifest
            .get("subject")
            .and_then(|subject| subject.get("digest"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let referenced_digest = media_types::referenced_image_digest(entry_annotations)
            .map(str::to_string)
            .or_else(|| subject_digest.clone());

        let predicates: Vec<AttestationPredicate> = manifest
            .get("layers")
            .and_then(Value::as_array)
            .map(|layers| layers.as_slice())
            .unwrap_or_default()
            .iter()
            .filter_map(|layer| AttestationPredicate::from_descriptor(layer, None))
            .collect();

        let mut kinds: Vec<String> = Vec::new();
        for predicate in &predicates {
            if !kinds.contains(&predicate.kind) {
                kinds.push(predicate.kind.clone());
            }
        }
        if kinds.is_empty() {
            kinds.push("attestation".to_string());
        }

        Self {
            digest: digest.to_string(),
            media_type: media_type.to_string(),
            artifact_type,
            size,
            kinds,
            predicates,
            subject_digest,
            referenced_digest,
        }
    }

    /// The image manifest this attestation was built for, if any.
    pub fn target_digest(&self) -> Option<&str> {
        self.referenced_digest
            .as_deref()
            .or(self.subject_digest.as_deref())
    }
}

/// True when a manifest body is an attestation wrapper rather than an image.
///
/// A body qualifies when it declares an in-toto `artifactType`, carries an
/// in-toto statement layer, or is annotated as an attestation index entry.
pub fn is_attestation(manifest: &Value, entry_annotations: Option<&Value>) -> bool {
    if media_types::is_attestation_reference(entry_annotations) {
        return true;
    }
    let artifact_type = manifest.get("artifactType").and_then(Value::as_str);
    let layer_types: Vec<String> = manifest
        .get("layers")
        .and_then(Value::as_array)
        .map(|layers| {
            layers
                .iter()
                .filter_map(|layer| layer.get("mediaType").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    media_types::is_attestation_manifest(artifact_type, &layer_types)
}

/// True when an index entry advertises the `unknown/unknown` placeholder
/// platform BuildKit uses for in-index attestations.
pub fn is_placeholder_platform(platform: &Value) -> bool {
    let field = |key: &str| {
        platform
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("unknown"))
    };
    field("os") && field("architecture")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buildkit_attestation_manifest(statement_digest: &str, predicate: &str) -> Value {
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": media_types::OCI_IMAGE_MANIFEST,
            "artifactType": media_types::DOCKER_ATTESTATION_MANIFEST,
            "config": {
                "mediaType": media_types::OCI_EMPTY,
                "digest": "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
                "size": 2,
            },
            "layers": [{
                "mediaType": media_types::IN_TOTO_JSON,
                "digest": statement_digest,
                "size": 1741,
                "annotations": { media_types::PREDICATE_TYPE_ANNOTATION: predicate },
            }],
            "subject": {
                "mediaType": media_types::OCI_IMAGE_MANIFEST,
                "digest": "sha256:15b657aed6e28ad2777367458605920a0f806395501ff264df77c18b27844e59",
                "size": 2956,
            },
        })
    }

    #[test]
    fn extracts_buildkit_provenance_attestation() {
        let body = buildkit_attestation_manifest("sha256:fc4f36c839eb", "https://slsa.dev/provenance/v1");
        let entry = serde_json::json!({
            media_types::REFERENCE_TYPE_ANNOTATION: media_types::ATTESTATION_MANIFEST_REFERENCE,
            media_types::REFERENCE_DIGEST_ANNOTATION: "sha256:15b657aed6e28ad2777367458605920a0f806395501ff264df77c18b27844e59",
        });
        let attestation = Attestation::from_manifest(
            "sha256:a77201ec1111",
            media_types::OCI_IMAGE_MANIFEST,
            837,
            &body,
            Some(&entry),
        );

        assert_eq!(attestation.kinds, vec!["provenance".to_string()]);
        assert_eq!(attestation.predicates.len(), 1);
        assert_eq!(attestation.predicates[0].digest, "sha256:fc4f36c839eb");
        assert_eq!(attestation.predicates[0].size, 1741);
        assert_eq!(
            attestation.predicates[0].predicate_type.as_deref(),
            Some("https://slsa.dev/provenance/v1")
        );
        assert_eq!(
            attestation.artifact_type.as_deref(),
            Some(media_types::DOCKER_ATTESTATION_MANIFEST)
        );
        assert_eq!(
            attestation.target_digest(),
            Some("sha256:15b657aed6e28ad2777367458605920a0f806395501ff264df77c18b27844e59")
        );
        assert!(is_attestation(&body, Some(&entry)));
    }

    #[test]
    fn deduplicates_kinds_and_keeps_statement_order() {
        let body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": media_types::OCI_IMAGE_MANIFEST,
            "artifactType": media_types::DOCKER_ATTESTATION_MANIFEST,
            "layers": [
                {
                    "mediaType": media_types::IN_TOTO_JSON,
                    "digest": "sha256:aa",
                    "size": 10,
                    "annotations": { media_types::PREDICATE_TYPE_ANNOTATION: "https://spdx.dev/Document" },
                },
                {
                    "mediaType": media_types::IN_TOTO_JSON,
                    "digest": "sha256:bb",
                    "size": 20,
                    "annotations": { media_types::PREDICATE_TYPE_ANNOTATION: "https://slsa.dev/provenance/v0.2" },
                },
                {
                    "mediaType": media_types::IN_TOTO_JSON,
                    "digest": "sha256:cc",
                    "size": 30,
                    "annotations": { media_types::PREDICATE_TYPE_ANNOTATION: "https://slsa.dev/provenance/v1" },
                },
            ],
        });
        let attestation =
            Attestation::from_manifest("sha256:abc", media_types::OCI_IMAGE_MANIFEST, 1, &body, None);
        assert_eq!(
            attestation.kinds,
            vec!["sbom".to_string(), "provenance".to_string()]
        );
        assert_eq!(attestation.predicates.len(), 3);
        assert_eq!(attestation.predicates[2].digest, "sha256:cc");
    }

    #[test]
    fn falls_back_to_the_statement_predicate_type() {
        let body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": media_types::OCI_IMAGE_MANIFEST,
            "layers": [{
                "mediaType": media_types::IN_TOTO_JSON,
                "digest": "sha256:aa",
                "size": 10,
            }],
        });
        let statement = serde_json::json!({
            "_type": "https://in-toto.io/Statement/v1",
            "predicateType": "https://slsa.dev/provenance/v1",
        });
        let predicate =
            AttestationPredicate::from_descriptor(&body["layers"][0], Some(&statement))
                .expect("predicate");
        assert_eq!(predicate.kind, "provenance");
        assert_eq!(
            predicate.predicate_type.as_deref(),
            Some("https://slsa.dev/provenance/v1")
        );
    }

    #[test]
    fn ignores_non_in_toto_layers() {
        let body = serde_json::json!({
            "mediaType": media_types::OCI_IMAGE_LAYER_GZIP,
            "digest": "sha256:aa",
            "size": 10,
        });
        assert_eq!(AttestationPredicate::from_descriptor(&body, None), None);
    }

    #[test]
    fn referrers_attestation_uses_the_subject_as_target() {
        let body = buildkit_attestation_manifest("sha256:fc4f36c839eb", "https://spdx.dev/Document");
        let attestation = Attestation::from_manifest(
            "sha256:beef",
            media_types::OCI_IMAGE_MANIFEST,
            500,
            &body,
            None,
        );
        assert_eq!(attestation.kinds, vec!["sbom".to_string()]);
        assert_eq!(
            attestation.target_digest(),
            Some("sha256:15b657aed6e28ad2777367458605920a0f806395501ff264df77c18b27844e59")
        );
        assert!(is_attestation(&body, None));
    }

    #[test]
    fn plain_image_manifest_is_not_an_attestation() {
        let body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": media_types::OCI_IMAGE_MANIFEST,
            "config": { "mediaType": media_types::OCI_IMAGE_CONFIG, "digest": "sha256:aa", "size": 7 },
            "layers": [{ "mediaType": media_types::OCI_IMAGE_LAYER_GZIP, "digest": "sha256:bb", "size": 42 }],
        });
        assert!(!is_attestation(&body, None));
    }

    #[test]
    fn recognises_the_placeholder_platform() {
        assert!(is_placeholder_platform(&serde_json::json!({
            "os": "unknown", "architecture": "unknown"
        })));
        assert!(!is_placeholder_platform(&serde_json::json!({
            "os": "linux", "architecture": "arm64"
        })));
        assert!(!is_placeholder_platform(&serde_json::json!({})));
    }
}
