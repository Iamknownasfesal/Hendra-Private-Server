//! One connection, from hello to goodbye.
//!
//! A session owns its link and nothing else. It decodes what arrives, turns it into a [`ToWorld`]
//! message, and hands it on; it never touches the world directly. Snapshots travel the other way
//! without passing through here at all — the world writes to the player's connection itself.

use hendra_net::message::{ClientMessage, PROTOCOL_VERSION, RejectReason, ServerMessage};
use hendra_net::{Delivery, Reader, Writer};
use hendra_sim::Handle;
use hendra_transport::{Link, Received};

use crate::world_task::{ToWorld, WorldHandle};

/// Handles one connection for its lifetime.
pub async fn serve(mut link: Link, world: WorldHandle) {
    let peer = link.remote_address();

    let Some(joined) = handshake(&mut link, &world).await else {
        link.close("handshake refused");
        return;
    };

    tracing::info!(%peer, name = %joined.name, "session started");

    while let Some(received) = link.recv().await {
        if !dispatch(&received, joined.handle, &world).await {
            break;
        }
    }

    world.send(ToWorld::Leave { handle: joined.handle }).await;
    tracing::info!(%peer, name = %joined.name, "session ended");
}

struct Joined {
    handle: Handle,
    name: String,
}

/// Reads the opening message and either admits the player or explains why not.
async fn handshake(link: &mut Link, world: &WorldHandle) -> Option<Joined> {
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

    let (reply, answer) = tokio::sync::oneshot::channel();
    if !world
        .send(ToWorld::Join {
            name: name.clone(),
            sender: link.sender(),
            reply,
        })
        .await
    {
        refuse(link, RejectReason::Full).await;
        return None;
    }

    let handle = match answer.await {
        Ok(handle) => handle,
        Err(_) => {
            refuse(link, RejectReason::Full).await;
            return None;
        }
    };

    let mut buf = Vec::new();
    ServerMessage::Welcome {
        player: handle.to_entity_id(),
        tick: hendra_net::Tick::ZERO,
        world: &world.name,
    }
    .encode(&mut Writer::new(&mut buf));

    if link.send(Delivery::Stream, &buf).await.is_err() {
        return None;
    }

    Some(Joined { handle, name })
}

async fn refuse(link: &mut Link, reason: RejectReason) {
    let mut buf = Vec::new();
    ServerMessage::Rejected { reason }.encode(&mut Writer::new(&mut buf));
    let _ = link.send(Delivery::Stream, &buf).await;
}

/// Turns one arriving payload into a world command. Returns false to end the session.
async fn dispatch(received: &Received, handle: Handle, world: &WorldHandle) -> bool {
    let mut reader = Reader::new(received.payload());

    let message = match ClientMessage::decode(&mut reader) {
        Ok(message) => message,
        Err(err) => {
            // One undecodable packet is not worth dropping a player over — a datagram can arrive
            // corrupted — but it is worth knowing about.
            tracing::debug!(%err, "ignoring an undecodable message");
            return true;
        }
    };

    match message {
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

        ClientMessage::UsePortal { entity } => {
            tracing::debug!(?entity, "portal use is not implemented yet");
            true
        }

        // A second hello on an established session is a client fault, not an attack; ignoring it is
        // safer than re-joining someone who is already in the world.
        ClientMessage::Hello { .. } => {
            tracing::debug!("ignoring a repeated hello");
            true
        }

        ClientMessage::Pong { .. } => true,
    }
}
