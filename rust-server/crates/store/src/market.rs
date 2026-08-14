//! Items listed for sale.
//!
//! # Where a listed item lives
//!
//! In the listing. When something is listed it leaves the seller's inventory entirely, because an
//! item that existed in both places would be an item that could be sold and kept, and no amount of
//! care at the buying end can repair that.
//!
//! Cancelling gives it back and buying gives it to the buyer. Exactly one of the two happens, which
//! is what the status is for: a listing leaves `open` once, decided by a conditional update rather
//! than by reading the status first.
//!
//! # What makes a purchase safe
//!
//! Four things in one transaction, and the order matters:
//!
//! 1. Close the listing, conditional on it still being open. Two buyers race here and one loses.
//! 2. Take the money, conditional on the balance. A buyer who cannot pay stops here.
//! 3. Claim a slot for the item, the same way `give_item` does.
//! 4. Pay the seller.
//!
//! Closing first is what makes the race resolve at all: everything after it is safe precisely
//! because only one transaction can have got past it.

use crate::model::Currency;
use crate::{Result, Store, StoreError};

/// Something for sale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub id: i64,
    pub seller_id: i64,
    pub seller: String,
    pub item: uuid::Uuid,
    pub currency: Currency,
    pub price: i32,
}

impl Currency {
    fn code(self) -> i16 {
        match self {
            Currency::Gold => 0,
            Currency::Fame => 1,
            Currency::Tokens => 2,

            // The market does not take prestige, and a listing that claimed to would be a listing
            // nobody could pay for. Mapped to gold so a stored code is never ambiguous, and
            // refused before it reaches here.
            Currency::Prestige => 0,
        }
    }

    fn from_code(code: i16) -> Currency {
        match code {
            1 => Currency::Fame,
            2 => Currency::Tokens,
            _ => Currency::Gold,
        }
    }
}

impl Store {
    /// Lists an item, taking it out of the seller's inventory.
    pub async fn list_item(
        &self,
        seller_id: i64,
        character_id: i64,
        slot: i16,
        item: uuid::Uuid,
        currency: Currency,
        price: i32,
    ) -> Result<i64> {
        if price <= 0 {
            return Err(StoreError::Refused("a price must be more than nothing"));
        }

        let mut transaction = self.pool().begin().await?;

        // Conditional on the slot still holding what the seller thinks it does, which is the same
        // guard every other item move uses and for the same reason.
        let taken = sqlx::query(
            "DELETE FROM inventory_slot WHERE character_id = $1 AND slot = $2 AND item = $3",
        )
        .bind(character_id)
        .bind(slot)
        .bind(item)
        .execute(&mut *transaction)
        .await?;

        if taken.rows_affected() == 0 {
            return Err(StoreError::Refused(
                "that item is no longer where you left it",
            ));
        }

        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO listing (seller_id, item, currency, price) VALUES ($1, $2, $3, $4)
             RETURNING id",
        )
        .bind(seller_id)
        .bind(item)
        .bind(currency.code())
        .bind(price)
        .fetch_one(&mut *transaction)
        .await?;

        transaction.commit().await?;
        Ok(id)
    }

    /// Everything currently for sale.
    pub async fn listings(&self, limit: i64) -> Result<Vec<Listing>> {
        let rows = sqlx::query_as::<_, (i64, i64, String, uuid::Uuid, i16, i32)>(
            "SELECT listing.id, listing.seller_id, account.name,
                    listing.item, listing.currency, listing.price
             FROM listing
             JOIN account ON account.id = listing.seller_id
             WHERE listing.status = 'open'
             ORDER BY listing.listed_at
             LIMIT $1",
        )
        .bind(limit.clamp(1, MAX_LISTINGS_READ))
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, seller_id, seller, item, currency, price)| Listing {
                id,
                seller_id,
                seller,
                item,
                currency: Currency::from_code(currency),
                price,
            })
            .collect())
    }

    /// What one account has listed, oldest first.
    pub async fn listings_of(&self, seller_id: i64) -> Result<Vec<Listing>> {
        let rows = sqlx::query_as::<_, (i64, i64, String, uuid::Uuid, i16, i32)>(
            "SELECT listing.id, listing.seller_id, account.name,
                    listing.item, listing.currency, listing.price
             FROM listing
             JOIN account ON account.id = listing.seller_id
             WHERE listing.seller_id = $1 AND listing.status = 'open'
             ORDER BY listing.listed_at",
        )
        .bind(seller_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, seller_id, seller, item, currency, price)| Listing {
                id,
                seller_id,
                seller,
                item,
                currency: Currency::from_code(currency),
                price,
            })
            .collect())
    }

    /// Takes a listing back, returning the item.
    pub async fn cancel_listing(
        &self,
        seller_id: i64,
        listing_id: i64,
        character_id: i64,
        first_slot: i16,
        last_slot: i16,
    ) -> Result<i16> {
        let mut transaction = self.pool().begin().await?;

        // Closed first, conditional on it being open and the seller's. A cancel racing a sale
        // loses here, before anything has moved.
        let (item,): (uuid::Uuid,) = sqlx::query_as(
            "UPDATE listing SET status = 'cancelled', closed_at = now()
             WHERE id = $1 AND seller_id = $2 AND status = 'open'
             RETURNING item",
        )
        .bind(listing_id)
        .bind(seller_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::Refused("that listing is no longer open"))?;

        let slot = claim_slot(&mut transaction, character_id, first_slot, last_slot).await?;

        sqlx::query("UPDATE inventory_slot SET item = $3 WHERE character_id = $1 AND slot = $2")
            .bind(character_id)
            .bind(slot)
            .bind(item)
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        Ok(slot)
    }

    /// Buys a listing.
    pub async fn buy_listing(&self, purchase: MarketPurchase) -> Result<i16> {
        let MarketPurchase {
            buyer_id,
            character_id,
            listing_id,
            first_slot,
            last_slot,
        } = purchase;

        let mut transaction = self.pool().begin().await?;

        // Closed first. Everything after this is safe precisely because only one transaction can
        // have got past it.
        let (seller_id, item, currency, price): (i64, uuid::Uuid, i16, i32) = sqlx::query_as(
            "UPDATE listing SET status = 'sold', closed_at = now()
             WHERE id = $1 AND status = 'open'
             RETURNING seller_id, item, currency, price",
        )
        .bind(listing_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::Refused("that listing is no longer open"))?;

        if seller_id == buyer_id {
            return Err(StoreError::Refused("you are already holding that"));
        }

        let currency = Currency::from_code(currency);
        let column = currency.column_name();

        let paid = sqlx::query(&format!(
            "UPDATE account SET {column} = {column} - $2 WHERE id = $1 AND {column} >= $2"
        ))
        .bind(buyer_id)
        .bind(price)
        .execute(&mut *transaction)
        .await?;

        if paid.rows_affected() == 0 {
            return Err(StoreError::Refused("you cannot afford that"));
        }

        let slot = claim_slot(&mut transaction, character_id, first_slot, last_slot).await?;

        sqlx::query("UPDATE inventory_slot SET item = $3 WHERE character_id = $1 AND slot = $2")
            .bind(character_id)
            .bind(slot)
            .bind(item)
            .execute(&mut *transaction)
            .await?;

        // The seller is paid last, because everything before it can refuse and roll the whole
        // thing back. Paying first would need a refund path that can itself fail.
        sqlx::query(&format!(
            "UPDATE account SET {column} = {column} + $2 WHERE id = $1"
        ))
        .bind(seller_id)
        .bind(price - fee(price))
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;
        Ok(slot)
    }
}

/// What one market purchase is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketPurchase {
    pub buyer_id: i64,
    pub character_id: i64,
    pub listing_id: i64,
    pub first_slot: i16,
    pub last_slot: i16,
}

/// What the market keeps from a sale.
///
/// Taken from the seller rather than added to the price, so what a buyer is quoted is what a buyer
/// pays. A fee that appeared at the till would be a different price from the one advertised.
pub fn fee(price: i32) -> i32 {
    (price / 100 * FEE_PERCENT).max(if price > 0 { 1 } else { 0 })
}

/// The share of a sale the market keeps.
pub const FEE_PERCENT: i32 = 5;

/// The most listings one read may return.
pub const MAX_LISTINGS_READ: i64 = 200;

/// Finds and reserves a free carried slot, the way `give_item` does.
async fn claim_slot(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    character_id: i64,
    first_slot: i16,
    last_slot: i16,
) -> Result<i16> {
    sqlx::query(
        "SELECT slot FROM inventory_slot
         WHERE character_id = $1 AND slot BETWEEN $2 AND $3
         FOR UPDATE",
    )
    .bind(character_id)
    .bind(first_slot)
    .bind(last_slot)
    .fetch_all(&mut **transaction)
    .await?;

    let taken = sqlx::query_as::<_, (i16,)>(
        "SELECT slot FROM inventory_slot WHERE character_id = $1 AND slot BETWEEN $2 AND $3",
    )
    .bind(character_id)
    .bind(first_slot)
    .bind(last_slot)
    .fetch_all(&mut **transaction)
    .await?;

    let free = (first_slot..=last_slot)
        .find(|slot| !taken.iter().any(|(used,)| used == slot))
        .ok_or(StoreError::Refused("there is no room for that"))?;

    let written = sqlx::query(
        "INSERT INTO inventory_slot (character_id, slot, item) VALUES ($1, $2, NULL)
         ON CONFLICT (character_id, slot) DO NOTHING",
    )
    .bind(character_id)
    .bind(free)
    .execute(&mut **transaction)
    .await?;

    if written.rows_affected() == 0 {
        return Err(StoreError::Refused("there is no room for that"));
    }

    Ok(free)
}
