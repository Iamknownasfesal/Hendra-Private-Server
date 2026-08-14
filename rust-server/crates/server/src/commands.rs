//! What a player can type, and what it means.
//!
//! Follows `wServer/realm/commands/`, which splits them the same way: anybody may use the unranked
//! ones, and the ranked ones ask for a rank first.
//!
//! # Why this is a table rather than a match
//!
//! Because it has to be counted. Ninety-one commands went unnoticed for as long as nothing listed
//! them, and a `match` with ninety-one arms is not something anybody can compare against another
//! server. A table can be walked, printed and checked, and [`crate::commands::ALL`] is what
//! `cargo run -p hendra-server --example commands` prints.
//!
//! # What a command is not
//!
//! A way around a rule. Every one of these goes through the same machinery the protocol does: `/tp`
//! is the world's teleport with all its refusals, `/trade` is the trade registry, `/ignore` is the
//! same list the whisper path reads. A command that reached past those would be a second, weaker
//! door into the same room.

use hendra_store::Admin;

/// What a command needs before it will run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Needs {
    /// Anybody.
    Nobody,

    /// A moderator or above.
    Moderator,

    /// An administrator.
    Administrator,
}

impl Needs {
    /// Whether a rank is enough.
    pub fn met_by(self, rank: Admin) -> bool {
        match self {
            Needs::Nobody => true,
            Needs::Moderator => rank.may_mute(),
            Needs::Administrator => rank.may_ban(),
        }
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildAction {
    Create(String),
    Invite(String),
    Join(String),
    Kick(String),
    Rank(String, String),
    Board(String),
    Who,
    Leave,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketAction {
    Browse,
    Mine,

    /// Take back the last thing listed, which is what somebody who mistyped a price wants and
    /// cannot say any other way: they do not know the number.
    Oops,
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
    Gift,
    Announce,
    Rename,
    Unname,
}

/// One command: what it is called, what it needs, and one line on what it does.
pub struct Command {
    pub name: &'static str,

    /// Other spellings of the same command.
    pub aliases: &'static [&'static str],

    pub needs: Needs,
    pub summary: &'static str,
}

/// Every command, in the order the help prints them.
///
/// The names are the original's. Where it has two spellings of one thing they are aliases here
/// rather than two entries, because a player who learns one should not find the other missing.
pub const ALL: &[Command] = &[
    // Talking.
    Command {
        name: "tell",
        aliases: &["t", "w", "whisper"],
        needs: Needs::Nobody,
        summary: "say something to one player",
    },
    Command {
        name: "g",
        aliases: &["guild"],
        needs: Needs::Nobody,
        summary: "say something to your guild",
    },
    Command {
        name: "l",
        aliases: &["local"],
        needs: Needs::Nobody,
        summary: "say something to this world",
    },
    // Going places.
    Command {
        name: "nexus",
        aliases: &["n"],
        needs: Needs::Nobody,
        summary: "go to the nexus",
    },
    Command {
        name: "realm",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "go to a realm",
    },
    Command {
        name: "vault",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "go to your vault",
    },
    Command {
        name: "ghall",
        aliases: &["guildhall"],
        needs: Needs::Nobody,
        summary: "go to your guild hall",
    },
    Command {
        name: "marketplace",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "go to the marketplace",
    },
    Command {
        name: "donorshop",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "go to the donor shop",
    },
    Command {
        name: "tp",
        aliases: &["teleport"],
        needs: Needs::Nobody,
        summary: "move to a player",
    },
    Command {
        name: "gland",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "go to the godlands, from a realm",
    },
    // People.
    Command {
        name: "trade",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "ask a player to trade",
    },
    Command {
        name: "ignore",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "stop hearing a player",
    },
    Command {
        name: "unignore",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "hear them again",
    },
    Command {
        name: "lock",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "stop a player teleporting to you",
    },
    Command {
        name: "unlock",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "let them again",
    },
    // Guilds.
    Command {
        name: "createguild",
        aliases: &["cg"],
        needs: Needs::Nobody,
        summary: "found a guild",
    },
    Command {
        name: "invite",
        aliases: &["ginvite"],
        needs: Needs::Nobody,
        summary: "bring somebody into your guild",
    },
    Command {
        name: "join",
        aliases: &["gjoin"],
        needs: Needs::Nobody,
        summary: "join a guild you were invited to",
    },
    Command {
        name: "gkick",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "remove somebody from your guild",
    },
    Command {
        name: "grank",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "change somebody's guild rank",
    },
    Command {
        name: "gboard",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "change the guild board",
    },
    Command {
        name: "gwho",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "who is in your guild",
    },
    Command {
        name: "gleave",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "leave your guild",
    },
    // The market.
    Command {
        name: "market",
        aliases: &["marketall"],
        needs: Needs::Nobody,
        summary: "what is for sale",
    },
    Command {
        name: "mymarket",
        aliases: &["rmarket"],
        needs: Needs::Nobody,
        summary: "what you have listed",
    },
    Command {
        name: "oops",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "take back the last thing you listed",
    },
    // Asking the server.
    Command {
        name: "who",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "who is in this world",
    },
    Command {
        name: "online",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "who is on the server",
    },
    Command {
        name: "pos",
        aliases: &["where"],
        needs: Needs::Nobody,
        summary: "where you are standing",
    },
    Command {
        name: "uptime",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "how long the server has been up",
    },
    Command {
        name: "commands",
        aliases: &["help"],
        needs: Needs::Nobody,
        summary: "this list",
    },
    Command {
        name: "ps",
        aliases: &["prestige"],
        needs: Needs::Nobody,
        summary: "how much prestige you hold",
    },
    Command {
        name: "lefttomax",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "how far each stat is from its maximum",
    },
    Command {
        name: "currentsong",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "what is playing",
    },
    Command {
        name: "time",
        aliases: &[],
        needs: Needs::Nobody,
        summary: "the time",
    },
    // Moderation.
    Command {
        name: "mute",
        aliases: &[],
        needs: Needs::Moderator,
        summary: "stop a player speaking",
    },
    Command {
        name: "unmute",
        aliases: &[],
        needs: Needs::Moderator,
        summary: "let them speak",
    },
    Command {
        name: "kick",
        aliases: &[],
        needs: Needs::Moderator,
        summary: "disconnect a player",
    },
    Command {
        name: "announce",
        aliases: &["oryxsay"],
        needs: Needs::Moderator,
        summary: "say something to everyone",
    },
    Command {
        name: "ban",
        aliases: &[],
        needs: Needs::Administrator,
        summary: "keep a player out",
    },
    Command {
        name: "unban",
        aliases: &[],
        needs: Needs::Administrator,
        summary: "let them back",
    },
    Command {
        name: "rank",
        aliases: &[],
        needs: Needs::Administrator,
        summary: "set a player's rank",
    },
    Command {
        name: "setfame",
        aliases: &[],
        needs: Needs::Administrator,
        summary: "set a player's fame",
    },
    Command {
        name: "setgold",
        aliases: &[],
        needs: Needs::Administrator,
        summary: "set a player's gold",
    },
    Command {
        name: "setprestige",
        aliases: &[],
        needs: Needs::Administrator,
        summary: "set a player's prestige",
    },
    Command {
        name: "gift",
        aliases: &[],
        needs: Needs::Administrator,
        summary: "send a player an item",
    },
    Command {
        name: "rename",
        aliases: &[],
        needs: Needs::Administrator,
        summary: "rename an account",
    },
    Command {
        name: "unname",
        aliases: &[],
        needs: Needs::Administrator,
        summary: "take an account's name away",
    },
];

/// Finds a command by what was typed, whatever its spelling or capitals.
pub fn find(name: &str) -> Option<&'static Command> {
    ALL.iter().find(|command| {
        command.name.eq_ignore_ascii_case(name)
            || command
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
    })
}

/// Reads a command and its words into something to do.
///
/// Returns the action, or a line to say when the words do not make sense. Reading is kept apart from
/// doing so that what a command means can be checked without a world, a store or a connection.
pub fn read(name: &str, rest: &str) -> Result<Action, String> {
    use hendra_store::ListKind;

    let rest = rest.trim();
    let (first, tail) = split(rest);

    let command = find(name).ok_or_else(|| format!("there is no /{name}"))?;

    let action = match command.name {
        "tell" => {
            if first.is_empty() || tail.is_empty() {
                return Err("who, and what?".to_string());
            }
            Action::Tell {
                to: first.to_string(),
                text: tail.to_string(),
            }
        }

        "g" => nonempty(rest, "say what?").map(|text| Action::GuildSay(text.to_string()))?,
        "l" => nonempty(rest, "say what?").map(|text| Action::Say(text.to_string()))?,

        "nexus" => Action::GoTo(NEXUS),
        "realm" => Action::GoTo("Realm"),
        "vault" => Action::GoTo("Vault"),
        "ghall" => Action::GoTo("Guild Hall"),
        "marketplace" => Action::GoTo("Marketplace"),
        "donorshop" => Action::GoTo("Donor Shop"),

        // The godlands are a place in the realm rather than a world, so this is a teleport to a
        // fixed point and the only command that names one.
        "gland" => Action::TeleportTo(GODLANDS.to_string()),

        "tp" => nonempty(first, "who?").map(|who| Action::TeleportTo(who.to_string()))?,
        "trade" => nonempty(first, "who?").map(|who| Action::Trade(who.to_string()))?,

        "ignore" | "unignore" => Action::List {
            kind: ListKind::Ignored,
            name: nonempty(first, "who?")?.to_string(),
            add: command.name == "ignore",
        },
        "lock" | "unlock" => Action::List {
            kind: ListKind::LockedOut,
            name: nonempty(first, "who?")?.to_string(),
            add: command.name == "lock",
        },

        "createguild" => Action::Guild(GuildAction::Create(
            nonempty(rest, "called what?")?.to_string(),
        )),
        "invite" => Action::Guild(GuildAction::Invite(nonempty(first, "who?")?.to_string())),
        "join" => Action::Guild(GuildAction::Join(
            nonempty(first, "which guild?")?.to_string(),
        )),
        "gkick" => Action::Guild(GuildAction::Kick(nonempty(first, "who?")?.to_string())),
        "grank" => {
            if first.is_empty() || tail.is_empty() {
                return Err("who, and to what rank?".to_string());
            }
            Action::Guild(GuildAction::Rank(first.to_string(), tail.to_string()))
        }
        "gboard" => Action::Guild(GuildAction::Board(rest.to_string())),
        "gwho" => Action::Guild(GuildAction::Who),
        "gleave" => Action::Guild(GuildAction::Leave),

        "market" => Action::Market(MarketAction::Browse),
        "mymarket" => Action::Market(MarketAction::Mine),
        "oops" => Action::Market(MarketAction::Oops),

        "who" => Action::Report(Report::Who),
        "online" => Action::Report(Report::Online),
        "pos" => Action::Report(Report::Position),
        "uptime" => Action::Report(Report::Uptime),
        "commands" => Action::Report(Report::Commands),
        "ps" => Action::Report(Report::Prestige),
        "lefttomax" => Action::Report(Report::LeftToMax),
        "currentsong" => Action::Report(Report::CurrentSong),
        "time" => Action::Report(Report::Time),

        other => {
            let what = match other {
                "mute" => Moderation::Mute,
                "unmute" => Moderation::Unmute,
                "kick" => Moderation::Kick,
                "announce" => Moderation::Announce,
                "ban" => Moderation::Ban,
                "unban" => Moderation::Unban,
                "rank" => Moderation::Rank,
                "setfame" => Moderation::SetFame,
                "setgold" => Moderation::SetGold,
                "setprestige" => Moderation::SetPrestige,
                "gift" => Moderation::Gift,
                "rename" => Moderation::Rename,
                "unname" => Moderation::Unname,
                _ => return Err(format!("there is no /{name}")),
            };

            // An announcement is the one that takes no name: everybody hears it.
            if what == Moderation::Announce {
                Action::Moderate {
                    what,
                    name: String::new(),
                    rest: nonempty(rest, "say what?")?.to_string(),
                }
            } else {
                Action::Moderate {
                    what,
                    name: nonempty(first, "who?")?.to_string(),
                    rest: tail.to_string(),
                }
            }
        }
    };

    Ok(action)
}

/// Where a player who escapes ends up, and where everybody arrives.
///
/// Named here because this is the file that has to name every world a command can reach, and one
/// place naming them all beats two places agreeing.
pub const NEXUS: &str = "Nexus";

/// Where `/gland` puts somebody, from the original.
///
/// Named rather than a position, because the world decides where a name is and this crate does not
/// know one map from another.
pub const GODLANDS: &str = "godlands";

fn split(rest: &str) -> (&str, &str) {
    match rest.split_once(char::is_whitespace) {
        Some((first, tail)) => (first.trim(), tail.trim()),
        None => (rest, ""),
    }
}

fn nonempty<'a>(text: &'a str, why: &str) -> Result<&'a str, String> {
    if text.trim().is_empty() {
        return Err(why.to_string());
    }
    Ok(text.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_found_by_its_own_name() {
        for command in ALL {
            assert!(find(command.name).is_some(), "/{}", command.name);
            for alias in command.aliases {
                assert!(find(alias).is_some(), "/{alias}");
            }
        }
    }

    #[test]
    fn no_two_commands_answer_to_the_same_word() {
        // Two entries claiming one word means one of them silently never runs.
        let mut seen: Vec<&str> = Vec::new();

        for command in ALL {
            for word in std::iter::once(&command.name).chain(command.aliases) {
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
        // A command in the list that reads into nothing is one that prints in the help and then
        // does nothing when it is typed.
        for command in ALL {
            let outcome = read(command.name, "somebody something else");
            assert!(
                outcome.is_ok(),
                "/{} read into nothing: {outcome:?}",
                command.name
            );
        }
    }

    #[test]
    fn a_command_that_needs_a_name_says_so_rather_than_doing_half_of_it() {
        for name in ["tp", "trade", "ignore", "lock", "gkick", "ban", "mute"] {
            assert!(read(name, "").is_err(), "/{name} with nobody named");
        }
    }

    #[test]
    fn ranks_are_what_they_say() {
        assert!(Needs::Nobody.met_by(Admin::None));
        assert!(!Needs::Moderator.met_by(Admin::None));
        assert!(Needs::Moderator.met_by(Admin::Moderator));

        // A moderator may not ban, which is the whole reason the two ranks are separate.
        assert!(!Needs::Administrator.met_by(Admin::Moderator));
        assert!(Needs::Administrator.met_by(Admin::Administrator));
    }

    #[test]
    fn only_moderation_asks_for_a_rank() {
        // Everything a player types about their own game should be theirs to type.
        for command in ALL.iter().filter(|command| command.needs != Needs::Nobody) {
            let action = read(command.name, "somebody something").expect("it reads");
            assert!(
                matches!(action, Action::Moderate { .. }),
                "/{} asks for a rank but is not moderation",
                command.name
            );
        }
    }

    #[test]
    fn a_whisper_needs_both_who_and_what() {
        assert!(read("tell", "Fesal").is_err(), "who but not what");
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
    fn spelling_and_capitals_do_not_matter() {
        assert_eq!(read("TELL", "Bo hi"), read("w", "Bo hi"));
        assert_eq!(read("Nexus", ""), read("n", ""));
    }
}
