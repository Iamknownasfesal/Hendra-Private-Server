//! One connection, from hello to goodbye.
//!
//! A session owns its link and nothing else. It decodes what arrives, turns it into a message for
//! whichever world the player is in, and hands it on; it never touches a world directly. Snapshots
//! travel the other way without passing through here at all — the world writes to the player's
//! connection itself.
//!
//! # Changing world
//!
//! A session outlives the world it is in. Stepping through a portal leaves one world and joins
//! another, and the player's identity changes with it: a handle names a slot in a particular
//! world's storage and means nothing anywhere else. The client is told by a second `Welcome`, which
//! it must treat as "forget everything" — its snapshot history was measured against a world that no
//! longer applies.

use std::sync::Arc;

use hendra_net::message::{ClientMessage, PROTOCOL_VERSION, RejectReason, ServerMessage};
use hendra_net::{Delivery, EntityId, Reader, Writer};
use hendra_sim::Handle;
use hendra_transport::{Link, Received};

use crate::world_task::{ToWorld, WorldHandle};
use crate::worlds::Worlds;

/// Where a player currently is.
struct Placement {
    world: WorldHandle,
    handle: Handle,
}

/// Handles one connection for its lifetime.
pub async fn serve(mut link: Link, worlds: Arc<Worlds>, entry: WorldHandle) {
    let peer = link.remote_address();

    let Some((name, mut placement)) = handshake(&mut link, &entry).await else {
        link.close("handshake refused");
        return;
    };

    tracing::info!(%peer, %name, world = %placement.world.name, "session started");

    loop {
        let Some(received) = link.recv().await else {
            break;
        };

        match dispatch(&received, &placement).await {
            Outcome::Continue => {}
            Outcome::Stop => break,

            Outcome::Travel(portal_type) => {
                match travel(&mut link, &placement, &name, portal_type, &worlds).await {
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
    tracing::info!(%peer, %name, "session ended");
}

enum Outcome {
    Continue,
    Stop,

    /// The player stepped into a portal of this object type.
    Travel(u16),
}

/// Reads the opening message and either admits the player or explains why not.
async fn handshake(link: &mut Link, entry: &WorldHandle) -> Option<(String, Placement)> {
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

    // Tokens are minted by the app server over HTTPS. Until that exists, any non-empty token is
    // accepted and used as the display name — this is the one place that has to change when real
    // authentication lands, and it is deliberately obvious.
    if token.is_empty() {
        refuse(link, RejectReason::BadToken).await;
        return None;
    }
    let name = format!("{token}#{character}");

    let handle = join(link, entry, &name).await?;
    Some((
        name,
        Placement {
            world: entry.clone(),
            handle,
        },
    ))
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
