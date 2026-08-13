//! The task that owns a world and drives it.
//!
//! Exactly one task owns one world, and it is the only thing that touches it. There is no lock
//! anywhere in here, and there does not need to be — sessions do not reach into the world, they
//! send it messages, and the world writes back through each player's connection directly.
//!
//! ```text
//!   session task ──ToWorld──▶  world task  ──snapshot──▶  player's connection
//!   (one per player)           (one per world)            (LinkSender, no hop)
//! ```
//!
//! # The tick
//!
//! Fixed timestep, driven by an interval that skips rather than catches up. A world that falls
//! behind should drop the ticks it missed, not run them all back to back and fall further behind
//! while doing it.

use std::sync::Arc;
use std::time::Instant;

use hendra_content::{Catalog, ObjectType};
use hendra_net::snapshot::{Acknowledgement, Baseline, BaselineRing};
use hendra_net::{Delivery, ServerMessage, SnapshotEncoder, WorldSnapshot, Writer};
use hendra_sim::world::{Entity, SIGHT_RADIUS};
use hendra_sim::{Handle, TickMetrics, World};
use hendra_transport::LinkSender;
use tokio::sync::mpsc;

/// How often the world advances.
pub const TICKS_PER_SECOND: u32 = 20;

/// What a session asks the world to do.
pub enum ToWorld {
    /// A player is arriving. The world replies with the handle it was given.
    Join {
        name: String,
        sender: LinkSender,
        reply: tokio::sync::oneshot::Sender<Handle>,
    },

    /// A player reports where it believes it is, and what it has received.
    Input {
        handle: Handle,
        x: f32,
        y: f32,
        client_time_ms: u32,
        ack: Acknowledgement,
    },

    Chat {
        handle: Handle,
        text: String,
    },

    Leave {
        handle: Handle,
    },
}

/// Everything the world keeps about one connected player.
struct Player {
    handle: Handle,
    name: String,
    sender: LinkSender,

    /// What this player has been sent, so a snapshot can be a delta against what they confirm.
    history: BaselineRing<WorldSnapshot>,
    encoder: SnapshotEncoder,
    acknowledged: Acknowledgement,

    /// Scratch, reused every tick.
    scratch: Vec<u8>,
}

/// Runs a world until its command channel closes.
pub async fn run(
    mut world: World,
    catalog: Arc<Catalog>,
    player_type: ObjectType,
    mut inbox: mpsc::Receiver<ToWorld>,
) {
    let mut players: Vec<Player> = Vec::new();
    let mut metrics = TickMetrics::for_rate(TICKS_PER_SECOND);
    let elapsed_ms = 1000 / TICKS_PER_SECOND;

    let mut ticker =
        tokio::time::interval(std::time::Duration::from_millis(elapsed_ms as u64));
    // Skip rather than burst: a world that falls behind should not try to run the ticks it missed.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut reported = Instant::now();

    loop {
        tokio::select! {
            command = inbox.recv() => {
                let Some(command) = command else { break };
                handle(&mut world, &catalog, player_type, &mut players, command);
            }

            _ = ticker.tick() => {
                let started = Instant::now();

                world.advance(&catalog, elapsed_ms);
                broadcast(&mut world, &mut players).await;

                metrics.record(started.elapsed());

                // A minute of silence is not evidence of health, so say so periodically.
                if reported.elapsed().as_secs() >= 30 {
                    tracing::info!(
                        world = %world.name,
                        players = players.len(),
                        entities = world.len(),
                        healthy = metrics.healthy(),
                        "{metrics}"
                    );
                    reported = Instant::now();
                }
            }
        }
    }

    tracing::info!(world = %world.name, "world stopped");
}

fn handle(
    world: &mut World,
    catalog: &Catalog,
    player_type: ObjectType,
    players: &mut Vec<Player>,
    command: ToWorld,
) {
    match command {
        ToWorld::Join {
            name,
            sender,
            reply,
        } => {
            let (x, y) = spawn_point(world);
            let mut entity = Entity::player(player_type, x, y, 800);
            entity.name = Some(name.as_str().into());

            let Some(handle) = world.spawn(entity) else {
                tracing::warn!(world = %world.name, "world is full; refusing a join");
                return;
            };

            // The encoder takes its budget from the connection rather than the default, because the
            // negotiated limit is what actually governs whether a datagram is accepted.
            let budget = sender
                .max_datagram_size()
                .unwrap_or(hendra_net::DATAGRAM_BUDGET);

            players.push(Player {
                handle,
                name: name.clone(),
                sender,
                history: BaselineRing::new(),
                encoder: SnapshotEncoder::with_budget(budget),
                acknowledged: Acknowledgement::NONE,
                scratch: Vec::new(),
            });

            tracing::info!(world = %world.name, %name, ?handle, "player joined");
            let _ = reply.send(handle);
        }

        ToWorld::Input {
            handle,
            x,
            y,
            client_time_ms,
            ack,
        } => {
            if let Some(player) = players.iter_mut().find(|player| player.handle == handle) {
                player.acknowledged = ack;
            }

            // The claim is advisory. The world decides where the player actually is.
            if let Some(outcome) = world.resolve_move(handle, catalog, x, y, tick_ms(client_time_ms))
            {
                world.place(handle, outcome);
            }
        }

        ToWorld::Chat { handle, text } => {
            let Some(speaker) = players
                .iter()
                .find(|player| player.handle == handle)
                .map(|player| player.name.clone())
            else {
                return;
            };

            let mut buf = Vec::new();
            ServerMessage::Chat {
                from: &speaker,
                text: &text,
            }
            .encode(&mut Writer::new(&mut buf));

            for player in players.iter() {
                // Chat must arrive, so it goes reliably, and a full queue means that player is not
                // keeping up rather than that the message should be dropped for everyone.
                let _ = player.sender.try_send(Delivery::Stream, &buf);
            }
        }

        ToWorld::Leave { handle } => {
            players.retain(|player| player.handle != handle);
            world.despawn(handle);
            tracing::info!(world = %world.name, ?handle, "player left");
        }
    }
}

/// Sends every player the world as they see it.
async fn broadcast(world: &mut World, players: &mut Vec<Player>) {
    // A connection that has gone should not be encoded for.
    players.retain(|player| !player.sender.is_closed());

    let tick = world.tick_number();

    for player in players.iter_mut() {
        let handle = player.handle;
        if world.get(handle).is_none() {
            continue;
        }

        let snapshot = world.snapshot_for(handle, SIGHT_RADIUS);

        player.scratch.clear();
        let mut writer = Writer::new(&mut player.scratch);
        hendra_net::begin_snapshot(&mut writer);

        let baseline = match player.history.baseline_for(player.acknowledged) {
            Baseline::Delta { tick, state } => Some((tick, state)),
            Baseline::Full => None,
        };
        let delivery = player.encoder.encode(tick, &snapshot, baseline, &mut writer);

        // Never awaits: a slow player must not hold up the tick for everyone else. A backlog is
        // reported and the next tick supersedes what was dropped.
        match player.sender.try_send(delivery, &player.scratch) {
            Ok(()) | Err(hendra_transport::TransportError::Backlogged) => {}
            Err(err) => tracing::debug!(%err, "dropping a snapshot"),
        }

        player.history.store(tick, snapshot);
    }
}

/// How long the world should believe elapsed for a movement claim.
///
/// Deliberately not the client's own figure. A client that reports a large interval would be
/// granting itself a proportionally larger step, so the server uses its own tick length and treats
/// the client's clock as diagnostic only.
fn tick_ms(_client_time_ms: u32) -> u32 {
    1000 / TICKS_PER_SECOND
}

/// Somewhere walkable to put an arriving player.
///
/// Prefers a spawn region when the map declares one, and otherwise walks outward from the middle
/// until it finds ground. Dropping a player into a wall is worse than putting them somewhere
/// arbitrary.
fn spawn_point(world: &World) -> (f32, f32) {
    if let Some((x, y, _)) = world
        .terrain()
        .map()
        .regions()
        .find(|(_, _, region)| *region == hendra_content::Region::Spawn)
    {
        return (x as f32 + 0.5, y as f32 + 0.5);
    }

    let terrain = world.terrain();
    let (centre_x, centre_y) = (terrain.width() / 2, terrain.height() / 2);

    for ring in 0..terrain.width().max(terrain.height()) {
        for dy in -(ring as i64)..=(ring as i64) {
            for dx in -(ring as i64)..=(ring as i64) {
                let x = centre_x as i64 + dx;
                let y = centre_y as i64 + dy;
                if x < 0 || y < 0 {
                    continue;
                }
                if terrain.walkable(x as u32, y as u32) {
                    return (x as f32 + 0.5, y as f32 + 0.5);
                }
            }
        }
    }

    (centre_x as f32, centre_y as f32)
}

/// A handle for sessions to talk to a world.
#[derive(Clone)]
pub struct WorldHandle {
    pub name: Arc<str>,
    pub inbox: mpsc::Sender<ToWorld>,
}

impl WorldHandle {
    pub async fn send(&self, command: ToWorld) -> bool {
        self.inbox.send(command).await.is_ok()
    }
}

/// Starts a world on its own task.
pub fn spawn(
    world: World,
    catalog: Arc<Catalog>,
    player_type: ObjectType,
) -> WorldHandle {
    let name: Arc<str> = Arc::from(world.name.as_str());
    let (inbox, receiver) = mpsc::channel(1024);

    tokio::spawn(run(world, catalog, player_type, receiver));

    WorldHandle { name, inbox }
}
