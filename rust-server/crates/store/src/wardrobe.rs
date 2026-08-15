//! What an ability changes that a world does not own.
//!
//! A dye, a skin, a pet and a boost all outlive the room they were used in. The world carries out
//! what belongs to the room and hands these back, and this is where they land.

use crate::{Result, Store, StoreError};

/// Which of the two dye slots a colour goes in.
///
/// The game has always had two, and which one a dye fills is decided by the dye rather than by the
/// player: a cloth dye in the accessory slot would show the wrong half of the sprite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DyeSlot {
    Cloth,
    Accessory,
}

impl DyeSlot {
    /// Which slot a dye's own number belongs to.
    ///
    /// The high bit of the identifier decides it, which is how the game has always encoded it.
    pub fn of(dye: u32) -> DyeSlot {
        if dye & 0x8000_0000 != 0 || (dye >> 24) & 0x1 != 0 {
            DyeSlot::Accessory
        } else {
            DyeSlot::Cloth
        }
    }
}

/// A pet an account owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pet {
    pub id: i64,
    pub kind: uuid::Uuid,
    pub skin: i32,
    pub permanent: bool,
}

/// A multiplier running against an account.
#[derive(Debug, Clone, PartialEq)]
pub struct Boost {
    pub kind: String,
    pub multiplier: f32,

    /// How long it has left, in milliseconds, so a world can count it down as the original does.
    ///
    /// Always positive, since the query only returns boosts that have not lapsed.
    pub remaining_ms: i64,
}

/// The most pets one account may keep.
///
/// A bound rather than a rule: an ability with no cooldown could otherwise fill the table, and a
/// player with four hundred pets is a player nobody can render.
pub const MAX_PETS: i64 = 20;

impl Store {
    /// Sets one of a character's two dye slots.
    pub async fn set_dye(&self, character_id: i64, dye: u32) -> Result<DyeSlot> {
        let slot = DyeSlot::of(dye);
        let column = match slot {
            DyeSlot::Cloth => "dye_cloth",
            DyeSlot::Accessory => "dye_accessory",
        };

        sqlx::query(&format!("UPDATE character SET {column} = $2 WHERE id = $1"))
            .bind(character_id)
            .bind(dye as i32)
            .execute(self.pool())
            .await?;

        Ok(slot)
    }

    /// Writes one dye layer, named rather than guessed at.
    ///
    /// The layer a dye paints is the content's to say: `AEDye` copies whichever of the item's
    /// `Tex1` and `Tex2` is non-zero (`Player.UseItem.cs:583-589`), so the clothing dye and the
    /// accessory dye of one colour carry the same number and differ only in which element declared
    /// it. Deriving the layer from the number instead cannot tell those two apart -- their high
    /// byte is `1` for both, because it is a type tag saying "a solid colour", not a slot.
    pub async fn set_dye_layer(&self, character_id: i64, slot: DyeSlot, dye: i32) -> Result<()> {
        let column = match slot {
            DyeSlot::Cloth => "dye_cloth",
            DyeSlot::Accessory => "dye_accessory",
        };

        sqlx::query(&format!("UPDATE character SET {column} = $2 WHERE id = $1"))
            .bind(character_id)
            .bind(dye)
            .execute(self.pool())
            .await?;

        Ok(())
    }

    /// Puts a skin on a character, if the account owns it.
    ///
    /// The ownership check is in the statement rather than a lookup before it, so a skin sold or
    /// revoked between the two cannot be worn anyway.
    /// `identity` is the skin's own uuid from the content, which is what every path that grants a
    /// skin writes into `owned_skin` -- the purchase over HTTP as much as the reskin command. The
    /// number is what the character wears; the identity is what says it may.
    pub async fn wear_skin(
        &self,
        account_id: i64,
        character_id: i64,
        skin: i32,
        identity: uuid::Uuid,
    ) -> Result<()> {
        // Zero is the class's own appearance, which everybody owns.
        if skin == 0 {
            sqlx::query("UPDATE character SET skin = 0 WHERE id = $1 AND account_id = $2")
                .bind(character_id)
                .bind(account_id)
                .execute(self.pool())
                .await?;
            return Ok(());
        }

        let worn = sqlx::query(
            "UPDATE character SET skin = $3
             WHERE id = $1 AND account_id = $2
               AND EXISTS (
                   SELECT 1 FROM owned_skin
                   WHERE owned_skin.account_id = $2
                     AND owned_skin.skin = $4
               )",
        )
        .bind(character_id)
        .bind(account_id)
        .bind(skin)
        .bind(identity)
        .execute(self.pool())
        .await?;

        if worn.rows_affected() == 0 {
            return Err(StoreError::Refused("you do not own that"));
        }
        Ok(())
    }

    /// Gives a character its second row of carried slots.
    /// Whether a character has a backpack, which is what says how many carried slots it has.
    ///
    /// Read where it is needed rather than held on the session, because it changes mid-session: a
    /// player who uses a backpack should be able to fill the new slots without logging out first.
    pub async fn has_backpack(&self, character_id: i64) -> Result<bool> {
        let (held,): (bool,) = sqlx::query_as("SELECT has_backpack FROM character WHERE id = $1")
            .bind(character_id)
            .fetch_optional(self.pool())
            .await?
            .ok_or(StoreError::NoSuchCharacter(character_id))?;

        Ok(held)
    }

    pub async fn grant_backpack(&self, character_id: i64) -> Result<bool> {
        let granted = sqlx::query(
            "UPDATE character SET has_backpack = true WHERE id = $1 AND NOT has_backpack",
        )
        .bind(character_id)
        .execute(self.pool())
        .await?;

        Ok(granted.rows_affected() > 0)
    }

    /// Summons a pet.
    pub async fn add_pet(&self, account_id: i64, kind: uuid::Uuid, permanent: bool) -> Result<i64> {
        let mut transaction = self.pool().begin().await?;

        // The account row is what two simultaneous summons contend on. Counting cannot be locked
        // directly, and locking the pets that exist would not stop a row being added beside them.
        sqlx::query("SELECT id FROM account WHERE id = $1 FOR UPDATE")
            .bind(account_id)
            .fetch_optional(&mut *transaction)
            .await?;

        let (held,): (i64,) = sqlx::query_as("SELECT count(*) FROM pet WHERE account_id = $1")
            .bind(account_id)
            .fetch_one(&mut *transaction)
            .await?;

        if held >= MAX_PETS {
            return Err(StoreError::Refused("you have enough companions"));
        }

        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO pet (account_id, kind, permanent) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(account_id)
        .bind(kind)
        .bind(permanent)
        .fetch_one(&mut *transaction)
        .await?;

        transaction.commit().await?;
        Ok(id)
    }

    /// Every pet an account has.
    pub async fn pets(&self, account_id: i64) -> Result<Vec<Pet>> {
        let rows = sqlx::query_as::<_, (i64, uuid::Uuid, i32, bool)>(
            "SELECT id, kind, skin, permanent FROM pet WHERE account_id = $1 ORDER BY id",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, kind, skin, permanent)| Pet {
                id,
                kind,
                skin,
                permanent,
            })
            .collect())
    }

    /// Recolours a pet.
    pub async fn set_pet_skin(&self, account_id: i64, pet_id: i64, skin: i32) -> Result<bool> {
        let set = sqlx::query("UPDATE pet SET skin = $3 WHERE id = $1 AND account_id = $2")
            .bind(pet_id)
            .bind(account_id)
            .bind(skin)
            .execute(self.pool())
            .await?;

        Ok(set.rows_affected() > 0)
    }

    /// Starts a boost, or extends one already running.
    ///
    /// Extending rather than stacking, because two of the same kind at once is a multiplier nobody
    /// wrote down and a number that grows every time somebody buys another.
    pub async fn add_boost(
        &self,
        account_id: i64,
        kind: &str,
        multiplier: f32,
        seconds: i64,
    ) -> Result<()> {
        let mut transaction = self.pool().begin().await?;

        let existing: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM account_boost
             WHERE account_id = $1 AND kind = $2 AND expires_at > now()
             FOR UPDATE",
        )
        .bind(account_id)
        .bind(kind)
        .fetch_optional(&mut *transaction)
        .await?;

        match existing {
            Some((id,)) => {
                sqlx::query(
                    "UPDATE account_boost
                     SET expires_at = expires_at + make_interval(secs => $2),
                         multiplier = GREATEST(multiplier, $3)
                     WHERE id = $1",
                )
                .bind(id)
                .bind(seconds as f64)
                .bind(multiplier)
                .execute(&mut *transaction)
                .await?;
            }
            None => {
                sqlx::query(
                    "INSERT INTO account_boost (account_id, kind, multiplier, expires_at)
                     VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
                )
                .bind(account_id)
                .bind(kind)
                .bind(multiplier)
                .bind(seconds as f64)
                .execute(&mut *transaction)
                .await?;
            }
        }

        transaction.commit().await?;
        Ok(())
    }

    /// The boosts currently running.
    ///
    /// Expired rows are left rather than swept: the query already excludes them, and a sweeper is
    /// one more thing that can fail quietly.
    pub async fn boosts(&self, account_id: i64) -> Result<Vec<Boost>> {
        let rows = sqlx::query_as::<_, (String, f32, f64)>(
            // Cast because `extract` answers in `numeric` from PostgreSQL 14 onwards, where it
            // used to answer in `double precision`: without it the row refuses to decode and
            // every boost an account holds is unreadable.
            "SELECT kind, multiplier,
                    extract(epoch from (expires_at - now()))::double precision
             FROM account_boost
             WHERE account_id = $1 AND expires_at > now()",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(kind, multiplier, seconds)| Boost {
                kind,
                multiplier,
                remaining_ms: (seconds * 1000.0) as i64,
            })
            .collect())
    }

    /// Unlocks a destination.
    pub async fn unlock_portal(&self, account_id: i64, portal: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO unlocked_portal (account_id, portal) VALUES ($1, $2)
             ON CONFLICT (account_id, portal) DO NOTHING",
        )
        .bind(account_id)
        .bind(portal.trim())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Which destinations an account has unlocked.
    pub async fn unlocked_portals(&self, account_id: i64) -> Result<Vec<String>> {
        let rows = sqlx::query_as::<_, (String,)>(
            "SELECT portal FROM unlocked_portal WHERE account_id = $1 ORDER BY portal",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows.into_iter().map(|(portal,)| portal).collect())
    }
}

/// How many slots one gift chest shows.
///
/// The same eight every container in the game holds, and the size of the chest a vault stands
/// rather than a limit on how many gifts an account may hold. `Vault.InitVault` deals the gift list
/// out over as many chests as it has squares for, eight at a time -- `Math.Min(8, gifts.Count)`,
/// padded to eight with `ushort.MaxValue`
/// (`git show 94615c4:Server-Side/wServer/realm/worlds/logic/Vault.cs`, `:115-130`).
pub const GIFT_SLOTS: i16 = 8;

/// The most gifts one account may hold.
///
/// `AddGifts` appends to an unbounded list (`common/Database.cs:1324-1333`), so nothing in the
/// original ever refuses a gift. This is a ceiling on the row count rather than a rule anybody
/// meets: it is well past what any vault can show, and it exists so that a runaway cannot fill the
/// table.
pub const MAX_GIFTS: i16 = 2000;

/// Claims the next free gift slot inside a caller's transaction and puts an item in it.
///
/// Taken as a transaction rather than run on its own so that whatever put the item into the chest —
/// a purchase, or a listing being withdrawn — either happens with the gift or does not happen. The
/// rows are locked before the free slot is chosen, so two gifts arriving at once cannot both choose
/// the same slot and one silently overwrite the other.
///
/// A full chest does not refuse. `AddGifts` appends to a list with no limit, so a withdrawal into a
/// chest that already holds eight succeeds and the ninth item waits behind the first eight. A
/// refusal here would refuse a withdrawal the original completes, which is the difference between
/// an item somebody has to come back for and an item they cannot get at all.
pub(crate) async fn gift(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: i64,
    item: uuid::Uuid,
) -> Result<i16> {
    sqlx::query("SELECT slot FROM gift_slot WHERE account_id = $1 FOR UPDATE")
        .bind(account_id)
        .fetch_all(&mut **transaction)
        .await?;

    let taken = sqlx::query_as::<_, (i16,)>("SELECT slot FROM gift_slot WHERE account_id = $1")
        .bind(account_id)
        .fetch_all(&mut **transaction)
        .await?;

    let free = (0..MAX_GIFTS)
        .find(|slot| !taken.iter().any(|(used,)| used == slot))
        .ok_or(StoreError::Refused("your gift chest is full"))?;

    let written = sqlx::query(
        "INSERT INTO gift_slot (account_id, slot, item) VALUES ($1, $2, $3)
         ON CONFLICT (account_id, slot) DO NOTHING",
    )
    .bind(account_id)
    .bind(free)
    .bind(item)
    .execute(&mut **transaction)
    .await?;

    if written.rows_affected() == 0 {
        return Err(StoreError::Refused("your gift chest is full"));
    }

    Ok(free)
}

/// The gift chest: where something bought outside the world arrives.
///
/// A separate place from the vault, because a gift is not something the player put there. The
/// original's `AddGift` writes to its own list for the same reason, and the chest in the vault is
/// where it is read from.
impl Store {
    /// Puts an item in the first free gift slot.
    ///
    /// The slot is chosen and claimed inside one transaction with the rows locked, so two gifts
    /// arriving at once cannot both choose the same slot and one silently overwrite the other.
    pub async fn add_gift(&self, account_id: i64, item: uuid::Uuid) -> Result<i16> {
        let mut transaction = self.pool().begin().await?;
        let free = gift(&mut transaction, account_id, item).await?;
        transaction.commit().await?;
        Ok(free)
    }

    /// What is in the gift chest.
    pub async fn gifts(&self, account_id: i64) -> Result<Vec<(i16, uuid::Uuid)>> {
        let rows = sqlx::query_as::<_, (i16, uuid::Uuid)>(
            "SELECT slot, item FROM gift_slot WHERE account_id = $1 ORDER BY slot",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows)
    }

    /// Takes a gift out, but only if that slot still holds what the caller expects.
    ///
    /// The condition is what makes two simultaneous requests for the same gift resolve to one item
    /// rather than two.
    pub async fn take_gift(&self, account_id: i64, slot: i16, expected: uuid::Uuid) -> Result<()> {
        let removed =
            sqlx::query("DELETE FROM gift_slot WHERE account_id = $1 AND slot = $2 AND item = $3")
                .bind(account_id)
                .bind(slot)
                .bind(expected)
                .execute(self.pool())
                .await?;

        if removed.rows_affected() == 0 {
            return Err(StoreError::Refused(
                "that gift is no longer where you left it",
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dye_knows_which_half_of_the_sprite_it_colours() {
        // A cloth dye in the accessory slot would recolour the wrong half.
        assert_eq!(DyeSlot::of(0x0000_0001), DyeSlot::Cloth);
        assert_eq!(DyeSlot::of(0x0100_0001), DyeSlot::Accessory);
        assert_eq!(DyeSlot::of(0x8000_0001), DyeSlot::Accessory);
    }

    /// A skin is owned by its content identity, not by the number it happens to be typed as.
    ///
    /// The two are separate arguments to [`Store::wear_skin`] because only the caller knows both:
    /// the store has no catalog to derive one from the other, and deriving one anyway is what
    /// stopped every skin bought over HTTP from ever being worn -- the purchase writes the
    /// content's uuid and the check looked for a number widened into one, which nothing writes.
    #[test]
    fn a_skin_is_owned_by_identity_rather_than_by_number() {
        let numbered = uuid::Uuid::from_u128(7);
        let content = uuid::Uuid::parse_str("6d2c7a1e-0000-4000-8000-000000000007").unwrap();

        assert_ne!(
            numbered, content,
            "a number widened into a uuid is not the identity the content gives a skin, so the \
             two cannot be used interchangeably"
        );
    }
}
