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

    /// Who is trading with whom.
    pub trades: Arc<crate::trades::Trades>,
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
        SlotLocation::Bag { .. } | SlotLocation::Ground => return None,
    })
}

/// Handles one connection for its lifetime.
pub async fn serve(mut link: Link, context: Arc<Context>, entry: WorldHandle) {
    let peer = link.remote_address();

    let Some((player, mut placement)) = handshake(&mut link, &context, &entry).await else {
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

    // Reachable by name from now on, which is what lets somebody else ask them to trade.
    context
        .trades
        .arrived(&name, player.character.id, link.sender());

    // The ground first, because a client that has not been told the map cannot place anything it
    // is about to be told about.
    send_terrain(&mut link, &placement).await;

    // Then what it is carrying, before it can move any of it.
    send_containers(&mut link, &context.catalog, &context.store, &player).await;

    // One per connection. A limit shared between players would let a busy world silence a quiet
    // one, and a limit that outlived a connection would follow the wrong person.
    let mut limit = crate::chat::Limit::new();

    loop {
        let Some(received) = link.recv().await else {
            break;
        };

        match dispatch(&received, &placement).await {
            Outcome::Continue => {}
            Outcome::Stop => break,

            Outcome::Move { from, to } => {
                move_item(&mut link, &context, &player, &placement, from, to).await;
            }

            Outcome::Chat(line) => {
                say_something(&mut link, &context, &player, &placement, &mut limit, &line).await;
            }

            Outcome::UseItem { slot, x, y } => {
                use_item(&mut link, &context, &player, &placement, slot, (x, y)).await;
            }

            Outcome::Trade(asked) => {
                trade(&mut link, &context, &player, &name, asked).await;
            }

            Outcome::Travel(portal_type) => {
                match travel(
                    &mut link,
                    &placement,
                    &name,
                    player.account.id,
                    portal_type,
                    &context.worlds,
                    arrival_of(&player, &context),
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

    let outcome = match locate(destination, player.character.id, player.account.id) {
        // A named durable slot: it has to be free, because there is nothing to swap with.
        Some(Location::Inventory { character_id, slot }) => context
            .store
            .give_item(character_id, identity, slot, slot)
            .await
            .map(|_| ()),

        Some(Location::Vault { .. }) | None => {
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
) -> Option<Handle> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    if !world
        .send(ToWorld::Join {
            name: name.to_string(),
            arrival,
            sender: link.sender(),
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
async fn moderate(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    command: &str,
    rest: &str,
) {
    use hendra_store::Admin;

    let rank = match context.store.account(player.account.id).await {
        Ok(account) => Admin::from_number(account.admin_rank),
        Err(_) => Admin::None,
    };

    // A player who is not a moderator is told the command does not exist rather than that they may
    // not use it, so the command list is not something anyone can enumerate by trying.
    let unknown = format!("there is no /{command}");

    let (name, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let name = name.trim();

    match command {
        "mute" | "unmute" | "ban" | "unban" if rank == Admin::None => say(link, &unknown).await,

        "mute" | "unmute" | "ban" | "unban" if name.is_empty() => {
            say(link, &format!("/{command} needs a name")).await;
        }

        "mute" | "unmute" => {
            if !rank.may_mute() {
                say(link, &unknown).await;
                return;
            }

            let Ok(target) = context.store.account_by_name(name).await else {
                say(link, "no such player").await;
                return;
            };

            // Minutes, defaulting to an hour. A mute with no end is a ban that nobody remembers
            // applying, so this one always has one.
            let until = (command == "mute").then(|| {
                let minutes = tail
                    .trim()
                    .parse::<i64>()
                    .unwrap_or(60)
                    .clamp(1, 60 * 24 * 30);
                chrono::Utc::now() + chrono::Duration::minutes(minutes)
            });

            match context.store.mute(target.id, until).await {
                Ok(()) => say(link, &format!("{} is {command}d", target.name)).await,
                Err(_) => say(link, "that could not be done").await,
            }
        }

        "ban" | "unban" => {
            if !rank.may_ban() {
                say(link, &unknown).await;
                return;
            }

            let Ok(target) = context.store.account_by_name(name).await else {
                say(link, "no such player").await;
                return;
            };

            // A moderator cannot ban somebody who outranks them, which is what stops one
            // disagreement removing everyone above it.
            if Admin::from_number(target.admin_rank) >= rank {
                say(link, "you cannot do that to them").await;
                return;
            }

            match context.store.set_banned(target.id, command == "ban").await {
                Ok(()) => say(link, &format!("{} is {command}ned", target.name)).await,
                Err(_) => say(link, "that could not be done").await,
            }
        }

        _ => say(link, &unknown).await,
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
) {
    use crate::chat::Said;

    let said = crate::chat::read(line);
    if said == Said::Nothing {
        return;
    }

    // Read fresh rather than from the session's copy, so a mute applied while someone is playing
    // takes effect without waiting for them to reconnect.
    if let Ok(account) = context.store.account(player.account.id).await
        && account
            .muted_until
            .is_some_and(|until| until > chrono::Utc::now())
    {
        say(link, "you cannot speak at the moment").await;
        return;
    }

    if !limit.allow(std::time::Instant::now()) {
        say(link, "you are speaking too quickly").await;
        return;
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
                return;
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

        Said::Command { name, rest } => {
            moderate(link, context, player, &name, &rest).await;
        }

        Said::Nothing => {}
    }
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

    hendra_sim::stats::equipment_boosts(worn)
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
async fn travel(
    link: &mut Link,
    from: &Placement,
    name: &str,
    account_id: i64,
    portal_type: u16,
    worlds: &Worlds,
    arrival: crate::world_task::Arrival,
) -> Option<Placement> {
    let destination = worlds.destination_of(portal_type)?.to_string();

    // A portal leading back into the world you are already in is a no-op, not a rejoin. Rejoining
    // would move the player to the spawn point for no reason.
    if destination == from.world.name.as_ref() {
        return None;
    }

    // A personal world gets one instance per account, so two players in the vault are in two
    // rooms rather than looking at each other's chests.
    let world = worlds.get_or_start_for(&destination, account_id)?;
    let handle = join(link, &world, name, arrival).await?;

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
