//! JWT access tokens and opaque refresh/e-mail/service tokens.
//!
//! Access tokens are HS256 JWTs whose `jti` doubles as the web-session id, so
//! `logout` can revoke the session carried by the access cookie without an extra
//! claim. Refresh, e-mail and service-account tokens are opaque random values;
//! only a SHA-256 (refresh/e-mail) or Argon2id (service account) derivative is
//! persisted.

use std::collections::HashSet;
use std::sync::OnceLock;

use anyhow::{Result, anyhow, bail};
use axum::Router;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use jsonwebtoken::crypto::{CryptoProvider, JwtSigner, JwtVerifier, KeyUtils};
use jsonwebtoken::errors::{Error as JwtError, ErrorKind, Result as JwtResult};
use jsonwebtoken::signature;
use jsonwebtoken::signature::{Signer, Verifier};
use jsonwebtoken::{
    Algorithm, DecodingKey, DecodingKeyKind, EncodingKey, Header, Validation, decode, encode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::auth::password;
use crate::config::Config;
use crate::state::AppState;

/// Marker written into the `kind` claim of every access token.
pub const ACCESS_KIND: &str = "access";
/// `kind` claim of a registry bearer token handed to `docker`.
pub const REGISTRY_KIND: &str = "registry";
/// `kind` claim of the half-authenticated token awaiting a TOTP code.
pub const MFA_KIND: &str = "mfa";
/// Audience of registry bearer tokens.
pub const REGISTRY_AUDIENCE: &str = "lighthouse-registry";
/// `email_tokens.kind` for e-mail verification.
pub const EMAIL_KIND_VERIFY: &str = "verify_email";
/// `email_tokens.kind` for password reset.
pub const EMAIL_KIND_RESET: &str = "reset_password";

/// Access-token claims. `jti` carries the web-session identifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: i64,
    pub username: String,
    pub kind: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
}

// ---- HS256 crypto provider -------------------------------------------------
//
// `jsonwebtoken` 11 ships no crypto backend unless a feature enables one, and
// the project must not add dependencies. Installing a minimal HMAC-SHA256
// provider backed by the already-present `sha2` crate keeps HS256 working.

struct HmacSigner {
    key: Vec<u8>,
}

struct HmacVerifier {
    key: Vec<u8>,
}

impl Signer<Vec<u8>> for HmacSigner {
    fn try_sign(&self, msg: &[u8]) -> Result<Vec<u8>, signature::Error> {
        Ok(hmac_sha256(&self.key, msg))
    }
}

impl JwtSigner for HmacSigner {
    fn algorithm(&self) -> Algorithm {
        Algorithm::HS256
    }
}

impl Verifier<Vec<u8>> for HmacVerifier {
    fn verify(&self, msg: &[u8], signature: &Vec<u8>) -> Result<(), signature::Error> {
        let expected = hmac_sha256(&self.key, msg);
        if expected.ct_eq(signature.as_slice()).into() {
            Ok(())
        } else {
            Err(signature::Error::new())
        }
    }
}

impl JwtVerifier for HmacVerifier {
    fn algorithm(&self) -> Algorithm {
        Algorithm::HS256
    }
}

static HMAC_PROVIDER: CryptoProvider = CryptoProvider {
    signer_factory: hmac_signer_factory,
    verifier_factory: hmac_verifier_factory,
    key_utils: KeyUtils::new_unimplemented(),
};

fn hmac_signer_factory(algorithm: &Algorithm, key: &EncodingKey) -> JwtResult<Box<dyn JwtSigner>> {
    if !matches!(algorithm, Algorithm::HS256) {
        return Err(JwtError::from(ErrorKind::InvalidAlgorithm));
    }
    Ok(Box::new(HmacSigner {
        key: key.as_bytes().to_vec(),
    }))
}

fn hmac_verifier_factory(
    algorithm: &Algorithm,
    key: &DecodingKey,
) -> JwtResult<Box<dyn JwtVerifier>> {
    if !matches!(algorithm, Algorithm::HS256) {
        return Err(JwtError::from(ErrorKind::InvalidAlgorithm));
    }
    let DecodingKeyKind::SecretOrDer(bytes) = key.kind() else {
        return Err(JwtError::from(ErrorKind::InvalidAlgorithm));
    };
    Ok(Box::new(HmacVerifier { key: bytes.clone() }))
}

fn ensure_provider() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let _ = HMAC_PROVIDER.install_default();
    });
}

/// RFC 2104 HMAC-SHA256.
fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        let digest = Sha256::digest(key);
        block[..digest.len()].copy_from_slice(&digest);
    } else {
        block[..key.len()].copy_from_slice(key);
    }

    let mut inner_pad = [0x36u8; BLOCK];
    let mut outer_pad = [0x5cu8; BLOCK];
    for index in 0..BLOCK {
        inner_pad[index] ^= block[index];
        outer_pad[index] ^= block[index];
    }

    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().to_vec()
}

// ---- access tokens ---------------------------------------------------------

/// Issues an HS256 access token for `user_id`, binding it to `session_id`.
pub fn issue_access_token(
    config: &Config,
    user_id: i64,
    username: &str,
    session_id: &str,
) -> Result<String> {
    ensure_provider();
    issue_access_token_at(
        config,
        user_id,
        username,
        session_id,
        Utc::now().timestamp(),
    )
}

fn issue_access_token_at(
    config: &Config,
    user_id: i64,
    username: &str,
    session_id: &str,
    issued_at: i64,
) -> Result<String> {
    let claims = Claims {
        sub: user_id,
        username: username.to_string(),
        kind: ACCESS_KIND.to_string(),
        iat: issued_at,
        exp: issued_at + config.access_token_ttl_secs,
        jti: session_id.to_string(),
    };
    let header = Header::new(Algorithm::HS256);
    let key = EncodingKey::from_secret(config.jwt_secret.as_bytes());
    encode(&header, &claims, &key).map_err(|err| anyhow!("failed to issue access token: {err}"))
}

/// Verifies an access token's signature, expiry and `kind` claim.
pub fn verify_access_token(config: &Config, token: &str) -> Result<Claims> {
    ensure_provider();
    let key = DecodingKey::from_secret(config.jwt_secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    validation.required_spec_claims = HashSet::from(["exp".to_string()]);
    let data = decode::<Claims>(token, &key, &validation)
        .map_err(|err| anyhow!("invalid access token: {err}"))?;
    if data.claims.kind != ACCESS_KIND {
        bail!("unexpected access token kind");
    }
    Ok(data.claims)
}

// ---- registry bearer tokens ------------------------------------------------
//
// Registry tokens let `docker` authenticate against /v2 without replaying a
// password on every request, and let a logged-out client pull public content
// with an anonymous token. They are signed with a key derived from the JWT
// secret (`sha256(secret || ":registry")`) so a registry token can never be
// accepted where an access token is required, even if the kind check were
// removed. `sub == 0` means anonymous.

/// One granted scope inside a registry token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryAccess {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub actions: Vec<String>,
}

/// Registry bearer-token claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryClaims {
    pub sub: i64,
    pub username: Option<String>,
    pub kind: String,
    pub aud: String,
    pub iss: String,
    pub sid: Option<i64>,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
    pub access: Vec<RegistryAccess>,
}

/// Claims of the short-lived token minted between the password and TOTP steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MfaClaims {
    pub sub: i64,
    pub kind: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
}

/// Domain-separates HMAC keys so tokens of different kinds cannot be swapped.
fn derived_key(secret: &str, label: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":");
    hasher.update(label.as_bytes());
    hasher.finalize().to_vec()
}

/// Issues a registry bearer token for `sub` (`0` for anonymous).
pub fn issue_registry_token(
    config: &Config,
    sub: i64,
    username: Option<&str>,
    service_account_id: Option<i64>,
    access: Vec<RegistryAccess>,
) -> Result<String> {
    ensure_provider();
    let now = Utc::now().timestamp();
    let mut id = [0u8; 16];
    rand::fill(&mut id);
    let claims = RegistryClaims {
        sub,
        username: username.map(str::to_string),
        kind: REGISTRY_KIND.to_string(),
        aud: REGISTRY_AUDIENCE.to_string(),
        iss: config.base_url.clone(),
        sid: service_account_id,
        iat: now,
        exp: now + config.registry_token_ttl_secs,
        jti: hex::encode(id),
        access,
    };
    let header = Header::new(Algorithm::HS256);
    let key = EncodingKey::from_secret(&derived_key(&config.jwt_secret, "registry"));
    encode(&header, &claims, &key).map_err(|err| anyhow!("failed to issue registry token: {err}"))
}

/// Verifies a registry token's signature, expiry, audience and `kind` claim.
pub fn verify_registry_token(config: &Config, token: &str) -> Result<RegistryClaims> {
    ensure_provider();
    let key = DecodingKey::from_secret(&derived_key(&config.jwt_secret, "registry"));
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    validation.required_spec_claims = HashSet::from(["exp".to_string()]);
    validation.set_audience(&[REGISTRY_AUDIENCE]);
    validation.set_issuer(&[config.base_url.as_str()]);
    let data = decode::<RegistryClaims>(token, &key, &validation)
        .map_err(|err| anyhow!("invalid registry token: {err}"))?;
    if data.claims.kind != REGISTRY_KIND {
        bail!("unexpected registry token kind");
    }
    Ok(data.claims)
}

/// Issues the short-lived token that stands in for a password once the caller
/// still owes a TOTP code.
pub fn issue_mfa_token(config: &Config, user_id: i64) -> Result<String> {
    ensure_provider();
    let now = Utc::now().timestamp();
    let mut id = [0u8; 16];
    rand::fill(&mut id);
    let claims = MfaClaims {
        sub: user_id,
        kind: MFA_KIND.to_string(),
        iat: now,
        exp: now + config.mfa_token_ttl_secs,
        jti: hex::encode(id),
    };
    let header = Header::new(Algorithm::HS256);
    let key = EncodingKey::from_secret(&derived_key(&config.jwt_secret, "mfa"));
    encode(&header, &claims, &key).map_err(|err| anyhow!("failed to issue mfa token: {err}"))
}

/// Verifies an MFA token. Never yields an identity on its own.
pub fn verify_mfa_token(config: &Config, token: &str) -> Result<MfaClaims> {
    ensure_provider();
    let key = DecodingKey::from_secret(&derived_key(&config.jwt_secret, "mfa"));
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    validation.required_spec_claims = HashSet::from(["exp".to_string()]);
    let data = decode::<MfaClaims>(token, &key, &validation)
        .map_err(|err| anyhow!("invalid mfa token: {err}"))?;
    if data.claims.kind != MFA_KIND {
        bail!("unexpected mfa token kind");
    }
    Ok(data.claims)
}


// ---- opaque tokens ---------------------------------------------------------

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    rand::fill(&mut bytes);
    bytes
}

/// Generates a 32-byte URL-safe refresh token.
pub fn generate_refresh_token() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<32>())
}

/// Generates a 32-byte hexadecimal e-mail token.
pub fn generate_email_token() -> String {
    hex::encode(random_bytes::<32>())
}

/// Generates a service-account credential `lhr_<random>` together with its
/// display prefix/suffix and Argon2id hash.
pub fn generate_service_token() -> (String, String, String, String) {
    let plaintext = format!("lhr_{}", hex::encode(random_bytes::<24>()));
    let prefix = plaintext.chars().take(3).collect::<String>();
    let suffix = plaintext
        .chars()
        .rev()
        .take(3)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let hash = password::hash_password(&plaintext).unwrap_or_default();
    (plaintext, prefix, suffix, hash)
}

/// Verifies a service-account token against its stored Argon2id hash.
pub fn verify_service_token(hash: &str, candidate: &str) -> bool {
    password::verify_password(hash, candidate)
}

/// Generates an app-password credential `lhp_<random>` together with its display
/// prefix/suffix and SHA-256 hash. App passwords are 192-bit random values, so a
/// fast indexed hash is sufficient and keeps Basic auth off the Argon2 path.
pub fn generate_app_password_token() -> (String, String, String, String) {
    let plaintext = format!("lhp_{}", hex::encode(random_bytes::<24>()));
    let prefix = plaintext.chars().take(4).collect::<String>();
    let suffix = plaintext
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let hash = hash_token(&plaintext);
    (plaintext, prefix, suffix, hash)
}

/// SHA-256 hash used to persist refresh and e-mail tokens.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Constant-time comparison of a presented opaque token against its stored hash.
pub fn verify_token_hash(stored_hash: &str, presented: &str) -> bool {
    let computed = hash_token(presented);
    stored_hash.as_bytes().ct_eq(computed.as_bytes()).into()
}

/// Empty router following the submodule convention.
pub fn router() -> Router<AppState> {
    Router::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::test_config;
    use jsonwebtoken::{EncodingKey, Header, encode};

    fn config() -> Config {
        let dir = tempfile::tempdir().expect("tempdir");
        test_config(dir.path())
    }

    #[test]
    fn access_token_round_trips() {
        let config = config();
        let token = issue_access_token(&config, 42, "alice", "session-1").expect("issue");
        let claims = verify_access_token(&config, &token).expect("verify");
        assert_eq!(claims.sub, 42);
        assert_eq!(claims.username, "alice");
        assert_eq!(claims.kind, ACCESS_KIND);
        assert_eq!(claims.jti, "session-1");
        assert!(claims.exp > claims.iat);
    }

    #[test]
    fn access_token_rejects_wrong_secret() {
        let mut config = config();
        let token = issue_access_token(&config, 1, "bob", "s").expect("issue");
        config.jwt_secret = "a-completely-different-secret".to_string();
        assert!(verify_access_token(&config, &token).is_err());
    }

    #[test]
    fn access_token_rejects_tampering() {
        let config = config();
        let token = issue_access_token(&config, 1, "bob", "s").expect("issue");
        let mut parts: Vec<&str> = token.split('.').collect();
        let signature = parts[2].to_string();
        let mut tampered: String = signature;
        tampered.push('A');
        parts[2] = &tampered;
        assert!(verify_access_token(&config, &parts.join(".")).is_err());
    }

    #[test]
    fn access_token_rejects_expiry() {
        let config = config();
        ensure_provider();
        let expired = Claims {
            sub: 7,
            username: "carol".to_string(),
            kind: ACCESS_KIND.to_string(),
            iat: 1_000,
            exp: 1_001,
            jti: "s".to_string(),
        };
        let key = EncodingKey::from_secret(config.jwt_secret.as_bytes());
        let token = encode(&Header::new(Algorithm::HS256), &expired, &key).expect("encode");
        assert!(verify_access_token(&config, &token).is_err());
    }

    #[test]
    fn opaque_tokens_are_unique_and_hashed() {
        let first = generate_refresh_token();
        let second = generate_refresh_token();
        assert_ne!(first, second);
        assert!(first.len() >= 40);

        let hash = hash_token(&first);
        assert!(verify_token_hash(&hash, &first));
        assert!(!verify_token_hash(&hash, &second));
        assert!(!verify_token_hash(&hash, "nonsense"));
    }

    #[test]
    fn service_tokens_are_verifiable_and_displayed() {
        let (plaintext, prefix, suffix, hash) = generate_service_token();
        assert!(plaintext.starts_with("lhr_"));
        assert_eq!(prefix, "lhr");
        assert!(plaintext.ends_with(&suffix));
        assert!(verify_service_token(&hash, &plaintext));
        assert!(!verify_service_token(&hash, "lhr_deadbeef"));
    }

    #[test]
    fn email_tokens_are_hex() {
        let token = generate_email_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
