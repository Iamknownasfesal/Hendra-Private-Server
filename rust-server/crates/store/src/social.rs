//! Friends, and messages left for someone who is not here.
//!
//! # Why a friendship is two rows
//!
//! One row per direction rather than one shared row. Two rows are what let each side remove the
//! other without deciding for them, and what makes "who are my friends" a lookup on one indexed
//! column rather than a search on two.
//!
//! A request is the first row alone. Accepting writes the second and marks both, so a friendship
//! is mutual exactly when both rows say so.

use crate::{Result, Store, StoreError};

/// Somebody on a friend list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Friend {
    pub account_id: i64,
    pub name: String,

    /// Whether they have accepted. A request shows on both lists and counts as neither side's
    /// friend until it does.
    pub accepted: bool,
}

/// One message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub from: String,
    pub body: String,
    pub read: bool,
}

impl Store {
    /// Asks somebody to be a friend, or accepts their asking.
    ///
    /// One call for both, because from the caller's side they are the same act: the difference is
    /// whether the other row already exists, which the database can see and the caller cannot
    /// without a race.
    pub async fn befriend(&self, account_id: i64, friend_id: i64) -> Result<bool> {
        if account_id == friend_id {
            return Err(StoreError::Refused("you are already your own company"));
        }

        let mut transaction = self.pool().begin().await?;

        sqlx::query(
            "INSERT INTO friendship (account_id, friend_id) VALUES ($1, $2)
             ON CONFLICT (account_id, friend_id) DO NOTHING",
        )
        .bind(account_id)
        .bind(friend_id)
        .execute(&mut *transaction)
        .await?;

        // Mutual exactly when the other side has asked too. Checked inside the transaction so two
        // simultaneous requests cannot both decide the other had not asked yet.
        let (mutual,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM friendship WHERE account_id = $1 AND friend_id = $2)",
        )
        .bind(friend_id)
        .bind(account_id)
        .fetch_one(&mut *transaction)
        .await?;

        if mutual {
            sqlx::query(
                "UPDATE friendship SET accepted = true
                 WHERE (account_id = $1 AND friend_id = $2)
                    OR (account_id = $2 AND friend_id = $1)",
            )
            .bind(account_id)
            .bind(friend_id)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(mutual)
    }

    /// Removes a friend, from both sides.
    ///
    /// Both, because a one-sided removal leaves the other believing in a friendship that is not
    /// there, which is worse than either state on its own.
    pub async fn unfriend(&self, account_id: i64, friend_id: i64) -> Result<()> {
        sqlx::query(
            "DELETE FROM friendship
             WHERE (account_id = $1 AND friend_id = $2)
                OR (account_id = $2 AND friend_id = $1)",
        )
        .bind(account_id)
        .bind(friend_id)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// Everyone on an account's list, accepted or not.
    pub async fn friends(&self, account_id: i64) -> Result<Vec<Friend>> {
        let rows = sqlx::query_as::<_, (i64, String, bool)>(
            "SELECT account.id, account.name, friendship.accepted
             FROM friendship
             JOIN account ON account.id = friendship.friend_id
             WHERE friendship.account_id = $1
             ORDER BY account.name",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(account_id, name, accepted)| Friend {
                account_id,
                name,
                accepted,
            })
            .collect())
    }

    /// Requests waiting for an answer.
    pub async fn friend_requests(&self, account_id: i64) -> Result<Vec<Friend>> {
        let rows = sqlx::query_as::<_, (i64, String)>(
            "SELECT account.id, account.name
             FROM friendship
             JOIN account ON account.id = friendship.account_id
             WHERE friendship.friend_id = $1
               AND NOT friendship.accepted
               AND NOT EXISTS (
                   SELECT 1 FROM friendship AS mine
                   WHERE mine.account_id = $1 AND mine.friend_id = friendship.account_id
               )
             ORDER BY account.name",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(account_id, name)| Friend {
                account_id,
                name,
                accepted: false,
            })
            .collect())
    }

    /// Leaves a message for somebody.
    pub async fn send_message(&self, from: i64, to: i64, body: &str) -> Result<i64> {
        let body = body.trim();
        if body.is_empty() {
            return Err(StoreError::Refused("there is nothing to send"));
        }
        if from == to {
            return Err(StoreError::Refused("you cannot write to yourself"));
        }

        // Bounded here as well as at the wire, because this is the side that keeps it.
        let body: String = body.chars().take(MAX_MESSAGE_LENGTH).collect();

        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO private_message (from_id, to_id, body) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(from)
        .bind(to)
        .bind(&body)
        .fetch_one(self.pool())
        .await?;

        Ok(id)
    }

    /// An account's messages, newest first.
    pub async fn messages(&self, account_id: i64, limit: i64) -> Result<Vec<Message>> {
        let rows = sqlx::query_as::<_, (i64, String, String, Option<chrono::DateTime<chrono::Utc>>)>(
            "SELECT private_message.id, account.name, private_message.body, private_message.read_at
             FROM private_message
             JOIN account ON account.id = private_message.from_id
             WHERE private_message.to_id = $1
             ORDER BY private_message.sent_at DESC
             LIMIT $2",
        )
        .bind(account_id)
        .bind(limit.clamp(1, MAX_MESSAGES_READ))
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, from, body, read_at)| Message {
                id,
                from,
                body,
                read: read_at.is_some(),
            })
            .collect())
    }

    /// Marks a message read, but only if it was addressed to this account.
    ///
    /// The ownership check is in the statement rather than a lookup before it, so there is no
    /// window between deciding a message may be read and reading it.
    pub async fn mark_read(&self, account_id: i64, message_id: i64) -> Result<bool> {
        let marked = sqlx::query(
            "UPDATE private_message SET read_at = now()
             WHERE id = $1 AND to_id = $2 AND read_at IS NULL",
        )
        .bind(message_id)
        .bind(account_id)
        .execute(self.pool())
        .await?;

        Ok(marked.rows_affected() > 0)
    }

    /// Deletes a message, if it was addressed to this account.
    pub async fn delete_message(&self, account_id: i64, message_id: i64) -> Result<bool> {
        let deleted = sqlx::query("DELETE FROM private_message WHERE id = $1 AND to_id = $2")
            .bind(message_id)
            .bind(account_id)
            .execute(self.pool())
            .await?;

        Ok(deleted.rows_affected() > 0)
    }
}

/// The longest message that will be kept.
pub const MAX_MESSAGE_LENGTH: usize = 1024;

/// The most messages one read may return.
pub const MAX_MESSAGES_READ: i64 = 100;

/// Linking an account to a Discord user.
///
/// The link exists so that something outside the game, a bot or a rank tool, can say who somebody
/// is. It is deliberately one-to-one in both directions, and that is enforced by a unique index
/// rather than by a check here: a check in the writer is a check a second writer can race past.
impl Store {
    /// Points a Discord id at an account, replacing whatever either of them pointed at before.
    ///
    /// Replacing rather than refusing, because the alternative is an account that cannot be relinked
    /// after somebody changes their Discord. Both old links go in the same transaction as the new
    /// one, so a failure part way leaves neither half done.
    pub async fn register_discord(&self, account_id: i64, discord_id: &str) -> Result<()> {
        if discord_id.trim().is_empty() {
            return Err(StoreError::Refused("that is not a discord id"));
        }

        let mut transaction = self.pool().begin().await?;

        // Whoever held this id loses it, and whatever this account held is replaced. Without the
        // first, the unique index refuses the insert and the caller sees a constraint rather than
        // an answer.
        sqlx::query("UPDATE account SET discord_id = NULL WHERE discord_id = $1")
            .bind(discord_id)
            .execute(&mut *transaction)
            .await?;

        let linked = sqlx::query("UPDATE account SET discord_id = $2 WHERE id = $1")
            .bind(account_id)
            .bind(discord_id)
            .execute(&mut *transaction)
            .await?;

        if linked.rows_affected() == 0 {
            return Err(StoreError::Refused("no such account"));
        }

        transaction.commit().await?;
        Ok(())
    }

    /// Removes the link, but only if the account actually holds that id.
    ///
    /// Naming the id rather than just the account is what stops a stale request from unlinking
    /// whatever happens to be there now.
    pub async fn unregister_discord(&self, account_id: i64, discord_id: &str) -> Result<()> {
        let removed =
            sqlx::query("UPDATE account SET discord_id = NULL WHERE id = $1 AND discord_id = $2")
                .bind(account_id)
                .bind(discord_id)
                .execute(self.pool())
                .await?;

        if removed.rows_affected() == 0 {
            return Err(StoreError::Refused(
                "that account is not linked to that discord id",
            ));
        }

        Ok(())
    }

    /// Which account a Discord id belongs to, if any.
    pub async fn account_of_discord(&self, discord_id: &str) -> Result<Option<i64>> {
        let found = sqlx::query_as::<_, (i64,)>("SELECT id FROM account WHERE discord_id = $1")
            .bind(discord_id)
            .fetch_optional(self.pool())
            .await?;

        Ok(found.map(|(id,)| id))
    }
}

/// One of the lists an account keeps about other people.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    /// People whose messages are not shown.
    Ignored = 0,

    /// People who may not teleport to this player.
    LockedOut = 1,
}

impl ListKind {
    fn number(self) -> i16 {
        self as i16
    }
}

/// Ignoring somebody, and locking somebody out.
///
/// Two lists rather than one, because they do different jobs: ignoring hides what somebody says, and
/// locking stops them arriving where you are. Somebody can be on both, one, or neither.
impl Store {
    /// Adds somebody to one of the lists.
    ///
    /// Adding somebody already on it is not an error. A client that sends the same request twice,
    /// or two clients that send it at once, should both end with the person listed once; refusing
    /// the second would make a doubled request look like a failure.
    pub async fn add_to_list(&self, owner_id: i64, other_id: i64, kind: ListKind) -> Result<()> {
        if owner_id == other_id {
            return Err(StoreError::Refused("you cannot list yourself"));
        }

        sqlx::query(
            "INSERT INTO account_list (owner_id, other_id, kind) VALUES ($1, $2, $3)
             ON CONFLICT (owner_id, other_id, kind) DO NOTHING",
        )
        .bind(owner_id)
        .bind(other_id)
        .bind(kind.number())
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// Removes somebody from one of the lists.
    pub async fn remove_from_list(
        &self,
        owner_id: i64,
        other_id: i64,
        kind: ListKind,
    ) -> Result<()> {
        let removed = sqlx::query(
            "DELETE FROM account_list WHERE owner_id = $1 AND other_id = $2 AND kind = $3",
        )
        .bind(owner_id)
        .bind(other_id)
        .bind(kind.number())
        .execute(self.pool())
        .await?;

        if removed.rows_affected() == 0 {
            return Err(StoreError::Refused("they are not on that list"));
        }

        Ok(())
    }

    /// Whether somebody is on one of an account's lists.
    pub async fn is_listed(&self, owner_id: i64, other_id: i64, kind: ListKind) -> Result<bool> {
        let found = sqlx::query_as::<_, (i64,)>(
            "SELECT 1 FROM account_list WHERE owner_id = $1 AND other_id = $2 AND kind = $3",
        )
        .bind(owner_id)
        .bind(other_id)
        .bind(kind.number())
        .fetch_optional(self.pool())
        .await?;

        Ok(found.is_some())
    }

    /// Everybody on one of an account's lists, by name.
    pub async fn listed(&self, owner_id: i64, kind: ListKind) -> Result<Vec<(i64, String)>> {
        let rows = sqlx::query_as::<_, (i64, String)>(
            "SELECT account.id, account.name FROM account_list
             JOIN account ON account.id = account_list.other_id
             WHERE account_list.owner_id = $1 AND account_list.kind = $2
             ORDER BY account.name",
        )
        .bind(owner_id)
        .bind(kind.number())
        .fetch_all(self.pool())
        .await?;

        Ok(rows)
    }
}
