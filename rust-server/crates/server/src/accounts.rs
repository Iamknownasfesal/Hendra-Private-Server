//! Turning a session token into a character.
//!
//! # What the token is
//!
//! The app server authenticates over HTTPS and mints a short-lived signed token. This checks the
//! signature and reads the account out of it. No password ever reaches the game socket, and this
//! process cannot mint a token — it holds the key only to verify.
//!
//! An expired or forged token is refused here rather than anywhere later, so nothing downstream
//! has to wonder whether the account id it is holding was vouched for.

use hendra_content::{Catalog, ObjectType};
use hendra_store::{Account, Character, Store, StoreError};

/// What a player arrives with when their account is new.
pub struct StartingKit<'a> {
    pub avatar: ObjectType,
    pub items: &'a [&'a str],
    pub max_hp: i32,
}

/// The same, owned, for holding across a session.
#[derive(Debug, Clone)]
pub struct StartingKitOwned {
    pub avatar: ObjectType,
    pub items: Vec<String>,
    pub max_hp: i32,
}

impl StartingKitOwned {
    pub fn borrowed(&self) -> (ObjectType, Vec<&str>, i32) {
        (
            self.avatar,
            self.items.iter().map(String::as_str).collect(),
            self.max_hp,
        )
    }
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
    kit: &StartingKit<'_>,
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

    let character = create_character(store, catalog, kit, account.id).await?;
    Ok(Session { account, character })
}

/// Makes a new character with its starting kit.
async fn create_character(
    store: &Store,
    catalog: &Catalog,
    kit: &StartingKit<'_>,
    account_id: i64,
) -> Result<Character, StoreError> {
    let character = store
        .create_character(account_id, kit.avatar.0 as i32, "Adventurer", kit.max_hp)
        .await?;

    // Placed by what each item is, not by the order they are listed. Filling slots 0 upwards puts
    // whatever comes third into the ring slot — the first attempt equipped two potions.
    let mut slots: Vec<(i16, i32)> = Vec::new();
    let mut next_carried = EQUIPPED_SLOTS;

    for name in kit.items {
        // Anything the catalog does not have is skipped rather than fatal: a half-converted content
        // directory should still let someone play.
        let Some(object_type) = catalog.type_of(name) else {
            tracing::warn!(
                item = name,
                "starting kit names an item the catalog does not have"
            );
            continue;
        };

        let worn = catalog
            .object(object_type)
            .and_then(|desc| desc.item.as_ref())
            .and_then(|item| equipment_slot_for(item.slot_type))
            .filter(|slot| !slots.iter().any(|(taken, _)| taken == slot));

        let slot = match worn {
            Some(slot) => slot,
            None => {
                let slot = next_carried;
                next_carried += 1;
                slot
            }
        };

        slots.push((slot, object_type.0 as i32));
    }

    if !slots.is_empty() {
        store.set_inventory(character.id, &slots).await?;
    }

    store.character(character.id).await
}

/// How many slots are worn rather than carried.
const EQUIPPED_SLOTS: i16 = 4;

/// Which worn slot an item's type belongs in, if any.
///
/// The game's four worn slots are weapon, ability, armour and ring, and each accepts a set of item
/// types — a wand and a bow are both weapons, a tome and a shield are both abilities. Anything not
/// listed is carried rather than worn.
fn equipment_slot_for(slot_type: i32) -> Option<i16> {
    Some(match slot_type {
        // Sword, dagger, bow, wand, staff, katana.
        1 | 2 | 3 | 8 | 17 | 24 => 0,
        // Tome, spell, cloak, quiver, helm, shield, seal, poison, skull, trap, orb, prism,
        // scepter, star.
        4 | 5 | 11 | 12 | 13 | 15 | 16 | 18 | 19 | 20 | 21 | 22 | 23 | 25 => 1,
        // Leather, plate, robe.
        6 | 7 | 14 => 2,
        // Ring.
        9 => 3,
        _ => return None,
    })
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
