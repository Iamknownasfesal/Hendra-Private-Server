//! Turning a session token into a character.
//!
//! # What the token is
//!
//! The app server authenticates over HTTPS and mints a short-lived signed token. This checks the
//! signature and reads the account out of it. No password ever reaches the game socket, and this
//! process cannot mint a token; it holds the key only to verify.
//!
//! An expired or forged token is refused here rather than anywhere later, so nothing downstream
//! has to wonder whether the account id it is holding was vouched for.

use hendra_content::{Catalog, ObjectType};
use hendra_store::{Account, Character, Store, StoreError};

/// What every class is given on top of the gear its own slots decide.
///
/// Only the things that are the same whatever you are playing. A wand and a robe are not on this
/// list any more: they come from the class, so a warrior no longer starts holding a wand it cannot
/// use.
#[derive(Debug, Clone)]
pub struct StartingKit {
    /// The class a new character is made as when nothing names one.
    pub default_class: ObjectType,

    pub common: hendra_characters::CommonItems,
}

/// Who is playing, and as what.
pub struct Session {
    pub account: Account,
    pub character: Character,
}

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("the token was empty")]
    NoToken,

    #[error("the token is not valid: {0}")]
    BadToken(hendra_auth::TokenError),

    #[error("this account is banned")]
    Banned,

    #[error("could not make a character: {0}")]
    NoCharacter(String),

    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Checks a token and finds the character to play.
///
/// `character_id` picks an existing character; zero means "any living one, or a new one". A token
/// naming a character takes precedence, because that is the one the player chose on the way in.
pub async fn log_in(
    store: &Store,
    catalog: &Catalog,
    kit: &StartingKit,
    key: &hendra_auth::TokenKey,
    token: &str,
    character_id: i64,
) -> Result<Session, LoginError> {
    if token.trim().is_empty() {
        return Err(LoginError::NoToken);
    }

    let claims =
        hendra_auth::verify(key, token.trim(), hendra_auth::now()).map_err(LoginError::BadToken)?;

    let account = store.account(claims.account_id).await?;
    if account.banned {
        return Err(LoginError::Banned);
    }

    // The token may name a character, which beats whatever the client asked for separately: one is
    // signed and the other is not.
    let character_id = if claims.character_id > 0 {
        claims.character_id
    } else {
        character_id
    };

    // A named character, if it belongs to this account. A client naming someone else's is answered
    // as though it named nothing, rather than told which of the two it got wrong.
    if character_id > 0
        && let Ok(character) = store.character(character_id).await
        && character.account_id == account.id
        && character.alive
    {
        return Ok(Session { account, character });
    }

    let living = store.characters(account.id).await?;
    if let Some(first) = living.first()
        && let Ok(character) = store.character(first.id).await
    {
        return Ok(Session { account, character });
    }

    // Nobody with a living character reaches here, so this is a first arrival: they are given one
    // of the class the content opens with.
    let character = hendra_characters::create(
        store,
        catalog,
        &kit.common,
        account.id,
        kit.default_class,
        "Adventurer",
    )
    .await
    .map_err(|err| match err {
        hendra_characters::CreateError::Store(err) => LoginError::Store(err),
        other => LoginError::NoCharacter(other.to_string()),
    })?;
    Ok(Session { account, character })
}

/// Writes back what a character became.
pub async fn save(
    store: &Store,
    character: &Character,
    hp: i32,
    mp: i32,
) -> Result<(), StoreError> {
    store
        .save_character(
            character.id,
            hp,
            mp,
            character.level,
            character.experience,
            character.fame,
        )
        .await
}
