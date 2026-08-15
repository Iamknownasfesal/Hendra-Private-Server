//! Every command the original accepts, and what answers it here.
//!
//! Counted for the same reason the packets and the endpoints are: ninety commands went unnoticed
//! for as long as nothing listed them, and the only thing that has ever found a gap in this server
//! is counting something rather than reasoning about it.
//!
//! The list below is the original's own, name by name and rank by rank, taken from the constructors
//! in `RankedCommands.cs` and `UnrankedCommands.cs`. It counts in both directions: a command the
//! original has and this server does not is a gap, and a command this server has and the original
//! does not is a door the original answers with `Unknown command!`.
//!
//!     cargo run -p hendra-server --example commands

// The same file the server compiles, rather than a copy of its table. A copy is how a census stops
// matching what it is counting.
#[path = "../src/commands.rs"]
#[allow(dead_code)]
mod commands;

fn main() {
    let (missing, extra, misranked) = differences();

    println!(
        "ANSWERED: {} of {} the original has, under {} names",
        ORIGINAL.len() - missing.len(),
        ORIGINAL.len(),
        commands::ALL.len() + commands::ALL.iter().filter(|c| c.alias.is_some()).count(),
    );

    if !missing.is_empty() {
        println!("nothing answers: {}", missing.join(", "));
    }
    if !extra.is_empty() {
        println!("here and not there: {}", extra.join(", "));
    }
    for (name, ours, theirs) in &misranked {
        println!("/{name} asks for {ours} here and {theirs} there");
    }
    println!();

    for (name, alias, rank) in ORIGINAL {
        let spelling = match alias {
            Some(alias) => format!("  [{alias}]"),
            None => String::new(),
        };
        let asks = if *rank == 0 {
            String::new()
        } else {
            format!(" (rank {rank})")
        };

        println!("/{name:<16}{asks}{spelling}");
    }

    if !missing.is_empty() || !extra.is_empty() || !misranked.is_empty() {
        std::process::exit(1);
    }
}

/// What the two tables disagree about: what is missing, what is extra, and what asks for a
/// different rank.
#[allow(clippy::type_complexity)]
fn differences() -> (
    Vec<&'static str>,
    Vec<&'static str>,
    Vec<(&'static str, i16, i16)>,
) {
    let missing: Vec<&str> = ORIGINAL
        .iter()
        .filter(|(name, _, _)| commands::find(name).is_none())
        .map(|(name, _, _)| *name)
        .collect();

    let extra: Vec<&str> = commands::ALL
        .iter()
        .filter(|command| {
            !ORIGINAL
                .iter()
                .any(|(name, _, _)| name.eq_ignore_ascii_case(command.name))
        })
        .map(|command| command.name)
        .collect();

    let misranked: Vec<(&str, i16, i16)> = ORIGINAL
        .iter()
        .filter_map(|(name, _, rank)| {
            let ours = commands::find(name)?;
            (ours.rank != *rank).then_some((*name, ours.rank, *rank))
        })
        .collect();

    (missing, extra, misranked)
}

/// Every command the original accepts, with the alias and the `permLevel` it declares.
///
/// The five it declares and comments out — `resetFame`, `wipeServer`, `removeAllGold`,
/// `addWelcomeMessage`, `npe` — are not here, because they are not commands there.
const ORIGINAL: &[(&str, Option<&str>, i16)] = &[
    // UnrankedCommands.cs, all of them rank 0.
    ("join", None, 0),
    ("tutorial", None, 0),
    ("ps", None, 0),
    ("world", None, 0),
    ("pause", None, 0),
    ("tp", Some("teleport"), 0),
    ("daccept", Some("da"), 0),
    ("dinvite", Some("di"), 0),
    ("tell", Some("t"), 0),
    ("g", Some("guild"), 0),
    ("l", None, 0),
    ("commands", None, 0),
    ("ignore", None, 0),
    ("unignore", None, 0),
    ("lock", None, 0),
    ("unlock", None, 0),
    ("uptime", None, 0),
    ("pos", Some("position"), 0),
    ("trade", None, 0),
    ("time", None, 0),
    ("realm", None, 0),
    ("nexus", None, 0),
    ("vault", None, 0),
    ("ghall", None, 0),
    ("lefttomax", None, 0),
    ("gland", Some("glands"), 0),
    ("who", None, 0),
    ("online", None, 0),
    ("market", None, 0),
    ("marketall", Some("mall"), 0),
    ("myMarket", None, 0),
    ("oops", None, 0),
    ("rmarket", None, 0),
    ("marketplace", None, 0),
    ("removeOverride", None, 0),
    ("currentsong", Some("song"), 0),
    ("gkick", None, 0),
    ("invite", Some("ginvite"), 0),
    ("gwho", Some("mates"), 0),
    ("spectate", None, 0),
    // RankedCommands.cs.
    ("level20", Some("l20"), 0),
    ("Set", None, 0),
    ("unlink", None, 8),
    ("size", None, 10),
    ("reskin", None, 10),
    ("glow", None, 10),
    ("donorshop", None, 10),
    ("gimme", Some("give"), 40),
    ("max", None, 40),
    ("clearspawn", Some("cs"), 80),
    ("cleargraves", Some("cgraves"), 80),
    ("kick", None, 80),
    ("announce", None, 80),
    ("summon", None, 80),
    ("mute", None, 80),
    ("unmute", None, 80),
    ("ban", None, 80),
    ("banip", Some("ipban"), 80),
    ("unban", None, 80),
    ("clearinv", None, 80),
    ("visit", None, 80),
    ("hide", Some("h"), 80),
    ("rename", None, 80),
    ("unname", None, 80),
    ("lootspawn", Some("ls"), 90),
    ("spawn", None, 90),
    ("eff", None, 90),
    ("killAll", Some("ka"), 90),
    ("getQuest", None, 90),
    ("oryxSay", Some("osay"), 90),
    ("summonall", None, 90),
    ("reboot", None, 90),
    ("rank", None, 90),
    ("music", None, 90),
    ("closerealm", None, 90),
    ("quake", None, 90),
    ("link", None, 90),
    ("gift", None, 90),
    ("setfame", None, 90),
    ("setgold", None, 90),
    ("setprestige", None, 90),
    ("grank", None, 95),
    ("setpiece", None, 100),
    ("debug", None, 100),
    ("killPlayer", None, 100),
    ("tq", None, 100),
    ("override", None, 100),
    ("warg", None, 100),
    ("compactLOH", None, 100),
    ("setstar", None, 100),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_tables_are_the_same_table() {
        let (missing, extra, misranked) = differences();

        assert!(missing.is_empty(), "nothing answers: {missing:?}");
        assert!(extra.is_empty(), "here and not there: {extra:?}");
        assert!(misranked.is_empty(), "the wrong rank: {misranked:?}");
    }

    #[test]
    fn the_aliases_are_the_originals() {
        for (name, alias, _) in ORIGINAL {
            let ours = commands::find(name).expect(name);
            assert_eq!(&ours.alias, alias, "/{name}");
        }
    }

    #[test]
    fn the_census_counts_ninety() {
        assert_eq!(ORIGINAL.len(), 90);
        assert_eq!(commands::ALL.len(), ORIGINAL.len());
    }

    #[test]
    fn every_command_reads_into_something() {
        // A name in the table that reads into nothing whatever it is given prints in the help and
        // then does nothing when it is typed.
        for command in commands::ALL {
            let read_something = ["somebody 20", "3 20", "20", "wizard", ""]
                .iter()
                .any(|args| commands::read(command.name, args).is_ok());

            assert!(read_something, "/{}", command.name);
        }
    }
}
