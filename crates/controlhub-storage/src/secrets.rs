//! Token generation, hashing and password handling (vetted libraries only).

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::TryRngCore;
use sha2::{Digest, Sha256};

use crate::StorageError;

/// 256-bit random token, URL-safe base64, with a type prefix
/// (`chi_` ingestion, `chs_` session, `chc_` csrf).
pub fn generate_token(prefix: &str) -> Result<String, StorageError> {
    let mut buf = [0u8; 32];
    rand::rngs::OsRng.try_fill_bytes(&mut buf).map_err(|e| StorageError::Other(format!("os rng unavailable: {e}")))?;
    Ok(format!("{prefix}{}", URL_SAFE_NO_PAD.encode(buf)))
}

/// Tokens are high-entropy, so a fast hash (SHA-256) is appropriate for
/// lookup; the plaintext is never stored.
pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

pub fn token_prefix(token: &str) -> String {
    token.chars().take(10).collect()
}

pub fn hash_password(password: &str) -> Result<String, StorageError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| StorageError::Other(format!("password hashing failed: {e}")))
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/// Human-typable random password for local bootstrap.
pub fn generate_password() -> Result<String, StorageError> {
    let mut buf = [0u8; 18];
    rand::rngs::OsRng.try_fill_bytes(&mut buf).map_err(|e| StorageError::Other(format!("os rng unavailable: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn password_roundtrip() {
        let h = hash_password("correct horse").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("x", "not-a-phc"));
    }

    #[test]
    fn tokens_are_unique_and_prefixed() {
        let a = generate_token("chi_").unwrap();
        let b = generate_token("chi_").unwrap();
        assert!(a.starts_with("chi_"));
        assert_ne!(a, b);
        assert_eq!(hash_token(&a).len(), 32);
        assert!(constant_time_eq(&hash_token(&a), &hash_token(&a)));
        assert!(!constant_time_eq(&hash_token(&a), &hash_token(&b)));
    }
}
