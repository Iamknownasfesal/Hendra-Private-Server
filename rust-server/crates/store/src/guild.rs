//! Guilds, and who may do what in them.
//!
//! # Why membership is a column
//!
//! An account is in one guild or none. A join table would allow a state the game has no meaning
//! for, and every read would have to rule it out; a nullable column cannot represent the thing
//! that cannot happen.
//!
//! # Ranks
//!
//! Four, ordered, and higher ranks may do everything a lower one may. Held as a number rather than
//! a set of permissions because that is what the ordering is for: asking "may they" is a
//! comparison rather than a lookup.

use crate::{Result, Store, StoreError};

/// What a member may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i16)]
pub enum Rank {
    Initiate = 0,
    Member = 10,
    Officer = 20,
    Founder = 40,
}

impl Rank {
    pub fn from_number(number: i16) -> Rank {
        match number {
            n if n >= Rank::Founder as i16 => Rank::Founder,
            n if n >= Rank::Officer as i16 => Rank::Officer,
            n if n >= Rank::Member as i16 => Rank::Member,
            _ => Rank::Initiate,
        }
    }

    pub fn number(self) -> i16 {
        self as i16
    }

    /// Whether this rank may invite and remove people.
    pub fn may_manage(self) -> bool {
        self >= Rank::Officer
    }

    /// Whether this rank may change the board.
    pub fn may_set_board(self) -> bool {
        self >= Rank::Officer
    }

    /// Whether this rank may promote and demote.
    pub fn may_rank(self) -> bool {
        self >= Rank::Founder
    }
}

/// A guild, as a member sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guild {
    pub id: i64,
    pub name: String,
    pub board: String,
    pub fame: i32,
}

/// One member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub account_id: i64,
    pub name: String,
    pub rank: Rank,
}

/// The longest board a guild may set.
pub const MAX_BOARD_LENGTH: usize = 512;

/// The longest name a guild may have.
pub const MAX_NAME_LENGTH: usize = 32;

impl Store {
    /// Founds a guild, with its creator as founder.
    ///
    /// Both in one transaction: a guild with no founder is one nobody can ever manage, and it
    /// cannot be repaired from outside because managing is what founding grants.
    pub async fn found_guild(&self, account_id: i64, name: &str) -> Result<Guild> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_LENGTH {
            return Err(StoreError::Refused(
                "a guild name must be between one and thirty-two characters",
            ));
        }

        let mut transaction = self.pool().begin().await?;

        // Refused rather than moved. Leaving one guild to found another is two decisions, and
        // doing both silently is how someone leaves a guild they meant to keep.
        let (existing,): (Option<i64>,) =
            sqlx::query_as("SELECT guild_id FROM account WHERE id = $1 FOR UPDATE")
                .bind(account_id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or(StoreError::NoSuchAccount(account_id.to_string()))?;

        if existing.is_some() {
            return Err(StoreError::Refused("you are already in a guild"));
        }

        let created =
            sqlx::query_as::<_, (i64,)>("INSERT INTO guild (name) VALUES ($1) RETURNING id")
                .bind(name)
                .fetch_one(&mut *transaction)
                .await;

        let (id,) = match created {
            Ok(row) => row,
            Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
                return Err(StoreError::Refused("that guild name is taken"));
            }
            Err(err) => return Err(err.into()),
        };

        sqlx::query("UPDATE account SET guild_id = $2, guild_rank = $3 WHERE id = $1")
            .bind(account_id)
            .bind(id)
            .bind(Rank::Founder.number())
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;

        Ok(Guild {
            id,
            name: name.to_string(),
            board: String::new(),
            fame: 0,
        })
    }

    /// Adds somebody to a guild.
    pub async fn join_guild(&self, account_id: i64, guild_id: i64) -> Result<()> {
        let joined = sqlx::query(
            "UPDATE account SET guild_id = $2, guild_rank = $3
             WHERE id = $1 AND guild_id IS NULL",
        )
        .bind(account_id)
        .bind(guild_id)
        .bind(Rank::Initiate.number())
        .execute(self.pool())
        .await?;

        if joined.rows_affected() == 0 {
            return Err(StoreError::Refused("they are already in a guild"));
        }
        Ok(())
    }

    /// Removes somebody from their guild.
    pub async fn leave_guild(&self, account_id: i64) -> Result<()> {
        sqlx::query("UPDATE account SET guild_id = NULL, guild_rank = 0 WHERE id = $1")
            .bind(account_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// A guild by id.
    pub async fn guild(&self, guild_id: i64) -> Result<Guild> {
        let row = sqlx::query_as::<_, (i64, String, String, i32)>(
            "SELECT id, name, board, fame FROM guild WHERE id = $1",
        )
        .bind(guild_id)
        .fetch_optional(self.pool())
        .await?;

        row.map(|(id, name, board, fame)| Guild {
            id,
            name,
            board,
            fame,
        })
        .ok_or(StoreError::Refused("there is no such guild"))
    }

    /// Everyone in a guild, highest rank first.
    pub async fn guild_members(&self, guild_id: i64) -> Result<Vec<Member>> {
        let rows = sqlx::query_as::<_, (i64, String, i16)>(
            "SELECT id, name, guild_rank FROM account
             WHERE guild_id = $1
             ORDER BY guild_rank DESC, name",
        )
        .bind(guild_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(account_id, name, rank)| Member {
                account_id,
                name,
                rank: Rank::from_number(rank),
            })
            .collect())
    }

    /// Changes somebody's rank, if the one asking outranks them both before and after.
    ///
    /// Both, because promoting somebody above yourself is how a guild loses its founder, and
    /// demoting somebody who outranks you is how it loses one to a mutiny.
    pub async fn set_guild_rank(&self, actor: i64, target: i64, rank: Rank) -> Result<()> {
        let mut transaction = self.pool().begin().await?;

        let (actor_guild, actor_rank): (Option<i64>, i16) =
            sqlx::query_as("SELECT guild_id, guild_rank FROM account WHERE id = $1 FOR UPDATE")
                .bind(actor)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or(StoreError::NoSuchAccount(actor.to_string()))?;

        let (target_guild, target_rank): (Option<i64>, i16) =
            sqlx::query_as("SELECT guild_id, guild_rank FROM account WHERE id = $1 FOR UPDATE")
                .bind(target)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or(StoreError::NoSuchAccount(target.to_string()))?;

        let actor_rank = Rank::from_number(actor_rank);
        if actor_guild.is_none() || actor_guild != target_guild {
            return Err(StoreError::Refused("they are not in your guild"));
        }
        if !actor_rank.may_rank() {
            return Err(StoreError::Refused("you cannot change ranks"));
        }
        if Rank::from_number(target_rank) >= actor_rank || rank >= actor_rank {
            return Err(StoreError::Refused(
                "you cannot rank somebody at or above yourself",
            ));
        }

        sqlx::query("UPDATE account SET guild_rank = $2 WHERE id = $1")
            .bind(target)
            .bind(rank.number())
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        Ok(())
    }

    /// Sets the board, if the one asking may.
    pub async fn set_guild_board(&self, actor: i64, board: &str) -> Result<()> {
        let board: String = board.trim().chars().take(MAX_BOARD_LENGTH).collect();

        let updated = sqlx::query(
            "UPDATE guild SET board = $2
             WHERE id = (SELECT guild_id FROM account WHERE id = $1 AND guild_rank >= $3)",
        )
        .bind(actor)
        .bind(&board)
        .bind(Rank::Officer.number())
        .execute(self.pool())
        .await?;

        if updated.rows_affected() == 0 {
            return Err(StoreError::Refused("you cannot change the board"));
        }
        Ok(())
    }
}

impl Store {
    /// Which guild an account is in, and at what rank.
    pub async fn guild_of(&self, account_id: i64) -> Result<Option<(i64, Rank)>> {
        let found = sqlx::query_as::<_, (Option<i64>, i16)>(
            "SELECT guild_id, guild_rank FROM account WHERE id = $1",
        )
        .bind(account_id)
        .fetch_optional(self.pool())
        .await?;

        Ok(found.and_then(|(guild, rank)| guild.map(|guild| (guild, Rank::from_number(rank)))))
    }

    /// Puts somebody into the caller's guild.
    ///
    /// Officers and above, as in the original, which gates it at rank twenty. The check is against
    /// the caller's stored rank rather than anything the client says, and the target must be in no
    /// guild: joining somebody who is already in one is how a guild takes a member from another.
    pub async fn invite_to_guild(&self, actor: i64, target: i64) -> Result<i64> {
        let mut transaction = self.pool().begin().await?;

        // Both rows locked before either is read, so two officers inviting the same person cannot
        // both find them guildless.
        let (low, high) = if actor <= target {
            (actor, target)
        } else {
            (target, actor)
        };
        for account in [low, high] {
            sqlx::query("SELECT id FROM account WHERE id = $1 FOR UPDATE")
                .bind(account)
                .execute(&mut *transaction)
                .await?;
        }

        let mine = sqlx::query_as::<_, (Option<i64>, i16)>(
            "SELECT guild_id, guild_rank FROM account WHERE id = $1",
        )
        .bind(actor)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::Refused("no such account"))?;

        let Some(guild) = mine.0 else {
            return Err(StoreError::Refused("you are not in a guild"));
        };
        if Rank::from_number(mine.1) < Rank::Officer {
            return Err(StoreError::Refused("insufficient privileges"));
        }

        let joined = sqlx::query(
            "UPDATE account SET guild_id = $2, guild_rank = $3
             WHERE id = $1 AND guild_id IS NULL",
        )
        .bind(target)
        .bind(guild)
        .bind(Rank::Initiate.number())
        .execute(&mut *transaction)
        .await?;

        if joined.rows_affected() == 0 {
            return Err(StoreError::Refused("they are already in a guild"));
        }

        transaction.commit().await?;
        Ok(guild)
    }

    /// Takes somebody out of a guild.
    ///
    /// Leaving is always allowed. Removing somebody else needs a higher rank than theirs, which is
    /// what stops a member from throwing out the founder.
    pub async fn remove_from_guild(&self, actor: i64, target: i64) -> Result<()> {
        if actor == target {
            return self.leave_guild(actor).await;
        }

        let mut transaction = self.pool().begin().await?;

        let (low, high) = if actor <= target {
            (actor, target)
        } else {
            (target, actor)
        };
        for account in [low, high] {
            sqlx::query("SELECT id FROM account WHERE id = $1 FOR UPDATE")
                .bind(account)
                .execute(&mut *transaction)
                .await?;
        }

        let mine = sqlx::query_as::<_, (Option<i64>, i16)>(
            "SELECT guild_id, guild_rank FROM account WHERE id = $1",
        )
        .bind(actor)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::Refused("no such account"))?;

        let theirs = sqlx::query_as::<_, (Option<i64>, i16)>(
            "SELECT guild_id, guild_rank FROM account WHERE id = $1",
        )
        .bind(target)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::Refused("no such account"))?;

        if mine.0.is_none() || mine.0 != theirs.0 {
            return Err(StoreError::Refused("they are not in your guild"));
        }
        if Rank::from_number(mine.1) <= Rank::from_number(theirs.1) {
            return Err(StoreError::Refused("insufficient privileges"));
        }

        sqlx::query("UPDATE account SET guild_id = NULL, guild_rank = 0 WHERE id = $1")
            .bind(target)
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        Ok(())
    }
}
