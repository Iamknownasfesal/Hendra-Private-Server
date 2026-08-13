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
