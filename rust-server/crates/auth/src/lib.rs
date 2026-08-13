//! Passwords and session tokens.
//!
//! Two things happen here and they are deliberately separate. A password is checked once, by the
//! app server, over HTTPS. A token is checked on every game connection, by the game server, which
//! never sees the password at all.
//!
//! That separation is the reason the game socket can carry a token in the clear without it
//! mattering much: a stolen token expires, and it is not the thing the player reuses on other
//! sites.

pub mod password;
pub mod token;

pub use password::{PasswordError, hash_password, verify_password};
pub use token::{Claims, LIFETIME_SECONDS, Token, TokenError, TokenKey, mint, now, verify};
