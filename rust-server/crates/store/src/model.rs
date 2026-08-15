//! Accounts and characters.

use crate::{AccountLock, Result, Store, StoreError};

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

    /// The fame this account has ever earned, which spending never reduces.
    ///
    /// `Database.UpdateFame` (`common/Database.cs:812-833`) raises this only when the amount is
    /// positive and applies every amount to [`fame`](Self::fame), so the pair reads as "earned" and
    /// "left". The character list reports them as `<TotalFame>` and `<Fame>`.
    pub total_fame: i32,

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

/// How far an account is trusted, as a number.
///
/// A ladder rather than a handful of tiers, because the original's is one:
/// `Command.HasPermission` compares the account's rank against the level the command was declared
/// with, and those levels run 0, 8, 10, 40, 80, 90, 95, 100 across the fifty-four ranked commands.
/// Collapsing them makes handing out an item as trusted as stamping a setpiece into a live world,
/// which are not the same amount of damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Admin(pub i16);

impl Admin {
    /// An ordinary player.
    pub const NONE: Admin = Admin(0);

    /// A supporter: cosmetics and the donor shop.
    pub const SUPPORTER: Admin = Admin(10);

    /// Trusted with items.
    pub const TESTER: Admin = Admin(40);

    /// Trusted with the players in a world: kick, mute, ban.
    pub const MODERATOR: Admin = Admin(80);

    /// Trusted to put things into a world that were not there.
    pub const CONTENT: Admin = Admin(90);

    /// Trusted with the ranks of others.
    pub const GUILD_MASTER: Admin = Admin(95);

    /// Trusted with the map itself.
    pub const OWNER: Admin = Admin(100);

    pub fn from_number(number: i16) -> Admin {
        Admin(number.clamp(0, 100))
    }

    pub fn rank(self) -> i16 {
        self.0
    }

    /// Whether this rank meets a command's declared level.
    pub fn meets(self, needed: Admin) -> bool {
        self >= needed
    }

    pub fn may_mute(self) -> bool {
        self.meets(Admin::MODERATOR)
    }

    pub fn may_ban(self) -> bool {
        self.meets(Admin::MODERATOR)
    }
}

/// The names an account is handed before it picks one.
///
/// `Database.GuestNames` (`common/Database.cs:62-75`), used verbatim. They are reserved rather than
/// decorative: `setName` refuses them, which is what lets a name from this list mean "this account
/// has not picked one yet" instead of being a name somebody happens to have.
const GUEST_NAMES: [&str; 45] = [
    "Darq", "Deyst", "Drac", "Drol", "Eango", "Eashy", "Eati", "Eendi", "Ehoni", "Gharr", "Iatho",
    "Iawa", "Idrae", "Iri", "Issz", "Itani", "Laen", "Lauk", "Lorz", "Oalei", "Odaru", "Oeti",
    "Orothi", "Oshyu", "Queq", "Radph", "Rayr", "Ril", "Rilr", "Risrr", "Saylt", "Scheev", "Sek",
    "Serl", "Seus", "Tal", "Tiar", "Uoro", "Urake", "Utanu", "Vorck", "Vorv", "Yangu", "Yimi",
    "Zhiar",
];

/// Whether a name is one of the reserved ones, and so not a name anybody chose.
///
/// This is what `Account.NameChosen` amounts to here, and it is asked from two places that cannot
/// see each other: the app charges for a rename only when the current name was never chosen, and
/// the world server colours a name over a head by the same test (`Player.getNameColor`,
/// `Player.as:757-764`). It lives beside [`Account`] so that both ask one list.
///
/// Trailing digits are ignored, because a reserved name already taken is handed out with a number
/// after it. A chosen name can never collide: `setName` takes letters only.
pub fn is_guest_name(name: &str) -> bool {
    let stem = name.trim_end_matches(|character: char| character.is_ascii_digit());
    GUEST_NAMES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
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

    /// The column holding what has ever been earned of this currency, where one is kept.
    ///
    /// `Database.UpdateFame` and `Database.UpdateCredit` (`common/Database.cs:787-833`) each keep a
    /// running total beside the balance and raise it only on a credit. Tokens have no such total
    /// here, and gold's is not a column we hold, so those report `None` and are simply added to.
    fn lifetime_column(self) -> Option<&'static str> {
        match self {
            Currency::Fame => Some("total_fame"),
            Currency::Prestige => Some("total_prestige"),
            Currency::Gold | Currency::Tokens => None,
        }
    }
}

/// A character, with everything needed to put it into a world.
#[derive(Debug, Clone, PartialEq)]
pub struct Character {
    pub id: i64,
    pub account_id: i64,
    pub class: uuid::Uuid,

    /// The account's name, read alongside the character rather than stored on it.
    ///
    /// `realm/entities/player/Player.cs:423` is `Name = client.Account.Name;` and `DbChar` holds
    /// no name of its own, so every character an account owns wears the same name and a rename
    /// reaches all of them at once.
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

    /// Whether this character has been given a backpack, which is eight more carried slots.
    pub has_backpack: bool,

    /// The eight base stats levelling has produced, one slot per stat.
    ///
    /// A slot is `None` where the row has no record of that stat, which is the state every
    /// character that predates the column is left in: the two maxima are recoverable from their own
    /// columns and the other six are not. Whoever reads this has the class descriptor and fills the
    /// gaps from it; the column deliberately does not guess. An entirely empty vector means the
    /// same thing for all eight.
    pub stats: Vec<Option<i32>>,

    /// The skin this character wears, or zero for the class's own sprite.
    ///
    /// Held on the character rather than the account, as `DbChar.Skin` is
    /// (`common/DbModels.cs:696`): a skin is chosen per character, while owning one is an account's
    /// business.
    pub skin: i32,

    /// The two dyes, which the original calls `Tex1` and `Tex2` (`common/DbModels.cs:684-690`).
    pub dye_cloth: i32,
    pub dye_accessory: i32,
}

/// Enough to draw a character-select screen without loading inventories.
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterSummary {
    pub id: i64,
    pub class: uuid::Uuid,

    /// The account's name, which is the only name a character has.
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
                i32,
            ),>(
            "INSERT INTO account (name) VALUES ($1)
             RETURNING id, name, vault_chests, banned, password_hash, gold, fame, tokens, muted_until, admin_rank, credits, discord_id, total_fame",
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
                total_fame,
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
                total_fame,
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
                i32,
            ),
        >(
            "SELECT id, name, vault_chests, banned, password_hash, gold, fame, tokens, muted_until, admin_rank, credits, discord_id, total_fame
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
                total_fame,
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
                    total_fame,
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
                i32,
            ),
        >(
            "SELECT id, name, vault_chests, banned, password_hash, gold, fame, tokens, muted_until, admin_rank, credits, discord_id, total_fame
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
                total_fame,
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
                    total_fame,
                }
            },
        )
        .ok_or_else(|| StoreError::NoSuchAccount(id.to_string()))
    }

    /// Creates a character for an account.
    ///
    /// Takes no name: the character answers to the account's, which is read back with it.
    pub async fn create_character(
        &self,
        account_id: i64,
        class: uuid::Uuid,
        max_hp: i32,
    ) -> Result<Character> {
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO character (account_id, class, hp, max_hp)
             VALUES ($1, $2, $3, $3) RETURNING id",
        )
        .bind(account_id)
        .bind(class)
        .bind(max_hp)
        .fetch_one(self.pool())
        .await?;

        self.character(id).await
    }

    /// Loads a character and its inventory.
    ///
    /// The name comes from the account it belongs to, as `Player.cs:423` reads it: a character has
    /// no name of its own to have got out of date.
    ///
    /// `character.created_at` is deliberately not among the columns read. The original stores the
    /// same thing — `DbChar.CreateTime` (`common/DbModels.cs:708-712`), written once at creation
    /// (`common/Database.cs:1016`) — and equally never sends it: `Character.ToXml` ends at
    /// `HasBackpack`, and the AS3 client's `SavedCharacter.bornOn()` guards with `hasOwnProperty`
    /// and answers "Unknown" because no server it ever spoke to sent one. The column is kept for
    /// the same reason the original keeps its field, but reading it here would only tempt somebody
    /// to put it back on the wire.
    pub async fn character(&self, id: i64) -> Result<Character> {
        use sqlx::Row;

        let row = sqlx::query(
            "SELECT character.id, character.account_id, character.class, account.name,
                    character.hp, character.max_hp, character.mp, character.max_mp,
                    character.level, character.experience, character.fame, character.alive,
                    character.health_potions, character.magic_potions,
                    character.has_backpack, character.stats, character.skin,
                    character.dye_cloth, character.dye_accessory
             FROM character JOIN account ON account.id = character.account_id
             WHERE character.id = $1",
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
            id: row.try_get("id")?,
            account_id: row.try_get("account_id")?,
            class: row.try_get("class")?,
            name: row.try_get("name")?,
            hp: row.try_get("hp")?,
            max_hp: row.try_get("max_hp")?,
            mp: row.try_get("mp")?,
            max_mp: row.try_get("max_mp")?,
            level: row.try_get("level")?,
            experience: row.try_get("experience")?,
            fame: row.try_get("fame")?,
            alive: row.try_get("alive")?,
            health_potions: row.try_get("health_potions")?,
            magic_potions: row.try_get("magic_potions")?,
            has_backpack: row.try_get("has_backpack")?,

            // Read as an array of options rather than of numbers: a slot the row has no record of
            // is null, and decoding it as a number would turn "not recorded" into an error that
            // refuses the login of every character that predates the column.
            stats: row.try_get("stats")?,

            skin: row.try_get("skin")?,
            dye_cloth: row.try_get("dye_cloth")?,
            dye_accessory: row.try_get("dye_accessory")?,
            inventory,
        })
    }

    /// Every living character on an account.
    pub async fn characters(&self, account_id: i64) -> Result<Vec<CharacterSummary>> {
        let rows = sqlx::query_as::<_, (i64, uuid::Uuid, String, i16, i32, bool)>(
            "SELECT character.id, character.class, account.name,
                    character.level, character.fame, character.alive
             FROM character JOIN account ON account.id = character.account_id
             WHERE character.account_id = $1 AND character.alive ORDER BY character.id",
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
    /// Called at logout, at death, at every world change and at the periodic checkpoint.
    /// Deliberately does not touch the inventory: item movement has its own transactional path, and
    /// letting a checkpoint rewrite slots wholesale would be a way to undo a move that had already
    /// committed.
    ///
    /// The maxima are written alongside the current figures. They are not a separate fact about the
    /// character — `MaxHitPoints` and `MaxMagicPoints` are two of the eight stats — but the row
    /// holds them in their own columns, and a column left behind while the stat it mirrors grows is
    /// a level-twenty wizard whose row says a hundred health.
    ///
    /// Appearance is left alone for the same reason the inventory is. `Store::wear_skin` and
    /// `Store::set_dye` write those columns themselves, checking ownership in the same statement,
    /// and a checkpoint carrying a snapshot taken at login would undo a skin chosen since.
    ///
    /// `under` is the lock the writer is playing the account under, and the write happens only
    /// while that lock is still theirs. Answers whether it did. This is
    /// `Database.SaveCharacter`'s `lockAcc` (`common/Database.cs:1058-1069`), which puts
    /// `Condition.StringEqual($"lock:{acc.AccountId}", acc.LockToken)` on the transaction that
    /// writes the character: a session whose account was taken over while it was computing a
    /// snapshot no longer writes, and the figures the session that holds the account wrote stand.
    /// Without it the last write wins by arrival order rather than by who is playing, and a
    /// snapshot from before a handover lands after the save that followed it. `None` writes
    /// unconditionally, which is the original's `lockAcc: false`.
    pub async fn save_character(
        &self,
        id: i64,
        saved: &Saved,
        under: Option<&AccountLock>,
    ) -> Result<bool> {
        let wrote = sqlx::query(
            "UPDATE character
             SET max_hp = GREATEST(1, $4), max_mp = GREATEST(0, $5),
                 hp = LEAST(GREATEST(1, $2), GREATEST(1, $4)), mp = $3,
                 level = $6, experience = $7, fame = $8, stats = $9, last_seen = now()
             WHERE id = $1 AND (
                 $10::uuid IS NULL OR EXISTS (
                     SELECT 1 FROM account_lock
                      WHERE account_lock.account_id = character.account_id
                        AND account_lock.token = $10
                        AND account_lock.expires_at > now()))",
        )
        .bind(id)
        .bind(saved.hp)
        .bind(saved.mp)
        .bind(saved.max_hp)
        .bind(saved.max_mp)
        .bind(saved.level)
        .bind(saved.experience)
        .bind(saved.fame)
        .bind(saved.stats.as_slice())
        .bind(under.map(|held| held.token))
        .execute(self.pool())
        .await?;

        Ok(wrote.rows_affected() == 1)
    }

    /// Marks a character dead. The row stays, because the graveyard is part of the game.
    pub async fn kill_character(&self, id: i64) -> Result<()> {
        sqlx::query("UPDATE character SET alive = false, hp = 0, last_seen = now() WHERE id = $1")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Adds to what a character has done.
    ///
    /// Added rather than set, so a session that ends without saving loses what it did and not what
    /// every earlier session did. Written when a character leaves a world, which is the same moment
    /// its health and experience are.
    ///
    /// Held to the same lock as `Store::save_character`, and for the same reason: the original
    /// writes both out of one `DbChar` inside one conditional transaction
    /// (`common/Database.cs:1062-1066`), so counts from a session that has lost the account do not
    /// land on a row somebody else is now playing.
    pub async fn add_tally(
        &self,
        character_id: i64,
        tally: &TallyRow,
        under: Option<&AccountLock>,
    ) -> Result<bool> {
        let added = sqlx::query(
            "UPDATE character SET
                 shots = shots + $2,
                 shots_that_hit = shots_that_hit + $3,
                 abilities_used = abilities_used + $4,
                 tiles_seen = tiles_seen + $5,
                 teleports = teleports + $6,
                 potions_drunk = potions_drunk + $7,
                 monster_kills = monster_kills + $8,
                 god_kills = god_kills + $9,
                 cube_kills = cube_kills + $10,
                 oryx_kills = oryx_kills + $11,
                 quests_completed = quests_completed + $12,
                 level_up_assists = level_up_assists + $13,
                 dungeons_completed = dungeons_completed | $14
             WHERE id = $1 AND (
                 $15::uuid IS NULL OR EXISTS (
                     SELECT 1 FROM account_lock
                      WHERE account_lock.account_id = character.account_id
                        AND account_lock.token = $15
                        AND account_lock.expires_at > now()))",
        )
        .bind(character_id)
        .bind(tally.shots)
        .bind(tally.shots_that_hit)
        .bind(tally.abilities_used)
        .bind(tally.tiles_seen)
        .bind(tally.teleports)
        .bind(tally.potions_drunk)
        .bind(tally.monster_kills)
        .bind(tally.god_kills)
        .bind(tally.cube_kills)
        .bind(tally.oryx_kills)
        .bind(tally.quests_completed)
        .bind(tally.level_up_assists)
        .bind(tally.dungeons_completed)
        .bind(under.map(|held| held.token))
        .execute(self.pool())
        .await?;

        Ok(added.rows_affected() == 1)
    }

    /// Everything a character has done.
    pub async fn tally(&self, character_id: i64) -> Result<TallyRow> {
        let row = sqlx::query_as::<
            _,
            (
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i64,
            ),
        >(
            "SELECT shots, shots_that_hit, abilities_used, tiles_seen, teleports, potions_drunk,
                    monster_kills, god_kills, cube_kills, oryx_kills, quests_completed,
                    level_up_assists, dungeons_completed
             FROM character WHERE id = $1",
        )
        .bind(character_id)
        .fetch_optional(self.pool())
        .await?
        .ok_or(StoreError::NoSuchCharacter(character_id))?;

        Ok(TallyRow {
            shots: row.0,
            shots_that_hit: row.1,
            abilities_used: row.2,
            tiles_seen: row.3,
            teleports: row.4,
            potions_drunk: row.5,
            monster_kills: row.6,
            god_kills: row.7,
            cube_kills: row.8,
            oryx_kills: row.9,
            quests_completed: row.10,
            level_up_assists: row.11,
            dungeons_completed: row.12,
        })
    }

    /// The most fame any of an account's characters has ever finished with.
    ///
    /// What the first-born bonus is measured against: beating every character the account has had.
    pub async fn best_final_fame(&self, account_id: i64) -> Result<i32> {
        let best = sqlx::query_as::<_, (Option<i32>,)>(
            "SELECT max(final_fame) FROM death WHERE account_id = $1",
        )
        .bind(account_id)
        .fetch_one(self.pool())
        .await?;

        Ok(best.0.unwrap_or(0))
    }

    /// How many characters this account made before this one.
    ///
    /// What stands in for the original's per-account character id, which counts from zero and is
    /// what the ancestor bonus asks about: `character.CharId < 2` (`FameStats.cs:122`) means the
    /// first two characters an account ever made. Ours are numbered across the whole server, so the
    /// position has to be counted rather than read. Deleted characters still count, as they do
    /// there: the original's counter only ever goes up.
    pub async fn characters_made_before(&self, account_id: i64, character_id: i64) -> Result<i64> {
        let (made,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM character WHERE account_id = $1 AND id < $2")
                .bind(account_id)
                .bind(character_id)
                .fetch_one(self.pool())
                .await?;

        Ok(made)
    }

    /// Records a death, and marks the character dead, in one transaction.
    ///
    /// Both or neither. A character marked dead with no death recorded loses the only account of
    /// what happened to it, and a death recorded against a character still alive is a graveyard
    /// entry for somebody still playing.
    ///
    /// Refuses a second death for the same character. A character dies once, and a repeat is either
    /// a bug or two worlds both deciding they killed the same person; either way, recording it
    /// twice would put one character in the graveyard twice and pay its fame twice.
    ///
    /// `under` is the lock the writer plays the account under, and a death from a session that no
    /// longer holds it is refused. The original sets `character.Dead` and writes it through
    /// `Database.SaveCharacter` with the same condition on it (`common/Database.cs:1081`, `:1091`,
    /// `:1058-1069`), so a body that dies in a world after its session lost the account cannot bury
    /// the character somebody else is now playing.
    pub async fn record_death(&self, death: Death, under: Option<&AccountLock>) -> Result<i64> {
        let Death {
            account_id,
            character_id,
            killed_by,
            final_fame,
            first_born,
            bonuses,
        } = death;

        let mut transaction = self.pool().begin().await?;

        // The character is locked and read before anything is written, so two deaths racing on one
        // character resolve to one: the second finds it already dead and refuses.
        // The name is the account's, taken here as a copy on purpose: the graveyard remembers what
        // the character was called when it died, and a later rename does not reach back into it.
        let held = sqlx::query_as::<_, (String, uuid::Uuid, i16, bool, i64)>(
            "SELECT (SELECT name FROM account WHERE account.id = character.account_id),
                    class, level, alive, account_id
             FROM character WHERE id = $1 FOR UPDATE",
        )
        .bind(character_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::NoSuchCharacter(character_id))?;

        if held.4 != account_id {
            return Err(StoreError::Refused("that is not your character"));
        }
        if !held.3 {
            return Err(StoreError::Refused("that character is already dead"));
        }

        // Checked inside the transaction that holds the row, so the account cannot change hands
        // between the check and the burial.
        if let Some(playing) = under {
            let ours = sqlx::query_as::<_, (i32,)>(
                "SELECT 1 FROM account_lock
                  WHERE account_id = $1 AND token = $2 AND expires_at > now()",
            )
            .bind(account_id)
            .bind(playing.token)
            .fetch_optional(&mut *transaction)
            .await?;

            if ours.is_none() {
                return Err(StoreError::Refused(
                    "that account is being played elsewhere",
                ));
            }
        }

        // The fame column keeps what the character earned by living, and is not overwritten with
        // what its death came to. The original holds the two apart in the same way -- `FinalFame` is
        // a field of its own and `Fame` is left as it was (`Database.cs:1090`) -- and the death
        // screen needs both to say how much of a total was bonuses.
        sqlx::query("UPDATE character SET alive = false, hp = 0, last_seen = now() WHERE id = $1")
            .bind(character_id)
            .execute(&mut *transaction)
            .await?;

        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO death
                 (account_id, character_id, name, class, level, final_fame, killed_by, first_born,
                  bonuses)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING id",
        )
        .bind(account_id)
        .bind(character_id)
        .bind(&held.0)
        .bind(held.1)
        .bind(held.2)
        .bind(final_fame)
        .bind(&killed_by)
        .bind(first_born)
        .bind(write_bonuses(&bonuses))
        .fetch_one(&mut *transaction)
        .await?;

        // The fame a character finished with is the account's to keep, which is what makes a death
        // worth anything at all. It lands on both numbers: the balance it may spend and the
        // lifetime total that spending never touches, as `Database.UpdateFame` writes them.
        sqlx::query("UPDATE account SET fame = fame + $2, total_fame = total_fame + $2 WHERE id = $1")
            .bind(account_id)
            .bind(final_fame.max(0))
            .execute(&mut *transaction)
            .await?;

        // And the guild's, where there is one. Read inside the transaction, so a guild joined or
        // left between the death and the payment cannot take fame to the wrong place.
        let guild =
            sqlx::query_as::<_, (Option<i64>,)>("SELECT guild_id FROM account WHERE id = $1")
                .bind(account_id)
                .fetch_optional(&mut *transaction)
                .await?
                .and_then(|(guild,)| guild);

        if let Some(guild) = guild {
            sqlx::query("UPDATE guild SET fame = fame + $2 WHERE id = $1")
                .bind(guild)
                .bind(final_fame.max(0))
                .execute(&mut *transaction)
                .await?;
        }

        transaction.commit().await?;
        Ok(id)
    }

    /// Whether this account has ever had a character die.
    ///
    /// What decides the first-born bonus, which can only be earned once and so is remembered rather
    /// than recomputed from anything that could change.
    pub async fn has_died_before(&self, account_id: i64) -> Result<bool> {
        let found =
            sqlx::query_as::<_, (i64,)>("SELECT id FROM death WHERE account_id = $1 LIMIT 1")
                .bind(account_id)
                .fetch_optional(self.pool())
                .await?;

        Ok(found.is_some())
    }

    /// An account's graveyard, most recent first.
    pub async fn graveyard(&self, account_id: i64, limit: i64) -> Result<Vec<Departed>> {
        let rows = sqlx::query_as::<_, DeathRow>(
            "SELECT id, character_id, name, class, level, final_fame, killed_by, first_born, at,
                    bonuses
             FROM death WHERE account_id = $1 ORDER BY at DESC LIMIT $2",
        )
        .bind(account_id)
        .bind(limit.clamp(1, MOST_DEATHS_READ))
        .fetch_all(self.pool())
        .await?;

        Ok(rows.into_iter().map(departed).collect())
    }

    /// The most famous deaths on the server, which is what a leaderboard shows.
    pub async fn best_deaths(&self, limit: i64) -> Result<Vec<Departed>> {
        let rows = sqlx::query_as::<_, DeathRow>(
            "SELECT id, character_id, name, class, level, final_fame, killed_by, first_born, at,
                    bonuses
             FROM death ORDER BY final_fame DESC, at DESC LIMIT $1",
        )
        .bind(limit.clamp(1, MOST_DEATHS_READ))
        .fetch_all(self.pool())
        .await?;

        Ok(rows.into_iter().map(departed).collect())
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

        // Fame alone carries its lifetime total with it: `SetFameCommand`
        // (`wServer/realm/commands/RankedCommands.cs:2436-2438`) assigns `TotalFame` and `Fame` the
        // same number, while `SetPrestigeCommand` (`:2496`) leaves the prestige total alone.
        let lifetime = match currency {
            Currency::Fame => ", total_fame = $2",
            _ => "",
        };

        let changed = sqlx::query(&format!(
            "UPDATE account SET {column} = $2{lifetime} WHERE id = $1"
        ))
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
            .bind(rank.rank())
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

    /// Renames an account, charging fame for it, where both happen or neither does.
    ///
    /// `ChooseNameHandler` (`networking/handlers/ChooseNameHandler.cs:63-77`) charges an account
    /// that has already chosen a name five thousand fame to choose another, refusing when it cannot
    /// pay. It takes the fame first and renames after, and its rename is a `while (!RenameIGN(..))`
    /// loop that never gives up, so a name taken in between costs the fame and spins. One
    /// transaction instead: a refused rename leaves the fame where it was, so the total across the
    /// accounts is the same before and after whichever way it ends.
    ///
    /// A `price` of zero is the first naming, which is free there and here.
    pub async fn rename_account_for(&self, account_id: i64, name: &str, price: i32) -> Result<()> {
        self.rename_account_charging(account_id, name, price, "fame")
            .await
    }

    /// The same, charging credits.
    ///
    /// The original renames from two places and they charge differently: the world server's
    /// `ChooseNameHandler` takes fame, while the app engine's `account/setName.cs:38-40` takes a
    /// thousand credits. Two prices in two currencies for the same act, kept as they are because a
    /// player who paid one of them did not pay the other.
    pub async fn rename_account_for_credits(
        &self,
        account_id: i64,
        name: &str,
        price: i32,
    ) -> Result<()> {
        self.rename_account_charging(account_id, name, price, "credits")
            .await
    }

    /// Renames and charges together, out of whichever column was named.
    ///
    /// The column is a literal from the two callers above rather than anything a request supplies,
    /// which is what keeps it out of reach of the query it is pasted into.
    async fn rename_account_charging(
        &self,
        account_id: i64,
        name: &str,
        price: i32,
        column: &'static str,
    ) -> Result<()> {
        let mut transaction = self.pool().begin().await?;

        if price > 0 {
            let charged = sqlx::query(&format!(
                "UPDATE account SET {column} = {column} - $2 WHERE id = $1 AND {column} >= $2"
            ))
            .bind(account_id)
            .bind(price)
            .execute(&mut *transaction)
            .await?;

            if charged.rows_affected() == 0 {
                return Err(match column {
                    "credits" => StoreError::Refused("Not enough credits"),
                    _ => StoreError::Refused("Not enough fame"),
                });
            }
        }

        let renamed = sqlx::query("UPDATE account SET name = $2 WHERE id = $1")
            .bind(account_id)
            .bind(name)
            .execute(&mut *transaction)
            .await;

        match renamed {
            Ok(_) => {}
            Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
                return Err(StoreError::NameTaken);
            }
            Err(err) => return Err(err.into()),
        }

        transaction.commit().await?;
        Ok(())
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

    /// Adds to what an account can spend, and to what it has ever earned.
    ///
    /// Both halves of `Database.UpdateFame` (`common/Database.cs:812-833`) in one statement. The
    /// lifetime total moves only here, never in the debiting half, which is what makes it a
    /// lifetime total rather than a second copy of the balance.
    pub async fn credit(&self, account_id: i64, currency: Currency, amount: i32) -> Result<()> {
        if amount <= 0 {
            return Ok(());
        }

        let column = currency.column();
        let lifetime = currency
            .lifetime_column()
            .map(|total| format!(", {total} = {total} + $2"))
            .unwrap_or_default();

        sqlx::query(&format!(
            "UPDATE account SET {column} = {column} + $2{lifetime} WHERE id = $1"
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

    /// Empties a vault slot, refusing when it no longer holds what the caller believed.
    ///
    /// The condition is what makes two requests for the same potion resolve to one: the second finds
    /// the row already gone and is told so, rather than reading an empty slot and carrying on.
    pub async fn take_vault_slot(
        &self,
        account_id: i64,
        slot: i16,
        expected: uuid::Uuid,
    ) -> Result<()> {
        let taken =
            sqlx::query("DELETE FROM vault_slot WHERE account_id = $1 AND slot = $2 AND item = $3")
                .bind(account_id)
                .bind(slot)
                .bind(expected)
                .execute(self.pool())
                .await?;

        if taken.rows_affected() == 0 {
            return Err(StoreError::Refused(
                "that item is no longer where you left it",
            ));
        }

        Ok(())
    }

    /// Buys one more vault chest, charging the price in fame.
    ///
    /// Both in one transaction, as the original's `ClosedVaultChest.Buy` puts them — one redis
    /// `MULTI` carrying the `vaultCount` increment and the fame debit together
    /// (`wServer/realm/entities/vendors/ClosedVaultChest.cs:31-34`, `common/Database.cs:905-911`):
    /// if either half will not go through, no chest is added and nothing is charged. The ceiling and
    /// the balance are both conditions on the statements rather than values read first, so two
    /// clicks arriving together cannot both see the same coin or the same last chest. The original
    /// reads the balance beforehand instead (`SellableObject.cs:100-101`) and has no ceiling at all;
    /// what stops it there is that the entity you clicked leaves the world when it is bought
    /// (`wServer/realm/worlds/logic/Vault.cs:177`).
    ///
    /// Returns how many chests the account now owns.
    pub async fn buy_vault_chest(&self, account_id: i64, limit: i16, price: i32) -> Result<i16> {
        let mut transaction = self.pool().begin().await?;

        let charged =
            sqlx::query("UPDATE account SET fame = fame - $2 WHERE id = $1 AND fame >= $2")
                .bind(account_id)
                .bind(price.max(0))
                .execute(&mut *transaction)
                .await?;

        if charged.rows_affected() == 0 {
            return Err(StoreError::Refused("you do not have the fame for that"));
        }

        let chests: Option<(i16,)> = sqlx::query_as(
            "UPDATE account SET vault_chests = vault_chests + 1
             WHERE id = $1 AND vault_chests < $2
             RETURNING vault_chests",
        )
        .bind(account_id)
        .bind(limit)
        .fetch_optional(&mut *transaction)
        .await?;

        let Some((chests,)) = chests else {
            return Err(StoreError::Refused("no more vault chests are available"));
        };

        transaction.commit().await?;

        Ok(chests)
    }
}

/// What a character became, as one checkpoint writes it.
///
/// Grouped rather than passed as eight positional numbers, because six of them are integers of the
/// same width and swapping two of them is a mistake nothing would catch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Saved {
    pub hp: i32,
    pub mp: i32,

    /// The maxima the stats currently produce, equipment included, which is what the body is
    /// actually playing with.
    pub max_hp: i32,
    pub max_mp: i32,

    pub level: i16,
    pub experience: i32,
    pub fame: i32,

    /// The eight base stats, before equipment. Only the base layer persists; equipment and
    /// temporary boosts are rebuilt from what is worn.
    pub stats: [i32; 8],
}

/// What a character has done, as the database holds it.
///
/// The simulation's own tally is a different type on purpose: this one crosses a database and is
/// all i32 and a bitset, and that crate has no business knowing either.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TallyRow {
    pub shots: i32,
    pub shots_that_hit: i32,
    pub abilities_used: i32,
    pub tiles_seen: i32,
    pub teleports: i32,
    pub potions_drunk: i32,
    pub monster_kills: i32,
    pub god_kills: i32,
    pub cube_kills: i32,
    pub oryx_kills: i32,
    pub quests_completed: i32,
    pub level_up_assists: i32,
    pub dungeons_completed: i64,
}

/// One graveyard row as the database hands it over.
type DeathRow = (
    i64,
    i64,
    String,
    uuid::Uuid,
    i16,
    i32,
    String,
    bool,
    chrono::DateTime<chrono::Utc>,
    String,
);

fn departed(row: DeathRow) -> Departed {
    let (id, character_id, name, class, level, final_fame, killed_by, first_born, at, bonuses) =
        row;
    let bonuses = read_bonuses(&bonuses);

    Departed {
        id,
        character_id,
        name,
        class,
        level,
        final_fame,
        killed_by,
        first_born,
        at,
        bonuses,
    }
}

/// What a death is, as it goes into the graveyard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Death {
    pub account_id: i64,
    pub character_id: i64,
    pub killed_by: String,

    /// What the character finished with, after the bonuses.
    pub final_fame: i32,

    /// Whether this is the first character on the account ever to die.
    pub first_born: bool,

    /// The bonuses it earned, kept so a graveyard can say why a number is what it is.
    pub bonuses: Vec<Awarded>,
}

/// One bonus a death earned: what it was called, and what it paid.
///
/// The wording that goes with the name is the same for every death that earns it, so it is not kept
/// here; only what is particular to this death is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Awarded {
    pub name: String,
    pub fame: i32,
}

/// How the bonuses are held in the column: one `name: fame` line each.
///
/// Written and read in one place so the two halves cannot drift apart. A name never contains the
/// separator, so the last one in a line is the one that splits it.
fn write_bonuses(bonuses: &[Awarded]) -> String {
    bonuses
        .iter()
        .map(|bonus| format!("{}: {}", bonus.name, bonus.fame))
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_bonuses(text: &str) -> Vec<Awarded> {
    text.lines()
        .filter_map(|line| {
            let (name, fame) = line.rsplit_once(": ")?;
            Some(Awarded {
                name: name.to_string(),
                fame: fame.trim().parse().ok()?,
            })
        })
        .collect()
}

/// One row of a graveyard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Departed {
    pub id: i64,
    pub character_id: i64,
    pub name: String,
    pub class: uuid::Uuid,
    pub level: i16,
    pub final_fame: i32,
    pub killed_by: String,
    pub first_born: bool,
    pub at: chrono::DateTime<chrono::Utc>,

    /// The bonuses that made `final_fame` what it is, which is what a death screen itemises.
    pub bonuses: Vec<Awarded>,
}

/// How many graveyard rows one read may ask for.
///
/// A length a caller controls is bounded before anything is reserved.
const MOST_DEATHS_READ: i64 = 100;

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
    /// `starting` is the class's eight starting stats, which the character is put back to along
    /// with its level: `PrestigeHandler.cs:56-64` writes `pd.Stats[i].StartingValue` into
    /// `Player.Stats.Base` before saving. The two maxima are the first two of the eight, so the
    /// columns that mirror them go back with the rest.
    ///
    /// Returns how much prestige was earned.
    pub async fn prestige(
        &self,
        account_id: i64,
        character_id: i64,
        starting: &[i32; 8],
    ) -> Result<i32> {
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

        // Everything the fame bought goes with it, the stats levelling granted included. A
        // character that kept its level would be a character that could be prestiged again the
        // moment it earned the fame back, and one that kept its stats would be a level-one
        // character with a level-twenty body.
        sqlx::query(
            "UPDATE character
                SET fame = 0, experience = 0, level = 1, stats = $2,
                    max_hp = GREATEST(1, $3), max_mp = GREATEST(0, $4), hp = GREATEST(1, $3),
                    mp = $4
              WHERE id = $1",
        )
        .bind(character_id)
        .bind(starting.as_slice())
        .bind(starting[0])
        .bind(starting[1])
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

/// An administrator setting a character's numbers outright.
///
/// Written durably rather than to the body in the world, so it survives walking out. The world
/// reads them on the next arrival, which is why both say so.
impl Store {
    pub async fn set_level(&self, character_id: i64, level: i16) -> Result<()> {
        let changed = sqlx::query("UPDATE character SET level = $2 WHERE id = $1")
            .bind(character_id)
            .bind(level.clamp(1, 20))
            .execute(self.pool())
            .await?;

        if changed.rows_affected() == 0 {
            return Err(StoreError::NoSuchCharacter(character_id));
        }

        Ok(())
    }

    /// Sets a character's health and magic to the most its class allows.
    ///
    /// The eight stats live in the world rather than in a column, so what is stored is the two that
    /// are: everything else the world recomputes from the class when the character arrives.
    pub async fn max_stats(&self, character_id: i64) -> Result<()> {
        let changed = sqlx::query("UPDATE character SET hp = max_hp, mp = max_mp WHERE id = $1")
            .bind(character_id)
            .execute(self.pool())
            .await?;

        if changed.rows_affected() == 0 {
            return Err(StoreError::NoSuchCharacter(character_id));
        }

        Ok(())
    }
}

/// Addresses kept out, rather than accounts.
///
/// A different question from an account ban: that stops one person playing, and this stops whoever
/// is behind an address making another account and carrying on.
impl Store {
    pub async fn ban_address(&self, address: &str) -> Result<()> {
        let address = address.trim();
        if address.is_empty() {
            return Err(StoreError::Refused("that is not an address"));
        }

        sqlx::query(
            "INSERT INTO banned_address (address) VALUES ($1) ON CONFLICT (address) DO NOTHING",
        )
        .bind(address)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    pub async fn unban_address(&self, address: &str) -> Result<()> {
        sqlx::query("DELETE FROM banned_address WHERE address = $1")
            .bind(address.trim())
            .execute(self.pool())
            .await?;

        Ok(())
    }

    /// Whether an address is kept out.
    ///
    /// Asked at the handshake, before anything else is done with a connection, so a banned address
    /// costs one query rather than a login.
    pub async fn address_banned(&self, address: &str) -> Result<bool> {
        let found =
            sqlx::query_as::<_, (String,)>("SELECT address FROM banned_address WHERE address = $1")
                .bind(address.trim())
                .fetch_optional(self.pool())
                .await?;

        Ok(found.is_some())
    }
}

/// Setting one of a character's stored numbers by name.
impl Store {
    /// Sets a stat an administrator names.
    ///
    /// Only the two that are stored: the other six live in the world, recomputed from the class and
    /// what is worn every time the character arrives, so writing them here would change nothing and
    /// look as though it had.
    pub async fn set_stat(&self, character_id: i64, which: &str, amount: i32) -> Result<()> {
        let column = match which.trim().to_ascii_lowercase().as_str() {
            "hp" | "health" | "maxhitpoints" => "max_hp",
            "mp" | "magic" | "maxmagicpoints" => "max_mp",
            "fame" => "fame",
            "experience" | "xp" => "experience",
            _ => {
                return Err(StoreError::Refused(
                    "only hp, mp, fame and experience are stored",
                ));
            }
        };

        let changed = sqlx::query(&format!("UPDATE character SET {column} = $2 WHERE id = $1"))
            .bind(character_id)
            .bind(amount.max(0))
            .execute(self.pool())
            .await?;

        if changed.rows_affected() == 0 {
            return Err(StoreError::NoSuchCharacter(character_id));
        }

        Ok(())
    }
}
