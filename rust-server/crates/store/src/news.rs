//! News, and the daily calendar.
//!
//! Both exist so a server can change what it says without a deployment, which is the only reason
//! either belongs in a database rather than a file.

use crate::{Result, Store, StoreError};

/// One item of news.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct News {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub in_game: bool,
}

/// How many days in a row an account has claimed, and whether today is still available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Calendar {
    pub streak: i64,
    pub claimed_today: bool,
}

/// The most news items one read may return.
pub const MAX_NEWS: i64 = 50;

impl Store {
    /// Posts an item of news.
    pub async fn post_news(&self, title: &str, body: &str, in_game: bool) -> Result<i64> {
        let title = title.trim();
        if title.is_empty() {
            return Err(StoreError::Refused("news needs a title"));
        }

        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO news (title, body, in_game) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(title)
        .bind(body.trim())
        .bind(in_game)
        .fetch_one(self.pool())
        .await?;

        Ok(id)
    }

    /// The most recent news, newest first.
    pub async fn news(&self, in_game: bool, limit: i64) -> Result<Vec<News>> {
        let rows = sqlx::query_as::<_, (i64, String, String, bool)>(
            "SELECT id, title, body, in_game FROM news
             WHERE in_game = $1
             ORDER BY posted_at DESC
             LIMIT $2",
        )
        .bind(in_game)
        .bind(limit.clamp(1, MAX_NEWS))
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, title, body, in_game)| News {
                id,
                title,
                body,
                in_game,
            })
            .collect())
    }

    /// Claims today, if it has not been claimed.
    ///
    /// The date is the primary key, so claiming twice on one day is refused by the table rather
    /// than by a check that can race with itself.
    pub async fn claim_today(&self, account_id: i64) -> Result<bool> {
        let claimed = sqlx::query(
            "INSERT INTO daily_claim (account_id, claimed_on) VALUES ($1, current_date)
             ON CONFLICT (account_id, claimed_on) DO NOTHING",
        )
        .bind(account_id)
        .execute(self.pool())
        .await?;

        Ok(claimed.rows_affected() > 0)
    }

    /// How many days in a row have been claimed, counting back from today.
    ///
    /// Counted from the rows rather than kept as a number, because a counter has to be reset by
    /// something that notices a missed day, and nothing is watching while nobody is playing.
    pub async fn calendar(&self, account_id: i64) -> Result<Calendar> {
        let days: Vec<(chrono::NaiveDate,)> = sqlx::query_as(
            "SELECT claimed_on FROM daily_claim
             WHERE account_id = $1
             ORDER BY claimed_on DESC
             LIMIT 400",
        )
        .bind(account_id)
        .fetch_all(self.pool())
        .await?;

        let today = chrono::Utc::now().date_naive();
        let claimed_today = days.first().is_some_and(|(day,)| *day == today);

        // A streak may end today or yesterday: somebody who has not claimed yet today still has
        // yesterday's run, and telling them it is broken before the day is out would be wrong.
        let mut expected = if claimed_today {
            today
        } else {
            today.pred_opt().unwrap_or(today)
        };

        let mut streak = 0;
        for (day,) in &days {
            if *day != expected {
                break;
            }
            streak += 1;
            expected = expected.pred_opt().unwrap_or(expected);
        }

        Ok(Calendar {
            streak,
            claimed_today,
        })
    }
}
