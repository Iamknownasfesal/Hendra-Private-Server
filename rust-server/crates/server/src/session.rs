//! One connection, from hello to goodbye.
//!
//! A session owns its link and nothing else. It decodes what arrives, turns it into a message for
//! whichever world the player is in, and hands it on; it never touches a world directly. Snapshots
//! travel the other way without passing through here at all — the world writes to the player's
//! connection itself.
//!
//! # Where items live
//!
//! Equipment and the backpack are one table. Slots 0 to 3 are worn and 4 upwards are carried, which
//! is how the game has always numbered them, and it means every move between them is a move within
//! one container — so it goes through the same transactional path a vault move does rather than
//! needing a second mechanism.
//!
//! Bags are not durable and are handled separately; a move touching one is refused for now rather
//! than done unsafely.
//!
//! # Changing world
//!
//! A session outlives the world it is in. Stepping through a portal leaves one world and joins
//! another, and the player's identity changes with it: a handle names a slot in a particular
//! world's storage and means nothing anywhere else. The client is told by a second `Welcome`, which
//! it must treat as "forget everything" — its snapshot history was measured against a world that no
//! longer applies.

use std::sync::Arc;

use hendra_net::message::{
    ClientMessage, ContainerId, PROTOCOL_VERSION, RejectReason, ServerMessage, SlotLocation,
};
use hendra_net::{Delivery, EntityId, Reader, Writer};
use hendra_store::{Location, Store};
use hendra_sim::Handle;
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
    pub kit: crate::accounts::StartingKitOwned,
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

        SlotLocation::Bag { .. } => return None,
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

    // The client needs to see what it is carrying before it can move any of it.
    send_containers(&mut link, &context.store, &player).await;

    loop {
        let Some(received) = link.recv().await else {
            break;
        };

        match dispatch(&received, &placement).await {
            Outcome::Continue => {}
            Outcome::Stop => break,

            Outcome::Move { from, to } => {
                move_item(&mut link, &context, &player, from, to).await;
            }

            Outcome::Travel(portal_type) => {
                match travel(&mut link, &placement, &name, portal_type, &context.worlds).await {
                    Some(next) => {
                        tracing::info!(
                            %name,
                            from = %placement.world.name,
                            to = %next.world.name,
                            "travelled"
                        );
                        placement = next;
                    }
                    // Staying put is the right answer when a destination cannot be opened: the
                    // player keeps playing where they are rather than being disconnected over a
                    // dungeon that failed to load.
                    None => tracing::warn!(%name, "portal led nowhere; staying put"),
                }
            }
        }
    }

    placement
        .world
        .send(ToWorld::Leave {
            handle: placement.handle,
        })
        .await;

    // Write back what the character became. Items are not saved here — they are written as they
    // move, so a checkpoint that rewrote slots wholesale could undo a move that had committed.
    if let Err(err) = crate::accounts::save(
        &context.store,
        &player.character,
        player.character.hp,
        player.character.mp,
    )
    .await
    {
        tracing::warn!(%err, %name, "could not save the character");
    }

    tracing::info!(%peer, %name, "session ended");
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
        tracing::info!(protocol, expected = PROTOCOL_VERSION, "refused: wrong version");
        return None;
    }

    let (avatar, items, max_hp) = context.kit.borrowed();
    let kit = crate::accounts::StartingKit {
        avatar,
        items: &items,
        max_hp,
    };

    let player = match crate::accounts::log_in(
        &context.store,
        &context.catalog,
        &kit,
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
        Err(crate::accounts::LoginError::NoToken) => {
            refuse(link, RejectReason::BadToken).await;
            return None;
        }
        Err(err) => {
            tracing::warn!(%err, "login failed");
            refuse(link, RejectReason::BadToken).await;
            return None;
        }
    };

    let handle = join(link, entry, &player.character.name).await?;
    Some((
        player,
        Placement {
            world: entry.clone(),
            handle,
        },
    ))
}

/// Sends the player everything they are carrying and storing.
async fn send_containers(link: &mut Link, store: &Store, player: &crate::accounts::Session) {
    let inventory = store
        .character(player.character.id)
        .await
        .map(|character| character.inventory)
        .unwrap_or_default();

    // Worn and carried live in one table, so they are split back apart on the way out.
    let worn: Vec<(u16, u16)> = inventory
        .iter()
        .filter(|(slot, _)| *slot < EQUIPPED_SLOTS as i16)
        .map(|(slot, item)| (*slot as u16, *item as u16))
        .collect();
    let carried: Vec<(u16, u16)> = inventory
        .iter()
        .filter(|(slot, _)| *slot >= EQUIPPED_SLOTS as i16)
        .map(|(slot, item)| ((*slot - EQUIPPED_SLOTS as i16) as u16, *item as u16))
        .collect();

    let vault: Vec<(u16, u16)> = store
        .vault(player.account.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(slot, item)| (slot as u16, item as u16))
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
    from: SlotLocation,
    to: SlotLocation,
) {
    let character_id = player.character.id;
    let account_id = player.account.id;

    let (Some(source), Some(destination)) = (
        locate(from, character_id, account_id),
        locate(to, character_id, account_id),
    ) else {
        say(link, "that cannot be moved yet").await;
        return;
    };

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
        .move_item(source, destination, expected)
        .await
    {
        Ok(_) => send_containers(link, &context.store, player).await,
        Err(hendra_store::StoreError::Refused(reason)) => {
            say(link, reason).await;
            // Re-read rather than assume: the client's picture is now known to be wrong.
            send_containers(link, &context.store, player).await;
        }
        Err(err) => {
            tracing::warn!(%err, "a move failed");
            say(link, "that could not be done").await;
        }
    }
}

/// What is currently in a durable slot.
async fn read_slot(store: &Store, at: Location) -> Option<i32> {
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
async fn join(link: &mut Link, world: &WorldHandle, name: &str) -> Option<Handle> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    if !world
        .send(ToWorld::Join {
            name: name.to_string(),
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

/// Moves a player from one world to another.
///
/// The order matters. The destination is opened and joined *before* the old world is left, so a
/// world that fails to start leaves the player where they were rather than nowhere at all.
async fn travel(
    link: &mut Link,
    from: &Placement,
    name: &str,
    portal_type: u16,
    worlds: &Worlds,
) -> Option<Placement> {
    let destination = worlds.destination_of(portal_type)?.to_string();

    // A portal leading back into the world you are already in is a no-op, not a rejoin — rejoining
    // would move the player to the spawn point for no reason.
    if destination == from.world.name.as_ref() {
        return None;
    }

    let world = worlds.get_or_start(&destination)?;
    let handle = join(link, &world, name).await?;

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
            // One undecodable packet is not worth dropping a player over — a datagram can arrive
            // corrupted — but it is worth knowing about.
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
            world
                .send(ToWorld::Chat {
                    handle,
                    text: text.to_owned(),
                })
                .await
        }

        ClientMessage::Shoot { angle, .. } => world.send(ToWorld::Shoot { handle, angle }).await,

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
