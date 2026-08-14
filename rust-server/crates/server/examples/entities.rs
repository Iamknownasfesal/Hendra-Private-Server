//! Every kind of thing the original puts in a world, and what answers it here.
//!
//! The last category to be counted, and counting it found the player merchant: a market listing
//! that could only be reached by typing rather than by walking up to it.
//!
//! Names differ on purpose. The original names a class after what it is; this server names things
//! after what they do, so `GiftChest` is a one-way container placed by region and `StaticObject` is
//! scenery on a tile. The rows say which is which, because a census that only matched names would
//! call every one of these a gap.
//!
//!     cargo run -p hendra-server --example entities

/// One class in `wServer/realm/entities/`, and what carries it here.
const ENTITIES: &[(&str, Option<&str>)] = &[
    ("Player", Some("Kind::Player")),
    (
        "Enemy",
        Some("Kind::Enemy, with its behaviour from the content"),
    ),
    ("Projectile", Some("crates/sim/src/projectile.rs")),
    ("Container", Some("Kind::Container, durable or a bag")),
    (
        "OneWayContainer",
        Some("ContainerKind::Merchant, which takes nothing back"),
    ),
    (
        "GiftChest",
        Some("the gift chest, placed by region and one-way"),
    ),
    (
        "Portal",
        Some("Kind::Portal, leading where its world claims"),
    ),
    (
        "GuildHallPortal",
        Some("a portal to the caller's own guild's hall"),
    ),
    (
        "StaticObject",
        Some("scenery on a tile, sent with the ground"),
    ),
    ("Wall", Some("scenery, and the collision bitmap")),
    (
        "ConnectedObject",
        Some("scenery: which walls join is what the client draws"),
    ),
    ("Sign", Some("scenery, unless the map names it")),
    ("Decoy", Some("Effect::Decoy, which enemies chase")),
    ("Trap", Some("Effect::Trap, sprung by what walks over it")),
    ("Character", Some("the class descriptions, in the catalog")),
    (
        "Merchant",
        Some("a shop stall, its stock dealt out by region"),
    ),
    ("WorldMerchant", Some("the same stall: a shop's own stock")),
    (
        "PlayerMerchant",
        Some("a market listing, standing in the row its kind belongs to"),
    ),
    (
        "GuildMerchant",
        Some("the hall upgrade, paid from the guild's fame"),
    ),
    ("SellableObject", Some("what a stall holds, on the entity")),
    ("ShopItem", Some("one line of a shop's stock table")),
    (
        "Placeholder",
        Some("a delayed spawn, which the world holds until it lands"),
    ),
    ("TimeCop", Some("nothing: the original's file is empty")),
    (
        "UpdatedSet",
        Some("nothing: bookkeeping for the original's own update loop"),
    ),
    (
        "ConnectionInfo",
        Some("nothing: bookkeeping for the original's own connections"),
    ),
    (
        "ConnectionComputer",
        Some("nothing: bookkeeping for the original's own connections"),
    ),
];

fn main() {
    let answered = ENTITIES.iter().filter(|(_, by)| by.is_some()).count();

    println!(
        "ANSWERED: {answered} of {} kinds of thing a world holds",
        ENTITIES.len()
    );
    println!();

    for (class, by) in ENTITIES {
        match by {
            Some(by) => println!("ok   {class:<20} {by}"),
            None => println!("TODO {class:<20}"),
        }
    }

    if answered < ENTITIES.len() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_left_unanswered() {
        let unanswered: Vec<&str> = ENTITIES
            .iter()
            .filter(|(_, by)| by.is_none())
            .map(|(class, _)| *class)
            .collect();

        assert!(unanswered.is_empty(), "{unanswered:?}");
    }
}
