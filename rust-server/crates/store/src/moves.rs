//! Moving an item between durable slots.
//!
//! # Why this is not the in-memory move
//!
//! An in-memory move is safe because it reads, validates and writes with nothing in between: one
//! thread, one world, no window. Neither assumption holds here. Two connections can attempt the
//! same move at the same instant, and the read and the write are separated by a network round trip
//! wide enough to drive a duplication through.
//!
//! So the two rows are locked before either is read. Both attempts serialise, the second sees what
//! the first did, and the move that would have duplicated an item instead finds an empty slot and
//! refuses.
//!
//! # Why the rows are locked in a fixed order
//!
//! Two players swapping items with each other, simultaneously and in opposite directions, will each
//! lock one row and wait for the other forever. Ordering the locks by a rule both sides compute the
//! same way removes the cycle: one of them takes both locks and the other waits, briefly.

use crate::{Result, Store, StoreError};

/// Where a durable item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Location {
    /// A slot in a character's inventory.
    Inventory { character_id: i64, slot: i16 },

    /// A slot in an account's vault.
    Vault { account_id: i64, slot: i16 },

    /// A slot in an account's gift chest.
    ///
    /// A place of its own rather than part of the vault, because a gift is not something the player
    /// put there: mixing them would let a full vault refuse a purchase already paid for.
    Gift { account_id: i64, slot: i16 },
}

impl Location {
    /// A total order both parties to a swap compute identically, so locks are always taken the same
    /// way round and two opposing swaps cannot deadlock.
    fn lock_key(&self) -> (u8, i64, i16) {
        match self {
            Location::Inventory { character_id, slot } => (0, *character_id, *slot),
            Location::Vault { account_id, slot } => (1, *account_id, *slot),
            Location::Gift { account_id, slot } => (2, *account_id, *slot),
        }
    }
}

/// What a move did.
/// Where an item went when it entered a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub slot: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveOutcome {
    /// What ended up in the source slot. Zero means empty.
    pub source: Option<uuid::Uuid>,
    /// What ended up in the destination slot.
    pub destination: Option<uuid::Uuid>,
}

impl Store {
    /// Moves or swaps the contents of two durable slots.
    ///
    /// `expected` is what the caller believed was in the source. A move whose source no longer
    /// holds that is refused. This is what stops the same item being moved twice by two requests
    /// that both read it before either wrote.
    pub async fn move_item(
        &self,
        from: Location,
        to: Location,
        expected: Option<uuid::Uuid>,
    ) -> Result<MoveOutcome> {
        if from == to {
            return Err(StoreError::Refused("moving a slot onto itself"));
        }

        // A gift chest is one-way, as the original's `OneWayContainer` is: what is in it is what the
        // server put there. A chest that took deposits would be vault space nobody paid for, and
        // refusing here rather than in the session is what makes that true of every path into it.
        if matches!(to, Location::Gift { .. }) {
            return Err(StoreError::Refused("nothing goes into a gift chest"));
        }

        let mut transaction = self.pool().begin().await?;

        // Lock in a fixed order, then read. Reading before locking would put the window back.
        let (first, second) = if from.lock_key() <= to.lock_key() {
            (from, to)
        } else {
            (to, from)
        };
        let held_first = lock_and_read(&mut transaction, first).await?;
        let held_second = lock_and_read(&mut transaction, second).await?;

        let (source_item, destination_item) = if first == from {
            (held_first, held_second)
        } else {
            (held_second, held_first)
        };

        if source_item != expected {
            // Somebody else moved it first. Refusing is the whole point.
            return Err(StoreError::Refused(
                "that item is no longer where you left it",
            ));
        }

        write(&mut transaction, from, destination_item).await?;
        write(&mut transaction, to, source_item).await?;

        transaction.commit().await?;

        Ok(MoveOutcome {
            source: destination_item,
            destination: source_item,
        })
    }
}

/// What one purchase is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Purchase {
    pub account_id: i64,
    pub character_id: i64,
    pub item: uuid::Uuid,
    pub currency: crate::model::Currency,
    pub price: i32,

    /// The carried range the item may land in. Worn slots are not somewhere a purchase arrives.
    pub first_slot: i16,
    pub last_slot: i16,
}

impl Store {
    /// Buys an item, paying for it and receiving it in one transaction.
    ///
    /// Both halves or neither. Paying outside the transaction that grants the item is how a player
    /// loses the money and gets nothing, and granting outside it is how they get the item twice:
    /// the balance check and the slot claim have to agree at the same instant.
    pub async fn buy_item(&self, purchase: Purchase) -> Result<i16> {
        let Purchase {
            account_id,
            character_id,
            item,
            currency,
            price,
            first_slot,
            last_slot,
        } = purchase;

        let mut transaction = self.pool().begin().await?;

        // Paid first. A purchase that fails for want of room should leave the money alone, and
        // rolling back is what does that; the reverse order would need a refund path that can
        // itself fail.
        if price > 0 {
            let column = currency.column_name();
            let paid = sqlx::query(&format!(
                "UPDATE account SET {column} = {column} - $2 WHERE id = $1 AND {column} >= $2"
            ))
            .bind(account_id)
            .bind(price)
            .execute(&mut *transaction)
            .await?;

            if paid.rows_affected() == 0 {
                return Err(StoreError::Refused("you cannot afford that"));
            }
        }

        // The same claim `give_item` makes, in the same transaction as the payment: lock every
        // candidate that exists, find a free one, and insert conditionally so a slot taken between
        // the scan and the write refuses rather than overwrites.
        sqlx::query(
            "SELECT slot FROM inventory_slot
             WHERE character_id = $1 AND slot BETWEEN $2 AND $3
             FOR UPDATE",
        )
        .bind(character_id)
        .bind(first_slot)
        .bind(last_slot)
        .fetch_all(&mut *transaction)
        .await?;

        let taken = sqlx::query_as::<_, (i16,)>(
            "SELECT slot FROM inventory_slot WHERE character_id = $1 AND slot BETWEEN $2 AND $3",
        )
        .bind(character_id)
        .bind(first_slot)
        .bind(last_slot)
        .fetch_all(&mut *transaction)
        .await?;

        let free = (first_slot..=last_slot)
            .find(|slot| !taken.iter().any(|(used,)| used == slot))
            .ok_or(StoreError::Refused("there is no room for that"))?;

        let written = sqlx::query(
            "INSERT INTO inventory_slot (character_id, slot, item) VALUES ($1, $2, $3)
             ON CONFLICT (character_id, slot) DO NOTHING",
        )
        .bind(character_id)
        .bind(free)
        .bind(item)
        .execute(&mut *transaction)
        .await?;

        if written.rows_affected() == 0 {
            return Err(StoreError::Refused("there is no room for that"));
        }

        transaction.commit().await?;
        Ok(free)
    }

    /// Puts an item into the first free slot of a character's inventory.
    ///
    /// The search and the write happen inside one transaction with the rows locked, so two
    /// simultaneous pickups cannot both choose the same slot and one of them silently overwrite the
    /// other's item.
    pub async fn give_item(
        &self,
        character_id: i64,
        item: uuid::Uuid,
        first_slot: i16,
        last_slot: i16,
    ) -> Result<Placed> {
        let mut transaction = self.pool().begin().await?;

        // Lock every candidate row that exists, so a concurrent pickup waits rather than racing.
        // Rows that do not exist cannot be locked, which is why the insert below is conditional on
        // the slot still being free.
        sqlx::query(
            "SELECT slot FROM inventory_slot
             WHERE character_id = $1 AND slot BETWEEN $2 AND $3
             FOR UPDATE",
        )
        .bind(character_id)
        .bind(first_slot)
        .bind(last_slot)
        .fetch_all(&mut *transaction)
        .await?;

        let taken = sqlx::query_as::<_, (i16,)>(
            "SELECT slot FROM inventory_slot WHERE character_id = $1 AND slot BETWEEN $2 AND $3",
        )
        .bind(character_id)
        .bind(first_slot)
        .bind(last_slot)
        .fetch_all(&mut *transaction)
        .await?;

        let free = (first_slot..=last_slot)
            .find(|slot| !taken.iter().any(|(used,)| used == slot))
            .ok_or(StoreError::Refused("there is no room for that"))?;

        // `ON CONFLICT DO NOTHING` with a checked row count: if another transaction inserted the
        // same slot between the scan and here, this affects nothing and the pickup is refused
        // rather than overwriting what they put there.
        let written = sqlx::query(
            "INSERT INTO inventory_slot (character_id, slot, item) VALUES ($1, $2, $3)
             ON CONFLICT (character_id, slot) DO NOTHING",
        )
        .bind(character_id)
        .bind(free)
        .bind(item)
        .execute(&mut *transaction)
        .await?;

        if written.rows_affected() == 0 {
            return Err(StoreError::Refused("there is no room for that"));
        }

        transaction.commit().await?;
        Ok(Placed { slot: free })
    }

    /// Removes an item from a slot, but only if that slot still holds what the caller expects.
    ///
    /// The condition is what makes two simultaneous requests to drop the same item
    /// resolve to one drop rather than two.
    pub async fn take_item(
        &self,
        character_id: i64,
        slot: i16,
        expected: uuid::Uuid,
    ) -> Result<()> {
        let removed = sqlx::query(
            "DELETE FROM inventory_slot
             WHERE character_id = $1 AND slot = $2 AND item = $3",
        )
        .bind(character_id)
        .bind(slot)
        .bind(expected)
        .execute(self.pool())
        .await?;

        if removed.rows_affected() == 0 {
            return Err(StoreError::Refused(
                "that item is no longer where you left it",
            ));
        }
        Ok(())
    }
}

/// Locks a slot's row and reads it. Zero means the slot is empty.
///
/// An absent row is an empty slot, and an empty slot still has to be locked or two moves could both
/// decide to fill it. `INSERT … ON CONFLICT DO UPDATE … RETURNING` creates the row if it is missing
/// and locks it either way, which a plain `SELECT … FOR UPDATE` cannot do for a row that is not
/// there yet.
async fn lock_and_read(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    at: Location,
) -> Result<Option<uuid::Uuid>> {
    let (item,): (Option<uuid::Uuid>,) = match at {
        Location::Inventory { character_id, slot } => {
            sqlx::query_as(
                "INSERT INTO inventory_slot (character_id, slot, item) VALUES ($1, $2, NULL)
                 ON CONFLICT (character_id, slot) DO UPDATE SET item = inventory_slot.item
                 RETURNING item",
            )
            .bind(character_id)
            .bind(slot)
            .fetch_one(&mut **transaction)
            .await?
        }
        Location::Vault { account_id, slot } => {
            sqlx::query_as(
                "INSERT INTO vault_slot (account_id, slot, item) VALUES ($1, $2, NULL)
                 ON CONFLICT (account_id, slot) DO UPDATE SET item = vault_slot.item
                 RETURNING item",
            )
            .bind(account_id)
            .bind(slot)
            .fetch_one(&mut **transaction)
            .await?
        }
        Location::Gift { account_id, slot } => {
            // The gift table takes no nulls, so an empty slot is an absent row and locking it means
            // locking the account rather than a row that is not there. Two takes of the same gift
            // still serialise, which is what stops one gift becoming two.
            sqlx::query("SELECT id FROM account WHERE id = $1 FOR UPDATE")
                .bind(account_id)
                .execute(&mut **transaction)
                .await?;

            sqlx::query_as("SELECT item FROM gift_slot WHERE account_id = $1 AND slot = $2")
                .bind(account_id)
                .bind(slot)
                .fetch_optional(&mut **transaction)
                .await?
                .unwrap_or((None,))
        }
    };

    Ok(item)
}

/// Writes a slot, removing the row when it becomes empty so the tables stay sparse.
async fn write(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    at: Location,
    item: Option<uuid::Uuid>,
) -> Result<()> {
    match (at, item) {
        (Location::Inventory { character_id, slot }, None) => {
            sqlx::query("DELETE FROM inventory_slot WHERE character_id = $1 AND slot = $2")
                .bind(character_id)
                .bind(slot)
                .execute(&mut **transaction)
                .await?;
        }
        (Location::Inventory { character_id, slot }, Some(item)) => {
            sqlx::query(
                "UPDATE inventory_slot SET item = $3 WHERE character_id = $1 AND slot = $2",
            )
            .bind(character_id)
            .bind(slot)
            .bind(item)
            .execute(&mut **transaction)
            .await?;
        }
        (Location::Vault { account_id, slot }, None) => {
            sqlx::query("DELETE FROM vault_slot WHERE account_id = $1 AND slot = $2")
                .bind(account_id)
                .bind(slot)
                .execute(&mut **transaction)
                .await?;
        }
        (Location::Vault { account_id, slot }, Some(item)) => {
            sqlx::query("UPDATE vault_slot SET item = $3 WHERE account_id = $1 AND slot = $2")
                .bind(account_id)
                .bind(slot)
                .bind(item)
                .execute(&mut **transaction)
                .await?;
        }
        (Location::Gift { account_id, slot }, None) => {
            sqlx::query("DELETE FROM gift_slot WHERE account_id = $1 AND slot = $2")
                .bind(account_id)
                .bind(slot)
                .execute(&mut **transaction)
                .await?;
        }
        (Location::Gift { account_id, slot }, Some(item)) => {
            sqlx::query(
                "INSERT INTO gift_slot (account_id, slot, item) VALUES ($1, $2, $3)
                 ON CONFLICT (account_id, slot) DO UPDATE SET item = $3",
            )
            .bind(account_id)
            .bind(slot)
            .bind(item)
            .execute(&mut **transaction)
            .await?;
        }
    }

    Ok(())
}
