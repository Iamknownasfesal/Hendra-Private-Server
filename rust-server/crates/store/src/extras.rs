//! The rest of what an account carries: strings, offers, quests, pictures and skins.
//!
//! Grouped because they share a shape rather than a subject. Each is a small table an operator
//! fills, read by an endpoint that would otherwise have to be recompiled to change what it says.

use crate::{Result, Store, StoreError};

/// The most bytes a picture may be.
///
/// Small, because this is a portrait rather than a gallery, and because bytes in a row are bytes in
/// every backup of that row.
pub const MAX_PICTURE_BYTES: usize = 256 * 1024;

/// Something an account may buy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub id: i64,
    pub name: String,
    pub credits: i32,
    pub price_cents: i32,
}

/// A quest, and how far one account has got with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quest {
    pub id: i64,
    pub key: String,
    pub title: String,
    pub goal: i32,
    pub weekly: bool,
    pub progress: i32,
    pub finished: bool,
}

impl Store {
    // -- strings -------------------------------------------------------------------------------

    /// Sets one translated string.
    pub async fn set_string(&self, language: &str, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO language_string (language, key, value) VALUES ($1, $2, $3)
             ON CONFLICT (language, key) DO UPDATE SET value = EXCLUDED.value",
        )
        .bind(language.trim())
        .bind(key.trim())
        .bind(value)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Every string for a language.
    pub async fn strings(&self, language: &str) -> Result<Vec<(String, String)>> {
        Ok(sqlx::query_as::<_, (String, String)>(
            "SELECT key, value FROM language_string WHERE language = $1 ORDER BY key",
        )
        .bind(language.trim())
        .fetch_all(self.pool())
        .await?)
    }

    // -- credits -------------------------------------------------------------------------------

    /// What is currently for sale.
    pub async fn credit_offers(&self) -> Result<Vec<Offer>> {
        let rows = sqlx::query_as::<_, (i64, String, i32, i32)>(
            "SELECT id, name, credits, price_cents FROM credit_offer
             WHERE available ORDER BY price_cents",
        )
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, name, credits, price_cents)| Offer {
                id,
                name,
                credits,
                price_cents,
            })
            .collect())
    }

    /// Adds credits to an account.
    ///
    /// Takes the offer rather than an amount, so the number of credits comes from the row an
    /// operator wrote and never from whoever is asking.
    pub async fn grant_offer(&self, account_id: i64, offer_id: i64) -> Result<i32> {
        let mut transaction = self.pool().begin().await?;

        let (credits,): (i32,) =
            sqlx::query_as("SELECT credits FROM credit_offer WHERE id = $1 AND available")
                .bind(offer_id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or(StoreError::Refused("that offer is not available"))?;

        sqlx::query("UPDATE account SET credits = credits + $2 WHERE id = $1")
            .bind(account_id)
            .bind(credits)
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        Ok(credits)
    }

    // -- quests --------------------------------------------------------------------------------

    /// Records a quest, or updates one already there.
    pub async fn define_quest(
        &self,
        key: &str,
        title: &str,
        goal: i32,
        weekly: bool,
    ) -> Result<i64> {
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO quest (key, title, goal, weekly) VALUES ($1, $2, $3, $4)
             ON CONFLICT (key) DO UPDATE
             SET title = EXCLUDED.title, goal = EXCLUDED.goal, weekly = EXCLUDED.weekly
             RETURNING id",
        )
        .bind(key.trim())
        .bind(title.trim())
        .bind(goal.max(1))
        .bind(weekly)
        .fetch_one(self.pool())
        .await?;

        Ok(id)
    }

    /// Every quest, with how far this account has got.
    pub async fn quests(&self, account_id: i64, weekly: Option<bool>) -> Result<Vec<Quest>> {
        let rows = sqlx::query_as::<
            _,
            (
                i64,
                String,
                String,
                i32,
                bool,
                Option<i32>,
                Option<chrono::DateTime<chrono::Utc>>,
            ),
        >(
            "SELECT quest.id, quest.key, quest.title, quest.goal, quest.weekly,
                    quest_progress.progress, quest_progress.finished_at
             FROM quest
             LEFT JOIN quest_progress
               ON quest_progress.quest_id = quest.id AND quest_progress.account_id = $1
             WHERE $2::boolean IS NULL OR quest.weekly = $2
             ORDER BY quest.id",
        )
        .bind(account_id)
        .bind(weekly)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(id, key, title, goal, weekly, progress, finished_at)| Quest {
                    id,
                    key,
                    title,
                    goal,
                    weekly,
                    progress: progress.unwrap_or(0),
                    finished: finished_at.is_some(),
                },
            )
            .collect())
    }

    /// Advances a quest, marking it finished when it reaches its goal.
    ///
    /// The goal is read from the quest rather than passed in, and the finish is decided in the
    /// statement, so two advances arriving together cannot both decide they were the one that
    /// finished it.
    pub async fn advance_quest(&self, account_id: i64, key: &str, by: i32) -> Result<bool> {
        let finished: Option<(bool,)> = sqlx::query_as(
            "WITH target AS (SELECT id, goal FROM quest WHERE key = $2)
             INSERT INTO quest_progress (account_id, quest_id, progress, finished_at)
             SELECT $1, target.id, LEAST($3, target.goal),
                    CASE WHEN $3 >= target.goal THEN now() END
             FROM target
             ON CONFLICT (account_id, quest_id) DO UPDATE
             SET progress = LEAST(quest_progress.progress + $3,
                                  (SELECT goal FROM target)),
                 finished_at = COALESCE(
                     quest_progress.finished_at,
                     CASE WHEN quest_progress.progress + $3 >= (SELECT goal FROM target)
                          THEN now() END)
             RETURNING finished_at IS NOT NULL",
        )
        .bind(account_id)
        .bind(key.trim())
        .bind(by.max(0))
        .fetch_optional(self.pool())
        .await?;

        Ok(finished.map(|(done,)| done).unwrap_or(false))
    }

    // -- pictures ------------------------------------------------------------------------------

    /// Stores a picture for an account, replacing any it had.
    pub async fn set_picture(&self, account_id: i64, kind: &str, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Err(StoreError::Refused("there is nothing to store"));
        }
        if bytes.len() > MAX_PICTURE_BYTES {
            return Err(StoreError::Refused("that picture is too large"));
        }

        sqlx::query(
            "INSERT INTO picture (account_id, kind, bytes) VALUES ($1, $2, $3)
             ON CONFLICT (account_id) DO UPDATE
             SET kind = EXCLUDED.kind, bytes = EXCLUDED.bytes, uploaded_at = now()",
        )
        .bind(account_id)
        .bind(kind.trim())
        .bind(bytes)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// An account's picture, if it has one.
    pub async fn picture(&self, account_id: i64) -> Result<Option<(String, Vec<u8>)>> {
        Ok(sqlx::query_as::<_, (String, Vec<u8>)>(
            "SELECT kind, bytes FROM picture WHERE account_id = $1",
        )
        .bind(account_id)
        .fetch_optional(self.pool())
        .await?)
    }

    // -- skins and age -------------------------------------------------------------------------

    /// Records that an account owns a skin.
    pub async fn grant_skin(&self, account_id: i64, skin: uuid::Uuid) -> Result<()> {
        sqlx::query(
            "INSERT INTO owned_skin (account_id, skin) VALUES ($1, $2)
             ON CONFLICT (account_id, skin) DO NOTHING",
        )
        .bind(account_id)
        .bind(skin)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Which skins an account owns.
    pub async fn owned_skins(&self, account_id: i64) -> Result<Vec<uuid::Uuid>> {
        let rows =
            sqlx::query_as::<_, (uuid::Uuid,)>("SELECT skin FROM owned_skin WHERE account_id = $1")
                .bind(account_id)
                .fetch_all(self.pool())
                .await?;

        Ok(rows.into_iter().map(|(skin,)| skin).collect())
    }

    /// Buys a skin with credits, in one transaction.
    ///
    /// Paying and receiving together, for the same reason every other purchase does it: paying
    /// outside is how somebody loses credits and gets nothing.
    pub async fn buy_skin(&self, account_id: i64, skin: uuid::Uuid, price: i32) -> Result<()> {
        let mut transaction = self.pool().begin().await?;

        if price > 0 {
            let paid = sqlx::query(
                "UPDATE account SET credits = credits - $2 WHERE id = $1 AND credits >= $2",
            )
            .bind(account_id)
            .bind(price)
            .execute(&mut *transaction)
            .await?;

            if paid.rows_affected() == 0 {
                return Err(StoreError::Refused("you cannot afford that"));
            }
        }

        let granted = sqlx::query(
            "INSERT INTO owned_skin (account_id, skin) VALUES ($1, $2)
             ON CONFLICT (account_id, skin) DO NOTHING",
        )
        .bind(account_id)
        .bind(skin)
        .execute(&mut *transaction)
        .await?;

        // Buying one twice is refused rather than charged for, which rolls the payment back.
        if granted.rows_affected() == 0 {
            return Err(StoreError::Refused("you already own that"));
        }

        transaction.commit().await?;
        Ok(())
    }

    /// Buys a class unlock with credits, in one transaction.
    ///
    /// The same shape as [`Store::buy_skin`] and for the same reason: `char/purchaseClassUnlock.cs`
    /// charges and unlocks in two separate calls, which is one crash away from a player who paid
    /// for a class they cannot play. Paying for one already unlocked is refused rather than charged
    /// for, which rolls the payment back.
    pub async fn buy_class(&self, account_id: i64, class: uuid::Uuid, price: i32) -> Result<()> {
        let mut transaction = self.pool().begin().await?;

        if price > 0 {
            let paid = sqlx::query(
                "UPDATE account SET credits = credits - $2 WHERE id = $1 AND credits >= $2",
            )
            .bind(account_id)
            .bind(price)
            .execute(&mut *transaction)
            .await?;

            if paid.rows_affected() == 0 {
                return Err(StoreError::Refused("you cannot afford that"));
            }
        }

        let granted = sqlx::query(
            "INSERT INTO class_unlock (account_id, class) VALUES ($1, $2)
             ON CONFLICT (account_id, class) DO NOTHING",
        )
        .bind(account_id)
        .bind(class)
        .execute(&mut *transaction)
        .await?;

        if granted.rows_affected() == 0 {
            return Err(StoreError::Refused("you already have that"));
        }

        transaction.commit().await?;
        Ok(())
    }

    /// Records that an account has confirmed its age.
    pub async fn set_age_verified(&self, account_id: i64, verified: bool) -> Result<()> {
        sqlx::query("UPDATE account SET age_verified = $2 WHERE id = $1")
            .bind(account_id)
            .bind(verified)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Whether an account has confirmed its age.
    pub async fn age_verified(&self, account_id: i64) -> Result<bool> {
        let found: Option<(bool,)> =
            sqlx::query_as("SELECT age_verified FROM account WHERE id = $1")
                .bind(account_id)
                .fetch_optional(self.pool())
                .await?;

        Ok(found.map(|(verified,)| verified).unwrap_or(false))
    }

    /// When this account was last playing, as a unix timestamp.
    ///
    /// Derived rather than stored. The original keeps a `lastSeen` field on the account and stamps
    /// it at registration and at each sign-in (`DbModels.cs`, `RefreshLastSeen`); there is no such
    /// column here, so the answer is taken from the newest character the account has touched, and
    /// from when the account itself was made when it has touched none. Both are facts already
    /// recorded, and together they say the same thing that field says.
    pub async fn last_seen(&self, account_id: i64) -> Result<i64> {
        // Cast, because `extract` answers in `numeric` and the row is read as a float.
        let found: Option<(Option<f64>,)> = sqlx::query_as(
            "SELECT extract(epoch FROM GREATEST(
                 account.created_at,
                 (SELECT max(last_seen) FROM character WHERE account_id = account.id)
             ))::float8
             FROM account WHERE account.id = $1",
        )
        .bind(account_id)
        .fetch_optional(self.pool())
        .await?;

        Ok(found
            .and_then(|(seconds,)| seconds)
            .map(|seconds| seconds as i64)
            .unwrap_or(0))
    }
}

/// Settings an administrator changes without restarting.
impl Store {
    pub async fn set_setting(&self, name: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO setting (name, value) VALUES ($1, $2)
             ON CONFLICT (name) DO UPDATE SET value = $2, at = now()",
        )
        .bind(name)
        .bind(value)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    pub async fn setting(&self, name: &str) -> Result<Option<String>> {
        let found = sqlx::query_as::<_, (String,)>("SELECT value FROM setting WHERE name = $1")
            .bind(name)
            .fetch_optional(self.pool())
            .await?;

        Ok(found.map(|(value,)| value))
    }
}
