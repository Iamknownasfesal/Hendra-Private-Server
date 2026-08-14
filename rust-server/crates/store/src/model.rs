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

    /// What the account can spend. Held here rather than on a character, because a purchase made
    /// by one is paid for by all of them and a death does not take it away.
    pub gold: i32,
    pub fame: i32,
    pub tokens: i32,

    /// When the account may speak again, or `None` if it always may.
    pub muted_until: Option<chrono::DateTime<chrono::Utc>>,

    /// What this account may do to others. Zero is an ordinary player.
    pub admin_rank: i16,

    /// The Discord user this account is linked to, for anything outside the game that needs to say
    /// who somebody is.
    pub discord_id: Option<String>,

    /// What it may spend on skins and the like.
    pub credits: i32,
}

/// What a moderator may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i16)]
pub enum Admin {
    /// An ordinary player.
    None = 0,

    /// May mute and kick.
    Moderator = 10,

    /// May ban, and may raise others.
    Administrator = 20,
}

impl Admin {
    pub fn from_number(number: i16) -> Admin {
        match number {
            n if n >= Admin::Administrator as i16 => Admin::Administrator,
            n if n >= Admin::Moderator as i16 => Admin::Moderator,
            _ => Admin::None,
        }
    }

    pub fn may_mute(self) -> bool {
        self >= Admin::Moderator
    }

    pub fn may_ban(self) -> bool {
        self >= Admin::Administrator
    }
}

/// The things an account spends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    Gold,
    Fame,
    Tokens,
    Prestige,
}

impl Currency {
    /// The column this currency lives in.
    pub(crate) fn column_name(self) -> &'static str {
        self.column()
    }

    fn column(self) -> &'static str {
        match self {
            Currency::Gold => "gold",
            Currency::Fame => "fame",
            Currency::Tokens => "tokens",
            Currency::Prestige => "prestige",
        }
    }
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

    /// Potions carried outside the inventory, which is where the game has always kept them.
    pub health_potions: i32,
    pub magic_potions: i32,
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
        let row = sqlx::query_as::<_,
            (
                i64,
                String,
                i16,
                bool,
                Option<String>,
                i32,
                i32,
                i32,
                Option<chrono::DateTime<chrono::Utc>>,
                i16,
                i32,
                Option<String>,
            ),>(
            "INSERT INTO account (name) VALUES ($1)
             RETURNING id, name, vault_chests, banned, password_hash, gold, fame, tokens, muted_until, admin_rank, credits, discord_id",
        )
        .bind(name)
        .fetch_one(self.pool())
        .await;

        match row {
            Ok((
                id,
                name,
                vault_chests,
                banned,
                password_hash,
                gold,
                fame,
                tokens,
                muted_until,
                admin_rank,
                credits,
                discord_id,
            )) => Ok(Account {
                id,
                name,
                vault_chests,
                banned,
                password_hash,
                gold,
                fame,
                tokens,
                muted_until,
                admin_rank,
                credits,
                discord_id,
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
        let row = sqlx::query_as::<
            _,
            (
                i64,
                String,
                i16,
                bool,
                Option<String>,
                i32,
                i32,
                i32,
                Option<chrono::DateTime<chrono::Utc>>,
                i16,
                i32,
                Option<String>,
            ),
        >(
            "SELECT id, name, vault_chests, banned, password_hash, gold, fame, tokens, muted_until, admin_rank, credits, discord_id
             FROM account WHERE lower(name) = lower($1)",
        )
        .bind(name)
        .fetch_optional(self.pool())
        .await?;

        row.map(
            |(
                id,
                name,
                vault_chests,
                banned,
                password_hash,
                gold,
                fame,
                tokens,
                muted_until,
                admin_rank,
                credits,
                discord_id,
            )| {
                Account {
                    id,
                    name,
                    vault_chests,
                    banned,
                    password_hash,
                    gold,
                    fame,
                    tokens,
                    muted_until,
                    admin_rank,
                    credits,
                    discord_id,
                }
            },
        )
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
        let row = sqlx::query_as::<
            _,
            (
                i64,
                String,
                i16,
                bool,
                Option<String>,
                i32,
                i32,
                i32,
                Option<chrono::DateTime<chrono::Utc>>,
                i16,
                i32,
                Option<String>,
            ),
        >(
            "SELECT id, name, vault_chests, banned, password_hash, gold, fame, tokens, muted_until, admin_rank, credits, discord_id
             FROM account WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await?;

        row.map(
            |(
                id,
                name,
                vault_chests,
                banned,
                password_hash,
                gold,
                fame,
                tokens,
                muted_until,
                admin_rank,
                credits,
                discord_id,
            )| {
                Account {
                    id,
                    name,
                    vault_chests,
                    banned,
                    password_hash,
                    gold,
                    fame,
                    tokens,
                    muted_until,
                    admin_rank,
                    credits,
                    discord_id,
                }
            },
        )
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
    /// The living character of that name, if there is one.
    ///
    /// Names are compared without case, as everywhere else a player types one: somebody asked to
    /// trade by typing a name, and they should not have to match its capitals.
    pub async fn character_named(&self, name: &str) -> Result<Option<Character>> {
        let found = sqlx::query_as::<_, (i64,)>(
            "SELECT id FROM character WHERE lower(name) = lower($1) AND alive LIMIT 1",
        )
        .bind(name)
        .fetch_optional(self.pool())
        .await?;

        match found {
            Some((id,)) => Ok(Some(self.character(id).await?)),
            None => Ok(None),
        }
    }

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
                i32,
                i32,
            ),
        >(
            "SELECT id, account_id, class, name, hp, max_hp, mp, max_mp,
                    level, experience, fame, alive, health_potions, magic_potions
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
            health_potions: row.12,
            magic_potions: row.13,
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

    /// Records a failed login against a name.
    ///
    /// Keyed by the name rather than the account, because a name that does not exist has to be
    /// counted the same as one that does: counting only real accounts would make the limiter
    /// answer the question the login endpoint refuses to.
    pub async fn record_failed_login(&self, name: &str) -> Result<()> {
        sqlx::query("INSERT INTO failed_login (name) VALUES (lower($1))")
            .bind(name.trim())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// How many failures a name has accumulated inside a window.
    pub async fn recent_failed_logins(&self, name: &str, window_seconds: i64) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM failed_login
             WHERE name = lower($1) AND at > now() - make_interval(secs => $2)",
        )
        .bind(name.trim())
        .bind(window_seconds as f64)
        .fetch_one(self.pool())
        .await?;

        Ok(count)
    }

    /// Forgets a name's failures, which a correct password does.
    pub async fn clear_failed_logins(&self, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM failed_login WHERE name = lower($1)")
            .bind(name.trim())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Drops failures old enough that nothing counts them.
    ///
    /// The table is filled by unauthenticated requests naming whatever they like, so something has
    /// to shrink it.
    pub async fn forget_old_failed_logins(&self, window_seconds: i64) -> Result<u64> {
        let removed =
            sqlx::query("DELETE FROM failed_login WHERE at <= now() - make_interval(secs => $1)")
                .bind(window_seconds as f64)
                .execute(self.pool())
                .await?;

        Ok(removed.rows_affected())
    }

    /// Bans or unbans an account.
    /// Sets one of an account's currencies outright.
    ///
    /// For an administrator setting a number, which is the only thing that should ever assign one
    /// rather than add to or subtract from it: every other path is a transaction that has to be
    /// conditional on the balance.
    pub async fn set_currency(
        &self,
        account_id: i64,
        currency: Currency,
        amount: i32,
    ) -> Result<()> {
        if amount < 0 {
            return Err(StoreError::Refused("that is not an amount"));
        }

        let column = currency.column_name();
        let changed = sqlx::query(&format!("UPDATE account SET {column} = $2 WHERE id = $1"))
            .bind(account_id)
            .bind(amount)
            .execute(self.pool())
            .await?;

        if changed.rows_affected() == 0 {
            return Err(StoreError::Refused("no such account"));
        }

        Ok(())
    }

    pub async fn set_banned(&self, account_id: i64, banned: bool) -> Result<()> {
        sqlx::query("UPDATE account SET banned = $2 WHERE id = $1")
            .bind(account_id)
            .bind(banned)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Sets what an account may do to others.
    pub async fn set_admin_rank(&self, account_id: i64, rank: Admin) -> Result<()> {
        sqlx::query("UPDATE account SET admin_rank = $2 WHERE id = $1")
            .bind(account_id)
            .bind(rank as i16)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Renames an account, or reports that the name is taken.
    ///
    /// The unique index decides it, not a prior lookup: a check-then-rename has a window in which
    /// somebody else takes the name.
    pub async fn rename_account(&self, account_id: i64, name: &str) -> Result<()> {
        let renamed = sqlx::query("UPDATE account SET name = $2 WHERE id = $1")
            .bind(account_id)
            .bind(name)
            .execute(self.pool())
            .await;

        match renamed {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
                Err(StoreError::NameTaken)
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Silences an account until a time, or lifts a mute when given `None`.
    pub async fn mute(
        &self,
        account_id: i64,
        until: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<()> {
        sqlx::query("UPDATE account SET muted_until = $2 WHERE id = $1")
            .bind(account_id)
            .bind(until)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Adds to what an account can spend.
    pub async fn credit(&self, account_id: i64, currency: Currency, amount: i32) -> Result<()> {
        if amount <= 0 {
            return Ok(());
        }

        let column = currency.column();
        sqlx::query(&format!(
            "UPDATE account SET {column} = {column} + $2 WHERE id = $1"
        ))
        .bind(account_id)
        .bind(amount)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// Takes from what an account can spend, refusing when there is not enough.
    ///
    /// The balance check is in the statement rather than read first, so two purchases arriving
    /// together cannot both see the same coin.
    pub async fn debit(&self, account_id: i64, currency: Currency, amount: i32) -> Result<bool> {
        if amount <= 0 {
            return Ok(true);
        }

        let column = currency.column();
        let spent = sqlx::query(&format!(
            "UPDATE account SET {column} = {column} - $2 WHERE id = $1 AND {column} >= $2"
        ))
        .bind(account_id)
        .bind(amount)
        .execute(self.pool())
        .await?;

        Ok(spent.rows_affected() > 0)
    }

    /// The most potions of one kind a character may carry.
    pub const POTION_LIMIT: i32 = 6;

    /// Adds a potion to a stack, refusing once it is full.
    ///
    /// The ceiling is enforced in the statement rather than by reading first, so two pickups
    /// arriving together cannot both see room for the last one.
    pub async fn add_potion(&self, character_id: i64, magic: bool) -> Result<bool> {
        let column = if magic {
            "magic_potions"
        } else {
            "health_potions"
        };

        let updated = sqlx::query(&format!(
            "UPDATE character SET {column} = {column} + 1
             WHERE id = $1 AND {column} < $2"
        ))
        .bind(character_id)
        .bind(Self::POTION_LIMIT)
        .execute(self.pool())
        .await?;

        Ok(updated.rows_affected() > 0)
    }

    /// Takes a potion from a stack, refusing when it is empty.
    pub async fn take_potion(&self, character_id: i64, magic: bool) -> Result<bool> {
        let column = if magic {
            "magic_potions"
        } else {
            "health_potions"
        };

        let updated = sqlx::query(&format!(
            "UPDATE character SET {column} = {column} - 1 WHERE id = $1 AND {column} > 0"
        ))
        .bind(character_id)
        .execute(self.pool())
        .await?;

        Ok(updated.rows_affected() > 0)
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

/// How much fame one prestige costs.
pub const FAME_PER_PRESTIGE: i32 = 1500;

/// Prestige: what a character's fame becomes when the character is given up.
///
/// Follows `PrestigeHandler`: every fifteen hundred fame becomes one prestige, and the character is
/// returned to level one with nothing. The exchange and the reset are one transaction, because
/// either half alone is a way to lose a character or to mint prestige from one.
impl Store {
    /// Trades a character's fame for prestige and starts it over.
    ///
    /// Returns how much prestige was earned.
    pub async fn prestige(&self, account_id: i64, character_id: i64) -> Result<i32> {
        let mut transaction = self.pool().begin().await?;

        // The character is locked before its fame is read, so two requests cannot both see the same
        // fame and both be paid for it.
        let held = sqlx::query_as::<_, (i32, i64)>(
            "SELECT fame, account_id FROM character WHERE id = $1 AND alive FOR UPDATE",
        )
        .bind(character_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::NoSuchCharacter(character_id))?;

        if held.1 != account_id {
            return Err(StoreError::Refused("that is not your character"));
        }

        let earned = held.0 / FAME_PER_PRESTIGE;
        if earned <= 0 {
            return Err(StoreError::Refused("you need fifteen hundred fame or more"));
        }

        // Everything the fame bought goes with it. A character that kept its level would be a
        // character that could be prestiged again the moment it earned the fame back.
        sqlx::query("UPDATE character SET fame = 0, experience = 0, level = 1 WHERE id = $1")
            .bind(character_id)
            .execute(&mut *transaction)
            .await?;

        sqlx::query(
            "UPDATE account SET prestige = prestige + $2, total_prestige = total_prestige + $2
             WHERE id = $1",
        )
        .bind(account_id)
        .bind(earned)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;
        Ok(earned)
    }

    /// Spends prestige on something, and says whether there was enough.
    ///
    /// Conditional on the balance in the same statement that reduces it, so two requests cannot
    /// both see enough and both be granted. The lifetime total is untouched: a shop that reduced it
    /// would make the total mean nothing.
    pub async fn spend_prestige(&self, account_id: i64, price: i32) -> Result<()> {
        if price <= 0 {
            return Err(StoreError::Refused("that is not a price"));
        }

        let paid = sqlx::query(
            "UPDATE account SET prestige = prestige - $2 WHERE id = $1 AND prestige >= $2",
        )
        .bind(account_id)
        .bind(price)
        .execute(self.pool())
        .await?;

        if paid.rows_affected() == 0 {
            return Err(StoreError::Refused("you cannot afford that"));
        }

        Ok(())
    }

    /// How much prestige an account holds, and how much it has ever earned.
    pub async fn prestige_of(&self, account_id: i64) -> Result<(i32, i32)> {
        let held = sqlx::query_as::<_, (i32, i32)>(
            "SELECT prestige, total_prestige FROM account WHERE id = $1",
        )
        .bind(account_id)
        .fetch_optional(self.pool())
        .await?
        .ok_or(StoreError::Refused("no such account"))?;

        Ok(held)
    }
}
