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

    /// How much likelier this account is to be given loot, from whatever boost it holds.
    ///
    /// One for everybody without one. Read once when the session starts rather than at every door:
    /// a boost lasts half an hour and a player walks through a dozen doors in one.
    pub loot_drop: f32,

    /// How many stars this account has earned, which is what everybody else sees beside the name.
    ///
    /// A record of the account rather than of the character being played, and nothing that happens
    /// inside a world moves it, so it is read once here.
    pub stars: u8,
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
        let loot_drop = loot_drop_for(store, account.id).await;
        let stars = stars_for(store, account.id).await;
        return Ok(Session {
            account,
            character,
            loot_drop,
            stars,
        });
    }

    let living = store.characters(account.id).await?;
    if let Some(first) = living.first()
        && let Ok(character) = store.character(first.id).await
    {
        let loot_drop = loot_drop_for(store, account.id).await;
        let stars = stars_for(store, account.id).await;
        return Ok(Session {
            account,
            character,
            loot_drop,
            stars,
        });
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
    let loot_drop = loot_drop_for(store, account.id).await;
    let stars = stars_for(store, account.id).await;
    Ok(Session {
        account,
        character,
        loot_drop,
        stars,
    })
}

/// How many stars an account has earned.
///
/// Every class contributes what its best fame is worth. An account whose progress cannot be read
/// shows none rather than refusing the login: a star is a decoration, and losing one for a moment
/// is better than not being let in.
async fn stars_for(store: &Store, account_id: i64) -> u8 {
    match hendra_characters::Unlocks::load(store, account_id).await {
        Ok(unlocks) => unlocks.stars(),
        Err(_) => 0,
    }
}

/// How much likelier an account is to be given loot right now.
///
/// One for everybody without a boost. The original reads whether the boost has time left and
/// multiplies by one and a half; the multiplier is stored with the boost here, so a content drop
/// can offer a different one without a code change.
async fn loot_drop_for(store: &Store, account_id: i64) -> f32 {
    store
        .boosts(account_id)
        .await
        .unwrap_or_default()
        .iter()
        .find(|boost| boost.kind == "loot_drop")
        .map(|boost| boost.multiplier)
        .unwrap_or(1.0)
}
