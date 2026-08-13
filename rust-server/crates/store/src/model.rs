//! Accounts and characters.

use crate::{Result, Store, StoreError};

#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub vault_chests: i16,
    pub banned: bool,

    /// `None` for an account that predates authentication, which cannot be logged into until it
    /// sets one.
    pub password_hash: Option<String>,
}

/// A character, with everything needed to put it into a world.
#[derive(Debug, Clone, PartialEq)]
pub struct Character {
    pub id: i64,
    pub account_id: i64,
    pub class: uuid::Uuid,
    pub name: String,
    pub hp: i32,
    pub max_hp: i32,
    pub mp: i32,
    pub max_mp: i32,
    pub level: i16,
    pub experience: i32,
    pub fame: i32,
    pub alive: bool,

    /// Slot index to item identity, only for occupied slots.
    pub inventory: Vec<(i16, uuid::Uuid)>,
}

/// Enough to draw a character-select screen without loading inventories.
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterSummary {
    pub id: i64,
    pub class: uuid::Uuid,
    pub name: String,
    pub level: i16,
    pub fame: i32,
    pub alive: bool,
}

impl Store {
    /// Creates an account, or reports that the name is taken.
    pub async fn create_account(&self, name: &str) -> Result<Account> {
        let row = sqlx::query_as::<_, (i64, String, i16, bool, Option<String>)>(
            "INSERT INTO account (name) VALUES ($1)
             RETURNING id, name, vault_chests, banned, password_hash",
        )
        .bind(name)
        .fetch_one(self.pool())
        .await;

        match row {
            Ok((id, name, vault_chests, banned, password_hash)) => Ok(Account {
                id,
                name,
                vault_chests,
                banned,
                password_hash,
            }),
            // The unique index is what decides this, not a prior lookup. A check-then-insert has a
            // window between the two in which someone else inserts the same name.
            Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
                Err(StoreError::NameTaken)
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Finds an account by name, ignoring case.
    pub async fn account_by_name(&self, name: &str) -> Result<Account> {
        let row = sqlx::query_as::<_, (i64, String, i16, bool, Option<String>)>(
            "SELECT id, name, vault_chests, banned, password_hash
             FROM account WHERE lower(name) = lower($1)",
        )
        .bind(name)
        .fetch_optional(self.pool())
        .await?;

        row.map(|(id, name, vault_chests, banned, password_hash)| Account {
            id,
            name,
            vault_chests,
            banned,
            password_hash,
        })
        .ok_or_else(|| StoreError::NoSuchAccount(name.to_string()))
    }

    /// Sets or replaces an account's password hash.
    pub async fn set_password(&self, account_id: i64, hash: &str) -> Result<()> {
        sqlx::query(
            "UPDATE account SET password_hash = $2, password_changed_at = now() WHERE id = $1",
        )
        .bind(account_id)
        .bind(hash)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// An account by id.
    pub async fn account(&self, id: i64) -> Result<Account> {
        let row = sqlx::query_as::<_, (i64, String, i16, bool, Option<String>)>(
            "SELECT id, name, vault_chests, banned, password_hash FROM account WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await?;

        row.map(|(id, name, vault_chests, banned, password_hash)| Account {
            id,
            name,
            vault_chests,
            banned,
            password_hash,
        })
        .ok_or_else(|| StoreError::NoSuchAccount(id.to_string()))
    }

    /// Creates a character for an account.
    pub async fn create_character(
        &self,
        account_id: i64,
        class: uuid::Uuid,
        name: &str,
        max_hp: i32,
    ) -> Result<Character> {
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO character (account_id, class, name, hp, max_hp)
             VALUES ($1, $2, $3, $4, $4) RETURNING id",
        )
        .bind(account_id)
        .bind(class)
        .bind(name)
        .bind(max_hp)
        .fetch_one(self.pool())
        .await?;

        self.character(id).await
    }

    /// Loads a character and its inventory.
    pub async fn character(&self, id: i64) -> Result<Character> {
        let row = sqlx::query_as::<
            _,
            (
                i64,
                i64,
                uuid::Uuid,
                String,
                i32,
                i32,
                i32,
                i32,
                i16,
                i32,
                i32,
                bool,
            ),
        >(
            "SELECT id, account_id, class, name, hp, max_hp, mp, max_mp,
                    level, experience, fame, alive
             FROM character WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await?
        .ok_or(StoreError::NoSuchCharacter(id))?;

        let inventory = sqlx::query_as::<_, (i16, uuid::Uuid)>(
            "SELECT slot, item FROM inventory_slot
             WHERE character_id = $1 AND item IS NOT NULL ORDER BY slot",
        )
        .bind(id)
        .fetch_all(self.pool())
        .await?;

        Ok(Character {
            id: row.0,
            account_id: row.1,
            class: row.2,
            name: row.3,
            hp: row.4,
            max_hp: row.5,
            mp: row.6,
            max_mp: row.7,
            level: row.8,
            experience: row.9,
            fame: row.10,
            alive: row.11,
            inventory,
        })
    }

    /// Every living character on an account.
    pub async fn characters(&self, account_id: i64) -> Result<Vec<CharacterSummary>> {
        let rows = sqlx::query_as::<_, (i64, uuid::Uuid, String, i16, i32, bool)>(
            "SELECT id, class, name, level, fame, alive
             FROM character WHERE account_id = $1 AND alive ORDER BY id",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, class, name, level, fame, alive)| CharacterSummary {
                id,
                class,
                name,
                level,
                fame,
                alive,
            })
            .collect())
    }

    /// Writes back what a character became.
    ///
    /// Called at logout, at death and at the periodic checkpoint. Deliberately does not touch the
    /// inventory: item movement has its own transactional path, and letting a checkpoint rewrite
    /// slots wholesale would be a way to undo a move that had already committed.
    pub async fn save_character(
        &self,
        id: i64,
        hp: i32,
        mp: i32,
        level: i16,
        experience: i32,
        fame: i32,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE character
             SET hp = LEAST($2, max_hp), mp = $3, level = $4, experience = $5, fame = $6,
                 last_seen = now()
             WHERE id = $1",
        )
        .bind(id)
        .bind(hp)
        .bind(mp)
        .bind(level)
        .bind(experience)
        .bind(fame)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// Marks a character dead. The row stays, because the graveyard is part of the game.
    pub async fn kill_character(&self, id: i64) -> Result<()> {
        sqlx::query("UPDATE character SET alive = false, hp = 0, last_seen = now() WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Deletes a character, if it belongs to the account asking.
    ///
    /// The ownership check is in the statement rather than in a lookup before it, so there is no
    /// window between deciding a character may be deleted and deleting it. Returns whether a row
    /// went; a character that was not there and one belonging to someone else are the same answer,
    /// which is what stops this being a way to find out which ids exist.
    ///
    /// Inventory rows go with it through the foreign key. The items are gone rather than dropped
    /// somewhere, which is what deleting a character means.
    pub async fn delete_character(&self, account_id: i64, id: i64) -> Result<bool> {
        let deleted = sqlx::query("DELETE FROM character WHERE id = $1 AND account_id = $2")
            .bind(id)
            .bind(account_id)
            .execute(self.pool())
            .await?;

        Ok(deleted.rows_affected() > 0)
    }

    /// Whether a character belongs to an account, without loading it.
    pub async fn owns_character(&self, account_id: i64, id: i64) -> Result<bool> {
        let found: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM character WHERE id = $1 AND account_id = $2 AND alive")
                .bind(id)
                .bind(account_id)
                .fetch_optional(self.pool())
                .await?;

        Ok(found.is_some())
    }

    /// The best level and fame this account has reached with each class.
    ///
    /// Returned as a map because the caller asks about several classes at once. Deciding which of
    /// fourteen are playable is one question, not fourteen.
    pub async fn class_progress(
        &self,
        account_id: i64,
    ) -> Result<std::collections::HashMap<uuid::Uuid, (i16, i32)>> {
        let rows = sqlx::query_as::<_, (uuid::Uuid, i16, i32)>(
            "SELECT class, best_level, best_fame FROM class_progress WHERE account_id = $1",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(class, level, fame)| (class, (level, fame)))
            .collect())
    }

    /// Raises the high-water mark for a class, and never lowers it.
    ///
    /// `GREATEST` rather than a read-then-write, so two characters of the same class finishing at
    /// once cannot have the higher one overwritten by the lower.
    pub async fn record_class_progress(
        &self,
        account_id: i64,
        class: uuid::Uuid,
        level: i16,
        fame: i32,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO class_progress (account_id, class, best_level, best_fame)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (account_id, class) DO UPDATE
             SET best_level = GREATEST(class_progress.best_level, EXCLUDED.best_level),
                 best_fame  = GREATEST(class_progress.best_fame,  EXCLUDED.best_fame),
                 updated_at = now()",
        )
        .bind(account_id)
        .bind(class)
        .bind(level)
        .bind(fame)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// Classes this account has bought.
    pub async fn purchased_classes(&self, account_id: i64) -> Result<Vec<uuid::Uuid>> {
        let rows = sqlx::query_as::<_, (uuid::Uuid,)>(
            "SELECT class FROM class_unlock WHERE account_id = $1",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows.into_iter().map(|(class,)| class).collect())
    }

    /// Records a class as bought. Buying one twice is not an error and costs nothing extra.
    pub async fn purchase_class(&self, account_id: i64, class: uuid::Uuid) -> Result<()> {
        sqlx::query(
            "INSERT INTO class_unlock (account_id, class) VALUES ($1, $2)
             ON CONFLICT (account_id, class) DO NOTHING",
        )
        .bind(account_id)
        .bind(class)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// Replaces a character's whole inventory.
    ///
    /// For giving a new character its starting kit, not for saving one mid-play. See
    /// [`Store::move_item`] for that.
    pub async fn set_inventory(
        &self,
        character_id: i64,
        slots: &[(i16, uuid::Uuid)],
    ) -> Result<()> {
        let mut transaction = self.pool().begin().await?;

        sqlx::query("DELETE FROM inventory_slot WHERE character_id = $1")
            .bind(character_id)
            .execute(&mut *transaction)
            .await?;

        for (slot, item) in slots {
            sqlx::query(
                "INSERT INTO inventory_slot (character_id, slot, item) VALUES ($1, $2, $3)",
            )
            .bind(character_id)
            .bind(slot)
            .bind(item)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(())
    }

    /// What an account has in its vault.
    pub async fn vault(&self, account_id: i64) -> Result<Vec<(i16, uuid::Uuid)>> {
        Ok(sqlx::query_as::<_, (i16, uuid::Uuid)>(
            "SELECT slot, item FROM vault_slot
             WHERE account_id = $1 AND item IS NOT NULL ORDER BY slot",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?)
    }

    /// Puts an item straight into a vault slot. For tests and administration.
    pub async fn set_vault_slot(&self, account_id: i64, slot: i16, item: uuid::Uuid) -> Result<()> {
        sqlx::query(
            "INSERT INTO vault_slot (account_id, slot, item) VALUES ($1, $2, $3)
             ON CONFLICT (account_id, slot) DO UPDATE SET item = EXCLUDED.item",
        )
        .bind(account_id)
        .bind(slot)
        .bind(item)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Empties a vault slot. For tests and administration.
    pub async fn clear_vault_slot(&self, account_id: i64, slot: i16) -> Result<()> {
        sqlx::query("DELETE FROM vault_slot WHERE account_id = $1 AND slot = $2")
            .bind(account_id)
            .bind(slot)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// How many vault chests an account has, each of eight slots.
    pub async fn buy_vault_chest(&self, account_id: i64, limit: i16) -> Result<i16> {
        let (chests,): (i16,) = sqlx::query_as(
            "UPDATE account SET vault_chests = vault_chests + 1
             WHERE id = $1 AND vault_chests < $2
             RETURNING vault_chests",
        )
        .bind(account_id)
        .bind(limit)
        .fetch_optional(self.pool())
        .await?
        .ok_or(StoreError::Refused("no more vault chests are available"))?;

        Ok(chests)
    }
}
