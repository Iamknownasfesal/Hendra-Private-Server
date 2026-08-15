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

    /// When this account's experience boost runs out, if one is running.
    ///
    /// The original keeps `XPBoostTime` on the character and counts it down as the world ticks
    /// (`Player.cs:593-602`); ours is durable and account-wide, so the session holds the moment it
    /// ends and every world the player walks into is handed what is left of it. A deadline rather
    /// than a duration, or walking through a door would restart a boost that had already lapsed.
    pub experience_boost_ends: Option<std::time::Instant>,

    /// When the loot-drop and loot-tier boosts run out, if either is running.
    ///
    /// `LDBoostTime` and `LTBoostTime` (`Player.cs:357-358`). Deadlines rather than durations for
    /// the same reason the experience one above is: walking through a door must not restart a boost
    /// that had already lapsed. [`Self::loot_drop`] is the multiplier the first of them applies and
    /// says nothing about how long it has left, which is why the clock is kept beside it.
    pub loot_drop_boost_ends: Option<std::time::Instant>,
    pub loot_tier_boost_ends: Option<std::time::Instant>,

    /// How many stars this account has earned, which is what everybody else sees beside the name.
    ///
    /// A record of the account rather than of the character being played, and nothing that happens
    /// inside a world moves it, so it is read once here.
    pub stars: u8,

    /// The prestige the account holds, which the prestige shop charges against.
    ///
    /// Read once here rather than per world, the way the stars are: only `/prestige` moves it, and
    /// that ends the character being played.
    pub prestige: i32,

    /// The lock this session plays the account under, once the handshake has taken one.
    ///
    /// Every durable write to the character quotes it, so a session whose account has been taken
    /// over while it was working out what to write stops writing rather than laying an older
    /// snapshot over whatever the session that holds the account has since saved. The original
    /// keeps it in the same place, on the account (`DbAccount.LockToken`), and hands it to
    /// `Database.SaveCharacter` as the condition on the transaction that writes the character
    /// (`common/Database.cs:1058-1069`).
    ///
    /// `None` until the lock is taken, which is the original's null token: a session that has not
    /// claimed the account has nothing to write under.
    pub lock: Option<hendra_store::AccountLock>,
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
        let boosts = boosts_for(store, account.id).await;
        let stars = stars_for(store, account.id).await;
        let prestige = prestige_for(store, account.id).await;
        return Ok(Session {
            account,
            character,
            loot_drop: boosts.loot_drop,
            experience_boost_ends: boosts.experience_ends,
            loot_drop_boost_ends: boosts.loot_drop_ends,
            loot_tier_boost_ends: boosts.loot_tier_ends,
            stars,
            prestige,
            lock: None,
        });
    }

    let living = store.characters(account.id).await?;
    if let Some(first) = living.first()
        && let Ok(character) = store.character(first.id).await
    {
        let boosts = boosts_for(store, account.id).await;
        let stars = stars_for(store, account.id).await;
        let prestige = prestige_for(store, account.id).await;
        return Ok(Session {
            account,
            character,
            loot_drop: boosts.loot_drop,
            experience_boost_ends: boosts.experience_ends,
            loot_drop_boost_ends: boosts.loot_drop_ends,
            loot_tier_boost_ends: boosts.loot_tier_ends,
            stars,
            prestige,
            lock: None,
        });
    }

    // Nobody with a living character reaches here, so this is a first arrival: they are given one
    // of the class the content opens with.
    let character = hendra_characters::create(store, catalog, account.id, kit.default_class)
        .await
        .map_err(|err| match err {
            hendra_characters::CreateError::Store(err) => LoginError::Store(err),
            other => LoginError::NoCharacter(other.to_string()),
        })?;
    let boosts = boosts_for(store, account.id).await;
    let stars = stars_for(store, account.id).await;
    let prestige = prestige_for(store, account.id).await;
    Ok(Session {
        account,
        character,
        loot_drop: boosts.loot_drop,
        experience_boost_ends: boosts.experience_ends,
        loot_drop_boost_ends: boosts.loot_drop_ends,
        loot_tier_boost_ends: boosts.loot_tier_ends,
        stars,
        prestige,
        lock: None,
    })
}

/// The prestige an account holds. Nothing to show rather than a refused login when it cannot be
/// read, for the same reason the stars fall back to none.
async fn prestige_for(store: &Store, account_id: i64) -> i32 {
    store
        .prestige_of(account_id)
        .await
        .map(|(held, _)| held)
        .unwrap_or(0)
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

/// The fame the next class quest asks for, given the best fame a class has reached.
///
/// `Player.GetFameGoal` (`Player.Leveling.cs:19-27`), thresholds and all. Zero once two thousand is
/// past, which the client reads as "no more to earn" rather than as an empty bar.
pub fn fame_goal(fame: i32) -> i32 {
    match fame {
        f if f >= 2000 => 0,
        f if f >= 800 => 2000,
        f if f >= 400 => 800,
        f if f >= 150 => 400,
        f if f >= 20 => 150,
        _ => 20,
    }
}

/// What an account's running boosts are worth: its loot multiplier, and when its experience boost
/// ends.
///
/// Read together because both come from the same table and a login should ask it once.
///
/// The loot figure is one for everybody without a boost. The original reads whether the boost has
/// time left and multiplies by one and a half; the multiplier is stored with the boost here, so a
/// content drop can offer a different one without a code change.
///
/// The experience figure is a clock rather than a multiplier because `DamageCounter` only asks
/// whether the clock is running (`DamageCounter.cs:98`) and always doubles.
async fn boosts_for(store: &Store, account_id: i64) -> Boosts {
    let boosts = store.boosts(account_id).await.unwrap_or_default();

    let loot_drop = boosts
        .iter()
        .find(|boost| boost.kind == "loot_drop")
        .map(|boost| boost.multiplier)
        .unwrap_or(1.0);

    // Every clock is read the same way: the row's remaining time turned into a deadline. A boost
    // the table has no row for has no clock, which is what the client draws as no timer at all.
    let deadline = |kind: &str| {
        boosts
            .iter()
            .find(|boost| boost.kind == kind)
            .and_then(|boost| u64::try_from(boost.remaining_ms).ok())
            .map(|left| std::time::Instant::now() + std::time::Duration::from_millis(left))
    };

    Boosts {
        loot_drop,
        experience_ends: deadline("experience"),
        loot_drop_ends: deadline("loot_drop"),
        loot_tier_ends: deadline("loot_tier"),
    }
}

/// What one account's running boosts amount to, read together.
struct Boosts {
    loot_drop: f32,
    experience_ends: Option<std::time::Instant>,
    loot_drop_ends: Option<std::time::Instant>,
    loot_tier_ends: Option<std::time::Instant>,
}
