//! What a player can type, and what it means.
//!
//! Follows `wServer/realm/commands/`, which splits them the same way: anybody may use the unranked
//! ones, and the ranked ones ask for a rank first.
//!
//! # Why this is a table rather than a match
//!
//! Because it has to be counted. Ninety commands went unnoticed for as long as nothing listed them,
//! and a `match` with ninety arms is not something anybody can compare against another server. A
//! table can be walked, printed and checked, and [`crate::commands::ALL`] is what
//! `cargo run -p hendra-server --example commands` prints.
//!
//! # The table is the original's, exactly
//!
//! Ninety commands under a hundred and ten names: fifty from `RankedCommands.cs`, forty from
//! `UnrankedCommands.cs`, and twenty of them answer to a second spelling. Five more are present in
//! the original's source and commented out — `resetFame`, `wipeServer`, `removeAllGold`,
//! `addWelcomeMessage`, `npe` — so they are not commands there and must not be here: typing one gets
//! `Unknown command!` from both servers.
//!
//! The rank beside each is the original's `permLevel`, as a number rather than a name, because the
//! ladder has rungs no name covers: `/unlink` sits on eight.
//!
//! # What a command is not
//!
//! A way around a rule. Every one of these goes through the same machinery the protocol does: `/tp`
//! is the world's teleport with all its refusals, `/trade` is the trade registry, `/ignore` is the
//! same list the whisper path reads. A command that reached past those would be a second, weaker
//! door into the same room.

use hendra_store::Admin;

/// What a command does, once the words have been read.
///
/// Named by intent rather than by handler, so two spellings of the same thing are one entry and the
/// session has one place to carry each out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Say something to everyone in the world.
    Say(String),

    /// Say something to one named player.
    Tell { to: String, text: String },

    /// Say something to everyone in the guild.
    GuildSay(String),

    /// Go to a named world.
    GoTo(&'static str),

    /// Move to a named player.
    TeleportTo(String),

    /// Ask a named player to trade.
    Trade(String),

    /// Add or remove somebody from one of the account's lists.
    List {
        kind: hendra_store::ListKind,
        name: String,
        add: bool,
    },

    /// Something to do with the caller's guild.
    Guild(GuildAction),

    /// Something to do with the market.
    Market(MarketAction),

    /// Tell the player something the server knows.
    Report(Report),

    /// Something a moderator or administrator does to the world in front of them.
    Wield { what: Wielded, rest: String },

    /// Something to do with a dungeon somebody opened.
    Dungeon(DungeonAction),

    /// Something a moderator or administrator does to somebody.
    Moderate {
        what: Moderation,
        name: String,
        rest: String,
    },
}

/// Something the server can tell a player about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Report {
    /// Where they are standing.
    Position,

    /// Which world they are in.
    World,

    /// Where their quest is.
    Quest,

    /// Who is in this world.
    Who,

    /// Who is on the server.
    Online,

    /// How long the server has been up.
    Uptime,

    /// Every command they may use.
    Commands,

    /// How much prestige they hold.
    Prestige,

    /// How far each stat is from its maximum.
    LeftToMax,

    /// What is playing.
    CurrentSong,

    /// The time, as the original answers it.
    Time,
}

/// What a player wants done about a dungeon somebody opened.
///
/// `/daccept` and `/dinvite` in the original, which are about entering a player-opened dungeon
/// rather than about duelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DungeonAction {
    /// Enter the world with this id, if invited.
    Accept(i32),

    /// Invite these players, or a guild with `-g`.
    Invite(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildAction {
    Invite(String),
    Join(String),
    Kick(String),

    /// Set somebody's guild rank, which in the original is staff tooling rather than a leader's.
    Rank(String, i32),
    Who,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketAction {
    /// List what is in an inventory slot, one-based as the original counts them.
    Sell { slot: i32, price: i32 },

    /// List every copy of a named item in the pack.
    SellAll { item: String, price: i32 },

    /// What the caller has listed.
    Mine,

    /// Take a listing back by its number.
    Remove(u32),

    /// Take back the last thing listed, which is what somebody who mistyped a price wants and
    /// cannot say any other way: they do not know the number.
    Oops,
}

/// Something an administrator does to the world rather than to an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wielded {
    Spawn,
    Pause,
    Spectate,
    Effect,
    Glow,
    Music,
    KillPlayer,
    Summon,
    SummonAll,
    OryxSay,
    Override,
    RemoveOverride,
    Link,
    Unlink,
    Setpiece,
    LootSpawn,
    ClearGraves,
    ClearSpawn,

    /// The starting gear of a named class, from `/Set`.
    Gear,
    Reskin,
    ToQuest,
    Warg,
    Debug,
    Reboot,

    /// Compact the large object heap, which is a thing only the original's runtime has.
    CompactLoh,
    Give,
    KillAll,
    MaxStats,
    MaxLevel,

    /// Go to the godlands, which is a place in a realm rather than a world of its own.
    Godlands,
    Size,
    Hide,
    ClearPack,
    Quake,
    CloseRealm,
    Visit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moderation {
    Ban,
    Unban,
    Mute,
    Unmute,
    Kick,
    Rank,
    SetFame,
    SetGold,
    SetPrestige,
    SetStar,
    Gift,
    Announce,
    Rename,
    Unname,

    /// Keep an address out, rather than an account.
    BanAddress,
}

/// One command: what it is called, what rank it asks for, and whether the help lists it.
pub struct Command {
    /// Spelled as the original spells it, capitals included: `killAll`, `myMarket`, `Set`.
    pub name: &'static str,

    /// The one other spelling it answers to, where the original gives it one.
    pub alias: Option<&'static str>,

    /// The original's `permLevel`, compared against the account's rank.
    pub rank: i16,

    /// Whether `/commands` names it. The original hides three.
    pub listed: bool,
}

impl Command {
    /// Whether a rank is enough to use this.
    pub fn allows(&self, rank: Admin) -> bool {
        rank.meets(Admin(self.rank))
    }
}

/// Every command the original has, with the rank and the alias it declares.
///
/// Grouped for reading; the help sorts them itself, as the original's does.
pub const ALL: &[Command] = &[
    // Talking.
    listed("tell", Some("t"), 0),
    listed("g", Some("guild"), 0),
    listed("l", None, 0),
    listed("commands", None, 0),
    // Going places.
    listed("nexus", None, 0),
    listed("realm", None, 0),
    listed("vault", None, 0),
    listed("ghall", None, 0),
    listed("marketplace", None, 0),
    listed("tutorial", None, 0),
    listed("tp", Some("teleport"), 0),
    listed("gland", Some("glands"), 0),
    listed("donorshop", None, 10),
    // People.
    listed("trade", None, 0),
    listed("ignore", None, 0),
    listed("unignore", None, 0),
    listed("lock", None, 0),
    listed("unlock", None, 0),
    listed("spectate", None, 0),
    // Guilds.
    listed("join", None, 0),
    listed("invite", Some("ginvite"), 0),
    listed("gkick", None, 0),
    listed("gwho", Some("mates"), 0),
    listed("grank", None, 95),
    // Dungeons somebody opened.
    listed("daccept", Some("da"), 0),
    listed("dinvite", Some("di"), 0),
    // The market.
    listed("market", None, 0),
    listed("marketall", Some("mall"), 0),
    listed("myMarket", None, 0),
    listed("rmarket", None, 0),
    listed("oops", None, 0),
    // Asking the server.
    listed("who", None, 0),
    listed("online", None, 0),
    listed("pos", Some("position"), 0),
    listed("uptime", None, 0),
    listed("ps", None, 0),
    listed("lefttomax", None, 0),
    listed("currentsong", Some("song"), 0),
    listed("time", None, 0),
    listed("world", None, 0),
    listed("pause", None, 0),
    // What a player may do to their own character.
    listed("level20", Some("l20"), 0),
    listed("Set", None, 0),
    listed("max", None, 40),
    listed("gimme", Some("give"), 40),
    listed("size", None, 10),
    listed("reskin", None, 10),
    listed("glow", None, 10),
    // Moderation.
    listed("mute", None, 80),
    listed("unmute", None, 80),
    listed("kick", None, 80),
    listed("ban", None, 80),
    listed("banip", Some("ipban"), 80),
    listed("unban", None, 80),
    listed("rename", None, 80),
    listed("unname", None, 80),
    listed("announce", None, 80),
    listed("summon", None, 80),
    listed("visit", None, 80),
    listed("hide", Some("h"), 80),
    listed("clearinv", None, 80),
    listed("clearspawn", Some("cs"), 80),
    listed("cleargraves", Some("cgraves"), 80),
    listed("rank", None, 90),
    listed("setfame", None, 90),
    listed("setgold", None, 90),
    listed("setprestige", None, 90),
    listed("gift", None, 90),
    listed("setstar", None, 100),
    listed("killPlayer", None, 100),
    // Tools for the world in front of them.
    listed("spawn", None, 90),
    listed("lootspawn", Some("ls"), 90),
    listed("killAll", Some("ka"), 90),
    listed("summonall", None, 90),
    listed("getQuest", None, 90),
    listed("oryxSay", Some("osay"), 90),
    listed("eff", None, 90),
    listed("music", None, 90),
    listed("closerealm", None, 90),
    listed("quake", None, 90),
    listed("link", None, 90),
    listed("reboot", None, 90),
    listed("unlink", None, 8),
    listed("setpiece", None, 100),
    listed("tq", None, 100),
    listed("warg", None, 100),
    listed("override", None, 100),
    // The three the original's help does not name.
    Command {
        name: "removeOverride",
        alias: None,
        rank: 0,
        listed: false,
    },
    Command {
        name: "debug",
        alias: None,
        rank: 100,
        listed: false,
    },
    Command {
        name: "compactLOH",
        alias: None,
        rank: 100,
        listed: false,
    },
];

/// A command the help names, which is all but three of them.
const fn listed(name: &'static str, alias: Option<&'static str>, rank: i16) -> Command {
    Command {
        name,
        alias,
        rank,
        listed: true,
    }
}

/// Finds a command by what was typed, whatever its capitals.
///
/// Case-insensitive because the original keys its dictionary with
/// `StringComparer.InvariantCultureIgnoreCase`, which is what makes `/SET` and `/set` the same
/// command as `/Set`.
pub fn find(name: &str) -> Option<&'static Command> {
    ALL.iter().find(|command| {
        command.name.eq_ignore_ascii_case(name)
            || command
                .alias
                .is_some_and(|alias| alias.eq_ignore_ascii_case(name))
    })
}

/// Reads a command and its words into something to do.
///
/// Returns the action, or the line the original answers with when the words do not make sense — a
/// refusal with more than one line in it is separated by a newline, as `/Set` and `/reskin` answer.
/// Reading is kept apart from doing so that what a command means can be checked without a world, a
/// store or a connection.
///
/// The words arrive exactly as the original hands them over: everything after the first space, with
/// nothing trimmed, so `/gimme  Sword` looks for an item whose name starts with a space there and
/// here alike.
pub fn read(name: &str, args: &str) -> Result<Action, String> {
    use hendra_store::ListKind;

    let command = find(name).ok_or_else(|| UNKNOWN.to_string())?;

    let action = match command.name {
        "tell" => {
            let Some((to, text)) = args.split_once(' ') else {
                return Err("Usage: /tell <player name> <text>".to_string());
            };
            Action::Tell {
                to: to.to_string(),
                text: text.to_string(),
            }
        }

        // Neither of these refuses an empty line: the original passes whatever was typed to the
        // chat manager, which answers for itself.
        "g" => Action::GuildSay(args.to_string()),
        "l" => Action::Say(args.to_string()),
        "commands" => Action::Report(Report::Commands),

        "nexus" => Action::GoTo(NEXUS),
        "realm" => Action::GoTo("Realm"),
        "vault" => Action::GoTo("Vault"),
        "ghall" => Action::GoTo(GUILD_HALL),
        "marketplace" => Action::GoTo("Marketplace"),
        "tutorial" => Action::GoTo(TUTORIAL),
        "donorshop" => Action::GoTo(DONOR_SHOP),

        // The godlands are a place in the realm rather than a world, so this is a teleport to a
        // fixed square and the only command that names one.
        "gland" => wield(Wielded::Godlands, ""),

        "tp" => Action::TeleportTo(args.to_string()),

        "trade" => {
            if args.trim().is_empty() {
                return Err("Usage: /trade <player name>".to_string());
            }
            Action::Trade(args.to_string())
        }

        "ignore" | "unignore" => {
            let add = command.name == "ignore";
            if args.is_empty() {
                return Err(format!("Usage: /{} <player name>", command.name));
            }
            Action::List {
                kind: ListKind::Ignored,
                name: args.to_string(),
                add,
            }
        }

        "lock" | "unlock" => {
            let add = command.name == "lock";
            if args.is_empty() {
                return Err(format!("Usage: /{} <player name>", command.name));
            }
            Action::List {
                kind: ListKind::LockedOut,
                name: args.to_string(),
                add,
            }
        }

        "spectate" => {
            if args.trim().is_empty() {
                return Err("Usage: /spectate <player name>".to_string());
            }
            wield(Wielded::Spectate, args)
        }

        "join" => Action::Guild(GuildAction::Join(args.to_string())),
        "invite" => Action::Guild(GuildAction::Invite(args.to_string())),
        "gkick" => Action::Guild(GuildAction::Kick(args.to_string())),
        "gwho" => Action::Guild(GuildAction::Who),

        "grank" => {
            let Some((who, rank)) = args.split_once(' ') else {
                return Err("Usage: /grank <player name> <guild rank>".to_string());
            };

            let rank = match rank.parse::<i32>() {
                Ok(number) => number,
                Err(_) => guild_rank_numbered(rank),
            };
            if rank == -1 {
                return Err("Unknown rank!".to_string());
            }
            if rank % 10 != 0 {
                return Err("Valid ranks are multiples of 10!".to_string());
            }

            Action::Guild(GuildAction::Rank(who.to_string(), rank))
        }

        "daccept" => {
            let Ok(id) = args.trim().parse::<i32>() else {
                return Err("ID must be a number.".to_string());
            };
            Action::Dungeon(DungeonAction::Accept(id))
        }
        "dinvite" => Action::Dungeon(DungeonAction::Invite(args.to_string())),

        "market" => {
            // `^(\d+) (\d+)$`, and the slot is one of the sixteen the pack has.
            let refused = "Usage: /market <slot> <amount>. Only slot numbers 1-16 are valid and \
                           amount must be a positive value.";
            let Some((slot, price)) = args.split_once(' ') else {
                return Err(refused.to_string());
            };
            let (Ok(slot), Ok(price)) = (slot.parse::<i32>(), price.parse::<i32>()) else {
                return Err(refused.to_string());
            };
            if !(1..=16).contains(&slot) {
                return Err(refused.to_string());
            }

            Action::Market(MarketAction::Sell { slot, price })
        }

        "marketall" => {
            let Some((item, price)) = args.rsplit_once(' ') else {
                return Err("Usage: /marketall <item name> <price>.".to_string());
            };
            let (Ok(price), true) = (
                price.parse::<i32>(),
                !item.is_empty()
                    && item
                        .chars()
                        .all(|held| held.is_ascii_alphanumeric() || held == ' '),
            ) else {
                return Err("Usage: /marketall <item name> <price>.".to_string());
            };

            Action::Market(MarketAction::SellAll {
                item: item.to_string(),
                price,
            })
        }

        "myMarket" => Action::Market(MarketAction::Mine),
        "oops" => Action::Market(MarketAction::Oops),
        "rmarket" => {
            let Ok(id) = args.parse::<u32>() else {
                return Err(
                    "Usage: /rmarket <id>. Ids for your listed items can be found with \
                            the /mymarket command."
                        .to_string(),
                );
            };
            Action::Market(MarketAction::Remove(id))
        }

        "who" => Action::Report(Report::Who),
        "online" => Action::Report(Report::Online),
        "pos" => Action::Report(Report::Position),
        "uptime" => Action::Report(Report::Uptime),
        "ps" => Action::Report(Report::Prestige),
        "lefttomax" => Action::Report(Report::LeftToMax),
        "currentsong" => Action::Report(Report::CurrentSong),
        "time" => Action::Report(Report::Time),
        "world" => Action::Report(Report::World),
        "getQuest" => Action::Report(Report::Quest),

        "pause" => wield(Wielded::Pause, ""),
        "level20" => wield(Wielded::MaxLevel, ""),
        "max" => wield(Wielded::MaxStats, ""),
        "gimme" => wield(Wielded::Give, args),
        "Set" => wield(Wielded::Gear, args),

        "size" => {
            if args.is_empty() {
                return Err(
                    "Usage: /size <positive integer>. Using 0 will restore the default \
                            size for the sprite."
                        .to_string(),
                );
            }
            wield(Wielded::Size, args)
        }

        // The choices are read from the content, so the refusal for an empty argument is the
        // session's rather than this one's.
        "reskin" => wield(Wielded::Reskin, args),

        "glow" => {
            if args.trim().is_empty() {
                return Err("Usage: /glow <color>".to_string());
            }
            wield(Wielded::Glow, args)
        }

        "hide" => wield(Wielded::Hide, ""),
        "clearinv" => wield(Wielded::ClearPack, ""),
        "clearspawn" => wield(Wielded::ClearSpawn, ""),
        "cleargraves" => wield(Wielded::ClearGraves, ""),
        "spawn" => wield(Wielded::Spawn, args),
        "lootspawn" => wield(Wielded::LootSpawn, args),
        "killAll" => wield(Wielded::KillAll, args),
        "summonall" => wield(Wielded::SummonAll, ""),
        "oryxSay" => wield(Wielded::OryxSay, args),
        "eff" => wield(Wielded::Effect, args),
        "music" => wield(Wielded::Music, args),
        "closerealm" => wield(Wielded::CloseRealm, ""),
        "quake" => wield(Wielded::Quake, args),
        "link" => wield(Wielded::Link, ""),
        "unlink" => wield(Wielded::Unlink, ""),
        "reboot" => wield(Wielded::Reboot, args),
        "setpiece" => wield(Wielded::Setpiece, args),
        "tq" => wield(Wielded::ToQuest, ""),
        "debug" => wield(Wielded::Debug, ""),
        "compactLOH" => wield(Wielded::CompactLoh, ""),

        "warg" => {
            if args.trim().is_empty() {
                return Err("Usage: /warg <mob name>".to_string());
            }
            wield(Wielded::Warg, args)
        }

        "override" => {
            if args.trim().is_empty() {
                return Err("Usage: /override <player name>".to_string());
            }
            wield(Wielded::Override, args)
        }
        "removeOverride" => wield(Wielded::RemoveOverride, ""),

        "visit" => {
            if args.trim().is_empty() {
                return Err("Usage: /visit <player name>".to_string());
            }
            wield(Wielded::Visit, args)
        }

        "summon" => wield(Wielded::Summon, args),
        "kick" => moderate(Moderation::Kick, args, ""),
        "killPlayer" => wield(Wielded::KillPlayer, args),

        "announce" => moderate(Moderation::Announce, "", args),

        "mute" => {
            // `^(\w+)( \d+)?$`: a name, and optionally how many minutes.
            let (who, minutes) = match args.split_once(' ') {
                Some((who, minutes)) => (who, minutes),
                None => (args, ""),
            };

            let named = !who.is_empty() && who.chars().all(is_word);
            let numbered = minutes.is_empty() || minutes.chars().all(|held| held.is_ascii_digit());
            if !named || !numbered {
                return Err(
                    "Usage: /mute <player name> <time out in minutes>\\nTime parameter \
                            is optional. If left out player will be muted until unmuted."
                        .to_string(),
                );
            }

            moderate(Moderation::Mute, who, minutes)
        }

        "unmute" => {
            if args.trim().is_empty() {
                return Err("Usage: /unmute <player name>".to_string());
            }
            moderate(Moderation::Unmute, args, "")
        }

        "ban" | "banip" => {
            let what = if command.name == "ban" {
                Moderation::Ban
            } else {
                Moderation::BanAddress
            };

            // `^(\w+) (.+)$`: who, and why.
            let refused = format!("Usage: /{} <account id or name> <reason>", command.name);
            let Some((who, why)) = args.split_once(' ') else {
                return Err(refused);
            };
            if who.is_empty() || !who.chars().all(is_word) || why.is_empty() {
                return Err(refused);
            }

            moderate(what, who, why)
        }

        "unban" => {
            if args.is_empty() || !args.chars().all(is_word) {
                return Err("Usage: /unban <account id or name>".to_string());
            }
            moderate(Moderation::Unban, args, "")
        }

        "rank" => {
            let Some((who, rank)) = args.split_once(' ') else {
                return Err(
                    "Usage: /rank <player name> <rank>\\n0: Normal Player, 20: Donor, \
                            70: Former Staff, 80: GM, 90: Dev, 100: Owner"
                        .to_string(),
                );
            };

            // `int.Parse` throws on anything else, and the original answers a thrown command with
            // one line: keeping that means keeping what it says.
            if rank.trim().parse::<i32>().is_err() {
                return Err(THREW.to_string());
            }

            moderate(Moderation::Rank, who, rank)
        }

        "rename" => {
            let Some((who, to)) = args.split_once(' ') else {
                return Err("Usage: /rename <player name> <new player name>".to_string());
            };
            moderate(Moderation::Rename, who, to)
        }

        "unname" => {
            if args.trim().is_empty() {
                return Err("Usage: /unname <player name>".to_string());
            }
            moderate(Moderation::Unname, args, "")
        }

        "gift" => {
            let Some((who, item)) = args.split_once(' ') else {
                return Err("Usage: /gift <player name> <item name>".to_string());
            };
            moderate(Moderation::Gift, who, item)
        }

        // Four that read a name and a number with `Substring` and `Int32.Parse`, neither of which
        // is guarded: the original throws on anything else and answers with one line.
        "setfame" | "setgold" | "setprestige" | "setstar" => {
            let what = match command.name {
                "setfame" => Moderation::SetFame,
                "setgold" => Moderation::SetGold,
                "setprestige" => Moderation::SetPrestige,
                _ => Moderation::SetStar,
            };

            let Some((who, amount)) = args.split_once(' ') else {
                return Err(THREW.to_string());
            };
            if amount.trim().parse::<i32>().is_err() {
                return Err(THREW.to_string());
            }

            moderate(what, who, amount)
        }

        // Every name in the table is answered above; a name that is not in the table never gets
        // here, since `find` refused it.
        other => unreachable!("/{other} is in the table and reads into nothing"),
    };

    Ok(action)
}

/// What the original says to a name it does not have.
pub const UNKNOWN: &str = "Unknown command!";

/// What the original says to somebody whose rank is too low.
pub const NO_PERMISSION: &str = "No permission!";

/// What the original says when a command throws.
pub const THREW: &str = "Error when executing the command.";

/// Where a player who escapes ends up, and where everybody arrives.
///
/// Named here because this is the file that has to name every world a command can reach, and one
/// place naming them all beats two places agreeing.
pub const NEXUS: &str = "Nexus";

/// Where `/tutorial` goes.
pub const TUTORIAL: &str = "Tutorial";

/// The guild hall, which is one room per guild rather than one per player.
pub const GUILD_HALL: &str = "GuildHall";

/// Where `/donorshop` goes.
pub const DONOR_SHOP: &str = "Donor Shop";

fn wield(what: Wielded, rest: &str) -> Action {
    Action::Wield {
        what,
        rest: rest.to_string(),
    }
}

fn moderate(what: Moderation, name: &str, rest: &str) -> Action {
    Action::Moderate {
        what,
        name: name.to_string(),
        rest: rest.to_string(),
    }
}

/// What `\w` matches, which is what the original's patterns are written with.
fn is_word(held: char) -> bool {
    held.is_alphanumeric() || held == '_'
}

/// The number behind a guild rank's name, or -1 for a name there is no rank for.
///
/// `GuildRankCommand.RankNumberFromName`, whose five names are the only ones it knows.
fn guild_rank_numbered(name: &str) -> i32 {
    match name.trim().to_ascii_lowercase().as_str() {
        "initiate" => 0,
        "member" => 10,
        "officer" => 20,
        "leader" => 30,
        "founder" => 40,
        _ => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_the_size_the_original_is() {
        // Ninety commands under a hundred and ten names, from `RankedCommands.cs` and
        // `UnrankedCommands.cs`: twenty of them declare an alias. A number that moves is a command
        // gained or lost.
        assert_eq!(ALL.len(), 90);
        assert_eq!(
            ALL.len() + ALL.iter().filter(|command| command.alias.is_some()).count(),
            110
        );
        assert_eq!(ALL.iter().filter(|command| !command.listed).count(), 3);
    }

    #[test]
    fn the_five_commented_out_ones_are_not_commands() {
        // Present in the original's source and commented out, so typing one gets `Unknown command!`
        // there. A server that answered them would be answering something the original does not.
        for name in [
            "resetFame",
            "wipeServer",
            "removeAllGold",
            "addWelcomeMessage",
            "npe",
        ] {
            assert!(find(name).is_none(), "/{name} exists here and not there");
            assert_eq!(read(name, ""), Err(UNKNOWN.to_string()));
        }
    }

    #[test]
    fn every_command_is_found_by_its_own_name() {
        for command in ALL {
            assert!(find(command.name).is_some(), "/{}", command.name);
            if let Some(alias) = command.alias {
                assert!(find(alias).is_some(), "/{alias}");
            }
        }
    }

    #[test]
    fn no_two_commands_answer_to_the_same_word() {
        // Two entries claiming one word means one of them silently never runs. The original would
        // throw at startup instead, since its dictionary refuses a duplicate key.
        let mut seen: Vec<&str> = Vec::new();

        for command in ALL {
            for word in std::iter::once(command.name).chain(command.alias) {
                assert!(
                    !seen.iter().any(|held| held.eq_ignore_ascii_case(word)),
                    "/{word} is claimed twice"
                );
                seen.push(word);
            }
        }
    }

    #[test]
    fn every_command_reads_into_something_or_says_why_not() {
        // A command in the table that reads into nothing whatever it is given is one that prints in
        // the help and then does nothing when it is typed.
        for command in ALL {
            let read_something = ["somebody 20", "3 20", "20", "wizard", ""]
                .iter()
                .any(|args| read(command.name, args).is_ok());

            assert!(read_something, "/{} reads into nothing", command.name);
        }
    }

    #[test]
    fn the_ranks_are_the_originals() {
        // The ones a player actually touches, and the two the original spells oddly.
        for (name, rank) in [
            ("level20", 0),
            ("Set", 0),
            ("removeOverride", 0),
            ("getQuest", 90),
            ("size", 10),
            ("reskin", 10),
            ("glow", 10),
            ("donorshop", 10),
            ("max", 40),
            ("gimme", 40),
            ("summon", 80),
            ("visit", 80),
            ("killAll", 90),
            ("reboot", 90),
            ("grank", 95),
            ("unlink", 8),
            ("setstar", 100),
            ("killPlayer", 100),
            ("override", 100),
        ] {
            assert_eq!(find(name).expect(name).rank, rank, "/{name}");
        }
    }

    #[test]
    fn a_rank_of_eight_is_a_rung_of_its_own() {
        // `/unlink` declares `permLevel: 8` while everything around it is a multiple of ten, so an
        // account ranked eight may unlink and may do nothing else.
        let unlink = find("unlink").expect("unlink");
        assert!(unlink.allows(Admin(8)));
        assert!(!unlink.allows(Admin(7)));
        assert!(!find("size").expect("size").allows(Admin(8)));
    }

    #[test]
    fn spelling_and_capitals_do_not_matter() {
        assert_eq!(read("TELL", "Bo hi"), read("tell", "Bo hi"));
        assert_eq!(read("set", "wizard"), read("Set", "wizard"));
        assert_eq!(read("KA", "slime"), read("killAll", "slime"));
    }

    #[test]
    fn a_whisper_needs_both_who_and_what() {
        assert_eq!(
            read("tell", "Fesal"),
            Err("Usage: /tell <player name> <text>".to_string())
        );
        assert_eq!(
            read("tell", "Fesal hello there"),
            Ok(Action::Tell {
                to: "Fesal".to_string(),
                text: "hello there".to_string()
            })
        );
    }

    #[test]
    fn an_announcement_is_the_one_that_names_nobody() {
        assert_eq!(
            read("announce", "the server restarts shortly"),
            Ok(Action::Moderate {
                what: Moderation::Announce,
                name: String::new(),
                rest: "the server restarts shortly".to_string(),
            })
        );
    }

    #[test]
    fn a_thrown_command_says_the_one_line_the_original_says() {
        // `/setfame Bob` throws inside the original on `Substring(0, -1)`, and every thrown command
        // is answered with the same line by `Command.Execute`.
        for typed in [
            "setfame Bob",
            "setgold Bob",
            "setprestige Bob",
            "setstar Bob",
        ] {
            let (name, args) = typed.split_once(' ').expect("a name and a word");
            assert_eq!(read(name, args), Err(THREW.to_string()), "/{typed}");
        }
        assert_eq!(read("rank", "Bob wizard"), Err(THREW.to_string()));
    }

    #[test]
    fn a_guild_rank_is_a_name_or_a_multiple_of_ten() {
        assert_eq!(
            read("grank", "Bob officer"),
            Ok(Action::Guild(GuildAction::Rank("Bob".to_string(), 20)))
        );
        assert_eq!(
            read("grank", "Bob emperor"),
            Err("Unknown rank!".to_string())
        );
        assert_eq!(
            read("grank", "Bob 15"),
            Err("Valid ranks are multiples of 10!".to_string())
        );
    }

    #[test]
    fn the_market_reads_a_slot_and_a_price() {
        assert_eq!(
            read("market", "3 100"),
            Ok(Action::Market(MarketAction::Sell {
                slot: 3,
                price: 100
            }))
        );
        assert!(read("market", "17 100").is_err(), "slot seventeen");
        assert!(read("market", "0 100").is_err(), "slot zero");
        assert_eq!(
            read("marketall", "Sword of Splendor 500"),
            Ok(Action::Market(MarketAction::SellAll {
                item: "Sword of Splendor".to_string(),
                price: 500
            }))
        );
    }

    #[test]
    fn a_dungeon_invite_is_not_a_duel() {
        assert_eq!(
            read("daccept", "12"),
            Ok(Action::Dungeon(DungeonAction::Accept(12)))
        );
        assert_eq!(
            read("da", "nonsense"),
            Err("ID must be a number.".to_string())
        );
        assert_eq!(
            read("di", "Bo Fesal"),
            Ok(Action::Dungeon(DungeonAction::Invite(
                "Bo Fesal".to_string()
            )))
        );
    }
}
