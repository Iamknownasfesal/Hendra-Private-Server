//! The lock one account plays under.
//!
//! Follows the original's `Database.AcquireLock` / `RenewLock` / `ReleaseLock`
//! (`common/Database.cs:159-215`), which keeps a key per account with a sixty-second expiry and a
//! token naming its holder. Taken as a player connects (`realm/ConnectManager.cs:202`), renewed on
//! every ping (`player/Player.KeepAlive.cs:118`), released as the connection is saved and closed
//! (`networking/Client.cs:226`, `:236`).
//!
//! # Why it expires
//!
//! A held lock with no expiry is an account nobody can play after a crash. Sixty seconds is the
//! original's `_lockTTL`, and the renewal interval has to be well inside it: a session that is
//! alive renews long before the lock it holds could lapse, and one that is gone stops renewing and
//! frees the account within the minute.

use crate::{Result, Store};

/// How long a lock outlives the session holding it, in seconds.
///
/// `Database._lockTTL` (`common/Database.cs:18`).
pub const LOCK_SECONDS: i32 = 60;

/// A lock somebody is holding, and the token that proves it is theirs.
///
/// Carried by the session rather than looked up, because every operation on a lock has to name the
/// holder: a session that has already been taken over would otherwise renew or release a lock that
/// now belongs to somebody else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountLock {
    pub account_id: i64,
    pub token: uuid::Uuid,
}

impl Store {
    /// Takes the lock on an account, or reports that somebody else holds it.
    ///
    /// One statement, so two sessions racing for the same account cannot both be told they have it.
    /// An expired row is taken over rather than refused — that is a lock whose holder is gone — and
    /// a live one is left alone.
    pub async fn acquire_lock(&self, account_id: i64) -> Result<Option<AccountLock>> {
        let token = uuid::Uuid::new_v4();

        let taken = sqlx::query(
            "INSERT INTO account_lock (account_id, token, expires_at)
             VALUES ($1, $2, now() + ($3 * interval '1 second'))
             ON CONFLICT (account_id) DO UPDATE
                SET token = EXCLUDED.token, expires_at = EXCLUDED.expires_at
              WHERE account_lock.expires_at <= now()",
        )
        .bind(account_id)
        .bind(token)
        .bind(LOCK_SECONDS)
        .execute(self.pool())
        .await?;

        Ok((taken.rows_affected() == 1).then_some(AccountLock { account_id, token }))
    }

    /// Pushes the expiry out, and says whether the lock was still this session's to push.
    ///
    /// A false answer means the account was taken over while this session was playing, which is
    /// what the original disconnects on (`player/Player.KeepAlive.cs:118-119`).
    pub async fn renew_lock(&self, lock: &AccountLock) -> Result<bool> {
        let renewed = sqlx::query(
            "UPDATE account_lock SET expires_at = now() + ($3 * interval '1 second')
             WHERE account_id = $1 AND token = $2",
        )
        .bind(lock.account_id)
        .bind(lock.token)
        .bind(LOCK_SECONDS)
        .execute(self.pool())
        .await?;

        Ok(renewed.rows_affected() == 1)
    }

    /// Gives up a lock, if it is still the one this session took.
    ///
    /// Conditional on the token, so a session that was taken over and is only now finishing its
    /// shutdown cannot unlock the session that replaced it.
    pub async fn release_lock(&self, lock: &AccountLock) -> Result<()> {
        sqlx::query("DELETE FROM account_lock WHERE account_id = $1 AND token = $2")
            .bind(lock.account_id)
            .bind(lock.token)
            .execute(self.pool())
            .await?;

        Ok(())
    }

    /// How many seconds until an account's lock lapses, or zero if nobody holds one.
    ///
    /// What the original puts in front of whoever was refused: "Account in Use (N seconds until
    /// timeout)" (`realm/ConnectManager.cs:213-214`). A number somebody can wait out is the
    /// difference between a refusal and a wall.
    pub async fn lock_seconds_left(&self, account_id: i64) -> Result<i64> {
        let left = sqlx::query_as::<_, (Option<f64>,)>(
            // Cast, because `EXTRACT` answers in `numeric` and the number wanted here is a count of
            // seconds to put in front of somebody, not an exact decimal.
            "SELECT EXTRACT(EPOCH FROM (expires_at - now()))::double precision FROM account_lock
             WHERE account_id = $1",
        )
        .bind(account_id)
        .fetch_optional(self.pool())
        .await?;

        Ok(left
            .and_then(|(seconds,)| seconds)
            .map(|seconds| seconds.ceil().max(0.0) as i64)
            .unwrap_or(0))
    }
}
