//! Durable state: accounts, characters and the vault.
//!
//! # Where this sits
//!
//! Nothing here is read during a tick. Live state such as positions, health and projectiles belongs to a
//! world task in memory, and this is touched at boundaries: login, logout, death, and the periodic
//! checkpoint. A local round trip is a fraction of a millisecond against a 50 ms tick budget, and
//! it happens off the tick anyway.
//!
//! The exception is moving items, which is the one place where being wrong duplicates something.
//! That path is transactional and locks the rows it touches, because the read-validate-write
//! discipline that makes an in-memory move safe stops being enough the moment two processes can
//! attempt the same move at once.

pub mod email;
pub mod extras;
pub mod guild;
mod lock;
pub mod market;
mod model;
mod moves;
pub mod news;
pub mod social;
mod trade;
pub mod wardrobe;

pub use extras::{Offer as CreditOffer, Quest};
pub use guild::{Guild, MAX_GUILD_LEVEL, Member, Rank};
pub use lock::{AccountLock, LOCK_SECONDS};
pub use market::{Delivered, Listing, MarketPurchase, Withdrawal};
pub use model::{
    Account, Admin, Awarded, Character, CharacterSummary, Currency, Death, Departed, Saved,
    TallyRow, is_guest_name,
};
pub use moves::{Location, MoveOutcome, Placed, Purchase};
pub use news::{Calendar, News};
pub use social::{Friend, ListKind, Message};
pub use trade::{Offer, TradeOutcome};
pub use wardrobe::{Boost, DyeSlot, GIFT_SLOTS, Pet};

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migrating: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("no account named {0:?}")]
    NoSuchAccount(String),

    #[error("no character {0}")]
    NoSuchCharacter(i64),

    #[error("that name is already taken")]
    NameTaken,

    #[error("{0}")]
    Refused(&'static str),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// A connection to the durable store.
#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    /// Connects and brings the schema up to date.
    pub async fn connect(url: &str) -> Result<Store> {
        let pool = PgPoolOptions::new()
            // Sized for the number of things that talk to it at once, meaning sessions logging in
            // and out and the checkpoint task, not for the number of players, because players do not each
            // hold a connection.
            .max_connections(16)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(url)
            .await?;

        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Store { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Closes the pool, waiting for outstanding work.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}
