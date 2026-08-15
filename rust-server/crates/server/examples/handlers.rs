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
    ("ChooseName", Some("POST /name, with its rules and its 5000 fame")),
    ("CheckCredits", Some("nothing caches a balance here")),
    ("Reskin", Some("POST /skins")),
    ("EditAccountList", Some("ClientMessage::EditList")),
    ("Prestige", Some("ClientMessage::Prestige")),
    ("PrestigeBuy", Some("ClientMessage::PrestigeBuy")),
    // Things the original itself does not do. Reproducing them faithfully means having nothing.
    // `SetConditionHandler.cs:12` is a comment saying to implement something, and
    // `WeeklyQuestHandler.cs:16-19` is a method with an empty body: redeeming a weekly quest there
    // costs nothing and gives nothing. `GET /quests/weekly` lists them, as `GET /quests` does, and
    // there is no redeeming to answer.
    (
        "SetCondition",
        Some("nothing: the original's handler is empty"),
    ),
    (
        "WeeklyQuest",
        Some("nothing: the original's handler is empty"),
    ),
    // Handlers the original does not build. See `NOT_BUILT` below, which a test re-checks against
    // the original's project file.
    ("AlertNotice", Some("nothing: not in wServer.csproj")),
    ("LaunchRaid", Some("nothing: not in wServer.csproj")),
    ("ForgeItem", Some("nothing: not in wServer.csproj")),
    ("SorForgeRequest", Some("nothing: not in wServer.csproj")),
    ("MarkRequest", Some("nothing: not in wServer.csproj")),
    ("UnboxRequest", Some("nothing: not in wServer.csproj")),
    ("RequestGamble", Some("nothing: not in wServer.csproj")),
    ("QoLAction", Some("nothing: not in wServer.csproj")),
];

/// The handlers the original has the source of but does not compile.
///
/// `wServer.csproj` is an old-style project: every file it builds is named in a `<Compile Include>`
/// of its own and there is no globbing, so a file on disk that is not listed is not in the server.
/// Eight are not, and their incoming packets are missing in exactly the same eight, which is what
/// makes it a decision rather than an oversight.
///
/// They could not be built if they were listed. Seven of the eight name a `PacketId` that
/// `PacketIds.cs` does not declare, and between them they call some twenty members that exist
/// nowhere in the solution: `Player.Onrane`, `.SorStorage`, `.AlertToken`, `.MarksEnabled`,
/// `.Node1`, `.BronzeLootbox`, `.Kantos`, `.RequestGamble`, `.ascendSorCrystal`, `.betAmount`, and
/// `Database.UpdateOnrane`, `.UpdateSorStorage`, `.UpdateAlertToken`, `.UpdateBronzeLootbox`,
/// `.UpdateKantos`. `DbAccount` has no column for any of those currencies. The one name that does
/// survive, "Onrane", is an `ItemLoot` string in the Catacombs tables and not an account balance at
/// all, and `LootTemplates.RaidTokens()` returns an empty array, so the raid token the alert spends
/// has nothing that drops it either.
///
/// And nothing could reach them if they did build. `PacketHandlers`
/// (`networking/IPacketHandler.cs:39-48`) finds handlers by walking the compiled assembly's types,
/// so a file outside the build is never a type, never registered, and its packet id answers to
/// nothing.
///
/// So these are roughly eight hundred and forty lines of source that the original server never
/// runs. Writing them here would not be parity with it; it would be inventing four currencies it
/// does not have and giving players a forge, a lootbox and a raid that it does not offer.
pub const NOT_BUILT: &[&str] = &[
    "AlertNoticeHandler.cs",
    "ForgeItemHandler.cs",
    "LaunchRaidHandler.cs",
    "MarkRequestHandler.cs",
    "QoLActionHandler.cs",
    "RequestGambleHandler.cs",
    "SorForgeRequestHandler.cs",
    "UnboxRequestHandler.cs",
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
    fn the_handlers_the_original_does_not_build_are_the_ones_named() {
        // "The original does not build it" is the reason eight rows above give for answering with
        // nothing, and it is a claim about a file that can be read. So it is read. `wServer.csproj`
        // names every source file it compiles and globs none of them, which is what makes absence
        // from it mean something.
        let project =
            std::path::Path::new("../../../Server-Side/wServer/wServer.csproj");
        let Ok(project) = std::fs::read_to_string(project) else {
            eprintln!("skipping: the original's project file is not where the test looks for it");
            return;
        };

        let handlers = std::path::Path::new("../../../Server-Side/wServer/networking/handlers");
        let Ok(directory) = std::fs::read_dir(handlers) else {
            eprintln!("skipping: the original's handlers are not where the test looks for them");
            return;
        };

        let mut missing: Vec<String> = directory
            .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
            .filter(|file| file.ends_with("Handler.cs"))
            .filter(|file| !project.contains(&format!(r"networking\handlers\{file}")))
            .collect();
        missing.sort();

        assert_eq!(
            missing, NOT_BUILT,
            "the original builds a different set of handlers than the table above says"
        );

        // The packet each one reads is left out of the build in the same breath, which is the part
        // that says it was meant rather than forgotten.
        for handler in NOT_BUILT {
            let packet = handler.trim_end_matches("Handler.cs");
            assert!(
                !project.contains(&format!(r"packets\incoming\{packet}.cs")),
                "{packet} is built even though its handler is not"
            );
        }
    }

    #[test]
    fn most_of_those_handlers_name_a_packet_id_that_does_not_exist() {
        // The stronger half of the claim: they are not merely left out, they could not be put in.
        // Seven of the eight ask for a `PacketId` member that `PacketIds.cs` never declares, so
        // adding them to the project would not compile.
        let ids = std::path::Path::new("../../../Server-Side/wServer/networking/packets/PacketIds.cs");
        let Ok(ids) = std::fs::read_to_string(ids) else {
            eprintln!("skipping: the original's packet ids are not where the test looks for them");
            return;
        };

        let declared = |name: &str| {
            ids.lines()
                .any(|line| line.trim_start().starts_with(&format!("{name} =")))
        };

        assert!(
            !declared("ALERTNOTICE")
                && !declared("LAUNCHRAID")
                && !declared("FORGEITEM")
                && !declared("MARKREQUEST")
                && !declared("REQUESTGAMBLE")
                && !declared("SORFORGEREQUEST")
                && !declared("UNBOXREQUEST"),
            "the original now declares an id it used not to, so its handler may be buildable"
        );

        // The eighth has an id. What it does not have is anywhere for the fragments it spends to
        // live: `QoLActionHandler.cs:19-25` reads `Player.SorStorage` and calls
        // `Database.UpdateSorStorage`, and neither is defined anywhere in the solution.
        assert!(declared("QOLACTION"));
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
