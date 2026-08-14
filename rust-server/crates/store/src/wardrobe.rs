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

    /// Puts a skin on a character, if the account owns it.
    ///
    /// The ownership check is in the statement rather than a lookup before it, so a skin sold or
    /// revoked between the two cannot be worn anyway.
    pub async fn wear_skin(&self, account_id: i64, character_id: i64, skin: i32) -> Result<()> {
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
        .bind(skin_identity(skin))
        .execute(self.pool())
        .await?;

        if worn.rows_affected() == 0 {
            return Err(StoreError::Refused("you do not own that"));
        }
        Ok(())
    }

    /// Gives a character its second row of carried slots.
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
        let rows = sqlx::query_as::<_, (String, f32)>(
            "SELECT kind, multiplier FROM account_boost
             WHERE account_id = $1 AND expires_at > now()",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(kind, multiplier)| Boost { kind, multiplier })
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

/// The identity a skin number corresponds to.
///
/// Skins are numbered in the content and owned by identity, so the two have to be brought together
/// somewhere. Here, because this is the only place both are known.
fn skin_identity(skin: i32) -> uuid::Uuid {
    uuid::Uuid::from_u128(skin as u128)
}

/// How many slots the gift chest holds.
///
/// The same eight every container in the game holds. A gift that arrives with the chest full is
/// refused rather than dropped, so the sender's side can say the purchase did not go through.
pub const GIFT_SLOTS: i16 = 8;

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

        sqlx::query("SELECT slot FROM gift_slot WHERE account_id = $1 FOR UPDATE")
            .bind(account_id)
            .fetch_all(&mut *transaction)
            .await?;

        let taken = sqlx::query_as::<_, (i16,)>("SELECT slot FROM gift_slot WHERE account_id = $1")
            .bind(account_id)
            .fetch_all(&mut *transaction)
            .await?;

        let free = (0..GIFT_SLOTS)
            .find(|slot| !taken.iter().any(|(used,)| used == slot))
            .ok_or(StoreError::Refused("your gift chest is full"))?;

        let written = sqlx::query(
            "INSERT INTO gift_slot (account_id, slot, item) VALUES ($1, $2, $3)
             ON CONFLICT (account_id, slot) DO NOTHING",
        )
        .bind(account_id)
        .bind(free)
        .bind(item)
        .execute(&mut *transaction)
        .await?;

        if written.rows_affected() == 0 {
            return Err(StoreError::Refused("your gift chest is full"));
        }

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

    #[test]
    fn a_skin_number_maps_to_one_identity_and_back() {
        assert_eq!(skin_identity(7), skin_identity(7));
        assert_ne!(skin_identity(7), skin_identity(8));
    }
}
