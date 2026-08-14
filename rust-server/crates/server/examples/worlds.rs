//! Every world the original gives its own logic, and what answers it here.
//!
//! The last of the surfaces to be counted, and it had gaps: guild halls ignored their level, and
//! the nexus showed no portals. Counted now for the same reason as the rest.
//!
//!     cargo run -p hendra-server --example worlds

/// One world the original writes a class for, and what carries it here.
const WORLDS: &[(&str, Option<&str>)] = &[
    (
        "Realm",
        Some("populates itself, closes on a clock, sends everyone to the castle"),
    ),
    (
        "Vault",
        Some("one room per account, chests and the gift chest placed by region"),
    ),
    (
        "GuildHall",
        Some("one room per guild, the level chooses the map, upgrades raise it"),
    ),
    (
        "Nexus",
        Some("portals to every running world, labelled with who is in them"),
    ),
    (
        "Marketplace",
        Some("merchants placed by region, like every other shop"),
    ),
    (
        "DonorShop",
        Some("merchants placed by region, like every other shop"),
    ),
    (
        "Davy",
        Some("keys are announced as they are found, and told to whoever arrives after"),
    ),
    (
        "Candyland",
        Some("its spawners tick with everything else, which is what its class arranges"),
    ),
    (
        "Castle",
        Some("nothing: the original's class overrides nothing either"),
    ),
    (
        "Test",
        Some("nothing: a development map this server does not ship"),
    ),
];

fn main() {
    let answered = WORLDS.iter().filter(|(_, by)| by.is_some()).count();

    println!(
        "ANSWERED: {answered} of {} worlds with their own logic",
        WORLDS.len()
    );
    println!();

    for (world, by) in WORLDS {
        match by {
            Some(by) => println!("ok   {world:<12} {by}"),
            None => println!("TODO {world:<12}"),
        }
    }

    if answered < WORLDS.len() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_left_unanswered() {
        let unanswered: Vec<&str> = WORLDS
            .iter()
            .filter(|(_, by)| by.is_none())
            .map(|(world, _)| *world)
            .collect();

        assert!(unanswered.is_empty(), "{unanswered:?}");
    }
}
