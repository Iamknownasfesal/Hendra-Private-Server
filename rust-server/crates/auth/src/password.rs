//! Storing passwords so that a stolen database is not a stolen account.
//!
//! Argon2id, with a random salt per password. The parameters are the crate's defaults, which are
//! the current OWASP recommendation — deliberately not tuned down for speed, because the cost is
//! paid once per login and the thing it buys is that a leaked table is expensive to attack rather
//! than a list of passwords.

use argon2::Argon2;
use argon2::password_hash::{
    PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PasswordError {
    #[error("a password must be at least {minimum} characters")]
    TooShort { minimum: usize },

    #[error("a password must be at most {maximum} characters")]
    TooLong { maximum: usize },

    #[error("could not hash the password")]
    Hashing,

    #[error("the stored hash is not readable")]
    Corrupt,
}

/// The shortest password accepted.
///
/// Eight rather than twelve, and no rules about punctuation. Composition rules push people toward
/// `Password1!` and a longer minimum pushes them toward reuse; neither is worth the friction for a
/// game account, and the real protection is that the hash is expensive.
pub const MINIMUM_LENGTH: usize = 8;

/// The longest password accepted.
///
/// Not a security limit but a denial-of-service one: Argon2 will happily spend a long time on a
/// megabyte of input, and an unauthenticated endpoint that does so is a way to exhaust the server.
pub const MAXIMUM_LENGTH: usize = 256;

/// Hashes a password for storage.
pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    if password.chars().count() < MINIMUM_LENGTH {
        return Err(PasswordError::TooShort {
            minimum: MINIMUM_LENGTH,
        });
    }
    if password.len() > MAXIMUM_LENGTH {
        return Err(PasswordError::TooLong {
            maximum: MAXIMUM_LENGTH,
        });
    }

    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| PasswordError::Hashing)
}

/// Checks a password against a stored hash.
///
/// A wrong password and an unreadable hash are told apart by the return type rather than both
/// becoming "false", so a corrupted row is a visible fault rather than a player who can no longer
/// log in for no stated reason.
pub fn verify_password(password: &str, stored: &str) -> Result<bool, PasswordError> {
    if password.len() > MAXIMUM_LENGTH {
        return Ok(false);
    }

    let parsed = PasswordHash::new(stored).map_err(|_| PasswordError::Corrupt)?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_verifies_against_its_own_hash() {
        let stored = hash_password("correct horse battery").unwrap();
        assert!(verify_password("correct horse battery", &stored).unwrap());
    }

    #[test]
    fn a_wrong_password_does_not() {
        let stored = hash_password("correct horse battery").unwrap();
        assert!(!verify_password("correct horse batteries", &stored).unwrap());
        assert!(!verify_password("", &stored).unwrap());
        assert!(!verify_password("CORRECT HORSE BATTERY", &stored).unwrap());
    }

    #[test]
    fn the_same_password_hashes_differently_every_time() {
        // A per-password salt is what stops a leaked table revealing which accounts share a
        // password, and stops one precomputed table breaking all of them.
        let first = hash_password("correct horse battery").unwrap();
        let second = hash_password("correct horse battery").unwrap();

        assert_ne!(first, second);
        assert!(verify_password("correct horse battery", &first).unwrap());
        assert!(verify_password("correct horse battery", &second).unwrap());
    }

    #[test]
    fn the_hash_does_not_contain_the_password() {
        let stored = hash_password("hunter2hunter2").unwrap();
        assert!(!stored.contains("hunter2"));
        assert!(stored.starts_with("$argon2id$"));
    }

    #[test]
    fn short_passwords_are_refused_with_the_minimum_stated() {
        assert_eq!(
            hash_password("short"),
            Err(PasswordError::TooShort {
                minimum: MINIMUM_LENGTH
            })
        );
        // Counted in characters, so a short password of wide ones is still short.
        assert!(hash_password("日本語です").is_err());
        assert!(
            hash_password("日本語ですとても").is_ok(),
            "eight characters is enough"
        );
    }

    #[test]
    fn an_enormous_password_is_refused_rather_than_hashed() {
        // Argon2 is meant to be slow, which makes an unbounded input an easy way to exhaust an
        // unauthenticated endpoint.
        let huge = "a".repeat(MAXIMUM_LENGTH + 1);
        assert_eq!(
            hash_password(&huge),
            Err(PasswordError::TooLong {
                maximum: MAXIMUM_LENGTH
            })
        );

        // And verifying one is refused without doing the work.
        let stored = hash_password("correct horse battery").unwrap();
        assert!(!verify_password(&huge, &stored).unwrap());
    }

    #[test]
    fn a_corrupt_hash_is_a_fault_rather_than_a_wrong_password() {
        assert_eq!(
            verify_password("correct horse battery", "not a hash"),
            Err(PasswordError::Corrupt)
        );
        assert_eq!(verify_password("anything", ""), Err(PasswordError::Corrupt));
    }
}
