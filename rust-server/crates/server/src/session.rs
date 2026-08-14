//! One connection, from hello to goodbye.
//!
//! A session owns its link and nothing else. It decodes what arrives, turns it into a message for
//! whichever world the player is in, and hands it on; it never touches a world directly. Snapshots
//! travel the other way without passing through here at all. The world writes to the player's
//! connection itself.
//!
//! # Where items live
//!
//! Equipment and the backpack are one table. Slots 0 to 3 are worn and 4 upwards are carried, which
//! is how the game has always numbered them, and it means every move between them is a move within
//! one container, so it goes through the same transactional path a vault move does rather than
//! needing a second mechanism.
//!
//! # Bags cross a boundary
//!
//! A bag lives in the world, in memory and gone in a minute, while an inventory lives in the
//! database. A move between them cannot be one transaction, so it is two, and the order they happen
//! in decides what a crash between them costs.
//!
//! Removing from the ephemeral side first means a crash loses the item. Writing to the durable side
//! first means a crash duplicates it. Both are bad and they are not equally bad: a lost item is one
//! player's bad evening, and a duplicated one is an economy. So the ephemeral side always goes
//! first, and if the second step fails the first is undone.
//!
//! The window is a few milliseconds, on an item that was free and would have expired in a minute
//! regardless.
//!
//! # Changing world
//!
//! A session outlives the world it is in. Stepping through a portal leaves one world and joins
//! another, and the player's identity changes with it: a handle names a slot in a particular
//! world's storage and means nothing anywhere else. The client is told by a second `Welcome`, which
//! it must treat as "forget everything", because its snapshot history was measured against a world that no
//! longer applies.

use std::sync::Arc;

use tokio::sync::mpsc;

use hendra_content::ObjectType;
use hendra_net::message::{
    ClientMessage, ContainerId, PROTOCOL_VERSION, RejectReason, ServerMessage, SlotLocation,
};
use hendra_net::{Delivery, EntityId, Reader, Writer};
use hendra_sim::Handle;
use hendra_store::{Location, Store};
use hendra_transport::{Link, Received};

use crate::world_task::{ToWorld, WorldHandle};
use crate::worlds::Worlds;

/// Where a player currently is.
struct Placement {
    world: WorldHandle,
    handle: Handle,
}

/// How many slots are worn rather than carried.
const EQUIPPED_SLOTS: u8 = 4;

/// Everything a session needs to answer for one player.
pub struct Context {
    pub worlds: Arc<Worlds>,
    pub store: Store,
    pub catalog: Arc<hendra_content::Catalog>,
    pub kit: crate::accounts::StartingKit,

    /// Verifies session tokens. This process cannot mint one.
    pub key: hendra_auth::TokenKey,

    /// Who is trading with whom, and who is online.
    pub trades: Arc<crate::trades::Trades>,

    /// When the server started, which is all `/uptime` needs.
    pub started: std::time::Instant,
}

/// Turns a wire slot into a durable one.
///
/// Returns `None` for a container that is not durable, which today means a bag.
fn locate(where_: SlotLocation, character_id: i64, account_id: i64) -> Option<Location> {
    Some(match where_ {
        SlotLocation::Equipment { slot } if slot < EQUIPPED_SLOTS => Location::Inventory {
            character_id,
            slot: slot as i16,
        },
        // An equipment slot beyond what exists is not a slot at all.
        SlotLocation::Equipment { .. } => return None,

        SlotLocation::Inventory { slot } => Location::Inventory {
            character_id,
            // Carried slots sit above the worn ones in the same table.
            slot: (slot as i16).saturating_add(EQUIPPED_SLOTS as i16),
        },

        SlotLocation::Vault { slot } => Location::Vault {
            account_id,
            slot: slot as i16,
        },

        // Neither a bag nor the ground is durable; both are handled before this is reached.
        SlotLocation::Gift { slot } => Location::Gift {
            account_id,
            slot: slot as i16,
        },

        SlotLocation::Bag { .. } | SlotLocation::Ground => return None,
    })
}

/// Handles one connection for its lifetime.
pub async fn serve(mut link: Link, context: Arc<Context>, entry: WorldHandle) {
    let peer = link.remote_address();

    // A world can ask this session for something only a session can do, such as leaving for the
    // castle when a realm closes. One slot is enough: there is one such order, and it happens once.
    let (to_session, mut orders) = mpsc::channel(4);

    // One slot: a character dies once, and the session ends when it does.
    let (to_death, mut died) = mpsc::channel(1);

    let Some((player, mut placement)) =
        handshake(&mut link, &context, &entry, &to_session, &to_death).await
    else {
        link.close("handshake refused");
        return;
    };

    let name = player.character.name.clone();
    tracing::info!(
        %peer, %name,
        account = player.account.id,
        character = player.character.id,
        world = %placement.world.name,
        "session started"
    );

    // One account plays one character at a time. Whatever else was on it is ended first, rather
    // than this login being refused: the common case is somebody whose connection dropped trying to
    // get back in, and refusing them would hold them out until a timeout they cannot see.
    let ended = context.trades.claim(player.account.id);
    if ended > 0 {
        tracing::info!(%name, account = player.account.id, ended, "took over an account");
    }

    // Reachable by name from now on, which is what lets somebody else ask them to trade.
    context
        .trades
        .arrived(&name, player.account.id, player.character.id, link.sender());
    context.trades.moved(&name, &placement.world.name);

    // The ground first, because a client that has not been told the map cannot place anything it
    // is about to be told about.
    send_terrain(&mut link, &placement).await;

    // Then what it is carrying, before it can move any of it.
    send_containers(&mut link, &context.catalog, &context.store, &player).await;

    // One per connection. A limit shared between players would let a busy world silence a quiet
    // one, and a limit that outlived a connection would follow the wrong person.
    let mut limit = crate::chat::Limit::new();

    loop {
        let received = tokio::select! {
            received = link.recv() => received,

            // A death, which ends the session. Waited on beside the client rather than checked
            // between messages, because somebody who dies standing still sends nothing.
            departed = died.recv() => {
                let Some(departed) = departed else { break };

                if die(&mut link, &context, &player, &placement, departed).await {
                    break;
                }
                continue;
            }

            // A world asking for something only a session can do. Waited on beside the client
            // rather than checked between messages, because a player standing still sends nothing
            // and a closing realm should still empty.
            order = orders.recv() => {
                let Some(order) = order else { break };
                match order {
                    crate::world_task::Order::GoTo(destination) => {
                        if let Some(next) = go_to(
                            &mut link,
                            &placement,
                            &name,
                            player.account.id,
                            &destination,
                            &context.worlds,
                            arrival_of(&player, &context),
                            &to_session,
                            &to_death,
                        )
                        .await
                        {
                            placement = next;
                            send_terrain(&mut link, &placement).await;
                        }
                    }
                }
                continue;
            }
        };

        let Some(received) = received else {
            break;
        };

        match dispatch(&received, &placement).await {
            Outcome::Continue => {}
            Outcome::Stop => break,

            Outcome::Move { from, to } => {
                move_item(&mut link, &context, &player, &placement, from, to).await;
            }

            Outcome::Chat(line) => {
                if let Some((command, rest)) =
                    say_something(&mut link, &context, &player, &placement, &mut limit, &line).await
                {
                    run_command(
                        &mut link,
                        &context,
                        &player,
                        &name,
                        &mut placement,
                        &to_session,
                        &to_death,
                        &command,
                        &rest,
                    )
                    .await;
                }
            }

            Outcome::UseItem { slot, x, y } => {
                use_item(&mut link, &context, &player, &placement, slot, (x, y)).await;
            }

            Outcome::Trade(asked) => {
                trade(&mut link, &context, &player, &name, asked).await;
            }

            Outcome::Say(line) => say(&mut link, &line).await,

            Outcome::TeleportTo(to) => {
                teleport(&mut link, &context, &player, &placement, &to).await;
            }

            Outcome::Escape => {
                if let Some(next) = go_to(
                    &mut link,
                    &placement,
                    &name,
                    player.account.id,
                    crate::commands::NEXUS,
                    &context.worlds,
                    arrival_of(&player, &context),
                    &to_session,
                    &to_death,
                )
                .await
                {
                    placement = next;
                    context.trades.moved(&name, &placement.world.name);
                    send_terrain(&mut link, &placement).await;
                }
            }

            Outcome::Buy(sale) => buy(&mut link, &context, &player, sale).await,

            Outcome::BuyHallUpgrade(upgrade) => {
                buy_hall_upgrade(&mut link, &context, &player, upgrade).await;
            }

            Outcome::Prestige => {
                match context
                    .store
                    .prestige(player.account.id, player.character.id)
                    .await
                {
                    Ok(earned) => {
                        say(&mut link, &format!("You earned {earned} prestige.")).await;

                        // The character is level one with nothing now, so the world is holding a
                        // body that no longer matches what is stored. Leaving is the honest end.
                        break;
                    }
                    Err(hendra_store::StoreError::Refused(why)) => say(&mut link, why).await,
                    Err(_) => say(&mut link, "try again shortly").await,
                }
            }

            Outcome::PrestigeBuy(offer) => {
                buy_with_prestige(&mut link, &context, &player, offer).await;
            }

            Outcome::Guild(asked) => {
                guild(&mut link, &context, &player, &name, asked).await;
            }

            Outcome::Market(command) => {
                market(&mut link, &context, &player, command).await;
            }

            Outcome::EditList { list, name, add } => {
                edit_list(&mut link, &context, &player, list, &name, add).await;
            }

            Outcome::Travel(portal_type) => {
                // Stepping into the guild hall portal has to mean what typing the command means,
                // or the two are different doors into different rooms.
                if context.worlds.destination_of(portal_type) == Some(crate::commands::GUILD_HALL) {
                    if let Some(next) = enter_guild_hall(
                        &mut link,
                        &context,
                        &player,
                        &placement,
                        &name,
                        &to_session,
                        &to_death,
                    )
                    .await
                    {
                        placement = next;
                        context.trades.moved(&name, &placement.world.name);
                        send_terrain(&mut link, &placement).await;
                    }
                    continue;
                }

                match travel(
                    &mut link,
                    &placement,
                    &name,
                    player.account.id,
                    portal_type,
                    &context.worlds,
                    arrival_of(&player, &context),
                    &to_session,
                    &to_death,
                )
                .await
                {
                    Some(next) => {
                        tracing::info!(
                            %name,
                            from = %placement.world.name,
                            to = %next.world.name,
                            "travelled"
                        );
                        placement = next;

                        // Arriving in the vault means arriving at your own chests.
                        if placement.world.name.as_ref() == "Vault" {
                            place_chests(&context, &player, &placement).await;
                        }
                    }
                    // Staying put is the right answer when a destination cannot be opened: the
                    // player keeps playing where they are rather than being disconnected over a
                    // dungeon that failed to load.
                    None => tracing::warn!(%name, "portal led nowhere; staying put"),
                }
            }
        }
    }

    // Before anything else: a trade left half-agreed by a disconnection is the shape a duplication
    // is built on.
    context.trades.left(&name);

    placement
        .world
        .send(ToWorld::Leave {
            handle: placement.handle,
        })
        .await;

    // Write back what the character became. Items are not saved here; they are written as they
    // move, so a checkpoint that rewrote slots wholesale could undo a move that had committed.
    if let Err(err) = save_progress(&context, &player, &placement).await {
        tracing::warn!(%err, %name, "could not save the character");
    }

    tracing::info!(%peer, %name, "session ended");
}

/// What a player asked for about a trade.
#[derive(Debug, Clone, PartialEq)]
enum Trade {
    Request(String),
    Change(Vec<bool>),
    Accept { mine: Vec<bool>, theirs: Vec<bool> },
    Cancel,
}

/// A guild command with its names owned, so it can outlive the buffer it was decoded from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GuildAsk {
    Create(String),
    Invite(String),
    Join(String),
    Remove(String),
    SetRank(String, u8),
    SetBoard(String),
    Leave,
}

fn owned_guild(command: hendra_net::GuildCommand<'_>) -> GuildAsk {
    use hendra_net::GuildCommand as From;

    match command {
        From::Create { name } => GuildAsk::Create(name.to_owned()),
        From::Invite { name } => GuildAsk::Invite(name.to_owned()),
        From::Join { name } => GuildAsk::Join(name.to_owned()),
        From::Remove { name } => GuildAsk::Remove(name.to_owned()),
        From::SetRank { name, rank } => GuildAsk::SetRank(name.to_owned(), rank),
        From::SetBoard { text } => GuildAsk::SetBoard(text.to_owned()),
        From::Leave => GuildAsk::Leave,
    }
}

enum Outcome {
    Continue,
    Stop,

    /// The player stepped into a portal of this object type.
    Travel(u16),

    /// The player wants to move an item.
    Move {
        from: SlotLocation,
        to: SlotLocation,
    },

    /// The player said something.
    ///
    /// Handled by the caller rather than sent straight to the world, because whether they may speak
    /// at all is an account question and the world does not hold accounts.
    Chat(String),

    /// Something to do with trading.
    ///
    /// Handled by the caller for the same reason a move is: a trade is two connections and a
    /// durable inventory, and the world holds neither.
    Trade(Trade),

    /// The player is leaving for the nexus.
    Escape,

    /// The player wants to be where another player is.
    ///
    /// Handled by the caller rather than sent straight to the world, because whether the other
    /// player has locked them out is an account question and the world holds no accounts.
    TeleportTo(String),

    /// Something to say to the player and nothing else.
    Say(String),

    /// The player is buying what a merchant is selling.
    Buy(crate::world_task::Sale),

    /// The player is buying a larger hall for their guild.
    BuyHallUpgrade(crate::world_task::HallUpgrade),

    /// The player is giving up this character's fame for prestige.
    Prestige,

    /// The player is buying one of the things prestige buys.
    PrestigeBuy(u8),

    /// Something to do with a guild.
    Guild(GuildAsk),

    /// Something to do with the market.
    Market(hendra_net::MarketCommand),

    /// Add or remove somebody from one of the account's lists.
    EditList {
        list: hendra_net::AccountList,
        name: String,
        add: bool,
    },

    /// The player wants to use what is in a slot.
    ///
    /// Handled by the caller rather than sent straight to the world, because using an item can
    /// consume it and the inventory is durable.
    UseItem {
        slot: u16,
        x: f32,
        y: f32,
    },
}

/// Reads the opening message and either admits the player or explains why not.
async fn handshake(
    link: &mut Link,
    context: &Context,
    entry: &WorldHandle,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<(crate::accounts::Session, Placement)> {
    let received = link.recv().await?;
    let payload = received.into_payload();
    let mut reader = Reader::new(&payload);

    let message = match ClientMessage::decode(&mut reader) {
        Ok(message) => message,
        Err(err) => {
            tracing::warn!(%err, "undecodable opening message");
            return None;
        }
    };

    let ClientMessage::Hello {
        protocol,
        token,
        character,
    } = message
    else {
        tracing::warn!("first message was not a hello");
        return None;
    };

    if protocol != PROTOCOL_VERSION {
        refuse(link, RejectReason::VersionMismatch).await;
        tracing::info!(
            protocol,
            expected = PROTOCOL_VERSION,
            "refused: wrong version"
        );
        return None;
    }

    let player = match crate::accounts::log_in(
        &context.store,
        &context.catalog,
        &context.kit,
        &context.key,
        token,
        character as i64,
    )
    .await
    {
        Ok(player) => player,
        Err(crate::accounts::LoginError::Banned) => {
            refuse(link, RejectReason::Banned).await;
            return None;
        }
        Err(crate::accounts::LoginError::NoToken)
        | Err(crate::accounts::LoginError::BadToken(_)) => {
            refuse(link, RejectReason::BadToken).await;
            return None;
        }
        Err(err) => {
            tracing::warn!(%err, "login failed");
            refuse(link, RejectReason::BadToken).await;
            return None;
        }
    };

    let handle = join(
        link,
        entry,
        &player.character.name,
        arrival_of(&player, context),
        orders,
        died,
    )
    .await?;
    Some((
        player,
        Placement {
            world: entry.clone(),
            handle,
        },
    ))
}

/// Sends the player everything they are carrying and storing.
/// Recomputes what a player wears and tells the world.
///
/// Called after any move that succeeded rather than only after one touching a worn slot, because
/// the cost is one query and the failure mode of getting the condition wrong is a ring that keeps
/// working after it has been taken off.
async fn refresh_equipment(
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
) {
    let Ok(character) = context.store.character(player.character.id).await else {
        return;
    };

    let boosts = worn_boosts(&context.catalog, &character.inventory);
    placement
        .world
        .send(ToWorld::Equipment {
            handle: placement.handle,
            boosts,
        })
        .await;
}

async fn send_containers(
    link: &mut Link,
    catalog: &hendra_content::Catalog,
    store: &Store,
    player: &crate::accounts::Session,
) {
    let inventory = store
        .character(player.character.id)
        .await
        .map(|character| character.inventory)
        .unwrap_or_default();

    // Worn and carried live in one table, so they are split back apart on the way out.
    let worn: Vec<(u16, u16)> = inventory
        .iter()
        .filter(|(slot, _)| *slot < EQUIPPED_SLOTS as i16)
        .filter_map(|(slot, item)| Some((*slot as u16, number(catalog, *item)?)))
        .collect();
    let carried: Vec<(u16, u16)> = inventory
        .iter()
        .filter(|(slot, _)| *slot >= EQUIPPED_SLOTS as i16)
        .filter_map(|(slot, item)| {
            Some((
                (*slot - EQUIPPED_SLOTS as i16) as u16,
                number(catalog, *item)?,
            ))
        })
        .collect();

    let vault: Vec<(u16, u16)> = store
        .vault(player.account.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(slot, item)| Some((slot as u16, number(catalog, item)?)))
        .collect();

    for (container, slots) in [
        (ContainerId::Equipment, worn),
        (ContainerId::Inventory, carried),
        (ContainerId::Vault, vault),
    ] {
        let mut buf = Vec::new();
        ServerMessage::Container { container, slots }.encode(&mut Writer::new(&mut buf));
        let _ = link.send(Delivery::Stream, &buf).await;
    }

    // And the two stacks, which live outside the pack. A player who cannot see how many potions
    // they hold cannot decide whether to drink one.
    let held = store
        .character(player.character.id)
        .await
        .map(|character| (character.health_potions, character.magic_potions))
        .unwrap_or((0, 0));

    let mut buf = Vec::new();
    ServerMessage::Stacks {
        health: held.0.clamp(0, u16::MAX as i32) as u16,
        magic: held.1.clamp(0, u16::MAX as i32) as u16,
    }
    .encode(&mut Writer::new(&mut buf));
    let _ = link.send(Delivery::Stream, &buf).await;
}

/// Carries out a move, or explains why not.
async fn move_item(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    from: SlotLocation,
    to: SlotLocation,
) {
    let character_id = player.character.id;
    let account_id = player.account.id;

    // Bags are not durable, so a move touching one is two steps rather than one transaction.
    match (from, to) {
        (SlotLocation::Bag { entity, slot }, destination) => {
            take_from_bag(link, context, player, placement, entity, slot, destination).await;
            return;
        }
        (source, SlotLocation::Bag { entity, .. }) => {
            put_in_bag(link, context, player, placement, source, Some(entity)).await;
            return;
        }
        (source, SlotLocation::Ground) => {
            put_in_bag(link, context, player, placement, source, None).await;
            return;
        }
        (SlotLocation::Ground, _) => {
            say(link, "there is nothing at your feet to take").await;
            return;
        }
        _ => {}
    }

    // The vault is a place. Reaching into it from a dungeon would make the room decoration, and
    // the check belongs here rather than in the client, which is not in a position to be trusted
    // about where it is standing.
    let touches_vault =
        matches!(from, SlotLocation::Vault { .. }) || matches!(to, SlotLocation::Vault { .. });
    if touches_vault && placement.world.name.as_ref() != "Vault" {
        say(link, "you are not at your vault").await;
        return;
    }

    let (Some(source), Some(destination)) = (
        locate(from, character_id, account_id),
        locate(to, character_id, account_id),
    ) else {
        say(link, "that cannot be moved").await;
        return;
    };

    // A worn slot only takes what the class wears in it. Without this a wizard equips a sword and
    // shoots with it, and fourteen classes quietly become one.
    if !worn_slots_would_accept(context, player, from, to).await {
        say(link, "that does not go there").await;
        return;
    }

    // What the client believed was there. The store refuses the move if it is no longer, which is
    // what stops the same item being moved twice by two requests that both read it first.
    let expected = match read_slot(&context.store, source).await {
        Some(item) => item,
        None => {
            say(link, "there is nothing there").await;
            return;
        }
    };

    match context
        .store
        .move_item(source, destination, Some(expected))
        .await
    {
        Ok(_) => {
            send_containers(link, &context.catalog, &context.store, player).await;
            refresh_equipment(context, player, placement).await;
            if touches_vault {
                place_chests(context, player, placement).await;
            }
        }
        Err(hendra_store::StoreError::Refused(reason)) => {
            say(link, reason).await;
            // Re-read rather than assume: the client's picture is now known to be wrong.
            send_containers(link, &context.catalog, &context.store, player).await;
        }
        Err(err) => {
            tracing::warn!(%err, "a move failed");
            say(link, "that could not be done").await;
        }
    }
}

/// Puts the account's vault chests into the room it has just entered.
async fn place_chests(context: &Context, player: &crate::accounts::Session, placement: &Placement) {
    let slots: Vec<(u16, u16)> = context
        .store
        .vault(player.account.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(slot, item)| Some((slot as u16, number(&context.catalog, item)?)))
        .collect();

    let (reply, answer) = tokio::sync::oneshot::channel();
    if placement
        .world
        .send(ToWorld::PlaceVaultChests {
            slots,
            unlocked: player.account.vault_chests.max(0) as u16,
            reply,
        })
        .await
    {
        let placed = answer.await.unwrap_or(0);
        tracing::debug!(placed, account = player.account.id, "placed vault chests");
    }

    // The gift chest stands in the same room, wherever the map marks one. Placed here rather than
    // separately because a player who walks into their vault should find everything waiting at
    // once, and because a gift with nowhere to be opened is a player owed something they cannot
    // reach.
    let gifts: Vec<(u16, u16)> = context
        .store
        .gifts(player.account.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(slot, item)| Some((slot as u16, number(&context.catalog, item)?)))
        .collect();

    let (reply, answer) = tokio::sync::oneshot::channel();
    if placement
        .world
        .send(ToWorld::PlaceGiftChest {
            slots: gifts,
            reply,
        })
        .await
    {
        let placed = answer.await.unwrap_or(0);
        tracing::debug!(placed, account = player.account.id, "placed the gift chest");
    }
}

/// Takes an item out of a bag and into the player's inventory.
///
/// The bag gives it up first. If the durable write then fails the item goes back, and if the server
/// dies in between it is lost, which is the trade this order buys and the right way round.
async fn take_from_bag(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    bag: EntityId,
    slot: u8,
    destination: SlotLocation,
) {
    let (reply, answer) = tokio::sync::oneshot::channel();
    if !placement
        .world
        .send(ToWorld::TakeFromBag {
            player: placement.handle,
            bag,
            slot,
            reply,
        })
        .await
    {
        return;
    }

    let Some(item) = answer.await.ok().flatten() else {
        say(link, "there is nothing there to take").await;
        return;
    };

    // The world names items by runtime number; the durable side names them by identity. This is
    // the seam, and an item the catalog cannot name is one that must not be written down.
    let Some(identity) = context
        .catalog
        .object(ObjectType(item))
        .map(|desc| desc.uuid)
    else {
        say(link, "that is not something you can carry").await;
        return;
    };

    // A stacking potion goes to its own place rather than into the pack, which is what gives it a
    // ceiling of six and keeps it out of the eight slots everything else competes for.
    if let Some(magic) = stacks_as(&context.catalog, ObjectType(item)) {
        return match context.store.add_potion(player.character.id, magic).await {
            Ok(true) => {
                send_containers(link, &context.catalog, &context.store, player).await;
            }
            Ok(false) => say(link, "you cannot carry any more of those").await,
            Err(_) => say(link, "try again shortly").await,
        };
    }

    let outcome = match locate(destination, player.character.id, player.account.id) {
        // A named durable slot: it has to be free, because there is nothing to swap with.
        Some(Location::Inventory { character_id, slot }) => context
            .store
            .give_item(character_id, identity, slot, slot)
            .await
            .map(|_| ()),

        // A gift chest takes nothing back, so something taken from a bag and aimed at one goes to
        // the first free carried slot instead.
        Some(Location::Vault { .. }) | Some(Location::Gift { .. }) | None => {
            // Anywhere else, or nowhere in particular: the first free carried slot.
            context
                .store
                .give_item(
                    player.character.id,
                    identity,
                    EQUIPPED_SLOTS as i16,
                    LAST_CARRIED_SLOT,
                )
                .await
                .map(|_| ())
        }
    };

    if let Err(err) = outcome {
        // Undo the first step. The bag may have gone if it emptied, in which case this makes a new
        // one where the player stands. The item comes back either way.
        let (reply, _) = tokio::sync::oneshot::channel();
        let _ = placement
            .world
            .send(ToWorld::PutInBag {
                player: placement.handle,
                bag: Some(bag),
                item,

                // Going back where it came from, which is a bag that already exists and already
                // belongs to whoever it belongs to.
                owned: false,

                reply,
            })
            .await;

        tracing::debug!(%err, "returned an item to its bag");
        say(link, "there is no room for that").await;
        return;
    }

    send_containers(link, &context.catalog, &context.store, player).await;
}

/// Puts an item from the player's inventory into a bag.
async fn put_in_bag(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    source: SlotLocation,
    bag: Option<EntityId>,
) {
    let Some(location) = locate(source, player.character.id, player.account.id) else {
        say(link, "that cannot be dropped").await;
        return;
    };

    let Location::Inventory { character_id, slot } = location else {
        say(link, "things cannot be dropped straight from the vault").await;
        return;
    };

    let Some(item) = read_slot(&context.store, location).await else {
        say(link, "there is nothing there").await;
        return;
    };

    // The durable side gives it up first here too, for the same reason in reverse. Otherwise an
    // item exists in a bag and in the database at once.
    if let Err(err) = context.store.take_item(character_id, slot, item).await {
        say(link, &err.to_string()).await;
        return;
    }

    let (reply, answer) = tokio::sync::oneshot::channel();
    let sent = placement
        .world
        .send(ToWorld::PutInBag {
            player: placement.handle,
            bag,
            item: number(&context.catalog, item).unwrap_or(0),

            // A soulbound item drops into a bag only whoever dropped it may open, which is what
            // makes dropping one a way to move it rather than a way to give it away.
            owned: bag.is_none() && is_soulbound(&context.catalog, item),

            reply,
        })
        .await;

    let accepted = sent && answer.await.unwrap_or(false);
    if !accepted {
        // Put it back where it came from.
        let _ = context
            .store
            .give_item(character_id, item, slot, slot)
            .await;
        say(link, "there is nowhere to put that").await;
    }

    send_containers(link, &context.catalog, &context.store, player).await;
}

/// The two potions that stack rather than taking a slot.
///
/// `HealthPots` and `MagicPots` in the original, which gives each its own place outside the pack and
/// a ceiling of six. Named rather than numbered, since a runtime number means nothing once the
/// content is reordered.
pub const HEALTH_POTION: &str = "Health Potion";
pub const MAGIC_POTION: &str = "Magic Potion";

/// Where the two stacks are addressed from.
///
/// The original's slots 254 and 255, kept because they are already what a client names when it
/// drinks from a stack: they are outside the eight the pack has, which is what makes them a place
/// of their own rather than two more slots to compete for.
pub const HEALTH_STACK_SLOT: u16 = 254;
pub const MAGIC_STACK_SLOT: u16 = 255;

/// Which stack an item belongs in, if either.
fn stacks_as(catalog: &hendra_content::Catalog, item: ObjectType) -> Option<bool> {
    let id = catalog.object(item)?.id.as_str();

    match id {
        HEALTH_POTION => Some(false),
        MAGIC_POTION => Some(true),
        _ => None,
    }
}

/// The highest carried slot a player has.
const LAST_CARRIED_SLOT: i16 = 11;

/// What is currently in a durable slot.
async fn read_slot(store: &Store, at: Location) -> Option<uuid::Uuid> {
    match at {
        Location::Inventory { character_id, slot } => store
            .character(character_id)
            .await
            .ok()?
            .inventory
            .iter()
            .find(|(index, _)| *index == slot)
            .map(|(_, item)| *item),
        Location::Vault { account_id, slot } => store
            .vault(account_id)
            .await
            .ok()?
            .iter()
            .find(|(index, _)| *index == slot)
            .map(|(_, item)| *item),
        Location::Gift { account_id, slot } => store
            .gifts(account_id)
            .await
            .ok()?
            .iter()
            .find(|(index, _)| *index == slot)
            .map(|(_, item)| *item),
    }
}

async fn say(link: &mut Link, message: &str) {
    let mut buf = Vec::new();
    ServerMessage::Refused { message }.encode(&mut Writer::new(&mut buf));
    let _ = link.send(Delivery::Stream, &buf).await;
}

/// Puts the player into a world and tells the client about it.
async fn join(
    link: &mut Link,
    world: &WorldHandle,
    name: &str,
    arrival: crate::world_task::Arrival,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<Handle> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    if !world
        .send(ToWorld::Join {
            name: name.to_string(),
            arrival,
            sender: link.sender(),
            orders: orders.clone(),
            died: died.clone(),
            reply,
        })
        .await
    {
        refuse(link, RejectReason::Full).await;
        return None;
    }

    let handle = answer.await.ok()?;

    let mut buf = Vec::new();
    ServerMessage::Welcome {
        player: handle.to_entity_id(),
        tick: hendra_net::Tick::ZERO,
        world: &world.name,
    }
    .encode(&mut Writer::new(&mut buf));

    link.send(Delivery::Stream, &buf).await.ok()?;
    Some(handle)
}

/// Whether both ends of a move would leave every worn slot holding something it accepts.
///
/// Both ends, because a move is a swap: dragging a sword onto a wand puts the wand where the sword
/// was, and checking only the destination would let the second half through.
async fn worn_slots_would_accept(
    context: &Context,
    player: &crate::accounts::Session,
    from: SlotLocation,
    to: SlotLocation,
) -> bool {
    let Some(class) = context
        .catalog
        .type_of_uuid(player.character.class)
        .and_then(|found| context.catalog.class(found))
    else {
        // A character whose class is not in the catalog cannot have its slots judged, and refusing
        // every move it makes would be a worse answer than allowing them.
        return true;
    };

    let worn = |location: SlotLocation| match location {
        SlotLocation::Inventory { slot } | SlotLocation::Equipment { slot } => {
            (slot as i16) < hendra_characters::EQUIPPED_SLOTS
        }
        _ => false,
    };

    if !worn(from) && !worn(to) {
        return true;
    }

    let character_id = player.character.id;
    let account_id = player.account.id;
    let (Some(source), Some(destination)) = (
        locate(from, character_id, account_id),
        locate(to, character_id, account_id),
    ) else {
        return true;
    };

    let moving = read_slot(&context.store, source).await;
    let displaced = read_slot(&context.store, destination).await;

    // An empty slot accepts anything, which is what taking an item out of one means.
    let fits = |location: SlotLocation, item: Option<uuid::Uuid>| match location {
        SlotLocation::Inventory { slot } | SlotLocation::Equipment { slot } => {
            let kind = item
                .and_then(|item| context.catalog.type_of_uuid(item))
                .unwrap_or(ObjectType::NONE);
            hendra_characters::slot_accepts(&context.catalog, class, slot as i16, kind)
        }
        _ => true,
    };

    fits(to, moving) && fits(from, displaced)
}

/// Writes back what a character became, including what it learned.
///
/// The world is asked for the live figures rather than the session remembering them, because the
/// world is where levelling happens and a remembered copy would be one tick stale at best.
async fn save_progress(
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
) -> Result<(), hendra_store::StoreError> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    placement
        .world
        .send(ToWorld::Snapshot {
            handle: placement.handle,
            reply,
        })
        .await;

    // A world that has already dropped the player leaves nothing to write, which is not an error:
    // it happens whenever a world closes underneath a session.
    let Ok(Some(vitals)) = answer.await else {
        return Ok(());
    };

    context
        .store
        .save_character(
            player.character.id,
            vitals.hp,
            vitals.mp,
            vitals.level,
            vitals.experience,
            vitals.fame,
        )
        .await?;

    // What the character did while it was here, added to what it had done before. A session that
    // ends without saving loses this session's counting rather than every session's.
    if let Err(err) = context
        .store
        .add_tally(player.character.id, &vitals.tally)
        .await
    {
        tracing::warn!(%err, character = player.character.id, "could not save what a character did");
    }

    // What the account has now taken this class to, which is what opens the next one. Recorded
    // here rather than only at death, because an account whose best warrior is still alive has
    // still levelled a warrior.
    hendra_characters::record_progress(
        &context.store,
        player.account.id,
        &hendra_store::Character {
            level: vitals.level,
            fame: vitals.fame,
            ..player.character.clone()
        },
    )
    .await
}

/// Carries out a moderation command, or explains why not.
///
/// The rank is read fresh rather than taken from the session, so raising or lowering somebody takes
/// effect without waiting for them to reconnect, exactly as a mute does.
/// Carries out what a player typed.
///
/// Reading what they meant is `crate::commands::read`, which needs no world, store or connection.
/// This is the half that does need them, and every one of these goes through the same machinery the
/// protocol does: `/tp` is the world's teleport with all its refusals, `/trade` is the trade
/// registry, `/ignore` is the same list the whisper path reads. A command that reached past those
/// would be a second, weaker door into the same room.
#[allow(clippy::too_many_arguments)]
async fn run_command(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    name: &str,
    placement: &mut Placement,
    to_session: &mpsc::Sender<crate::world_task::Order>,
    to_death: &mpsc::Sender<crate::world_task::Departed>,
    command: &str,
    rest: &str,
) {
    use crate::commands::{Action, GuildAction, MarketAction};
    use hendra_store::Admin;

    let rank = match context.store.account(player.account.id).await {
        Ok(account) => Admin::from_number(account.admin_rank),
        Err(_) => Admin::None,
    };

    // Somebody who may not use a command is told it does not exist rather than that they may not,
    // so the list of what a moderator can do is not something anyone can find by trying.
    let unknown = format!("there is no /{command}");

    let Some(known) = crate::commands::find(command) else {
        return say(link, &unknown).await;
    };
    if !known.needs.met_by(rank) {
        return say(link, &unknown).await;
    }

    let action = match crate::commands::read(command, rest) {
        Ok(action) => action,
        Err(why) => return say(link, &why).await,
    };

    match action {
        Action::Say(text) => {
            placement
                .world
                .send(ToWorld::Chat {
                    handle: placement.handle,
                    text,
                })
                .await;
        }

        Action::Tell { to, text } => {
            tell(link, context, player, placement, &to, &text).await;
        }

        Action::GuildSay(text) => guild_say(link, context, player, &text).await,

        // The guild hall is one room per guild rather than one per player, and which map it loads
        // depends on what the guild has paid for, so it does not go through the ordinary door.
        Action::GoTo(world) if world == crate::commands::GUILD_HALL => {
            if let Some(next) =
                enter_guild_hall(link, context, player, placement, name, to_session, to_death).await
            {
                *placement = next;
                context.trades.moved(name, &placement.world.name);
                send_terrain(link, placement).await;
            }
        }

        Action::GoTo(world) => {
            if let Some(next) = go_to(
                link,
                placement,
                name,
                player.account.id,
                world,
                &context.worlds,
                arrival_of(player, context),
                to_session,
                to_death,
            )
            .await
            {
                *placement = next;
                context.trades.moved(name, &placement.world.name);
                send_terrain(link, placement).await;
            } else {
                say(link, "you cannot go there from here").await;
            }
        }

        Action::TeleportTo(to) => teleport(link, context, player, placement, &to).await,

        Action::Trade(with) => {
            trade(link, context, player, name, Trade::Request(with)).await;
        }

        Action::List {
            kind,
            name: who,
            add,
        } => {
            list_person(link, context, player, kind, &who, add).await;
        }

        Action::Guild(what) => {
            let asked = match what {
                GuildAction::Create(name) => GuildAsk::Create(name),
                GuildAction::Invite(name) | GuildAction::Join(name) => GuildAsk::Invite(name),
                GuildAction::Kick(name) => GuildAsk::Remove(name),
                GuildAction::Rank(name, rank) => GuildAsk::SetRank(name, guild_rank_named(&rank)),
                GuildAction::Board(text) => GuildAsk::SetBoard(text),
                GuildAction::Leave => GuildAsk::Leave,
                GuildAction::Who => return guild_who(link, context, player).await,
            };

            guild(link, context, player, name, asked).await;
        }

        Action::Market(what) => match what {
            MarketAction::Browse => {
                market(link, context, player, hendra_net::MarketCommand::Browse).await;
            }
            MarketAction::Mine => my_market(link, context, player).await,
            // Taking back the last thing listed, which is what somebody who mistyped a price wants
            // and cannot express any other way: they do not know the number.
            MarketAction::Oops => {
                let Some(listing) = last_listing(context, player).await else {
                    return say(link, "you have not listed anything").await;
                };

                market(
                    link,
                    context,
                    player,
                    hendra_net::MarketCommand::Cancel { listing },
                )
                .await;
            }
        },

        Action::Report(what) => report(link, context, player, placement, what).await,

        // Duels are the one thing the original's handler names and its own server never defines,
        // so this says so rather than doing nothing and looking as though it worked.
        Action::Duel(_) => say(link, "duelling is not on this server").await,

        Action::Wield { what, rest } => {
            wield(link, context, player, placement, what, &rest).await;
        }

        Action::Moderate { what, name, rest } => {
            moderate(link, context, what, &name, &rest).await;
        }
    }
}

/// Tells a client what the ground is.
///
/// Sent in strips, one row at a time, because a map is millions of squares and one message holding
/// all of it would be larger than the transport carries. Run-length encoded, because a map is
/// mostly the same square repeated.
async fn send_terrain(link: &mut Link, placement: &Placement) {
    let (reply, answer) = tokio::sync::oneshot::channel();
    placement.world.send(ToWorld::Terrain { reply }).await;

    let Ok(strips) = answer.await else {
        return;
    };

    for (y, runs) in strips {
        if runs.is_empty() {
            continue;
        }

        let mut buffer = Vec::new();
        ServerMessage::Terrain { x: 0, y, runs }.encode(&mut Writer::new(&mut buffer));

        // A client that cannot keep up with the map is one that cannot play, so this waits rather
        // than dropping strips and leaving holes in the ground.
        if link.send(Delivery::Stream, &buffer).await.is_err() {
            return;
        }
    }

    send_scenery(link, placement).await;

    // What has already been found here, for somebody arriving after it was. Sent with the ground
    // because it is the same kind of thing: the state of the room rather than something happening
    // in it.
    let (reply, answer) = tokio::sync::oneshot::channel();
    if placement.world.send(ToWorld::KeysFound { reply }).await
        && let Ok(found) = answer.await
    {
        for key in found {
            let mut buffer = Vec::new();
            ServerMessage::Notice {
                text: format!("{key} has been found."),
            }
            .encode(&mut Writer::new(&mut buffer));

            if link.send(Delivery::Stream, &buffer).await.is_err() {
                return;
            }
        }
    }
}

/// Tells a client where the scenery is.
///
/// Scenery is not an entity: it never moves, never acts and never changes. Sending it with the
/// ground rather than in the snapshot is what keeps a realm's quarter of a million trees out of
/// the world, where they would leave no room for a single enemy.
async fn send_scenery(link: &mut Link, placement: &Placement) {
    let (reply, answer) = tokio::sync::oneshot::channel();
    placement.world.send(ToWorld::Scenery { reply }).await;

    let Ok(rows) = answer.await else {
        return;
    };

    for (y, objects) in rows {
        // A row can hold more objects than one message may claim, so it goes in pieces of that
        // size rather than being trusted to fit.
        for piece in objects.chunks(hendra_net::MAX_SCENERY) {
            let mut buffer = Vec::new();
            ServerMessage::Scenery {
                y,
                objects: piece.to_vec(),
            }
            .encode(&mut Writer::new(&mut buffer));

            if link.send(Delivery::Stream, &buffer).await.is_err() {
                return;
            }
        }
    }
}

/// Carries out what a player asked for about a trade.
///
/// The registry decides who may trade with whom and what was agreed to. The store decides whether
/// the items actually move, and its refusal is an answer rather than a failure: an offer can name an
/// item that has been dropped, sold or traded elsewhere since it was made.
async fn trade(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    name: &str,
    asked: Trade,
) {
    use crate::trades::Step;

    let step = match asked {
        Trade::Request(to) => {
            let step = context.trades.request(name, &to);

            // A trade that has just begun needs both sides shown what the other is holding, which
            // is a read of two inventories rather than anything the registry knows.
            if step == Step::Done
                && let Some(partner) = context.trades.partner(name)
            {
                open_trade(context, name, &partner).await;
            }
            step
        }

        Trade::Change(offer) => context.trades.change(name, &offer),
        Trade::Accept { mine, theirs } => context.trades.accept(name, &mine, &theirs),
        Trade::Cancel => context.trades.cancel(name, "Trade cancelled."),
    };

    match step {
        Step::Done => {}
        Step::Say(reason) => say(link, &reason).await,

        Step::Settle {
            partner_character,
            partner_name,
            mine,
            theirs,
        } => {
            settle_trade(
                context,
                name,
                player.character.id,
                &partner_name,
                partner_character,
                mine,
                theirs,
            )
            .await;
        }
    }
}

/// Tells both sides what the other is holding, which is what a trade window shows.
async fn open_trade(context: &Context, name: &str, partner: &str) {
    let Some(first) = trade_slots(context, name).await else {
        context.trades.cancel(name, "Trade cancelled.");
        return;
    };
    let Some(second) = trade_slots(context, partner).await else {
        context.trades.cancel(name, "Trade cancelled.");
        return;
    };

    context.trades.send(
        name,
        &ServerMessage::TradeStart {
            mine: first.clone(),
            their_name: partner.to_string(),
            theirs: second.clone(),
        },
    );
    context.trades.send(
        partner,
        &ServerMessage::TradeStart {
            mine: second,
            their_name: name.to_string(),
            theirs: first,
        },
    );
}

/// What one player is holding, as the other side of a trade sees it.
///
/// Read by name rather than passed in, because the other side's inventory belongs to another
/// session and this is the only place that can ask for it.
async fn trade_slots(context: &Context, name: &str) -> Option<Vec<hendra_net::TradeSlot>> {
    let character = context.store.character_named(name).await.ok()??;

    let mut slots = vec![
        hendra_net::TradeSlot {
            item: None,
            slot_type: 0,
            included: false,
            tradeable: false,
        };
        crate::trades::TRADE_SLOTS
    ];

    for (slot, item) in &character.inventory {
        let (slot, item) = (*slot, *item);
        let Ok(index) = usize::try_from(slot) else {
            continue;
        };
        if index >= slots.len() {
            continue;
        }

        let desc = context
            .catalog
            .type_of_uuid(item)
            .and_then(|kind| context.catalog.object(kind));

        slots[index] = hendra_net::TradeSlot {
            item: desc.map(|desc| desc.object_type.0),
            slot_type: desc
                .and_then(|desc| desc.item.as_ref())
                .map(|item| item.slot_type)
                .unwrap_or(0),
            included: false,
            // Worn slots are not somewhere a trade may take from, and a soulbound item is one the
            // content says belongs to whoever found it.
            tradeable: index >= crate::trades::FIRST_TRADEABLE
                && desc
                    .and_then(|desc| desc.item.as_ref())
                    .is_some_and(|item| !item.soulbound),
        };
    }

    Some(slots)
}

/// Moves the items, and tells both sides how it went.
#[allow(clippy::too_many_arguments)]
async fn settle_trade(
    context: &Context,
    name: &str,
    character_id: i64,
    partner_name: &str,
    partner_character: i64,
    mine: Vec<i16>,
    theirs: Vec<i16>,
) {
    // What is in each offered slot, read now rather than remembered from when the offer was made.
    // The store refuses a trade whose items have moved since, and this is what it compares against.
    let Some(first) = offered_items(context, character_id, &mine).await else {
        context
            .trades
            .finished(name, partner_name, 2, "Trade unsuccessful.");
        return;
    };
    let Some(second) = offered_items(context, partner_character, &theirs).await else {
        context
            .trades
            .finished(name, partner_name, 2, "Trade unsuccessful.");
        return;
    };

    let outcome = context
        .store
        .trade(
            &hendra_store::Offer::new(character_id, first),
            &hendra_store::Offer::new(partner_character, second),
            EQUIPPED_SLOTS as i16,
            LAST_CARRIED_SLOT,
        )
        .await;

    match outcome {
        Ok(_) => context
            .trades
            .finished(name, partner_name, 0, "Trade successful."),
        Err(err) => {
            tracing::info!(%name, %partner_name, %err, "a trade was refused");
            context
                .trades
                .finished(name, partner_name, 2, &err.to_string());
        }
    }
}

/// What is in the slots one side offered.
///
/// A slot that turns out to be empty makes the whole offer wrong rather than smaller: the other
/// side agreed to what they were shown.
async fn offered_items(
    context: &Context,
    character_id: i64,
    slots: &[i16],
) -> Option<Vec<(i16, uuid::Uuid)>> {
    let held = context.store.character(character_id).await.ok()?.inventory;

    slots
        .iter()
        .map(|slot| {
            held.iter()
                .find(|(at, _)| at == slot)
                .map(|(at, item)| (*at, *item))
        })
        .collect()
}

/// Works out what a player meant and, if they may say it, says it.
async fn say_something(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    limit: &mut crate::chat::Limit,
    line: &str,
) -> Option<(String, String)> {
    use crate::chat::Said;

    let said = crate::chat::read(line);
    if said == Said::Nothing {
        return None;
    }

    // Read fresh rather than from the session's copy, so a mute applied while someone is playing
    // takes effect without waiting for them to reconnect.
    if let Ok(account) = context.store.account(player.account.id).await
        && account
            .muted_until
            .is_some_and(|until| until > chrono::Utc::now())
    {
        say(link, "you cannot speak at the moment").await;
        return None;
    }

    if !limit.allow(std::time::Instant::now()) {
        say(link, "you are speaking too quickly").await;
        return None;
    }

    match said {
        Said::Say(text) => {
            placement
                .world
                .send(ToWorld::Chat {
                    handle: placement.handle,
                    text,
                })
                .await;
        }

        Said::Tell { to, text } => {
            // Whispering to yourself is a typo rather than a message.
            if to.eq_ignore_ascii_case(&player.character.name) {
                say(link, "you cannot whisper to yourself").await;
                return None;
            }

            // Somebody who has ignored you does not hear you. Checked here rather than in the
            // world, which knows about bodies and not about accounts, and answered as though they
            // heard: telling the sender they were ignored is telling them to use another account.
            if let Ok(target) = context.store.account_by_name(&to).await
                && context
                    .store
                    .is_listed(
                        target.id,
                        player.account.id,
                        hendra_store::ListKind::Ignored,
                    )
                    .await
                    .unwrap_or(false)
            {
                return None;
            }

            placement
                .world
                .send(ToWorld::Tell {
                    to,
                    from: player.character.name.clone(),
                    text,
                })
                .await;
        }

        // Handed back rather than run here, because carrying one out can move the player between
        // worlds and the placement belongs to the caller.
        Said::Command { name, rest } => return Some((name, rest)),

        Said::Nothing => {}
    }

    None
}

/// Uses what is in a slot.
///
/// The item is read from the durable side rather than taken from the client, which names a slot
/// and nothing else: a client naming an item is making a claim, and a slot is a fact the server
/// can check.
async fn use_item(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    slot: u16,
    aim: (f32, f32),
) {
    // Drinking from a stack rather than from the pack. Taken durably first: a potion that heals and
    // is still in the stack is a potion that heals forever.
    if slot == HEALTH_STACK_SLOT || slot == MAGIC_STACK_SLOT {
        let magic = slot == MAGIC_STACK_SLOT;

        match context.store.take_potion(player.character.id, magic).await {
            Ok(true) => {}
            Ok(false) => return say(link, "you have none of those").await,
            Err(_) => return say(link, "try again shortly").await,
        }

        let name = if magic { MAGIC_POTION } else { HEALTH_POTION };
        let Some(kind) = context.catalog.type_of(name) else {
            return;
        };

        let (reply, answer) = tokio::sync::oneshot::channel();
        placement
            .world
            .send(ToWorld::UseItem {
                handle: placement.handle,
                item: kind,
                aim,
                reply,
            })
            .await;

        let _ = answer.await;
        send_containers(link, &context.catalog, &context.store, player).await;
        return;
    }

    let Some(identity) = read_slot(
        &context.store,
        Location::Inventory {
            character_id: player.character.id,
            slot: slot as i16,
        },
    )
    .await
    else {
        say(link, "there is nothing in that slot").await;
        return;
    };

    let Some(kind) = context.catalog.type_of_uuid(identity) else {
        say(link, "that is not something you can use").await;
        return;
    };

    let (reply, answer) = tokio::sync::oneshot::channel();
    placement
        .world
        .send(ToWorld::UseItem {
            handle: placement.handle,
            item: kind,
            aim,
            reply,
        })
        .await;

    let Ok(ran) = answer.await else {
        return;
    };
    if ran.is_empty() {
        return;
    }

    // What the world handed back. It carries out what belongs to the room and returns what does
    // not, so this is the other half of using an item rather than an afterthought: without it a
    // dye is read, returned and dropped.
    for effect in &ran {
        settle(link, context, player, placement, effect).await;
    }

    // Anything the world could not carry out because it changes something durable is settled here.
    let consumable = context
        .catalog
        .object(kind)
        .and_then(|object| object.item.as_ref())
        .is_some_and(|item| item.consumable);

    if consumable {
        let taken = context
            .store
            .take_item(player.character.id, slot as i16, identity)
            .await;

        if taken.is_ok() {
            send_containers(link, &context.catalog, &context.store, player).await;
            refresh_equipment(context, player, placement).await;
        }
    }
}

/// Carries out one effect the world handed back.
///
/// These change something no world owns: an account's currency, a character's wardrobe, a pet that
/// outlives the room. Each is durable, which is why the world refuses to guess at them.
async fn settle(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    effect: &hendra_content::Effect,
) {
    use hendra_content::Effect;
    use hendra_content::activate::{Appearance, Boost, Currency, Unlock};

    match effect {
        Effect::Currency { kind, amount } => {
            let currency = match kind {
                Currency::Fame => hendra_store::Currency::Fame,
                Currency::Token => hendra_store::Currency::Tokens,
            };
            let _ = context
                .store
                .credit(player.account.id, currency, *amount)
                .await;
        }

        Effect::Appearance { kind, value } => match kind {
            Appearance::Dye => {
                let _ = context.store.set_dye(player.character.id, *value).await;
            }
            Appearance::Skin => {
                // Granted before it is worn, because using a skin item is how it is obtained.
                let skin = uuid::Uuid::from_u128(*value as u128);
                let _ = context.store.grant_skin(player.account.id, skin).await;
                let _ = context
                    .store
                    .wear_skin(player.account.id, player.character.id, *value as i32)
                    .await;
            }
            Appearance::PetSkin => {
                // Applied to the first pet, which is the one a player has out.
                if let Ok(pets) = context.store.pets(player.account.id).await
                    && let Some(pet) = pets.first()
                {
                    let _ = context
                        .store
                        .set_pet_skin(player.account.id, pet.id, *value as i32)
                        .await;
                }
            }
        },

        Effect::Pet { name, permanent } => {
            let kind = name
                .as_deref()
                .and_then(|name| context.catalog.type_of(name))
                .and_then(|found| context.catalog.object(found))
                .map(|desc| desc.uuid);

            if let Some(kind) = kind {
                match context
                    .store
                    .add_pet(player.account.id, kind, *permanent)
                    .await
                {
                    Ok(_) => {}
                    Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
                    Err(_) => {}
                }
            }
        }

        Effect::Boost {
            kind,
            duration_ms,
            multiplier,
        } => {
            let name = match kind {
                Boost::Experience => "experience",
                Boost::LootDrop => "loot_drop",
                Boost::LootTier => "loot_tier",
            };
            let _ = context
                .store
                .add_boost(
                    player.account.id,
                    name,
                    *multiplier,
                    (*duration_ms / 1000).max(1) as i64,
                )
                .await;
        }

        Effect::Unlock { kind, value } => match kind {
            Unlock::Backpack => match context.store.grant_backpack(player.character.id).await {
                Ok(true) => send_containers(link, &context.catalog, &context.store, player).await,
                Ok(false) => say(link, "you already have a backpack").await,
                Err(_) => {}
            },
            Unlock::Class => {
                if let Some(class) = context
                    .catalog
                    .type_of(value)
                    .and_then(|found| context.catalog.object(found))
                {
                    let _ = context
                        .store
                        .purchase_class(player.account.id, class.uuid)
                        .await;
                }
            }
            Unlock::Portal => {
                let _ = context.store.unlock_portal(player.account.id, value).await;
            }
            Unlock::LootBox | Unlock::MysteryDye => {
                // What is inside is decided by content that names it, and nothing names one yet.
                say(link, "there is nothing inside").await;
            }
        },

        Effect::Portal { name, duration_ms } => {
            // Opened where the player stands, which is what a portal item means.
            let kind = context.catalog.type_of(name).unwrap_or(ObjectType::NONE);
            if !kind.is_none() {
                placement
                    .world
                    .send(ToWorld::OpenPortal {
                        at: placement.handle,
                        kind,
                        duration_ms: *duration_ms,
                    })
                    .await;
            }
        }

        // Dispatched on an id in the original, and nothing here reads one yet. Reported rather
        // than ignored, so an item that does nothing says so instead of looking broken.
        Effect::Generic { id } => {
            tracing::debug!(%id, "an item asked for something this server does not do");
            say(link, "nothing happens").await;
        }

        Effect::Unsupported { name } => {
            tracing::debug!(%name, "an unimplemented activate was used");
        }

        // Everything else was the world's, and it has already done it.
        _ => {}
    }
}

/// The runtime number an item identity currently has.
///
/// Resolved on the way to the wire rather than stored, because a runtime number is assigned at load
/// and only the identity survives content changing.
fn number(catalog: &hendra_content::Catalog, item: uuid::Uuid) -> Option<u16> {
    catalog.type_of_uuid(item).map(|found| found.0)
}

/// What the worn slots add, as a stat layer.
///
/// Read from what is worn rather than accumulated as items move, so a missed change cannot leave a
/// stat permanently wrong: the answer is always a function of the inventory as it stands.
fn worn_boosts(catalog: &hendra_content::Catalog, inventory: &[(i16, uuid::Uuid)]) -> [i32; 8] {
    let worn = inventory
        .iter()
        .filter(|(slot, _)| *slot < hendra_characters::EQUIPPED_SLOTS)
        .filter_map(|(_, item)| catalog.type_of_uuid(*item))
        .filter_map(|found| catalog.object(found))
        .filter_map(|desc| desc.item.as_ref());

    let mut boosts = hendra_sim::stats::equipment_boosts(worn);

    // And what a completed set adds on top, which is on none of its pieces: a set gives nothing for
    // three of its four, so this can only be answered by looking at all of them together.
    for set in catalog.sets_worn(&|slot| in_slot(catalog, inventory, slot)) {
        for activate in &set.gives {
            // A set's `IncrementStat` is a boost rather than the permanent rise the same effect
            // means on a potion: `ApplySetBonus` calls `IncrementBoost` for it. So it lasts exactly
            // as long as the set is worn, and taking a piece off recomputes this and takes it away.
            let raised = match hendra_content::Effect::of(activate) {
                hendra_content::Effect::IncrementStat { stat, amount } => Some((stat, amount)),
                hendra_content::Effect::StatBoost { stat, amount, .. } => Some((stat, amount)),
                _ => None,
            };

            if let Some((stat, amount)) = raised
                && let Some(held) = boosts.get_mut(stat as usize)
            {
                *held += amount;
            }
        }
    }

    boosts
}

/// What is worn in one slot, by object type.
fn in_slot(
    catalog: &hendra_content::Catalog,
    inventory: &[(i16, uuid::Uuid)],
    slot: u16,
) -> Option<hendra_content::ObjectType> {
    inventory
        .iter()
        .find(|(at, _)| *at == slot as i16)
        .and_then(|(_, item)| catalog.type_of_uuid(*item))
}

/// The body a character arrives in.
///
/// Health travels with the player rather than resetting at every door, which is the difference
/// between a dungeon and a series of unrelated rooms. The weapon is whatever is in slot zero, so
/// unequipping one and walking through a portal does not hand it back.
fn arrival_of(player: &crate::accounts::Session, context: &Context) -> crate::world_task::Arrival {
    // The class is stored as an identity; the world draws by number.
    let avatar = context
        .catalog
        .type_of_uuid(player.character.class)
        .unwrap_or(ObjectType::NONE);
    let max_hp = player.character.max_hp.max(1);

    // The class decides the base stats. A character whose class the catalog does not have keeps
    // the defaults rather than arriving with nothing.
    let stats = context
        .catalog
        .class(avatar)
        .map(hendra_sim::stats::Stats::starting)
        .unwrap_or_default();

    crate::world_task::Arrival {
        avatar,

        // Read when the session started rather than at every door: a boost lasts half an hour and a
        // player walks through a dozen doors in one, so re-reading it per world would be a query
        // per door for a number that has not moved.
        loot_drop: player.loot_drop,

        stats,
        boosts: worn_boosts(&context.catalog, &player.character.inventory),
        hp: player.character.hp.clamp(1, max_hp),
        max_hp,
        weapon: player
            .character
            .inventory
            .iter()
            .find(|(slot, _)| *slot == 0)
            .and_then(|(_, item)| context.catalog.type_of_uuid(*item))
            .filter(|item| context.catalog.object(*item).is_some()),
    }
}

/// Moves a player from one world to another.
///
/// The order matters. The destination is opened and joined *before* the old world is left, so a
/// world that fails to start leaves the player where they were rather than nowhere at all.
#[allow(clippy::too_many_arguments)]
async fn travel(
    link: &mut Link,
    from: &Placement,
    name: &str,
    account_id: i64,
    portal_type: u16,
    worlds: &Worlds,
    arrival: crate::world_task::Arrival,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<Placement> {
    let destination = worlds.destination_of(portal_type)?.to_string();
    go_to(
        link,
        from,
        name,
        account_id,
        &destination,
        worlds,
        arrival,
        orders,
        died,
    )
    .await
}

/// Moves a player to a named world.
///
/// The same move a portal makes, by name rather than by what was stepped into, because a world can
/// also send somebody somewhere: a realm that has closed sends everybody still in it to the castle.
#[allow(clippy::too_many_arguments)]
async fn go_to(
    link: &mut Link,
    from: &Placement,
    name: &str,
    account_id: i64,
    destination: &str,
    worlds: &Worlds,
    arrival: crate::world_task::Arrival,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<Placement> {
    // Going back into the world you are already in is a no-op, not a rejoin. Rejoining would move
    // the player to the spawn point for no reason.
    if destination == from.world.name.as_ref() {
        return None;
    }

    // A personal world gets one instance per account, so two players in the vault are in two
    // rooms rather than looking at each other's chests.
    let world = worlds.get_or_start_for(destination, account_id)?;
    let handle = join(link, &world, name, arrival, orders, died).await?;

    from.world
        .send(ToWorld::Leave {
            handle: from.handle,
        })
        .await;

    Some(Placement { world, handle })
}

async fn refuse(link: &mut Link, reason: RejectReason) {
    let mut buf = Vec::new();
    ServerMessage::Rejected { reason }.encode(&mut Writer::new(&mut buf));
    let _ = link.send(Delivery::Stream, &buf).await;
}

/// Turns one arriving payload into a world command.
async fn dispatch(received: &Received, placement: &Placement) -> Outcome {
    let mut reader = Reader::new(received.payload());

    let message = match ClientMessage::decode(&mut reader) {
        Ok(message) => message,
        Err(err) => {
            // One undecodable packet is not worth dropping a player over, since a datagram can
            // arrive corrupted, but it is worth knowing about.
            tracing::debug!(%err, "ignoring an undecodable message");
            return Outcome::Continue;
        }
    };

    let handle = placement.handle;
    let world = &placement.world;

    let delivered = match message {
        ClientMessage::Input(input) => {
            world
                .send(ToWorld::Input {
                    handle,
                    x: input.x,
                    y: input.y,
                    client_time_ms: input.client_time_ms,
                    ack: input.ack,
                })
                .await
        }

        ClientMessage::Chat { text } => {
            return Outcome::Chat(text.to_owned());
        }

        ClientMessage::Shoot { angle, .. } => world.send(ToWorld::Shoot { handle, angle }).await,

        ClientMessage::UseItem { slot, x, y } => {
            return Outcome::UseItem { slot, x, y };
        }

        // Handled by the caller, which owns the connection and the store.
        ClientMessage::MoveItem { from, to } => return Outcome::Move { from, to },

        ClientMessage::UsePortal { entity } => {
            return match ask_portal(world, handle, entity).await {
                Some(portal_type) => Outcome::Travel(portal_type),
                None => Outcome::Continue,
            };
        }

        // A second hello on an established session is a client fault, not an attack; ignoring it is
        // safer than re-joining someone who is already in a world.
        ClientMessage::Hello { .. } => {
            tracing::debug!("ignoring a repeated hello");
            true
        }

        // Handled by the caller, which owns the connection, the store and the trade registry.
        ClientMessage::RequestTrade { name } => {
            return Outcome::Trade(Trade::Request(name.to_owned()));
        }
        ClientMessage::ChangeTrade { offer } => return Outcome::Trade(Trade::Change(offer)),
        ClientMessage::AcceptTrade { mine, theirs } => {
            return Outcome::Trade(Trade::Accept { mine, theirs });
        }
        ClientMessage::CancelTrade => return Outcome::Trade(Trade::Cancel),

        // Not a portal: there is nothing to step into, and somebody stuck has to be able to leave.
        ClientMessage::Escape => return Outcome::Escape,

        ClientMessage::Teleport { name } => {
            // The locked-out list is an account question, so it is answered by the caller rather
            // than by the world.
            return Outcome::TeleportTo(name.to_owned());
        }

        ClientMessage::Buy { merchant } => {
            // A guild hall upgrade is bought from something standing in the world like anything
            // else, but what it buys is the hall rather than an item, so it is asked about first.
            let (reply, answer) = tokio::sync::oneshot::channel();
            if world
                .send(ToWorld::BuyHallUpgrade {
                    handle,
                    merchant,
                    reply,
                })
                .await
                && let Ok(Some(upgrade)) = answer.await
            {
                return Outcome::BuyHallUpgrade(upgrade);
            }

            let (reply, answer) = tokio::sync::oneshot::channel();
            if !world
                .send(ToWorld::Buy {
                    handle,
                    merchant,
                    reply,
                })
                .await
            {
                return Outcome::Stop;
            }

            // What is for sale is the world's to say. A client that named the item could name a
            // cheaper one.
            return match answer.await {
                Ok(Some(sale)) => Outcome::Buy(sale),
                _ => Outcome::Say("There is nothing to buy here.".to_string()),
            };
        }

        ClientMessage::Prestige => return Outcome::Prestige,
        ClientMessage::PrestigeBuy { offer } => return Outcome::PrestigeBuy(offer),
        ClientMessage::Guild(command) => return Outcome::Guild(owned_guild(command)),
        ClientMessage::Market(command) => return Outcome::Market(command),
        ClientMessage::EditList { list, name, add } => {
            return Outcome::EditList {
                list,
                name: name.to_owned(),
                add,
            };
        }

        ClientMessage::Pong { .. } => true,
    };

    if delivered {
        Outcome::Continue
    } else {
        Outcome::Stop
    }
}

/// Asks the world whether the player may use a portal, and what kind it is.
async fn ask_portal(world: &WorldHandle, handle: Handle, portal: EntityId) -> Option<u16> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    if !world
        .send(ToWorld::UsePortal {
            handle,
            portal,
            reply,
        })
        .await
    {
        return None;
    }
    answer.await.ok().flatten()
}

/// Buys what a merchant is selling.
///
/// The price and the item come from the world, which read them from where the merchant stands. The
/// payment and the item move together in one transaction, so a purchase that cannot be delivered is
/// a purchase that was not paid for.
async fn buy(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    sale: crate::world_task::Sale,
) {
    // A player's listing is bought from the market, which moves the item from whoever owns it and
    // pays them. Buying it as though it were a shop's stock would mint a second copy and pay nobody.
    if let Some(listing) = sale.listing {
        return market(
            link,
            context,
            player,
            hendra_net::MarketCommand::Buy {
                listing: listing as u64,
            },
        )
        .await;
    }

    let account = match context.store.account(player.account.id).await {
        Ok(account) => account,
        Err(_) => return say(link, "try again shortly").await,
    };

    // The one shop the original gates. Buying fame with fame is the thing it asks a rank for.
    if sale.rank > 0 && account.admin_rank < sale.rank && account.fame < sale.rank as i32 {
        return say(link, "Insufficient rank.").await;
    }

    let item = match context.catalog.object(sale.item) {
        Some(desc) => desc.uuid,
        None => return say(link, "There is nothing to buy here.").await,
    };

    let bought = context
        .store
        .buy_item(hendra_store::Purchase {
            account_id: player.account.id,
            character_id: player.character.id,
            item,
            currency: sale.currency,
            price: sale.price,
            first_slot: EQUIPPED_SLOTS as i16,
            last_slot: LAST_CARRIED_SLOT,
        })
        .await;

    match bought {
        Ok(_) => {
            send_containers(link, &context.catalog, &context.store, player).await;
            say(link, "Purchase successful.").await;
        }
        Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
        Err(err) => {
            tracing::error!(%err, "a purchase failed");
            say(link, "try again shortly").await;
        }
    }
}

/// Carries out a guild command.
///
/// Every rule lives in the store, which decides them against rows rather than against what a client
/// claims: who is in which guild, who outranks whom, and whether a name is taken.
async fn guild(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    name: &str,
    asked: GuildAsk,
) {
    use hendra_store::Rank;

    let me = player.account.id;

    let outcome = match asked {
        GuildAsk::Create(guild_name) => context
            .store
            .found_guild(me, guild_name.trim())
            .await
            .map(|guild| format!("{} founded.", guild.name)),

        // An invitation and a join are the same row from two sides: the store decides whether the
        // caller may add somebody, so this is one path rather than two.
        GuildAsk::Invite(who) | GuildAsk::Join(who) => {
            match context.store.account_by_name(who.trim()).await {
                Ok(target) => context
                    .store
                    .invite_to_guild(me, target.id)
                    .await
                    .map(|_| format!("{} joined.", target.name)),
                Err(_) => Err(hendra_store::StoreError::Refused("no such account")),
            }
        }

        GuildAsk::Remove(who) => match context.store.account_by_name(who.trim()).await {
            Ok(target) => context
                .store
                .remove_from_guild(me, target.id)
                .await
                .map(|()| format!("{} was removed.", target.name)),
            Err(_) => Err(hendra_store::StoreError::Refused("no such account")),
        },

        GuildAsk::SetRank(who, rank) => match context.store.account_by_name(who.trim()).await {
            Ok(target) => {
                let rank = Rank::from_number(rank as i16);
                context
                    .store
                    .set_guild_rank(me, target.id, rank)
                    .await
                    .map(|()| format!("{} is now {rank:?}.", target.name))
            }
            Err(_) => Err(hendra_store::StoreError::Refused("no such account")),
        },

        GuildAsk::SetBoard(text) => context
            .store
            .set_guild_board(me, text.trim())
            .await
            .map(|()| "The board was changed.".to_string()),

        GuildAsk::Leave => context
            .store
            .leave_guild(me)
            .await
            .map(|()| format!("{name} left the guild.")),
    };

    match outcome {
        Ok(said) => say(link, &said).await,
        Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
        Err(hendra_store::StoreError::NameTaken) => say(link, "that name is taken").await,
        Err(err) => {
            tracing::warn!(%err, "a guild command failed");
            say(link, "try again shortly").await;
        }
    }
}

/// Carries out a market command.
///
/// Listing takes the item out of the inventory and buying puts it into another, both in one
/// transaction each, so an item is never in two places and never in none.
async fn market(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    command: hendra_net::MarketCommand,
) {
    use hendra_net::MarketCommand as Ask;

    match command {
        Ask::Browse => {
            let listings = context
                .store
                .listings(MARKET_PAGE)
                .await
                .unwrap_or_default();

            for listing in listings {
                let name = context
                    .catalog
                    .type_of_uuid(listing.item)
                    .and_then(|kind| context.catalog.object(kind))
                    .map(|desc| desc.id.as_str())
                    .unwrap_or("something");

                say(
                    link,
                    &format!("#{} {} for {} gold", listing.id, name, listing.price),
                )
                .await;
            }
        }

        Ask::List { slot, price } => {
            let slot = (slot as i16).saturating_add(EQUIPPED_SLOTS as i16);

            let held = context
                .store
                .character(player.character.id)
                .await
                .ok()
                .and_then(|character| {
                    character
                        .inventory
                        .iter()
                        .find(|(at, _)| *at == slot)
                        .map(|(_, item)| *item)
                });

            let Some(item) = held else {
                return say(link, "there is nothing in that slot").await;
            };

            match context
                .store
                .list_item(
                    player.account.id,
                    player.character.id,
                    slot,
                    item,
                    hendra_store::Currency::Gold,
                    price,
                )
                .await
            {
                Ok(listing) => {
                    send_containers(link, &context.catalog, &context.store, player).await;
                    say(link, &format!("Listed as #{listing}.")).await;
                }
                Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
                Err(_) => say(link, "try again shortly").await,
            }
        }

        Ask::Cancel { listing } => {
            match context
                .store
                .cancel_listing(
                    player.account.id,
                    listing as i64,
                    player.character.id,
                    EQUIPPED_SLOTS as i16,
                    LAST_CARRIED_SLOT,
                )
                .await
            {
                Ok(_) => {
                    send_containers(link, &context.catalog, &context.store, player).await;
                    say(link, "Listing withdrawn.").await;
                }
                Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
                Err(_) => say(link, "try again shortly").await,
            }
        }

        Ask::Buy { listing } => {
            let bought = context
                .store
                .buy_listing(hendra_store::MarketPurchase {
                    buyer_id: player.account.id,
                    character_id: player.character.id,
                    listing_id: listing as i64,
                    first_slot: EQUIPPED_SLOTS as i16,
                    last_slot: LAST_CARRIED_SLOT,
                })
                .await;

            match bought {
                Ok(_) => {
                    send_containers(link, &context.catalog, &context.store, player).await;
                    say(link, "Bought.").await;
                }
                Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
                Err(_) => say(link, "try again shortly").await,
            }
        }
    }
}

/// How many listings one browse shows.
const MARKET_PAGE: i64 = 20;

/// Adds or removes somebody from one of the account's lists.
async fn edit_list(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    list: hendra_net::AccountList,
    name: &str,
    add: bool,
) {
    let Ok(target) = context.store.account_by_name(name.trim()).await else {
        return say(link, "no such account").await;
    };

    if target.id == player.account.id {
        return say(link, "you cannot list yourself").await;
    }

    let kind = match list {
        hendra_net::AccountList::Ignored => hendra_store::ListKind::Ignored,
        hendra_net::AccountList::Locked => hendra_store::ListKind::LockedOut,
    };

    let outcome = if add {
        context
            .store
            .add_to_list(player.account.id, target.id, kind)
            .await
    } else {
        context
            .store
            .remove_from_list(player.account.id, target.id, kind)
            .await
    };

    match outcome {
        Ok(()) => {
            let what = if add { "added to" } else { "removed from" };
            say(link, &format!("{} was {what} your list.", target.name)).await;
        }
        Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
        Err(_) => say(link, "try again shortly").await,
    }
}

/// Moves a player to another player, if the other player allows it.
///
/// Two checks, in two places, because they answer different questions. Whether the other player has
/// locked this one out is an account question, answered here. Whether the move itself is allowed,
/// which is the world's rules about cooldowns, invisibility and where teleporting is permitted at
/// all, is the world's, and answered there.
async fn teleport(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    to: &str,
) {
    if let Ok(target) = context.store.account_by_name(to).await
        && context
            .store
            .is_listed(
                target.id,
                player.account.id,
                hendra_store::ListKind::LockedOut,
            )
            .await
            .unwrap_or(false)
    {
        // Said as though they were not there. Telling somebody they have been locked out is telling
        // them to come back on another account.
        return say(link, &format!("{to} is not here.")).await;
    }

    let (reply, answer) = tokio::sync::oneshot::channel();
    if !placement
        .world
        .send(ToWorld::Teleport {
            handle: placement.handle,
            to: to.to_string(),
            reply,
        })
        .await
    {
        return;
    }

    if let Ok(Some(why)) = answer.await {
        say(link, &why).await;
    }
}

/// Buys one of the things prestige buys.
///
/// The offer is named by its place in the list rather than by item, so a client cannot ask for
/// something expensive at a cheap price. Paid first: a purchase that cannot be delivered leaves the
/// prestige alone, and the reverse order would need a refund that can itself fail.
async fn buy_with_prestige(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    offer: u8,
) {
    let Some((name, price)) = hendra_sim::shop::PRESTIGE_OFFERS.get(offer as usize) else {
        return say(link, "there is no such offer").await;
    };

    let Some(item) = context
        .catalog
        .type_of(name)
        .and_then(|kind| context.catalog.object(kind))
    else {
        return say(link, "that is not for sale here").await;
    };

    if let Err(err) = context
        .store
        .spend_prestige(player.account.id, *price)
        .await
    {
        return match err {
            hendra_store::StoreError::Refused(why) => say(link, why).await,
            _ => say(link, "try again shortly").await,
        };
    }

    // The vault rather than the inventory, as the original does: it goes to the gift chest, which
    // is where something bought outside the world arrives.
    match context.store.add_gift(player.account.id, item.uuid).await {
        Ok(_) => say(link, &format!("{name} was sent to your gift chest.")).await,
        Err(err) => {
            // Paid for and not delivered is the one outcome worth shouting about, because the
            // player is now owed something the server cannot hand over.
            tracing::error!(
                %err,
                account = player.account.id,
                %name,
                "prestige was spent and the item could not be delivered"
            );
            say(link, "try again shortly").await;
        }
    }
}

/// Whispers to a named player.
///
/// Split out because two things reach it: `/tell` and the older bare form the chat reader still
/// understands. Somebody who has ignored you does not hear you, and is answered as though they had.
async fn tell(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    to: &str,
    text: &str,
) {
    if to.eq_ignore_ascii_case(&player.character.name) {
        return say(link, "you cannot whisper to yourself").await;
    }

    if let Ok(target) = context.store.account_by_name(to).await
        && context
            .store
            .is_listed(
                target.id,
                player.account.id,
                hendra_store::ListKind::Ignored,
            )
            .await
            .unwrap_or(false)
    {
        return;
    }

    placement
        .world
        .send(ToWorld::Tell {
            to: to.to_string(),
            from: player.character.name.clone(),
            text: text.to_string(),
        })
        .await;
}

/// Says something to everybody in the caller's guild, wherever they are.
///
/// Guild chat crosses worlds, which is what makes it different from saying something aloud, so it
/// goes through the roster rather than through any one world.
async fn guild_say(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    text: &str,
) {
    let Ok(Some((guild, _))) = context.store.guild_of(player.account.id).await else {
        return say(link, "you are not in a guild").await;
    };

    let Ok(members) = context.store.guild_members(guild).await else {
        return say(link, "try again shortly").await;
    };

    let line = ServerMessage::Chat {
        from: &format!("{} [guild]", player.character.name),
        text,
    };

    for member in members {
        context.trades.send(&member.name, &line);
    }
}

/// Who is in the caller's guild.
async fn guild_who(link: &mut Link, context: &Context, player: &crate::accounts::Session) {
    let Ok(Some((guild, _))) = context.store.guild_of(player.account.id).await else {
        return say(link, "you are not in a guild").await;
    };

    let Ok(members) = context.store.guild_members(guild).await else {
        return say(link, "try again shortly").await;
    };

    let named: Vec<String> = members
        .iter()
        .map(|member| format!("{} ({:?})", member.name, member.rank))
        .collect();

    say(link, &format!("{}: {}", members.len(), named.join(", "))).await;
}

/// Reads a guild rank the way somebody would type it.
fn guild_rank_named(text: &str) -> u8 {
    use hendra_store::Rank;

    let rank = match text.trim().to_ascii_lowercase().as_str() {
        "founder" => Rank::Founder,
        "officer" | "leader" => Rank::Officer,
        "member" => Rank::Member,
        _ => Rank::Initiate,
    };

    rank.number().max(0) as u8
}

/// Adds somebody to one of the account's lists, or takes them off it.
async fn list_person(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    kind: hendra_store::ListKind,
    name: &str,
    add: bool,
) {
    let Ok(target) = context.store.account_by_name(name).await else {
        return say(link, "no such player").await;
    };

    if target.id == player.account.id {
        return say(link, "you cannot list yourself").await;
    }

    let outcome = if add {
        context
            .store
            .add_to_list(player.account.id, target.id, kind)
            .await
    } else {
        context
            .store
            .remove_from_list(player.account.id, target.id, kind)
            .await
    };

    match outcome {
        Ok(()) => {
            let what = if add { "added to" } else { "removed from" };
            say(link, &format!("{} was {what} your list.", target.name)).await;
        }
        Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
        Err(_) => say(link, "try again shortly").await,
    }
}

/// What the caller has listed on the market.
async fn my_market(link: &mut Link, context: &Context, player: &crate::accounts::Session) {
    let listings = context
        .store
        .listings_of(player.account.id)
        .await
        .unwrap_or_default();

    if listings.is_empty() {
        return say(link, "you have not listed anything").await;
    }

    for listing in listings {
        let name = context
            .catalog
            .type_of_uuid(listing.item)
            .and_then(|kind| context.catalog.object(kind))
            .map(|desc| desc.id.as_str())
            .unwrap_or("something");

        say(
            link,
            &format!("#{} {} for {} gold", listing.id, name, listing.price),
        )
        .await;
    }
}

/// The most recent thing the caller listed, for `/oops`.
async fn last_listing(context: &Context, player: &crate::accounts::Session) -> Option<u64> {
    let listings = context.store.listings_of(player.account.id).await.ok()?;
    listings.last().map(|listing| listing.id as u64)
}

/// Tells a player something the server knows.
async fn report(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    what: crate::commands::Report,
) {
    use crate::commands::Report;

    match what {
        Report::World => {
            let (reply, answer) = tokio::sync::oneshot::channel();
            placement.world.send(ToWorld::Who { reply }).await;
            let here = answer.await.unwrap_or_default();

            say(
                link,
                &format!("{} ({} here)", placement.world.name, here.len()),
            )
            .await;
        }

        Report::Quest => {
            let (reply, answer) = tokio::sync::oneshot::channel();
            placement
                .world
                .send(ToWorld::Quest {
                    handle: placement.handle,
                    reply,
                })
                .await;

            match answer.await {
                Ok(Some((name, x, y))) => {
                    say(link, &format!("{name}, at {x}, {y}.")).await;
                }
                _ => say(link, "nothing near enough to be worth it").await,
            }
        }

        Report::Position => {
            let (reply, answer) = tokio::sync::oneshot::channel();
            placement
                .world
                .send(ToWorld::Where {
                    handle: placement.handle,
                    reply,
                })
                .await;

            match answer.await {
                Ok(Some((x, y))) => say(link, &format!("You are at {x}, {y}.")).await,
                _ => say(link, "nowhere in particular").await,
            }
        }

        Report::Who => {
            let (reply, answer) = tokio::sync::oneshot::channel();
            placement.world.send(ToWorld::Who { reply }).await;

            match answer.await {
                Ok(names) if !names.is_empty() => {
                    say(link, &format!("{}: {}", names.len(), names.join(", "))).await;
                }
                _ => say(link, "nobody but you").await,
            }
        }

        Report::Online => {
            let names = context.trades.present();
            say(
                link,
                &format!("{} online: {}", names.len(), names.join(", ")),
            )
            .await;
        }

        Report::Uptime => {
            let up = context.started.elapsed();
            let (hours, minutes) = (up.as_secs() / 3600, (up.as_secs() % 3600) / 60);
            say(link, &format!("Up for {hours}h {minutes}m.")).await;
        }

        Report::Commands => {
            let rank = context
                .store
                .account(player.account.id)
                .await
                .map(|account| hendra_store::Admin::from_number(account.admin_rank))
                .unwrap_or(hendra_store::Admin::None);

            // Only what they may actually use. Listing the rest would be telling everybody what a
            // moderator can do and inviting them to try.
            for command in crate::commands::ALL
                .iter()
                .filter(|command| command.needs.met_by(rank))
            {
                say(link, &format!("/{} — {}", command.name, command.summary)).await;
            }
        }

        Report::Prestige => match context.store.prestige_of(player.account.id).await {
            Ok((held, total)) => {
                say(link, &format!("{held} prestige, {total} earned in all.")).await;
            }
            Err(_) => say(link, "try again shortly").await,
        },

        Report::LeftToMax => {
            let Some(class) = context
                .catalog
                .type_of_uuid(player.character.class)
                .and_then(|kind| {
                    context
                        .catalog
                        .classes()
                        .iter()
                        .find(|class| class.object_type == kind)
                })
            else {
                return say(link, "nothing to say about that class").await;
            };

            let (reply, answer) = tokio::sync::oneshot::channel();
            placement
                .world
                .send(ToWorld::Stats {
                    handle: placement.handle,
                    reply,
                })
                .await;

            let Ok(Some(base)) = answer.await else {
                return say(link, "try again shortly").await;
            };

            let left: Vec<String> = STAT_NAMES
                .iter()
                .enumerate()
                .filter_map(|(index, name)| {
                    let ceiling = class.stats.get(index)?.maximum;
                    let held = base[index];
                    (ceiling > held).then(|| format!("{name} {}", ceiling - held))
                })
                .collect();

            if left.is_empty() {
                say(link, "You are at maximum.").await;
            } else {
                say(link, &left.join(", ")).await;
            }
        }

        // The original answers both of these with a joke rather than a fact, and there is nothing
        // behind either to answer with instead.
        Report::CurrentSong => say(link, "Whatever your client is playing.").await,
        Report::Time => say(link, "Time for you to get a watch.").await,
    }
}

/// What each of the eight stats is called, in the order the content lists them.
const STAT_NAMES: [&str; 8] = [
    "HP",
    "MP",
    "Attack",
    "Defense",
    "Speed",
    "Dexterity",
    "Vitality",
    "Wisdom",
];

/// Something a moderator or administrator does to somebody.
async fn moderate(
    link: &mut Link,
    context: &Context,
    what: crate::commands::Moderation,
    name: &str,
    rest: &str,
) {
    use crate::commands::Moderation;

    // An announcement names nobody, so it is answered before anybody is looked up.
    if what == Moderation::Announce {
        let line = ServerMessage::Chat {
            from: "Server",
            text: rest,
        };
        for who in context.trades.present() {
            context.trades.send(&who, &line);
        }
        return say(link, "Said.").await;
    }

    // An address is not an account, so it is answered before anybody is looked up.
    if what == Moderation::BanAddress {
        return match context.store.ban_address(name).await {
            Ok(()) => say(link, &format!("{name} is banned.")).await,
            Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
            Err(_) => say(link, "try again shortly").await,
        };
    }

    let Ok(target) = context.store.account_by_name(name).await else {
        return say(link, "no such player").await;
    };

    let outcome = match what {
        Moderation::Mute | Moderation::Unmute => {
            // Minutes, defaulting to an hour. A mute with no end is a ban nobody remembers
            // applying, so this one always has one.
            let until = (what == Moderation::Mute).then(|| {
                let minutes = rest
                    .trim()
                    .parse::<i64>()
                    .unwrap_or(60)
                    .clamp(1, 60 * 24 * 30);
                chrono::Utc::now() + chrono::Duration::minutes(minutes)
            });

            context.store.mute(target.id, until).await.map(|()| {
                let what = if until.is_some() { "muted" } else { "unmuted" };
                format!("{} is {what}.", target.name)
            })
        }

        Moderation::Ban | Moderation::Unban => {
            let banned = what == Moderation::Ban;
            context.store.set_banned(target.id, banned).await.map(|()| {
                let what = if banned { "banned" } else { "unbanned" };
                format!("{} is {what}.", target.name)
            })
        }

        Moderation::Kick => {
            // Ending the connection is the session's, and the roster is what can reach it.
            context.trades.kick(&target.name);
            Ok(format!("{} was kicked.", target.name))
        }

        Moderation::Rank => {
            let rank = match rest.trim().to_ascii_lowercase().as_str() {
                "admin" | "administrator" => hendra_store::Admin::Administrator,
                "mod" | "moderator" => hendra_store::Admin::Moderator,
                _ => hendra_store::Admin::None,
            };

            context
                .store
                .set_admin_rank(target.id, rank)
                .await
                .map(|()| format!("{} is now {rank:?}.", target.name))
        }

        Moderation::SetFame | Moderation::SetGold | Moderation::SetPrestige => {
            let Ok(amount) = rest.trim().parse::<i32>() else {
                return say(link, "how much?").await;
            };

            let currency = match what {
                Moderation::SetGold => hendra_store::Currency::Gold,
                Moderation::SetPrestige => hendra_store::Currency::Prestige,
                _ => hendra_store::Currency::Fame,
            };

            context
                .store
                .set_currency(target.id, currency, amount)
                .await
                .map(|()| format!("{} now has {amount}.", target.name))
        }

        Moderation::Gift => {
            let Some(item) = context
                .catalog
                .type_of(rest.trim())
                .and_then(|kind| context.catalog.object(kind))
            else {
                return say(link, "there is no such item").await;
            };

            context
                .store
                .add_gift(target.id, item.uuid)
                .await
                .map(|_| format!("{} was sent {}.", target.name, item.id))
        }

        Moderation::Rename => context
            .store
            .rename_account(target.id, rest.trim())
            .await
            .map(|()| format!("{} is now {}.", target.name, rest.trim())),

        Moderation::Unname => context
            .store
            .rename_account(target.id, &format!("Player{}", target.id))
            .await
            .map(|()| format!("{} was unnamed.", target.name)),

        Moderation::BanAddress => unreachable!("answered above"),
        Moderation::Announce => unreachable!("answered above"),
    };

    match outcome {
        Ok(said) => say(link, &said).await,
        Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
        Err(hendra_store::StoreError::NameTaken) => say(link, "that name is taken").await,
        Err(err) => {
            tracing::warn!(%err, "a moderation command failed");
            say(link, "try again shortly").await;
        }
    }
}

/// Moves a player into a world that has already been chosen.
///
/// The other half of `go_to`, for the worlds whose instance is decided by something other than a
/// name: a guild hall belongs to a guild rather than to whoever asked for it.
async fn enter(
    link: &mut Link,
    from: &Placement,
    name: &str,
    world: WorldHandle,
    arrival: crate::world_task::Arrival,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<Placement> {
    let handle = join(link, &world, name, arrival, orders, died).await?;

    from.world
        .send(ToWorld::Leave {
            handle: from.handle,
        })
        .await;

    Some(Placement { world, handle })
}

/// Buys a larger hall for the caller's guild.
///
/// Paid from the guild's fame rather than the buyer's, because the hall belongs to the guild. Only
/// an officer or above may, for the same reason: a hall is something the guild owns and a member
/// spending its fame is a member spending everyone's.
async fn buy_hall_upgrade(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    upgrade: crate::world_task::HallUpgrade,
) {
    use hendra_store::Rank;

    let Ok(Some((guild, rank))) = context.store.guild_of(player.account.id).await else {
        return say(link, "you are not in a guild").await;
    };

    if rank < Rank::Officer {
        return say(link, "insufficient privileges").await;
    }

    // Paid first, and only then raised. A purchase that cannot be applied leaves the fame alone,
    // and the reverse order would need a refund that can itself fail.
    if let Err(err) = context.store.spend_guild_fame(guild, upgrade.price).await {
        return match err {
            hendra_store::StoreError::Refused(why) => say(link, why).await,
            _ => say(link, "try again shortly").await,
        };
    }

    match context.store.raise_guild_level(guild, upgrade.level).await {
        Ok(()) => {
            say(
                link,
                "Your hall has been upgraded. It will be larger next time you enter.",
            )
            .await;
        }
        Err(err) => {
            // Paid for and not applied is worth shouting about: the guild is now owed a hall.
            tracing::error!(%err, guild, "guild fame was spent and the hall was not raised");
            say(link, "try again shortly").await;
        }
    }
}

/// Carries out an administrator's tool.
///
/// Split by where the answer lives: some are the world's, some are the store's, and one is both.
async fn wield(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    what: crate::commands::Wielded,
    rest: &str,
) {
    use crate::commands::Wielded;
    use crate::world_task::Wielding;

    // The world's, and each answers with a line to repeat.
    let in_world = match what {
        Wielded::Spawn => {
            // "Slime 5" or just "Slime". A number at the end is how many, because that is how
            // somebody types it and there is nothing else it could mean.
            let (name, count) = match rest.rsplit_once(char::is_whitespace) {
                Some((name, tail)) => match tail.parse::<usize>() {
                    Ok(count) => (name.trim(), count),
                    Err(_) => (rest, 1),
                },
                None => (rest, 1),
            };

            Some(Wielding::Spawn {
                name: name.to_string(),
                count,
            })
        }
        Wielded::KillAll => Some(Wielding::KillAll {
            name: rest.to_string(),
        }),
        Wielded::Size => match rest.trim().parse::<u16>() {
            Ok(percent) => Some(Wielding::Size { percent }),
            Err(_) => return say(link, "how large?").await,
        },
        Wielded::Hide => Some(Wielding::Hide),
        Wielded::CloseRealm => Some(Wielding::CloseRealm),
        Wielded::Pause => Some(Wielding::Pause),
        Wielded::Effect => Some(Wielding::Effect {
            name: rest.to_string(),
        }),
        Wielded::Glow => match colour_named(rest.trim()) {
            Some(colour) => Some(Wielding::Glow { colour }),
            None => return say(link, "which colour?").await,
        },
        Wielded::KillPlayer => Some(Wielding::KillPlayer {
            name: rest.to_string(),
        }),
        Wielded::SummonAll => Some(Wielding::SummonAll),
        Wielded::Setpiece => Some(Wielding::Setpiece {
            name: rest.to_string(),
        }),
        Wielded::ClearSpawn | Wielded::ClearGraves => Some(Wielding::ClearSpawn),
        Wielded::Debug => Some(Wielding::Debug),
        _ => None,
    };

    if let Some(what) = in_world {
        let (reply, answer) = tokio::sync::oneshot::channel();
        if placement
            .world
            .send(ToWorld::Wield {
                handle: placement.handle,
                what,
                reply,
            })
            .await
            && let Ok(said) = answer.await
        {
            say(link, &said).await;
        }
        return;
    }

    // The rest are the store's, or the registry's.
    match what {
        Wielded::Give => {
            let Some(item) = context
                .catalog
                .type_of(rest.trim())
                .and_then(|kind| context.catalog.object(kind))
            else {
                return say(link, "there is no such item").await;
            };

            match context
                .store
                .give_item(
                    player.character.id,
                    item.uuid,
                    EQUIPPED_SLOTS as i16,
                    LAST_CARRIED_SLOT,
                )
                .await
            {
                Ok(_) => {
                    send_containers(link, &context.catalog, &context.store, player).await;
                    say(link, &format!("{} is yours.", item.id)).await;
                }
                Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
                Err(_) => say(link, "try again shortly").await,
            }
        }

        Wielded::ClearPack => {
            let Ok(character) = context.store.character(player.character.id).await else {
                return say(link, "try again shortly").await;
            };

            let mut emptied = 0;
            for (slot, item) in character.inventory {
                if slot < EQUIPPED_SLOTS as i16 {
                    continue;
                }
                if context
                    .store
                    .take_item(player.character.id, slot, item)
                    .await
                    .is_ok()
                {
                    emptied += 1;
                }
            }

            send_containers(link, &context.catalog, &context.store, player).await;
            say(link, &format!("{emptied} taken out.")).await;
        }

        Wielded::MaxStats | Wielded::MaxLevel => {
            // Both are the same write: the character's own numbers, which the world reads on the
            // next arrival. Applied durably rather than to the body, so it survives walking out.
            let outcome = if what == Wielded::MaxLevel {
                context
                    .store
                    .set_level(player.character.id, MAX_LEVEL)
                    .await
            } else {
                context.store.max_stats(player.character.id).await
            };

            match outcome {
                Ok(()) => say(link, "Done. It takes effect when you next arrive.").await,
                Err(_) => say(link, "try again shortly").await,
            }
        }

        Wielded::Quake => {
            let destination = if rest.trim().is_empty() {
                crate::commands::NEXUS
            } else {
                rest.trim()
            };

            let (reply, answer) = tokio::sync::oneshot::channel();
            placement.world.send(ToWorld::Who { reply }).await;
            let here = answer.await.unwrap_or_default();

            placement
                .world
                .send(ToWorld::SendEveryoneTo {
                    world: destination.to_string(),
                })
                .await;

            say(link, &format!("{} sent to {destination}.", here.len())).await;
        }

        Wielded::Visit => {
            let Some(world) = context.trades.world_of(rest.trim()) else {
                return say(link, "they are not here").await;
            };
            say(link, &format!("They are in {world}.")).await;
        }

        Wielded::Worlds => {
            let running = context.worlds.running();
            say(link, &format!("{}: {}", running.len(), running.join(", "))).await;
        }

        Wielded::Spectate => {
            // Watching somebody is a client-side camera move, and the only thing the server owes it
            // is where to look. Answered with the position rather than a mode, so a client that
            // cannot follow still learns something and one that can follows.
            let Some(world) = context.trades.world_of(rest.trim()) else {
                return say(link, "they are not here").await;
            };
            say(link, &format!("{} is in {world}.", rest.trim())).await;
        }

        Wielded::SetStar => {
            let (who, stars) = rest
                .trim()
                .split_once(char::is_whitespace)
                .unwrap_or((rest, ""));
            let Ok(stars) = stars.trim().parse::<i32>() else {
                return say(link, "how many stars?").await;
            };
            let Ok(target) = context.store.account_by_name(who.trim()).await else {
                return say(link, "no such player").await;
            };

            // Stars are earned fame, so setting them is setting that: there is no second number.
            match context
                .store
                .set_currency(target.id, hendra_store::Currency::Fame, stars.max(0))
                .await
            {
                Ok(()) => say(link, &format!("{} now has {stars}.", target.name)).await,
                Err(_) => say(link, "try again shortly").await,
            }
        }

        Wielded::LootSpawn => {
            let Some(item) = context
                .catalog
                .type_of(rest.trim())
                .and_then(|kind| context.catalog.object(kind))
            else {
                return say(link, "there is no such item").await;
            };

            // Given rather than dropped, because a bag on the ground is not durable and an
            // administrator asking for an item wants the item.
            match context
                .store
                .give_item(
                    player.character.id,
                    item.uuid,
                    EQUIPPED_SLOTS as i16,
                    LAST_CARRIED_SLOT,
                )
                .await
            {
                Ok(_) => {
                    send_containers(link, &context.catalog, &context.store, player).await;
                    say(link, &format!("{} is yours.", item.id)).await;
                }
                Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
                Err(_) => say(link, "try again shortly").await,
            }
        }

        Wielded::Reskin => {
            let Some(skin) = context.catalog.type_of(rest.trim()) else {
                return say(link, "there is no such skin").await;
            };

            // Granted and then worn, rather than worn without owning it: the ownership check is
            // what stops a skin being worn by somebody who has not bought it, and an administrator
            // reaching past it would be a second door into the wardrobe.
            let Some(identity) = context.catalog.object(skin).map(|desc| desc.uuid) else {
                return say(link, "there is no such skin").await;
            };

            if context
                .store
                .grant_skin(player.account.id, identity)
                .await
                .is_err()
            {
                return say(link, "try again shortly").await;
            }

            match context
                .store
                .wear_skin(player.account.id, player.character.id, skin.0 as i32)
                .await
            {
                Ok(()) => say(link, "Worn. It shows next time you arrive.").await,
                Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
                Err(_) => say(link, "try again shortly").await,
            }
        }

        Wielded::SetStat => {
            let (which, amount) = rest
                .trim()
                .split_once(char::is_whitespace)
                .unwrap_or((rest, ""));
            let Ok(amount) = amount.trim().parse::<i32>() else {
                return say(link, "to what?").await;
            };

            match context
                .store
                .set_stat(player.character.id, which.trim(), amount)
                .await
            {
                Ok(()) => say(link, "Done. It takes effect when you next arrive.").await,
                Err(hendra_store::StoreError::Refused(why)) => say(link, why).await,
                Err(_) => say(link, "try again shortly").await,
            }
        }

        Wielded::WelcomeMessage => match context.store.set_setting(WELCOME, rest.trim()).await {
            Ok(()) => say(link, "Set.").await,
            Err(_) => say(link, "try again shortly").await,
        },

        Wielded::Music => {
            // What is playing is the client's, and the world only names it. Kept with the world
            // rather than told to each client, so somebody arriving later hears the same thing.
            say(link, "what plays is chosen by your client").await;
        }

        Wielded::ToQuest => {
            report(
                link,
                context,
                player,
                placement,
                crate::commands::Report::Quest,
            )
            .await;
        }

        Wielded::Override | Wielded::RemoveOverride => {
            // Acting as another account means holding two identities on one connection, and every
            // durable write here names the account it belongs to. There is no safe way to do it
            // that is not just logging in as them.
            say(link, "this server has no way to act as another account").await;
        }

        Wielded::Link | Wielded::Unlink => {
            // A world here is reachable by the name its definition gives it, and that is decided at
            // load rather than at runtime, so there is nothing to link or unlink.
            say(link, "worlds here are reachable by the name they are given").await;
        }

        Wielded::Warg => {
            say(link, "this server has no way to control an enemy").await;
        }

        Wielded::Reboot => {
            tracing::warn!(
                account = player.account.id,
                "an administrator asked for a stop"
            );
            say(link, "not from here: stop the process").await;
        }

        Wielded::Refuse => {
            say(link, "this server does not do that").await;
        }

        _ => {}
    }
}

/// What an administrator's welcome message is stored under.
const WELCOME: &str = "welcome";

/// Reads a colour the way somebody types one.
fn colour_named(text: &str) -> Option<i32> {
    // A name for the handful worth typing, and a hex number for anything else, which is what an
    // administrator who wants a particular shade will reach for.
    Some(match text.to_ascii_lowercase().as_str() {
        "red" => 0xff_0000,
        "green" => 0x00_ff00,
        "blue" => 0x00_00ff,
        "white" => 0xff_ffff,
        "black" => 0x00_0000,
        "yellow" => 0xff_ff00,
        "purple" => 0xff_00ff,
        "none" | "off" => 0,
        other => i32::from_str_radix(other.trim_start_matches('#'), 16).ok()?,
    })
}

/// The highest level a character reaches.
const MAX_LEVEL: i16 = 20;

/// Takes a player to their own guild's hall.
///
/// One path, used by the command and by the portal alike: a hall is one room per guild and the
/// level chooses its map, and two doors that decided those differently would be two different rooms
/// with one name.
async fn enter_guild_hall(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    from: &Placement,
    name: &str,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<Placement> {
    let Ok(Some((guild, _))) = context.store.guild_of(player.account.id).await else {
        say(link, "you are not in a guild").await;
        return None;
    };

    let level = context
        .store
        .guild(guild)
        .await
        .map(|guild| guild.level)
        .unwrap_or(0);

    let Some(hall) = context
        .worlds
        .get_or_start_hall(crate::commands::GUILD_HALL, guild, level)
    else {
        say(link, "your hall is not available").await;
        return None;
    };

    enter(
        link,
        from,
        name,
        hall,
        arrival_of(player, context),
        orders,
        died,
    )
    .await
}

/// Answers a death.
///
/// Follows `Player.Death`, whose checks run in order and each of which can stop the rest: a
/// resurrection spends an item and sends them home, and the nexus never kills anybody. Returns
/// whether the session is over.
///
/// The character is written down as dead *before* anything is sent, because that is the half that
/// must not be lost: a death message the player sees and a character the database still calls alive
/// is a character they can log back into.
async fn die(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    departed: crate::world_task::Departed,
) -> bool {
    // Nowhere safe kills anybody. The nexus and the vault are places rather than fights, and dying
    // in one is a bug in whatever put damage there rather than the end of a character.
    if context.worlds.is_personal(&placement.world.name)
        || placement.world.name.as_ref() == crate::commands::NEXUS
    {
        say(link, "Something hurt you, but not here.").await;
        return false;
    }

    // A resurrection spends the item and sends them home alive. Checked before anything durable
    // happens, because the whole point is that the death does not.
    if let Some((slot, item)) = resurrection(context, player).await
        && context
            .store
            .take_item(player.character.id, slot, item)
            .await
            .is_ok()
    {
        {
            send_containers(link, &context.catalog, &context.store, player).await;
            say(link, "Your amulet breaks, and you are somewhere else.").await;
            return false;
        }
    }

    // The durable half first. Everything after this is telling people about it.
    //
    // The graveyard row and the character being marked dead go together in one transaction: a
    // character marked dead with no death recorded loses the only account of what happened to it,
    // and a death recorded against a living character is a graveyard entry for somebody still
    // playing.
    let fame = context
        .store
        .character(player.character.id)
        .await
        .map(|character| character.fame)
        .unwrap_or(0);

    // Only once per account, ever, which is why it is asked of the graveyard rather than worked out
    // from anything that could change.
    let ancestor = !context
        .store
        .has_died_before(player.account.id)
        .await
        .unwrap_or(true);

    // What the character did, which is what its death is worth beyond the fame it had. Read from
    // the database rather than from the world, because the world holds only this session's counting
    // and a character's life is longer than one session.
    let tally = context
        .store
        .tally(player.character.id)
        .await
        .unwrap_or_default();

    let (final_fame, earned) = hendra_sim::fame::bonuses(
        &remembered(&tally),
        hendra_sim::fame::Finished {
            level: player.character.level,
            fame,
            first_death: ancestor,
            equipment_bonus: worn_fame_bonus(context, player).await,

            // Beating every character the account has had, which is what first born is. Compared
            // against the graveyard rather than against anything living: a character still alive
            // has not finished, and a number that could still go up is not a record.
            best_yet: fame
                > context
                    .store
                    .best_final_fame(player.account.id)
                    .await
                    .unwrap_or(0),
        },
    );

    for bonus in &earned {
        say(
            link,
            &format!("{}: {} fame, {}", bonus.name, bonus.fame, bonus.why),
        )
        .await;
    }

    if let Err(err) = context
        .store
        .record_death(hendra_store::Death {
            account_id: player.account.id,
            character_id: player.character.id,
            killed_by: departed.killer.clone(),
            final_fame,
            first_born: ancestor,
            bonuses: earned
                .iter()
                .map(|bonus| format!("{}: {}", bonus.name, bonus.fame))
                .collect(),
        })
        .await
    {
        // Said loudly. A death that was not written down is a character the player can log back
        // into, which is the one outcome worth shouting about.
        tracing::error!(
            %err,
            character = player.character.id,
            account = player.account.id,
            "a death could not be recorded"
        );
    }

    // How much of the character was finished, which decides the stone and how long it stands.
    let maxed = maxed_stats(context, player).await;

    placement
        .world
        .send(ToWorld::Gravestone {
            at: (departed.x, departed.y),
            name: player.character.name.clone(),
            maxed,
            level: player.character.level,
            rekt: false,
        })
        .await;

    // Said everywhere for a death worth hearing about, and to the room otherwise. A death nobody
    // was near is still a death, and the original draws the line at six-of-eight or a thousand fame.
    let notable = maxed >= 6 || final_fame >= NOTABLE_FAME;
    let line = format!(
        "{} died to {} ({maxed}/8, {final_fame} fame)",
        player.character.name, departed.killer
    );

    if notable {
        let said = ServerMessage::Chat {
            from: "Server",
            text: &line,
        };
        for who in context.trades.present() {
            context.trades.send(&who, &said);
        }
    } else {
        placement.world.send(ToWorld::Announce { text: line }).await;
    }

    let mut buffer = Vec::new();
    ServerMessage::Died {
        character: player.character.id.max(0) as u32,
        killed_by: departed.killer,
        fame: final_fame,
    }
    .encode(&mut Writer::new(&mut buffer));
    let _ = link.send(Delivery::Stream, &buffer).await;

    true
}

/// A death worth telling the whole server about.
const NOTABLE_FAME: i32 = 1000;

/// The worn item that will spend itself to prevent a death, if there is one.
async fn resurrection(
    context: &Context,
    player: &crate::accounts::Session,
) -> Option<(i16, uuid::Uuid)> {
    let character = context.store.character(player.character.id).await.ok()?;

    character
        .inventory
        .iter()
        .filter(|(slot, _)| *slot < EQUIPPED_SLOTS as i16)
        .find(|(_, item)| {
            context
                .catalog
                .type_of_uuid(*item)
                .and_then(|kind| context.catalog.object(kind))
                .and_then(|desc| desc.item.as_ref())
                .is_some_and(|item| item.resurrects)
        })
        .copied()
}

/// How many of the eight stats are at their class's maximum.
async fn maxed_stats(context: &Context, player: &crate::accounts::Session) -> usize {
    let Some(class) = context
        .catalog
        .type_of_uuid(player.character.class)
        .and_then(|kind| {
            context
                .catalog
                .classes()
                .iter()
                .find(|class| class.object_type == kind)
        })
    else {
        return 0;
    };

    // Health and magic are the two the character stores; the other six live in the world and are
    // recomputed on arrival, so this counts what can actually be known here.
    let mut maxed = 0;
    if player.character.max_hp >= class.stats[0].maximum {
        maxed += 1;
    }
    if player.character.max_mp >= class.stats[1].maximum {
        maxed += 1;
    }
    maxed
}

/// What a character has done, as the bonuses need it.
fn remembered(tally: &hendra_store::TallyRow) -> hendra_sim::fame::Tally {
    hendra_sim::fame::Tally {
        shots: tally.shots,
        shots_that_hit: tally.shots_that_hit,
        abilities_used: tally.abilities_used,
        tiles_seen: tally.tiles_seen,
        teleports: tally.teleports,
        potions_drunk: tally.potions_drunk,
        monster_kills: tally.monster_kills,
        god_kills: tally.god_kills,
        cube_kills: tally.cube_kills,
        oryx_kills: tally.oryx_kills,
        quests_completed: tally.quests_completed,
        level_up_assists: tally.level_up_assists,
        dungeons_completed: tally.dungeons_completed.max(0) as u32,
    }
}

/// What the four worn items add to a death, as a percentage.
///
/// `FameBonus` in the content, which a handful of items carry. Only the worn slots count: what is
/// in the pack was not being used.
async fn worn_fame_bonus(context: &Context, player: &crate::accounts::Session) -> i32 {
    let Ok(character) = context.store.character(player.character.id).await else {
        return 0;
    };

    character
        .inventory
        .iter()
        .filter(|(slot, _)| *slot < EQUIPPED_SLOTS as i16)
        .filter_map(|(_, item)| {
            context
                .catalog
                .type_of_uuid(*item)
                .and_then(|kind| context.catalog.object(kind))
        })
        .filter_map(|desc| desc.item.as_ref())
        .map(|item| item.fame_bonus)
        .sum()
}

/// Whether an item belongs to whoever found it.
///
/// Soulbound in the content. It cannot be traded, and dropping it makes a bag only the dropper can
/// open, so there is no path by which it reaches somebody else.
fn is_soulbound(catalog: &hendra_content::Catalog, item: uuid::Uuid) -> bool {
    catalog
        .type_of_uuid(item)
        .and_then(|kind| catalog.object(kind))
        .and_then(|desc| desc.item.as_ref())
        .is_some_and(|item| item.soulbound)
}
