//! Every world the original gives its own logic, and what answers it here.
//!
//! The last of the surfaces to be counted, and it had gaps: guild halls ignored their level, and
//! the nexus showed no portals. Counted now for the same reason as the rest.
//!
//! Four of the ten classes answer to nothing, and that is the answer. A world's class is chosen by
//! `DynamicWorld.TryGetWorld`, which walks every subclass of `World` and takes the one whose
//! `type.Name.Equals(wData.name)` (`realm/worlds/DynamicWorld.cs:32-39`); everything that makes a
//! world goes through it (`RealmManager.cs:319`, `World.cs:178`, `Portal.cs:76`,
//! `Player.UseItem.cs:480`, `RankedCommands.cs:1883`, `AlertNoticeHandler.cs:43`) and every one of
//! them falls back to a plain `new World(proto)` when it comes back empty. The definitions are
//! keyed by the same `name` (`common/resources/WorldData.cs:60`, `:77`), so a class whose name no
//! shipped definition carries is code the server can never reach: `Candyland` against "Candyland
//! Hunting Grounds", `Davy` against "Davy Jones' Locker", `DonorShop` against "Donor Shop", and
//! `Test` against no definition at all. Those four worlds are plain worlds in the original, so
//! they are plain worlds here, and `no_class_can_reach` below is what keeps that honest.
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
        Some("one portal per realm, renamed in place with who is in it"),
    ),
    (
        "Marketplace",
        Some("merchants placed by region, like every other shop"),
    ),
    (
        "DonorShop",
        Some("merchants placed by region: the class is unreachable, so its world is a plain one"),
    ),
    (
        "Davy",
        Some("the keys taunt for themselves: the class is unreachable, and nothing is replayed"),
    ),
    (
        "Candyland",
        Some("its spawners wake when somebody is near: the class that would override that is unreachable"),
    ),
    (
        "Castle",
        Some("the first spawn square and only that one, so a quaked realm arrives together"),
    ),
    (
        "Test",
        Some("nothing: no definition names it, so the class is unreachable"),
    ),
];

/// The four classes no shipped definition can select, and the name each is looking for.
///
/// `DynamicWorld.TryGetWorld` compares a class's name to a definition's `name` field, so these are
/// the exact strings that would have to appear in a `.jw` for the class to instantiate.
const UNREACHABLE: [&str; 4] = ["Candyland", "Davy", "DonorShop", "Test"];

fn main() {
    let answered = WORLDS.iter().filter(|(_, by)| by.is_some()).count();

    println!(
        "ANSWERED: {answered} of {} worlds with their own logic",
        WORLDS.len()
    );
    println!();

    for (world, by) in WORLDS {
        let reach = if UNREACHABLE.contains(world) {
            "--"
        } else {
            "  "
        };
        match by {
            Some(by) => println!("ok {reach} {world:<12} {by}"),
            None => println!("TODO  {world:<12}"),
        }
    }

    println!();
    println!(
        "-- marks a class no shipped definition can select, so the original runs a plain world"
    );

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

    /// Every world the ten classes are named for is a world we know about.
    #[test]
    fn every_class_is_accounted_for() {
        assert_eq!(WORLDS.len(), 10, "there are ten classes in `worlds/logic`");
        for name in UNREACHABLE {
            assert!(
                WORLDS.iter().any(|(world, _)| *world == name),
                "{name} is not in the table"
            );
        }
    }

    /// No shipped definition names one of the four, so no path can select their classes.
    ///
    /// This is the whole reason those four worlds are plain here. If a definition were ever renamed
    /// to "Candyland" or "Davy" the original would start running that class -- Candyland would tick
    /// its spawners with nobody near them, Davy would show the key UI -- and this would have to be
    /// built rather than explained.
    #[test]
    fn no_class_can_reach_a_shipped_definition() {
        let worlds = std::path::Path::new("../../../Server-Side/XmlDatas/worlds");
        let Ok(entries) = std::fs::read_dir(worlds) else {
            return; // The content is not beside us; nothing to check.
        };

        let mut named: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|end| end.to_str()) != Some("jw") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(definition) = hendra_content::WorldDef::parse(&text) else {
                continue;
            };
            named.push(definition.name);
        }

        assert!(named.len() > 30, "only found {} definitions", named.len());
        for class in UNREACHABLE {
            assert!(
                !named.iter().any(|name| name == class),
                "a definition is named {class:?}, so `DynamicWorld` would select that class"
            );
        }

        // And the six that are reachable are reachable, which is the other half of the same claim.
        for class in ["Realm", "Vault", "GuildHall", "Nexus", "Marketplace", "Castle"] {
            assert!(
                named.iter().any(|name| name == class),
                "no definition is named {class:?}, so its class would never run"
            );
        }
    }
}
