//! Content digests: parsing, computation and streaming verification.
//!
//! A digest is written `algorithm:encoded` where the algorithm matches
//! `^[a-z0-9]+(?:[+._-][a-z0-9]+)*$` and the encoded part matches
//! `^[a-zA-Z0-9=_-]+$`. Algorithms this registry knows how to compute
//! (`sha256`, `sha512`, `blake3`) additionally require lowercase hexadecimal of
//! a fixed length; anything else that conforms to the grammar is accepted, as
//! the OCI specification mandates forward compatibility.

#![allow(dead_code)]

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256, Sha512};

use crate::error::RegistryError;

pub const ALGORITHM_SHA256: &str = "sha256";
pub const ALGORITHM_SHA512: &str = "sha512";
pub const ALGORITHM_BLAKE3: &str = "blake3";

/// A validated OCI digest (`algorithm:encoded`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest {
    algorithm: String,
    encoded: String,
}

impl Digest {
    /// Parses and fully validates a digest string.
    pub fn parse(input: &str) -> Result<Self, RegistryError> {
        let (algorithm, encoded) = input
            .split_once(':')
            .ok_or_else(|| RegistryError::digest_invalid(input))?;

        if !is_valid_algorithm(algorithm) || !is_valid_encoded(encoded) {
            return Err(RegistryError::digest_invalid(input));
        }

        if let Some(expected_len) = known_encoded_len(algorithm) {
            if encoded.len() != expected_len || !is_lowercase_hex(encoded) {
                return Err(RegistryError::digest_invalid(input));
            }
        }

        Ok(Self {
            algorithm: algorithm.to_string(),
            encoded: encoded.to_string(),
        })
    }

    /// The algorithm segment, e.g. `sha256`.
    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }

    /// The encoded segment, e.g. the lowercase hexadecimal hash.
    pub fn encoded(&self) -> &str {
        &self.encoded
    }

    /// True when this registry can compute and verify the algorithm.
    pub fn is_supported(&self) -> bool {
        matches!(
            self.algorithm.as_str(),
            ALGORITHM_SHA256 | ALGORITHM_SHA512
        )
    }

    /// Digests the bytes with SHA-256.
    pub fn from_bytes_sha256(bytes: &[u8]) -> Self {
        Self {
            algorithm: ALGORITHM_SHA256.to_string(),
            encoded: hex::encode(Sha256::digest(bytes)),
        }
    }

    /// The canonical digest of `bytes`, currently SHA-256.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_bytes_sha256(bytes)
    }

    /// Recomputes the digest over `bytes` and compares it to this value.
    ///
    /// Unknown algorithms cannot be recomputed and always fail verification.
    pub fn verify(&self, bytes: &[u8]) -> bool {
        match self.algorithm.as_str() {
            ALGORITHM_SHA256 => hex::encode(Sha256::digest(bytes)) == self.encoded,
            ALGORITHM_SHA512 => hex::encode(Sha512::digest(bytes)) == self.encoded,
            _ => false,
        }
    }

    /// Starts a streaming verifier for this digest, suitable for hashing an
    /// upload while it is being written.
    pub fn verifier(&self) -> DigestVerifier {
        DigestVerifier::new(self)
    }

    /// Builds a digest from already-validated parts. Used internally where the
    /// grammar has been checked by construction.
    fn from_parts(algorithm: String, encoded: String) -> Self {
        Self { algorithm, encoded }
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algorithm, self.encoded)
    }
}

impl FromStr for Digest {
    type Err = RegistryError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// Streaming digest computation.
pub trait Verifier {
    /// Feeds the next chunk of content into the hash.
    fn update(&mut self, data: &[u8]);

    /// Produces the digest of everything fed so far. The verifier may be used
    /// again afterwards.
    fn finish(&self) -> Digest;
}

/// Streaming verifier for `sha256` and `sha512` digests.
pub struct DigestVerifier {
    expected: Digest,
    backend: VerifierBackend,
}

enum VerifierBackend {
    Sha256(Sha256),
    Sha512(Sha512),
    Unsupported,
}

impl DigestVerifier {
    /// Builds a verifier that will compare its result against `expected`.
    pub fn new(expected: &Digest) -> Self {
        let backend = match expected.algorithm() {
            ALGORITHM_SHA256 => VerifierBackend::Sha256(Sha256::new()),
            ALGORITHM_SHA512 => VerifierBackend::Sha512(Sha512::new()),
            _ => VerifierBackend::Unsupported,
        };
        Self {
            expected: expected.clone(),
            backend,
        }
    }

    /// The digest this verifier checks against.
    pub fn expected(&self) -> &Digest {
        &self.expected
    }

    /// True when the algorithm backing this verifier is computable.
    pub fn is_supported(&self) -> bool {
        !matches!(self.backend, VerifierBackend::Unsupported)
    }

    /// True when the content hashed so far matches the expected digest.
    pub fn matches(&self) -> bool {
        self.is_supported() && self.finish() == self.expected
    }
}

impl Verifier for DigestVerifier {
    fn update(&mut self, data: &[u8]) {
        match &mut self.backend {
            VerifierBackend::Sha256(hasher) => hasher.update(data),
            VerifierBackend::Sha512(hasher) => hasher.update(data),
            VerifierBackend::Unsupported => {}
        }
    }

    fn finish(&self) -> Digest {
        match &self.backend {
            VerifierBackend::Sha256(hasher) => Digest::from_parts(
                ALGORITHM_SHA256.to_string(),
                hex::encode(hasher.clone().finalize()),
            ),
            VerifierBackend::Sha512(hasher) => Digest::from_parts(
                ALGORITHM_SHA512.to_string(),
                hex::encode(hasher.clone().finalize()),
            ),
            // An empty encoding cannot collide with any grammar-valid digest.
            VerifierBackend::Unsupported => {
                Digest::from_parts(self.expected.algorithm().to_string(), String::new())
            }
        }
    }
}

fn known_encoded_len(algorithm: &str) -> Option<usize> {
    match algorithm {
        ALGORITHM_SHA256 | ALGORITHM_BLAKE3 => Some(64),
        ALGORITHM_SHA512 => Some(128),
        _ => None,
    }
}

fn is_valid_algorithm(algorithm: &str) -> bool {
    if algorithm.is_empty() {
        return false;
    }
    let mut previous_separator = false;
    for (index, byte) in algorithm.bytes().enumerate() {
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            previous_separator = false;
            continue;
        }
        if !matches!(byte, b'+' | b'.' | b'_' | b'-') || index == 0 || previous_separator {
            return false;
        }
        previous_separator = true;
    }
    !previous_separator
}

fn is_valid_encoded(encoded: &str) -> bool {
    !encoded.is_empty()
        && encoded
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b'_' | b'-'))
}

fn is_lowercase_hex(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_sha256() {
        let encoded = "a".repeat(64);
        let digest = Digest::parse(&format!("sha256:{encoded}")).expect("valid digest");
        assert_eq!(digest.algorithm(), "sha256");
        assert_eq!(digest.encoded(), encoded);
        assert_eq!(digest.to_string(), format!("sha256:{encoded}"));
    }

    #[test]
    fn rejects_uppercase_hex_for_known_algorithm() {
        let encoded = "A".repeat(64);
        assert!(Digest::parse(&format!("sha256:{encoded}")).is_err());
    }

    #[test]
    fn rejects_wrong_length_for_known_algorithm() {
        assert!(Digest::parse(&format!("sha256:{}", "a".repeat(63))).is_err());
        assert!(Digest::parse(&format!("sha512:{}", "a".repeat(64))).is_err());
    }

    #[test]
    fn rejects_missing_colon_and_empty() {
        assert!(Digest::parse("").is_err());
        assert!(Digest::parse("sha256").is_err());
        assert!(Digest::parse("sha256:").is_err());
        assert!(Digest::parse(":abc").is_err());
    }

    #[test]
    fn rejects_non_hex_encoded() {
        assert!(Digest::parse(&format!("sha256:{}", "z".repeat(64))).is_err());
        assert!(Digest::parse(&format!("sha256:{}", "g".repeat(64))).is_err());
    }

    #[test]
    fn accepts_grammar_valid_unknown_algorithm() {
        let digest = Digest::parse("multihash+base58:QmR3V6").expect("unknown but valid");
        assert_eq!(digest.algorithm(), "multihash+base58");
        assert_eq!(digest.encoded(), "QmR3V6");
        assert!(!digest.is_supported());
        assert!(!digest.verify(b"anything"));
    }

    #[test]
    fn rejects_malformed_algorithm() {
        assert!(Digest::parse("sha--256:abcd").is_err());
        assert!(Digest::parse("sha_:abcd").is_err());
        assert!(Digest::parse("-sha256:abcd").is_err());
        assert!(Digest::parse("SHA256:abcd").is_err());
        assert!(Digest::parse("sha..256:abcd").is_err());
    }

    #[test]
    fn accepts_separator_bearing_unknown_algorithm() {
        let digest = Digest::parse("sha-256:abcd").expect("single separators are valid");
        assert_eq!(digest.algorithm(), "sha-256");
        assert!(!digest.is_supported());
    }

    #[test]
    fn rounds_trip_through_serde() {
        let digest = Digest::from_bytes(b"hello world");
        let json = serde_json::to_string(&digest).expect("serialize");
        assert_eq!(json, format!("\"{digest}\""));
        let decoded: Digest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, digest);
    }

    #[test]
    fn from_bytes_matches_known_sha256_of_empty() {
        let digest = Digest::from_bytes(b"");
        assert_eq!(
            digest.encoded(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(digest.verify(b""));
        assert!(!digest.verify(b"x"));
    }

    #[test]
    fn streaming_verifier_tracks_sha256() {
        let content = b"streamed content across chunks";
        let expected = Digest::from_bytes(content);
        let mut verifier = expected.verifier();
        assert!(verifier.is_supported());
        verifier.update(&content[..7]);
        verifier.update(&content[7..]);
        assert_eq!(verifier.finish(), expected);
        assert!(verifier.matches());
    }

    #[test]
    fn streaming_verifier_tracks_sha512() {
        let content = b"another payload";
        let encoded = hex::encode(Sha512::digest(content));
        let expected = Digest::parse(&format!("sha512:{encoded}")).expect("valid");
        let mut verifier = expected.verifier();
        verifier.update(content);
        assert!(verifier.matches());
    }

    #[test]
    fn streaming_verifier_rejects_mismatch() {
        let expected = Digest::from_bytes(b"expected");
        let mut verifier = expected.verifier();
        verifier.update(b"actual");
        assert!(!verifier.matches());
        assert_ne!(verifier.finish(), expected);
    }
}
