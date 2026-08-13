//! Turning a session token into a character.
//!
//! # What the token is, for now
//!
//! The design is that the app server authenticates over HTTPS and mints a short-lived token, and
//! the game socket only ever carries that token. The app server does not exist yet, so this treats
//! the token as an account name and creates the account if it is new.
//!
//! That is deliberately obvious rather than quietly convenient: it is one function, it is named,
//! and it is the only thing that has to change when real authentication lands. A server run this
//! way is a development server, and it says so at startup.

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

    #[error("this account is banned")]
    Banned,

    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Finds or creates the account a token names, and the character to play.
///
/// `character_id` picks an existing character; zero means "any living one, or a new one".
pub async fn log_in(
    store: &Store,
    catalog: &Catalog,
    kit: &StartingKit<'_>,
    token: &str,
    character_id: i64,
) -> Result<Session, LoginError> {
    if token.trim().is_empty() {
        return Err(LoginError::NoToken);
    }

    let account = match store.account_by_name(token).await {
        Ok(account) => account,
        Err(StoreError::NoSuchAccount(_)) => store.create_account(token).await?,
        Err(err) => return Err(err.into()),
    };

    if account.banned {
        return Err(LoginError::Banned);
    }

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
            tracing::warn!(item = name, "starting kit names an item the catalog does not have");
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
pub async fn save(store: &Store, character: &Character, hp: i32, mp: i32) -> Result<(), StoreError> {
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
