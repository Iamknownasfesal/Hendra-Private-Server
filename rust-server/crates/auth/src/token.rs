//! Session tokens: what the app server mints and the game server checks.
//!
//! # Shape
//!
//! ```text
//!   <account>.<character>.<expiry>.<signature>
//! ```
//!
//! The claims are readable, which is deliberate — there is nothing secret in them, and a token
//! whose contents can be read is one whose problems can be diagnosed. What matters is that they
//! cannot be *changed*, which the signature provides.
//!
//! # Why not a library format
//!
//! A JWT would carry an algorithm field that the verifier is expected to read, which is the source
//! of the best-known authentication failure of the last decade — a token that names `none` as its
//! algorithm and is accepted. There is one algorithm here, the verifier does not ask the token what
//! it is, and the token has no way to suggest one.

use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

const ENCODING: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::URL_SAFE_NO_PAD;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("the token is malformed")]
    Malformed,

    #[error("the signature does not match")]
    BadSignature,

    #[error("the token expired")]
    Expired,

    #[error("a signing key must be at least {minimum} bytes")]
    WeakKey { minimum: usize },
}

/// The secret both servers share.
///
/// One key, held by the app server to sign and the game server to verify. Rotating it invalidates
/// every token in flight, which is the intended behaviour for a key that has leaked.
#[derive(Clone)]
pub struct TokenKey {
    secret: Vec<u8>,
}

/// The shortest key accepted.
///
/// Thirty-two bytes, matching the output of the hash it keys. A shorter key does not make the HMAC
/// break in an obvious way — it just quietly lowers the work needed to forge a token, which is
/// exactly the kind of weakness that stays unnoticed.
pub const MINIMUM_KEY_BYTES: usize = 32;

impl TokenKey {
    pub fn new(secret: impl Into<Vec<u8>>) -> Result<TokenKey, TokenError> {
        let secret = secret.into();
        if secret.len() < MINIMUM_KEY_BYTES {
            return Err(TokenError::WeakKey {
                minimum: MINIMUM_KEY_BYTES,
            });
        }
        Ok(TokenKey { secret })
    }

    fn sign(&self, message: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.secret).expect("HMAC takes any key length");
        mac.update(message.as_bytes());
        ENCODING.encode(mac.finalize().into_bytes())
    }
}

/// What a token says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Claims {
    pub account_id: i64,

    /// The character this token is for, or zero for "any".
    pub character_id: i64,

    /// When it stops being valid, in seconds since the epoch.
    pub expires_at: u64,
}

/// A minted token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token(pub String);

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Signs a set of claims.
pub fn mint(key: &TokenKey, claims: Claims) -> Token {
    let body = format!(
        "{}.{}.{}",
        claims.account_id, claims.character_id, claims.expires_at
    );
    let signature = key.sign(&body);
    Token(format!("{body}.{signature}"))
}

/// Checks a token and returns what it says.
///
/// `now` is passed in rather than read from the clock so that expiry is testable and so that the
/// caller decides what time means — a server whose clock has jumped should not silently start
/// accepting or rejecting everything.
pub fn verify(key: &TokenKey, token: &str, now: u64) -> Result<Claims, TokenError> {
    let mut parts = token.rsplitn(2, '.');
    let signature = parts.next().ok_or(TokenError::Malformed)?;
    let body = parts.next().ok_or(TokenError::Malformed)?;

    // The signature is checked before the claims are read. Parsing first would mean acting on
    // numbers nobody has vouched for yet, and would leak whether a forged token was well-formed.
    let expected = key.sign(body);
    if expected.as_bytes().ct_eq(signature.as_bytes()).unwrap_u8() != 1 {
        return Err(TokenError::BadSignature);
    }

    let mut fields = body.split('.');
    let account_id: i64 = fields
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or(TokenError::Malformed)?;
    let character_id: i64 = fields
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or(TokenError::Malformed)?;
    let expires_at: u64 = fields
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or(TokenError::Malformed)?;

    if fields.next().is_some() {
        return Err(TokenError::Malformed);
    }

    if now >= expires_at {
        return Err(TokenError::Expired);
    }

    Ok(Claims {
        account_id,
        character_id,
        expires_at,
    })
}

/// Seconds since the epoch, for callers that want the real clock.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// How long a freshly minted token lasts.
///
/// Long enough to start playing and to reconnect after a brief drop, short enough that one taken
/// from a log or a screenshot is worth little by the time it is found.
pub const LIFETIME_SECONDS: u64 = 15 * 60;

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> TokenKey {
        TokenKey::new(vec![7u8; 32]).unwrap()
    }

    fn claims(now: u64) -> Claims {
        Claims {
            account_id: 42,
            character_id: 7,
            expires_at: now + LIFETIME_SECONDS,
        }
    }

    #[test]
    fn a_token_verifies_and_says_what_it_was_given() {
        let key = key();
        let token = mint(&key, claims(1000));

        let read = verify(&key, &token.0, 1000).unwrap();
        assert_eq!(read.account_id, 42);
        assert_eq!(read.character_id, 7);
    }

    #[test]
    fn a_token_from_another_key_is_refused() {
        let token = mint(&key(), claims(1000));
        let other = TokenKey::new(vec![9u8; 32]).unwrap();

        assert_eq!(
            verify(&other, &token.0, 1000),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn an_altered_claim_is_refused() {
        // The whole point: the claims are readable and must not be writable.
        let key = key();
        let token = mint(&key, claims(1000));

        let tampered = token.0.replacen("42.", "43.", 1);
        assert_ne!(tampered, token.0, "the test should have changed something");
        assert_eq!(verify(&key, &tampered, 1000), Err(TokenError::BadSignature));
    }

    #[test]
    fn extending_the_expiry_is_refused() {
        let key = key();
        let token = mint(&key, claims(1000));

        // Take the signature and put it on a longer-lived body.
        let signature = token.0.rsplit('.').next().unwrap();
        let forged = format!("42.7.99999999.{signature}");

        assert_eq!(verify(&key, &forged, 1000), Err(TokenError::BadSignature));
    }

    #[test]
    fn an_expired_token_is_refused() {
        let key = key();
        let token = mint(&key, claims(1000));

        assert!(verify(&key, &token.0, 1000 + LIFETIME_SECONDS - 1).is_ok());
        assert_eq!(
            verify(&key, &token.0, 1000 + LIFETIME_SECONDS),
            Err(TokenError::Expired)
        );
        assert_eq!(verify(&key, &token.0, u64::MAX), Err(TokenError::Expired));
    }

    #[test]
    fn a_token_with_no_signature_at_all_is_refused() {
        let key = key();
        assert!(verify(&key, "42.7.99999999", 1000).is_err());
        assert!(verify(&key, "", 1000).is_err());
        assert!(verify(&key, ".", 1000).is_err());
    }

    #[test]
    fn extra_fields_are_refused_rather_than_ignored() {
        // A verifier that ignores what it does not recognise is one that can be given something it
        // will later be taught to read.
        let key = key();
        let body = "42.7.99999999.8";
        let signature = key.sign(body);

        assert_eq!(
            verify(&key, &format!("{body}.{signature}"), 1000),
            Err(TokenError::Malformed)
        );
    }

    #[test]
    fn a_short_key_is_refused_at_construction() {
        assert_eq!(
            TokenKey::new(vec![1u8; 16]).err(),
            Some(TokenError::WeakKey {
                minimum: MINIMUM_KEY_BYTES
            })
        );
        assert!(TokenKey::new(vec![1u8; 32]).is_ok());
    }

    #[test]
    fn junk_never_panics() {
        let key = key();
        let mut seed = 0xabcd_1234u64;

        for _ in 0..2_000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let length = (seed % 60) as usize;
            let junk: String = (0..length)
                .map(|i| {
                    let byte = (seed >> (i % 8 * 8)) as u8;
                    if byte.is_multiple_of(8) {
                        '.'
                    } else {
                        (b'a' + byte % 26) as char
                    }
                })
                .collect();

            let _ = verify(&key, &junk, 1000);
        }
    }
}
