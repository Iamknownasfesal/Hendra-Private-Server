//! A server that does just enough to prove the client works.
//!
//! Run with: `cargo run -p hendra-transport --example demo_server -- [port]`
//!
//! It accepts connections, answers a hello with a welcome, and then ticks a small world at 20 per
//! second so a client has something to move and something to render. There is no simulation here —
//! that is phase two — only the protocol, so that the Godot extension can be exercised end to end
//! against something real rather than a mock.

use std::time::{Duration, Instant};

use hendra_net::message::{ClientMessage, PROTOCOL_VERSION, RejectReason, ServerMessage};
use hendra_net::snapshot::{Acknowledgement, Baseline, BaselineRing};
use hendra_net::{
    Delivery, EntityId, EntityState, Reader, SnapshotEncoder, Tick, WorldSnapshot, Writer,
};
use hendra_transport::{Link, Listener, Received, ServerIdentity};

const TPS: u64 = 20;
const ENTITIES: u32 = 40;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(2050);

    // Self-signed, paired with Trust::AnyCertificate on the client. A deployed server loads a real
    // chain instead — see ServerIdentity::from_pem_files.
    let identity = ServerIdentity::self_signed(&["localhost"]).expect("a development certificate");
    let listener = Listener::bind(
        format!("0.0.0.0:{port}").parse().expect("a valid address"),
        identity,
    )
    .expect("a listener");

    println!(
        "demo server listening on {} — {TPS} ticks per second, {ENTITIES} entities",
        listener.local_address().expect("a bound address")
    );
    println!("clients must connect with allow_any_certificate = true\n");

    while let Some(incoming) = listener.accept().await {
        match incoming {
            Ok(link) => {
                println!("connection from {}", link.remote_address());
                tokio::spawn(serve(link));
            }
            Err(err) => eprintln!("handshake failed: {err}"),
        }
    }
}

/// One connection, from hello to disconnect.
async fn serve(mut link: Link) {
    let peer = link.remote_address();

    // The budget comes from the connection rather than the default, because the negotiated limit is
    // what actually governs whether a datagram is accepted.
    let budget = link
        .max_datagram_size()
        .unwrap_or(hendra_net::DATAGRAM_BUDGET);
    let mut encoder = SnapshotEncoder::with_budget(budget);
    let mut history: BaselineRing<WorldSnapshot> = BaselineRing::new();

    let mut tick = Tick::ZERO;
    let mut acknowledged = Acknowledgement::NONE;
    let mut greeted = false;
    let mut scratch = Vec::new();

    let started = Instant::now();
    let mut ticker = tokio::time::interval(Duration::from_millis(1000 / TPS));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            received = link.recv() => {
                let Some(received) = received else {
                    println!("{peer} disconnected");
                    return;
                };

                match handle(&mut link, received, &mut acknowledged, &mut greeted, tick).await {
                    Ok(()) => {}
                    Err(reason) => {
                        println!("{peer} refused: {reason}");
                        return;
                    }
                }
            }

            _ = ticker.tick() => {
                if !greeted {
                    continue;
                }

                let world = world_at(started.elapsed().as_secs_f32());

                scratch.clear();
                let mut writer = Writer::new(&mut scratch);
                hendra_net::begin_snapshot(&mut writer);

                let baseline = match history.baseline_for(acknowledged) {
                    Baseline::Delta { tick, state } => Some((tick, state)),
                    Baseline::Full => None,
                };
                let delivery = encoder.encode(tick, &world, baseline, &mut writer);

                if link.send(delivery, &scratch).await.is_err() {
                    println!("{peer} send failed");
                    return;
                }

                history.store(tick, world);
                tick = tick.next();
            }
        }
    }
}

/// Handles one client message. `Err` means the connection should end.
async fn handle(
    link: &mut Link,
    received: Received,
    acknowledged: &mut Acknowledgement,
    greeted: &mut bool,
    tick: Tick,
) -> Result<(), String> {
    let payload = received.into_payload();
    let mut reader = Reader::new(&payload);

    let message = ClientMessage::decode(&mut reader).map_err(|err| err.to_string())?;

    match message {
        ClientMessage::Hello {
            protocol,
            token,
            character,
        } => {
            let mut out = Vec::new();

            if protocol != PROTOCOL_VERSION {
                ServerMessage::Rejected {
                    reason: RejectReason::VersionMismatch,
                }
                .encode(&mut Writer::new(&mut out));
                let _ = link.send(Delivery::Stream, &out).await;
                return Err(format!("protocol {protocol}, expected {PROTOCOL_VERSION}"));
            }

            if token.is_empty() {
                ServerMessage::Rejected {
                    reason: RejectReason::BadToken,
                }
                .encode(&mut Writer::new(&mut out));
                let _ = link.send(Delivery::Stream, &out).await;
                return Err("empty token".into());
            }

            println!("  hello: character {character}, token {} chars", token.len());

            ServerMessage::Welcome {
                player: EntityId(1),
                tick,
                world: "Nexus",
            }
            .encode(&mut Writer::new(&mut out));
            link.send(Delivery::Stream, &out)
                .await
                .map_err(|err| err.to_string())?;

            *greeted = true;
        }

        ClientMessage::Input(input) => {
            // The acknowledgement is the only part the demo acts on; validating the position is
            // simulation work.
            *acknowledged = input.ack;
        }

        ClientMessage::Chat { text } => {
            println!("  chat: {text}");
            let mut out = Vec::new();
            ServerMessage::Chat {
                from: "server",
                text: &format!("you said: {text}"),
            }
            .encode(&mut Writer::new(&mut out));
            link.send(Delivery::Stream, &out)
                .await
                .map_err(|err| err.to_string())?;
        }

        ClientMessage::Shoot { angle, .. } => println!("  shoot: {angle:.2} rad"),
        ClientMessage::MoveItem { from, to } => println!("  move item: {from:?} -> {to:?}"),
        ClientMessage::UsePortal { entity } => println!("  portal: {entity:?}"),
        ClientMessage::Pong { .. } => {}
    }

    Ok(())
}

/// A ring of entities orbiting a point, so movement is visible and predictable.
fn world_at(seconds: f32) -> WorldSnapshot {
    let entities = (0..ENTITIES)
        .map(|n| {
            let phase = seconds * 0.6 + n as f32 * std::f32::consts::TAU / ENTITIES as f32;
            (
                EntityId(n + 1),
                EntityState {
                    object_type: 0x0a00 + n as u16,
                    x: 100.0 + phase.cos() * 10.0,
                    y: 100.0 + phase.sin() * 10.0,
                    hp: 500 - (n as i32 % 7) * 20,
                    max_hp: 500,
                    mp: 200,
                    max_mp: 200,
                    conditions: 0,
                    size: 100,
                    name: None,
                },
            )
        })
        .collect();

    WorldSnapshot::from_unsorted(entities)
}
