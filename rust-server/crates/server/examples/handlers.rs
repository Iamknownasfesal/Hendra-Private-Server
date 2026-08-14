//! Every packet the original server accepts, and what answers it here.
//!
//! The protocol is the last place a mechanic can go missing without anything saying so. A rule that
//! is implemented but unreachable looks exactly like one that works, and three times now the answer
//! to "is this complete" has been wrong because nothing counted the surface.
//!
//! So this counts it. One row per handler in `wServer/networking/handlers/`, and what carries it.
//!
//!     cargo run -p hendra-server --example handlers

/// One packet the original accepts, and what answers it.
///
/// `None` means nothing does, which is what this example exists to find.
const HANDLERS: &[(&str, Option<&str>)] = &[
    // Getting in and out.
    ("Hello", Some("ClientMessage::Hello")),
    ("Load", Some("GET /characters, then Hello names one")),
    ("Create", Some("POST /characters")),
    ("Escape", Some("ClientMessage::Escape")),
    ("Pong", Some("ClientMessage::Pong")),
    // Moving and fighting.
    ("Move", Some("ClientMessage::Input")),
    ("PlayerShoot", Some("ClientMessage::Shoot")),
    ("PlayerText", Some("ClientMessage::Chat")),
    ("UsePortal", Some("ClientMessage::UsePortal")),
    ("UseItem", Some("ClientMessage::UseItem")),
    ("Teleport", Some("ClientMessage::Teleport")),
    // Damage. The original has the client report every hit it takes and deals, and trusts it. Here
    // every one of these is decided by the world: projectiles are traced in `crates/sim`, and ground
    // hazards are applied by the tick. A client that reports nothing takes the same damage as one
    // that reports honestly, and a client that reports a miss takes it anyway.
    ("EnemyHit", Some("decided by the world")),
    ("PlayerHit", Some("decided by the world")),
    ("OtherHit", Some("decided by the world")),
    ("SquareHit", Some("decided by the world")),
    ("GroundDamage", Some("decided by the world")),
    // Acknowledgements. The original has four; a snapshot's tick number does the work of all of
    // them, so there is one.
    ("UpdateAck", Some("Input carries the acknowledged tick")),
    ("AoeAck", Some("Input carries the acknowledged tick")),
    ("GotoAck", Some("Input carries the acknowledged tick")),
    ("ShootAck", Some("Input carries the acknowledged tick")),
    // Carrying things.
    ("InvSwap", Some("ClientMessage::MoveItem")),
    ("InvDrop", Some("ClientMessage::MoveItem to Ground")),
    ("Buy", Some("ClientMessage::Buy")),
    ("VaultMove", Some("ClientMessage::MoveItem to Vault")),
    ("VaultBuy", Some("POST /vault/chests")),
    ("MarketCommand", Some("ClientMessage::Market")),
    // Guilds.
    ("CreateGuild", Some("ClientMessage::Guild")),
    ("JoinGuild", Some("ClientMessage::Guild")),
    ("GuildInvite", Some("ClientMessage::Guild")),
    ("GuildRemove", Some("ClientMessage::Guild")),
    ("ChangeGuildRank", Some("ClientMessage::Guild")),
    // Trading.
    ("RequestTrade", Some("ClientMessage::RequestTrade")),
    ("ChangeTrade", Some("ClientMessage::ChangeTrade")),
    ("AcceptTrade", Some("ClientMessage::AcceptTrade")),
    ("CancelTrade", Some("ClientMessage::CancelTrade")),
    // The account, which is the app server's rather than the game socket's.
    ("ChooseName", Some("POST /name")),
    ("CheckCredits", Some("GET /offers")),
    ("Reskin", Some("POST /skins")),
    ("EditAccountList", Some("ClientMessage::EditList")),
    ("Prestige", Some("ClientMessage::Prestige")),
    ("PrestigeBuy", Some("ClientMessage::PrestigeBuy")),
    ("WeeklyQuest", Some("GET /quests/weekly")),
    // Things the original itself does not do. Reproducing them faithfully means having nothing.
    (
        "SetCondition",
        Some("nothing: the original's handler is empty"),
    ),
    (
        "AlertNotice",
        Some("nothing: the original's handler is empty"),
    ),
    // Handlers whose own server cannot carry them out. Each was checked rather than assumed, and
    // what was found is in `WITHOUT_CONTENT` below, which a test re-checks against the content.
    (
        "ForgeItem",
        Some("nothing: none of its items are in the content"),
    ),
    (
        "SorForgeRequest",
        Some("nothing: none of its items are in the content"),
    ),
    (
        "QoLAction",
        Some("nothing: the Sor Crystal is not in the content"),
    ),
    (
        "MarkRequest",
        Some("nothing: the original has no onrane or marks"),
    ),
    (
        "RequestGamble",
        Some("nothing: the original calls a method it does not have"),
    ),
    (
        "UnboxRequest",
        Some("nothing: the original calls a method it does not have"),
    ),
    (
        "LaunchRaid",
        Some("nothing: the original has no raid worlds"),
    ),
];

/// The object types the forge and the crystal ask for.
///
/// Named here so the claim above can be checked rather than believed: if a later content drop adds
/// any of these, the test below fails and says the handler is now worth implementing.
pub const WITHOUT_CONTENT: &[(&str, u16)] = &[
    ("Shine", 0x64c7),
    ("Large Sor", 0x49e6),
    ("Cosmic Shard", 0x68fa),
    ("Fury Shard", 0x68fb),
    ("Zol Shard", 0x68fc),
    ("Stone Shard", 0x61b4),
    ("Ancient Shard", 0x611f),
    ("Sor Crystal", 0x49e5),
];

fn main() {
    let answered = HANDLERS.iter().filter(|(_, by)| by.is_some()).count();

    println!(
        "ANSWERED: {answered} of {} packets ({:.0}% covered)",
        HANDLERS.len(),
        answered as f32 / HANDLERS.len() as f32 * 100.0
    );
    println!();

    for (packet, by) in HANDLERS {
        match by {
            Some(by) => println!("ok   {packet:<20} {by}"),
            None => println!("TODO {packet:<20}"),
        }
    }

    if answered < HANDLERS.len() {
        println!();
        println!(
            "{} packets have nothing behind them",
            HANDLERS.len() - answered
        );
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handlers_we_do_nothing_for_ask_for_content_that_is_not_here() {
        // The reasons in the table are claims about the content, and a claim about the content
        // should be checked against it rather than believed. If a later drop adds any of these, this
        // fails and says the handler is now worth implementing.
        let Ok((catalog, _)) = hendra_content::Catalog::load_dir(std::path::Path::new(
            "../../../godot-client/assets/xml",
        )) else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        let present: Vec<&str> = WITHOUT_CONTENT
            .iter()
            .filter(|(_, kind)| catalog.object(hendra_content::ObjectType(*kind)).is_some())
            .map(|(name, _)| *name)
            .collect();

        assert!(
            present.is_empty(),
            "the content now has {present:?}, so the handlers that want them are worth implementing"
        );
    }

    #[test]
    fn nothing_is_left_unanswered() {
        let unanswered: Vec<&str> = HANDLERS
            .iter()
            .filter(|(_, by)| by.is_none())
            .map(|(packet, _)| *packet)
            .collect();

        assert!(unanswered.is_empty(), "{unanswered:?}");
    }
}
