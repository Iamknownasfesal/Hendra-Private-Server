//! Exchanging items between two characters.
//!
//! # Why this is one transaction and not several moves
//!
//! A trade is not a sequence of moves that happen to run together. Halfway through a sequence one
//! player has given up their side and not received the other, and any failure there, whether a
//! full inventory, a dropped connection or a crash, leaves that asymmetry permanent. Worse, an
//! implementation that moves items one at a time can be interrupted by the same player trading the
//! same item elsewhere.
//!
//! So the whole exchange is one transaction: every slot on both sides is locked, every offered item
//! is confirmed still present, both inventories are checked for room, and only then does anything
//! move. It commits completely or it does not happen.
//!
//! # What actually prevents the duplication
//!
//! Two things do, and they guard different halves of the problem. This was established by removing
//! each in turn against the race test rather than assumed:
//!
//! - The delete is **conditional on the item still being there**. That alone stops one item
//!   reaching two people: the second transaction blocks on the row, re-evaluates after the first
//!   commits, deletes nothing, and the trade is refused.
//! - The **row locks** hold the room calculation still. Free slots are chosen before anything
//!   moves, and without the lock two trades can both decide the same slot is free, which fails as
//!   a primary-key violation rather than a clean refusal, and would fail silently if the
//!   destination were ever chosen less strictly.
//!
//! With either one present the race test passes. With both removed it produces two items where
//! there was one. Both are kept because they overlap on the case that matters most and cover
//! different ground either side of it.
//!
//! # Why the locks are ordered
//!
//! Two players trading with each other in both directions at once would each lock their own side
//! and wait for the other's. Locking in a fixed order, by character id, means one of them takes
//! every lock and the other waits.

use crate::{Result, Store, StoreError};

/// One side of a trade.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub character_id: i64,

    /// The slots being given up, and what the offering player believes is in them.
    ///
    /// The expected item matters: an offer is agreed at one moment and executed at another, and in
    /// between the player may have moved, dropped or traded the very thing they offered.
    pub items: Vec<(i16, uuid::Uuid)>,
}

impl Offer {
    pub fn new(character_id: i64, items: Vec<(i16, uuid::Uuid)>) -> Offer {
        Offer {
            character_id,
            items,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// What a completed trade moved.
#[derive(Debug, Clone, PartialEq)]
pub struct TradeOutcome {
    /// Where each item the first player received ended up.
    pub to_first: Vec<(i16, uuid::Uuid)>,
    pub to_second: Vec<(i16, uuid::Uuid)>,
}

impl Store {
    /// Exchanges the offered items between two characters, or does nothing at all.
    ///
    /// `first_slot` and `last_slot` bound the carried range items may land in. Worn slots are not
    /// a valid destination for something received, because nothing checks whether it fits.
    pub async fn trade(
        &self,
        first: &Offer,
        second: &Offer,
        first_slot: i16,
        last_slot: i16,
    ) -> Result<TradeOutcome> {
        if first.character_id == second.character_id {
            return Err(StoreError::Refused("a character cannot trade with itself"));
        }
        if first.is_empty() && second.is_empty() {
            return Err(StoreError::Refused("neither side offered anything"));
        }

        let mut transaction = self.pool().begin().await?;

        // Lock both inventories entirely, in a fixed order. Locking whole inventories rather than
        // individual slots, because a trade reads free space as well as specific slots,
        // and a slot that is free at the check and taken at the write would break the room
        // guarantee this is supposed to provide.
        let (low, high) = if first.character_id <= second.character_id {
            (first.character_id, second.character_id)
        } else {
            (second.character_id, first.character_id)
        };

        for character in [low, high] {
            sqlx::query("SELECT slot FROM inventory_slot WHERE character_id = $1 FOR UPDATE")
                .bind(character)
                .fetch_all(&mut *transaction)
                .await?;
        }

        let held_first = held(&mut transaction, first.character_id).await?;
        let held_second = held(&mut transaction, second.character_id).await?;

        // Confirm every offered item is still where it was offered from.
        confirm(&held_first, &first.items)?;
        confirm(&held_second, &second.items)?;

        // Room, counted after each side gives up what it offered.
        let to_first = place(
            &held_first,
            &first.items,
            second.items.len(),
            first_slot,
            last_slot,
        )?;
        let to_second = place(
            &held_second,
            &second.items,
            first.items.len(),
            first_slot,
            last_slot,
        )?;

        // Everything is checked. Now it happens.
        for (slot, item) in &first.items {
            remove(&mut transaction, first.character_id, *slot, *item).await?;
        }
        for (slot, item) in &second.items {
            remove(&mut transaction, second.character_id, *slot, *item).await?;
        }

        let received_by_first: Vec<(i16, uuid::Uuid)> = to_first
            .iter()
            .zip(second.items.iter())
            .map(|(slot, (_, item))| (*slot, *item))
            .collect();
        let received_by_second: Vec<(i16, uuid::Uuid)> = to_second
            .iter()
            .zip(first.items.iter())
            .map(|(slot, (_, item))| (*slot, *item))
            .collect();

        for (slot, item) in &received_by_first {
            insert(&mut transaction, first.character_id, *slot, *item).await?;
        }
        for (slot, item) in &received_by_second {
            insert(&mut transaction, second.character_id, *slot, *item).await?;
        }

        transaction.commit().await?;

        Ok(TradeOutcome {
            to_first: received_by_first,
            to_second: received_by_second,
        })
    }
}

/// Everything a character is holding.
async fn held(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    character_id: i64,
) -> Result<Vec<(i16, uuid::Uuid)>> {
    Ok(sqlx::query_as::<_, (i16, uuid::Uuid)>(
        "SELECT slot, item FROM inventory_slot WHERE character_id = $1",
    )
    .bind(character_id)
    .fetch_all(&mut **transaction)
    .await?)
}

/// Checks that every offered slot still holds what was offered.
fn confirm(held: &[(i16, uuid::Uuid)], offered: &[(i16, uuid::Uuid)]) -> Result<()> {
    for (slot, item) in offered {
        let actual = held
            .iter()
            .find(|(index, _)| index == slot)
            .map(|(_, item)| *item);

        if actual != Some(*item) {
            return Err(StoreError::Refused(
                "one of the offered items is no longer there",
            ));
        }
    }
    Ok(())
}

/// Chooses where incoming items will land, given what is leaving.
///
/// Returns one slot per incoming item, or refuses if there is not room for all of them. Deciding
/// this before anything moves is what makes the whole trade atomic rather than optimistic.
fn place(
    held: &[(i16, uuid::Uuid)],
    leaving: &[(i16, uuid::Uuid)],
    incoming: usize,
    first_slot: i16,
    last_slot: i16,
) -> Result<Vec<i16>> {
    let occupied: Vec<i16> = held
        .iter()
        .map(|(slot, _)| *slot)
        .filter(|slot| !leaving.iter().any(|(going, _)| going == slot))
        .collect();

    let free: Vec<i16> = (first_slot..=last_slot)
        .filter(|slot| !occupied.contains(slot))
        .take(incoming)
        .collect();

    if free.len() < incoming {
        return Err(StoreError::Refused(
            "there is not enough room for that trade",
        ));
    }
    Ok(free)
}

async fn remove(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    character_id: i64,
    slot: i16,
    item: uuid::Uuid,
) -> Result<()> {
    let removed = sqlx::query(
        "DELETE FROM inventory_slot WHERE character_id = $1 AND slot = $2 AND item = $3",
    )
    .bind(character_id)
    .bind(slot)
    .bind(item)
    .execute(&mut **transaction)
    .await?;

    if removed.rows_affected() == 0 {
        // This is not belt and braces: it is the check that stops one item reaching two people.
        // A concurrent trade that took the item first leaves this deleting nothing, and a trade
        // that silently moved nothing would be the duplication.
        return Err(StoreError::Refused("an offered item vanished mid-trade"));
    }
    Ok(())
}

async fn insert(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    character_id: i64,
    slot: i16,
    item: uuid::Uuid,
) -> Result<()> {
    sqlx::query("INSERT INTO inventory_slot (character_id, slot, item) VALUES ($1, $2, $3)")
        .bind(character_id)
        .bind(slot)
        .bind(item)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}
