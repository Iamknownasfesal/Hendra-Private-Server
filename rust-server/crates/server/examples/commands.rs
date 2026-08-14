//! Every command the original accepts, and what answers it here.
//!
//! Counted for the same reason the packets and the endpoints are: ninety-one commands went
//! unnoticed for as long as nothing listed them, and the only thing that has ever found a gap in
//! this server is counting something rather than reasoning about it.
//!
//!     cargo run -p hendra-server --example commands

// The same file the server compiles, rather than a copy of its table. A copy is how a census stops
// matching what it is counting.
#[path = "../src/commands.rs"]
#[allow(dead_code)]
mod commands;

fn main() {
    let unanswered: Vec<&str> = ORIGINAL
        .iter()
        .filter(|name| commands::find(name).is_none())
        .copied()
        .collect();

    println!(
        "ANSWERED: {} of {} the original has",
        ORIGINAL.len() - unanswered.len(),
        ORIGINAL.len()
    );
    println!(
        "{} commands here, {} of them for moderators or administrators",
        commands::ALL.len(),
        commands::ALL
            .iter()
            .filter(|command| command.needs != commands::Needs::Nobody)
            .count()
    );
    if !unanswered.is_empty() {
        println!("nothing answers: {}", unanswered.join(", "));
    }
    println!();

    for command in commands::ALL {
        let rank = match command.needs {
            commands::Needs::Nobody => "",
            commands::Needs::Moderator => " (moderator)",
            commands::Needs::Administrator => " (administrator)",
        };

        let spellings = if command.aliases.is_empty() {
            String::new()
        } else {
            format!("  [{}]", command.aliases.join(", "))
        };

        println!("/{:<16} {}{rank}{spellings}", command.name, command.summary);
    }

    if !unanswered.is_empty() {
        std::process::exit(1);
    }
}

/// Every command the original accepts, from `wServer/realm/commands/`.
///
/// Here so the census can check itself: a command the original has and this server does not is a
/// gap, and a gap that nothing counts is one nobody finds.
const ORIGINAL: &[&str] = &[
    "Set",
    "addWelcomeMessage",
    "announce",
    "ban",
    "banip",
    "cleargraves",
    "clearinv",
    "clearspawn",
    "closerealm",
    "commands",
    "compactLOH",
    "currentsong",
    "daccept",
    "debug",
    "dinvite",
    "donorshop",
    "eff",
    "g",
    "getQuest",
    "ghall",
    "gift",
    "gimme",
    "gkick",
    "gland",
    "glow",
    "grank",
    "gwho",
    "hide",
    "ignore",
    "invite",
    "join",
    "kick",
    "killAll",
    "killPlayer",
    "l",
    "lefttomax",
    "level20",
    "link",
    "lock",
    "lootspawn",
    "market",
    "marketall",
    "marketplace",
    "max",
    "music",
    "mute",
    "myMarket",
    "nexus",
    "npe",
    "online",
    "oops",
    "oryxSay",
    "override",
    "pause",
    "pos",
    "ps",
    "quake",
    "rank",
    "realm",
    "reboot",
    "removeAllGold",
    "removeOverride",
    "rename",
    "resetFame",
    "reskin",
    "rmarket",
    "setfame",
    "setgold",
    "setpiece",
    "setprestige",
    "setstar",
    "size",
    "spawn",
    "spectate",
    "summon",
    "summonall",
    "tell",
    "time",
    "tp",
    "tq",
    "trade",
    "tutorial",
    "unban",
    "unignore",
    "unlink",
    "unlock",
    "unmute",
    "unname",
    "uptime",
    "vault",
    "visit",
    "warg",
    "who",
    "wipeServer",
    "world",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_the_original_has_is_answered() {
        let missing: Vec<&str> = ORIGINAL
            .iter()
            .filter(|name| commands::find(name).is_none())
            .copied()
            .collect();

        assert!(missing.is_empty(), "nothing answers: {missing:?}");
    }

    #[test]
    fn every_command_reads_into_something() {
        // A name in the table that reads into nothing prints in the help and then does nothing.
        for command in commands::ALL {
            assert!(
                commands::read(command.name, "somebody something else").is_ok(),
                "/{}",
                command.name
            );
        }
    }
}
