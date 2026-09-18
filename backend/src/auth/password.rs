//! Argon2id password hashing and verification.
//!
//! Parameters follow the OWASP recommendation of 19 MiB of memory, two
//! iterations and one lane (`m=19456`, `t=2`, `p=1`). A fresh random salt is
//! generated for every password and the resulting PHC string is stored verbatim
//! in `users.password_hash`.

use anyhow::{Result, anyhow, bail};
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use axum::Router;

use crate::state::AppState;

/// Memory cost in KiB (`m`).
const MEMORY_COST_KIB: u32 = 19_456;
/// Iteration count (`t`).
const TIME_COST: u32 = 2;
/// Degree of parallelism (`p`).
const PARALLELISM: u32 = 1;

/// Builds the configured Argon2id hasher.
fn hasher() -> Result<Argon2<'static>> {
    let params = Params::new(MEMORY_COST_KIB, TIME_COST, PARALLELISM, None)
        .map_err(|err| anyhow!("invalid Argon2 parameters: {err}"))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// Hashes `password` with Argon2id and returns the PHC string.
///
/// Empty passwords are rejected because they can never be a legitimate
/// credential.
pub fn hash_password(password: &str) -> Result<String> {
    if password.is_empty() {
        bail!("password must not be empty");
    }
    let hasher = hasher()?;
    let hash = hasher
        .hash_password(password.as_bytes())
        .map_err(|err| anyhow!("failed to hash password: {err}"))?;
    Ok(hash.to_string())
}

/// Verifies `password` against a stored PHC hash.
///
/// Any malformed hash, unknown algorithm or mismatch yields `false`; the
/// comparison itself is constant-time inside `password-hash`.
pub fn verify_password(hash: &str, password: &str) -> bool {
    if password.is_empty() || hash.is_empty() {
        return false;
    }
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    let Ok(hasher) = hasher() else {
        return false;
    };
    hasher.verify_password(password.as_bytes(), &parsed).is_ok()
}

/// Empty router following the submodule convention.
pub fn router() -> Router<AppState> {
    Router::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_verifies_a_password() {
        let hash = hash_password("correct horse battery staple").expect("hash");
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password(&hash, "correct horse battery staple"));
    }

    #[test]
    fn rejects_a_wrong_password() {
        let hash = hash_password("s3cret-password").expect("hash");
        assert!(!verify_password(&hash, "s3cret-passwore"));
        assert!(!verify_password(&hash, ""));
    }

    #[test]
    fn salts_are_unique_per_hash() {
        let first = hash_password("same password").expect("first");
        let second = hash_password("same password").expect("second");
        assert_ne!(first, second);
        assert!(verify_password(&first, "same password"));
        assert!(verify_password(&second, "same password"));
    }

    #[test]
    fn rejects_empty_passwords_and_malformed_hashes() {
        assert!(hash_password("").is_err());
        assert!(!verify_password("not-a-phc-string", "whatever"));
        assert!(!verify_password("", "whatever"));
    }
}
