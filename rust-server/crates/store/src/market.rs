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
        // `AddToMarket` refuses only a price below zero, so nothing is a price somebody may ask.
        if price < 0 {
            return Err(StoreError::Refused(
                "Your asking price should be greater than or equal to 0.",
            ));
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
            return Err(StoreError::Refused("Inventory transfer failure."));
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

        // `Client.Account.LastMarketId = shopItem.Id`, and it is what decides whether taking this
        // listing back costs anything.
        sqlx::query("UPDATE account SET last_market_id = $2 WHERE id = $1")
            .bind(seller_id)
            .bind(id)
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        Ok(id)
    }

    /// Everything currently for sale, oldest first.
    ///
    /// The order is what breaks a tie on price when the marketplace decides which of two identical
    /// offers a merchant shows: `PlaceShopItem` puts a new listing after an equal price that was
    /// listed earlier (`realm/Market.cs:372-376`). Two listings can share a timestamp, so the id
    /// settles those rather than leaving the answer to whatever order the rows come back in.
    pub async fn listings(&self, limit: i64) -> Result<Vec<Listing>> {
        let rows = sqlx::query_as::<_, (i64, i64, String, uuid::Uuid, i16, i32)>(
            "SELECT listing.id, listing.seller_id, account.name,
                    listing.item, listing.currency, listing.price
             FROM listing
             JOIN account ON account.id = listing.seller_id
             WHERE listing.status = 'open'
             ORDER BY listing.listed_at, listing.id
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

    /// The id of the last listing this account created, whatever became of it.
    ///
    /// `DbAccount.LastMarketId`, which `/oops` hands straight to the withdrawal without checking
    /// that it names anything: an account that has never listed anything asks about listing zero
    /// and is told that no such listing exists.
    pub async fn last_market_id(&self, account_id: i64) -> Result<i64> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT last_market_id FROM account WHERE id = $1")
            .bind(account_id)
            .fetch_optional(self.pool())
            .await?;

        Ok(row.map(|(id,)| id).unwrap_or(0))
    }

    /// Takes a listing back, putting the item in the gift chest and charging the fee.
    ///
    /// `RemoveItemFromMarketAsync`, in its order, because the order is visible: somebody with less
    /// than five fame asking for a listing that does not exist is told about the fee rather than
    /// about the listing.
    ///
    /// The item goes to the gift chest and not back to the pack. That is where `db.AddGift` puts
    /// it, and it is why the original follows a withdrawal with `giftChestOccupied`: a withdrawal
    /// works with a full pack, and works while the seller is standing somewhere else entirely.
    pub async fn cancel_listing(
        &self,
        seller_id: i64,
        listing_id: i64,
        admin_rank: i16,
    ) -> Result<Withdrawal> {
        // `acc.Admin && acc.Rank < 100`, and `/setrank` keeps `Admin` as `rank >= 80`, so this is
        // the band from 80 up to but not including 100. Staff in that band cannot take a listing
        // back at all. An oddity, kept: it is what the original does to those accounts.
        if (ADMIN_FLAG_RANK..MARKET_EXEMPT_RANK).contains(&admin_rank) {
            return Err(StoreError::Refused("Marketplace Disabled."));
        }

        let mut transaction = self.pool().begin().await?;

        let (fame, last_market_id): (i32, i64) =
            sqlx::query_as("SELECT fame, last_market_id FROM account WHERE id = $1 FOR UPDATE")
                .bind(seller_id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or(StoreError::Refused("Marketplace Disabled."))?;

        // The last thing listed comes back for nothing, which is what makes `/oops` an undo rather
        // than a purchase. Anything older costs the fee.
        let charged = last_market_id != listing_id;
        if charged && fame < REMOVAL_FEE {
            return Err(StoreError::Refused(
                "Not enough fame. There is a 5 fame fee to remove that item.",
            ));
        }

        // Closed first, conditional on it being open and the seller's. A withdrawal racing a sale
        // loses here, before anything has moved or been charged.
        let (item,): (uuid::Uuid,) = sqlx::query_as(
            "UPDATE listing SET status = 'cancelled', closed_at = now()
             WHERE id = $1 AND seller_id = $2 AND status = 'open'
             RETURNING item",
        )
        .bind(listing_id)
        .bind(seller_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::Refused(
            "Market item does not exist. Perhaps someone bought it already or you removed it \
             already?",
        ))?;

        if charged {
            sqlx::query("UPDATE account SET fame = fame - $2 WHERE id = $1 AND fame >= $2")
                .bind(seller_id)
                .bind(REMOVAL_FEE)
                .execute(&mut *transaction)
                .await?;
        }

        let slot = crate::wardrobe::gift(&mut transaction, seller_id, item).await?;

        transaction.commit().await?;
        Ok(Withdrawal {
            item,
            slot,
            fee: if charged { REMOVAL_FEE } else { 0 },
        })
    }

    /// Buys a listing.
    ///
    /// A full pack does not stop the sale. `Merchant.TransactionItem` asks the inventory for a slot
    /// and, when there is none, calls `AddGift` instead and lets the purchase go through: the buyer
    /// paid, and the item is waiting in the gift chest. Refusing here would be refusing a purchase
    /// the original completes.
    pub async fn buy_listing(&self, purchase: MarketPurchase) -> Result<Delivered> {
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

        let delivered = match claim_slot(&mut transaction, character_id, first_slot, last_slot).await
        {
            Ok(slot) => {
                sqlx::query(
                    "UPDATE inventory_slot SET item = $3 WHERE character_id = $1 AND slot = $2",
                )
                .bind(character_id)
                .bind(slot)
                .bind(item)
                .execute(&mut *transaction)
                .await?;
                Delivered::ToPack(slot)
            }

            // No room. The original gifts it rather than refusing, and the buyer is told with
            // `giftChestOccupied`.
            Err(StoreError::Refused(_)) => {
                Delivered::ToGifts(crate::wardrobe::gift(&mut transaction, buyer_id, item).await?)
            }

            Err(other) => return Err(other),
        };

        // The seller is paid last, because everything before it can refuse and roll the whole
        // thing back. Paying first would need a refund path that can itself fail.
        //
        // The whole price. `PlayerMerchant` pays `Price - Tax`, and `Tax` is only ever assigned in
        // `Wmap.cs` from a `tax` property on an object placed in a map: the merchants the market
        // creates in `Market.AddMerchants` are never given one, so it is zero for every listing and
        // `AddToTreasury(Tax)` moves nothing. A cut taken here would be a cut the original does not
        // take.
        sqlx::query(&format!(
            "UPDATE account SET {column} = {column} + $2 WHERE id = $1"
        ))
        .bind(seller_id)
        .bind(price)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;
        Ok(delivered)
    }
}

/// Where a bought item ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivered {
    ToPack(i16),

    /// The pack was full, so it went to the gift chest instead and the buyer needs telling.
    ToGifts(i16),
}

/// What came of taking a listing back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Withdrawal {
    pub item: uuid::Uuid,

    /// Where in the gift chest it landed.
    pub slot: i16,

    /// What it cost, which is nothing for the last thing listed.
    pub fee: i32,
}

/// What it costs to take back anything but the last thing listed.
pub const REMOVAL_FEE: i32 = 5;

/// The rank at which `/setrank` starts setting the account's `Admin` flag.
pub const ADMIN_FLAG_RANK: i16 = 80;

/// The rank from which the flag stops standing in the way of a withdrawal.
pub const MARKET_EXEMPT_RANK: i16 = 100;

/// What one market purchase is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketPurchase {
    pub buyer_id: i64,
    pub character_id: i64,
    pub listing_id: i64,
    pub first_slot: i16,
    pub last_slot: i16,
}

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
