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

    #[test]
    fn tokens_carry_256_bits_of_url_safe_randomness() {
        for prefix in ["chi_", "chs_", "chc_", "cha_", ""] {
            let t = generate_token(prefix).unwrap();
            let body = t.strip_prefix(prefix).unwrap();
            assert_eq!(body.len(), 43, "32 bytes, unpadded base64url");
            assert_eq!(URL_SAFE_NO_PAD.decode(body).unwrap().len(), 32);
            assert!(body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "{t}");
        }
        let many: std::collections::HashSet<_> = (0..200).map(|_| generate_token("chi_").unwrap()).collect();
        assert_eq!(many.len(), 200);
    }

    #[test]
    fn token_hash_is_sha256_and_never_the_plaintext() {
        // Known SHA-256 test vector ("abc").
        assert_eq!(hex::encode(hash_token("abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let t = generate_token("chs_").unwrap();
        let h = hash_token(&t);
        assert_eq!(h.len(), 32);
        assert_ne!(h, t.as_bytes());
        assert!(!hex::encode(&h).contains(&t[4..]));
        assert_eq!(hash_token(&t), h, "deterministic, so it can be looked up");
    }

    #[test]
    fn constant_time_eq_handles_lengths_and_single_bit_differences() {
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(b"abc", b"abcd"), "prefix is not equal");
        assert!(!constant_time_eq(b"abcd", b"abc"));
        assert!(!constant_time_eq(b"", b"a"));
        let a = hash_token("x");
        for i in 0..a.len() {
            for bit in 0..8 {
                let mut b = a.clone();
                b[i] ^= 1 << bit;
                assert!(!constant_time_eq(&a, &b), "byte {i} bit {bit}");
            }
        }
    }

    #[test]
    fn prefix_is_a_short_non_secret_identifier() {
        let t = generate_token("chi_").unwrap();
        let p = token_prefix(&t);
        assert_eq!(p.chars().count(), 10);
        assert!(t.starts_with(&p));
        assert_eq!(token_prefix("short"), "short");
        assert_eq!(token_prefix("ééééééééééééé").chars().count(), 10, "never splits a UTF-8 character");
    }

    #[test]
    fn password_hashes_are_salted_and_malformed_hashes_fail_closed() {
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();
        assert_ne!(a, b, "random salt per hash");
        assert!(!a.contains("same"));
        assert!(verify_password("same", &a) && verify_password("same", &b));
        assert!(!verify_password("", &a));
        assert!(!verify_password("Same", &a));
        for bad in ["", "$argon2id$", "$argon2id$v=19$m=19456,t=2,p=1$bad", "plaintext"] {
            assert!(!verify_password("same", bad), "{bad:?}");
        }
        // An empty password still hashes (policy is enforced elsewhere) and does not verify others.
        let empty = hash_password("").unwrap();
        assert!(verify_password("", &empty) && !verify_password("x", &empty));
    }

    #[test]
    fn generated_passwords_are_typable() {
        let p = generate_password().unwrap();
        assert_eq!(p.len(), 24);
        assert!(p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_ne!(p, generate_password().unwrap());
    }
}
