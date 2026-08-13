//! Addresses, and the one-time tokens sent to them.
//!
//! # Why tokens are stored hashed
//!
//! For the same reason a password is. A leaked table must not be a set of working links, and the
//! only place the token itself exists is the message that was sent.
//!
//! # Why a token names its purpose
//!
//! A token that verifies an address and a token that resets a password reach the same inbox, and
//! one that worked for both would let a verification link change a password. The purpose is checked
//! when it is spent, not only when it is made.

use crate::{Result, Store, StoreError};

/// What a token is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Confirms an address belongs to whoever gave it.
    Verify,

    /// Allows one password change without the old one.
    Reset,
}

impl Purpose {
    fn name(self) -> &'static str {
        match self {
            Purpose::Verify => "verify",
            Purpose::Reset => "reset",
        }
    }
}

/// How long a token lasts.
///
/// Long enough to find the message, short enough that one left in an old inbox is worth little.
pub const LIFETIME_MINUTES: i64 = 60;

/// The longest address accepted.
pub const MAX_EMAIL_LENGTH: usize = 254;

/// What a token looks like before it is hashed.
///
/// Returned once, to be sent, and never stored. Long enough that guessing is not a strategy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token(pub String);

impl Store {
    /// Records an address against an account, unverified.
    pub async fn set_email(&self, account_id: i64, email: &str) -> Result<()> {
        let email = email.trim();
        if !looks_like_an_address(email) {
            return Err(StoreError::Refused("that does not look like an address"));
        }

        let set =
            sqlx::query("UPDATE account SET email = $2, email_verified = false WHERE id = $1")
                .bind(account_id)
                .bind(email)
                .execute(self.pool())
                .await;

        match set {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
                Err(StoreError::Refused("that address is already in use"))
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Makes a token, returning it once so it can be sent.
    ///
    /// Any earlier token for the same purpose is dropped, so a second request invalidates the
    /// first: two working links is one more than anybody needs and one more to be stolen.
    pub async fn issue_email_token(&self, account_id: i64, purpose: Purpose) -> Result<Token> {
        let token = Token(uuid::Uuid::new_v4().simple().to_string());
        let hash = hash_token(&token.0);

        let mut transaction = self.pool().begin().await?;

        sqlx::query("DELETE FROM email_token WHERE account_id = $1 AND purpose = $2")
            .bind(account_id)
            .bind(purpose.name())
            .execute(&mut *transaction)
            .await?;

        sqlx::query(
            "INSERT INTO email_token (token_hash, account_id, purpose, expires_at)
             VALUES ($1, $2, $3, now() + make_interval(mins => $4))",
        )
        .bind(&hash)
        .bind(account_id)
        .bind(purpose.name())
        .bind(LIFETIME_MINUTES as i32)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;
        Ok(token)
    }

    /// Spends a token, returning whose it was.
    ///
    /// Marked used in the same statement that reads it, so a token that arrives twice is spent
    /// once however close together the two arrive.
    pub async fn spend_email_token(&self, token: &str, purpose: Purpose) -> Result<i64> {
        let hash = hash_token(token.trim());

        let spent: Option<(i64,)> = sqlx::query_as(
            "UPDATE email_token SET used_at = now()
             WHERE token_hash = $1
               AND purpose = $2
               AND used_at IS NULL
               AND expires_at > now()
             RETURNING account_id",
        )
        .bind(&hash)
        .bind(purpose.name())
        .fetch_optional(self.pool())
        .await?;

        spent
            .map(|(account_id,)| account_id)
            .ok_or(StoreError::Refused("that link is no longer valid"))
    }

    /// Marks an address confirmed.
    pub async fn mark_email_verified(&self, account_id: i64) -> Result<()> {
        sqlx::query("UPDATE account SET email_verified = true WHERE id = $1")
            .bind(account_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Finds an account by address, for a reset request.
    pub async fn account_by_email(&self, email: &str) -> Result<i64> {
        let found: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM account WHERE lower(email) = lower($1)")
                .bind(email.trim())
                .fetch_optional(self.pool())
                .await?;

        found
            .map(|(id,)| id)
            .ok_or(StoreError::NoSuchAccount(email.to_string()))
    }
}

/// Whether something is plausibly an address.
///
/// Deliberately shallow. A full check is impossible without sending to it, which is what the
/// verification token is for; anything stricter here would reject real addresses to no benefit.
fn looks_like_an_address(email: &str) -> bool {
    if email.is_empty() || email.len() > MAX_EMAIL_LENGTH || email.contains(char::is_whitespace) {
        return false;
    }

    match email.split_once('@') {
        Some((user, host)) => !user.is_empty() && host.contains('.') && !host.starts_with('.'),
        None => false,
    }
}

/// Hashes a token for storage.
///
/// A plain digest rather than a password hash: a token is long and random, so there is nothing to
/// guess and no reason to make checking it slow.
fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn something_without_an_at_or_a_dot_is_not_an_address() {
        assert!(looks_like_an_address("someone@example.com"));
        assert!(looks_like_an_address("a.b+c@sub.example.co.uk"));

        assert!(!looks_like_an_address(""));
        assert!(!looks_like_an_address("someone"));
        assert!(!looks_like_an_address("someone@localhost"));
        assert!(!looks_like_an_address("@example.com"));
        assert!(!looks_like_an_address("someone@.com"));
        assert!(!looks_like_an_address("some one@example.com"));
    }

    #[test]
    fn an_enormous_address_is_refused_rather_than_stored() {
        let huge = format!("{}@example.com", "a".repeat(MAX_EMAIL_LENGTH));
        assert!(!looks_like_an_address(&huge));
    }

    #[test]
    fn the_stored_hash_is_not_the_token() {
        // A leaked table must not be a set of working links.
        let token = "0123456789abcdef";
        let hash = hash_token(token);

        assert_ne!(hash, token);
        assert!(!hash.contains(token));
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn the_same_token_hashes_the_same_way_every_time() {
        assert_eq!(hash_token("abc"), hash_token("abc"));
        assert_ne!(hash_token("abc"), hash_token("abd"));
    }
}
