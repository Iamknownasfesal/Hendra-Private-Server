//! The Godot side of the protocol.
//!
//! This is the whole reason the protocol lives in a shared crate. The extension links the same
//! `hendra-net` the server does, so the client never decodes a byte of the wire format itself —
//! what crosses into Godot is decoded state.
//!
//! # Threads
//!
//! Godot is single-threaded and calls [`HendraConnection::poll`] once per frame; it must never
//! block. A worker thread runs a tokio runtime that owns the connection, decodes everything that
//! arrives, and publishes the result. The two communicate only through the shared state below:
//!
//! ```text
//!   Godot frame ──▶ poll()  ──reads──▶  events queue, world view
//!                     │
//!                     └──sends──▶  command channel  ──▶  worker ──▶ QUIC
//! ```
//!
//! Nothing in `_process` waits on the network, and nothing on the worker touches a Godot object.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use godot::classes::Node;
use godot::prelude::*;

use hendra_net::message::{ClientMessage, Input, PROTOCOL_VERSION, RejectReason, ServerMessage};
use hendra_net::snapshot::{Acknowledgement, BaselineRing};
use hendra_net::{
    Delivery, EntityId, Reader, Tick, WorldSnapshot, Writer, decode_body, read_header,
};
use hendra_transport::{Received, Trust, connect};

struct HendraExtension;

#[gdextension]
unsafe impl ExtensionLibrary for HendraExtension {}

/// Where a connection currently stands, as a value Godot can read without locking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    Idle = 0,
    Connecting = 1,
    Connected = 2,
    Failed = 3,
    Closed = 4,
}

impl Status {
    fn from_code(code: u8) -> Status {
        match code {
            1 => Status::Connecting,
            2 => Status::Connected,
            3 => Status::Failed,
            4 => Status::Closed,
            _ => Status::Idle,
        }
    }
}

/// Something worth telling the game about. Snapshots are not here — they land in the world view,
/// because the client wants the current state rather than a history of changes to it.
enum Event {
    Connected,
    Welcome {
        player: u32,
        tick: u32,
        world: String,
    },
    Rejected(RejectReason),
    Chat {
        from: String,
        text: String,
    },
    Disconnected(String),

    /// The whole contents of one of the player's containers.
    Container {
        container: u8,
        slots: Vec<(u16, u16)>,
    },

    /// Something the player asked for was refused, with a line to show them.
    Refused(String),

    /// A projectile was fired. Its whole flight follows from these fields, so this arrives once and
    /// the client animates the rest itself.
    Shot {
        projectile: u32,
        owner: u32,
        object_type: u16,
        x: f32,
        y: f32,
        angle: f32,
        speed: f32,
        lifetime_ms: u32,
    },
}

/// The decoded world, laid out for the client to read cheaply.
///
/// Parallel arrays rather than a list of dictionaries: building 120 Godot dictionaries every frame
/// would cost more than the networking does, and the client wants to walk these column-wise anyway.
#[derive(Default)]
struct WorldView {
    ids: Vec<i32>,
    types: Vec<i32>,
    /// Interleaved x, y.
    positions: Vec<f32>,
    hp: Vec<i32>,
    max_hp: Vec<i32>,

    /// Bumped whenever the contents change, so the client can skip rebuilding when nothing has.
    revision: u64,
}

impl WorldView {
    fn replace_with(&mut self, world: &WorldSnapshot) {
        self.ids.clear();
        self.types.clear();
        self.positions.clear();
        self.hp.clear();
        self.max_hp.clear();

        for (id, state) in world.iter() {
            self.ids.push(id.0 as i32);
            self.types.push(state.object_type as i32);
            self.positions.push(state.x);
            self.positions.push(state.y);
            self.hp.push(state.hp);
            self.max_hp.push(state.max_hp);
        }
        self.revision += 1;
    }
}

#[derive(Default)]
struct Shared {
    events: Mutex<VecDeque<Event>>,
    world: Mutex<WorldView>,
    status: AtomicU8,
    /// Round-trip time in milliseconds, as QUIC estimates it.
    rtt_ms: AtomicU8,
}

impl Shared {
    fn push(&self, event: Event) {
        if let Ok(mut events) = self.events.lock() {
            // A game that stops polling should not grow this without bound.
            if events.len() < 512 {
                events.push_back(event);
            }
        }
    }

    fn set_status(&self, status: Status) {
        self.status.store(status as u8, Ordering::Release);
    }
}

/// What Godot asks the worker to do.
enum Command {
    Input { x: f32, y: f32, time_ms: u32 },
    Shoot { angle: f32 },
    MoveItem {
        from: (u8, u16),
        to: (u8, u16),
    },
    PickUp {
        bag: u32,
        slot: u8,
    },
    Drop {
        slot: u16,
    },
    Chat(String),
    UsePortal(u32),
    Disconnect,
}

/// A connection to a Hendra server.
#[derive(GodotClass)]
#[class(base=Node)]
pub struct HendraConnection {
    base: Base<Node>,
    shared: Arc<Shared>,
    commands: Option<tokio::sync::mpsc::UnboundedSender<Command>>,
    runtime: Option<tokio::runtime::Runtime>,
}

#[godot_api]
impl INode for HendraConnection {
    fn init(base: Base<Node>) -> Self {
        HendraConnection {
            base,
            shared: Arc::new(Shared::default()),
            commands: None,
            runtime: None,
        }
    }
}

#[godot_api]
impl HendraConnection {
    /// The protocol version this build speaks. The server refuses anything else.
    #[func]
    fn protocol_version(&self) -> i64 {
        PROTOCOL_VERSION as i64
    }

    /// Opens a connection and sends the opening message.
    ///
    /// `token` comes from the app server over HTTPS; no password ever reaches this socket.
    /// `allow_any_certificate` exists for local development and turns off server verification —
    /// a released client must leave it false.
    #[func]
    fn connect_to_server(
        &mut self,
        host: GString,
        port: i32,
        token: GString,
        character: i32,
        allow_any_certificate: bool,
    ) -> bool {
        if self.runtime.is_some() {
            godot_warn!("hendra: already connected; disconnect first");
            return false;
        }

        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                godot_error!("hendra: could not start the network runtime: {err}");
                return false;
            }
        };

        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let shared = Arc::clone(&self.shared);
        shared.set_status(Status::Connecting);

        let host = host.to_string();
        let token = token.to_string();
        let trust = if allow_any_certificate {
            Trust::AnyCertificate
        } else {
            Trust::Roots
        };

        runtime.spawn(async move {
            run(shared, receiver, host, port as u16, token, character as u32, trust).await;
        });

        self.commands = Some(sender);
        self.runtime = Some(runtime);
        true
    }

    /// Closes the connection and stops the worker.
    #[func]
    fn disconnect_from_server(&mut self) {
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(Command::Disconnect);
        }
        if let Some(runtime) = self.runtime.take() {
            // Background rather than blocking: shutting a runtime down waits for its tasks, and
            // this is called from a frame.
            runtime.shutdown_background();
        }
        self.shared.set_status(Status::Closed);
    }

    /// [`Status`] as an integer, matching the constants below.
    #[func]
    fn status(&self) -> i64 {
        self.shared.status.load(Ordering::Acquire) as i64
    }

    #[func]
    fn is_connected_to_server(&self) -> bool {
        Status::from_code(self.shared.status.load(Ordering::Acquire)) == Status::Connected
    }

    /// Round-trip time in milliseconds, saturating at 255.
    #[func]
    fn rtt_ms(&self) -> i64 {
        self.shared.rtt_ms.load(Ordering::Relaxed) as i64
    }

    /// Drains everything that has happened since the last frame.
    ///
    /// Each entry is a dictionary with a `kind` key. Returning an array rather than emitting
    /// signals keeps the whole frame's work in one place the client controls.
    #[func]
    fn poll(&mut self) -> Array<VarDictionary> {
        let mut out = Array::new();

        let Ok(mut events) = self.shared.events.lock() else {
            return out;
        };

        while let Some(event) = events.pop_front() {
            let mut entry = VarDictionary::new();
            match event {
                Event::Connected => {
                    entry.set("kind", "connected");
                }
                Event::Welcome {
                    player,
                    tick,
                    world,
                } => {
                    entry.set("kind", "welcome");
                    entry.set("player", player as i64);
                    entry.set("tick", tick as i64);
                    entry.set("world", world);
                }
                Event::Rejected(reason) => {
                    entry.set("kind", "rejected");
                    entry.set("reason", reason as i64);
                    entry.set("reason_name", format!("{reason:?}"));
                }
                Event::Chat { from, text } => {
                    entry.set("kind", "chat");
                    entry.set("from", from);
                    entry.set("text", text);
                }
                Event::Disconnected(why) => {
                    entry.set("kind", "disconnected");
                    entry.set("reason", why);
                }
                Event::Container { container, slots } => {
                    entry.set("kind", "container");
                    entry.set("container", container as i64);

                    // Parallel arrays for the same reason the world view uses them: one marshalled
                    // block per field beats a dictionary per slot.
                    let indices: Vec<i32> = slots.iter().map(|(slot, _)| *slot as i32).collect();
                    let items: Vec<i32> = slots.iter().map(|(_, item)| *item as i32).collect();
                    entry.set("slots", &PackedInt32Array::from(indices.as_slice()));
                    entry.set("items", &PackedInt32Array::from(items.as_slice()));
                }
                Event::Refused(message) => {
                    entry.set("kind", "refused");
                    entry.set("message", message);
                }
                Event::Shot {
                    projectile,
                    owner,
                    object_type,
                    x,
                    y,
                    angle,
                    speed,
                    lifetime_ms,
                } => {
                    entry.set("kind", "shot");
                    entry.set("projectile", projectile as i64);
                    entry.set("owner", owner as i64);
                    entry.set("object_type", object_type as i64);
                    entry.set("x", x);
                    entry.set("y", y);
                    entry.set("angle", angle);
                    entry.set("speed", speed);
                    entry.set("lifetime_ms", lifetime_ms as i64);
                }
            }
            out.push(&entry);
        }

        out
    }

    /// How many times the world has changed. Unchanged between frames means nothing to rebuild.
    #[func]
    fn world_revision(&self) -> i64 {
        self.shared
            .world
            .lock()
            .map(|world| world.revision as i64)
            .unwrap_or(0)
    }

    #[func]
    fn entity_count(&self) -> i64 {
        self.shared
            .world
            .lock()
            .map(|world| world.ids.len() as i64)
            .unwrap_or(0)
    }

    #[func]
    fn entity_ids(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.ids.as_slice()))
    }

    #[func]
    fn entity_types(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.types.as_slice()))
    }

    /// Positions as interleaved x, y — two entries per entity, in the same order as the ids.
    #[func]
    fn entity_positions(&self) -> PackedFloat32Array {
        self.with_world(|world| PackedFloat32Array::from(world.positions.as_slice()))
    }

    #[func]
    fn entity_hp(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.hp.as_slice()))
    }

    #[func]
    fn entity_max_hp(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.max_hp.as_slice()))
    }

    /// Reports where the player believes it is, and acknowledges the newest snapshot held.
    #[func]
    fn send_input(&self, x: f32, y: f32, time_ms: i64) {
        self.send(Command::Input {
            x,
            y,
            time_ms: time_ms.max(0) as u32,
        });
    }

    /// Asks to move an item between two slots.
    ///
    /// Containers are named by tag rather than by entity, so a client cannot address someone
    /// else's inventory: 0 is what you are carrying, 1 what you are wearing, 2 the vault.
    #[func]
    fn move_item(&self, from_container: i64, from_slot: i64, to_container: i64, to_slot: i64) {
        self.send(Command::MoveItem {
            from: (from_container as u8, from_slot.max(0) as u16),
            to: (to_container as u8, to_slot.max(0) as u16),
        });
    }

    /// Takes an item out of a bag on the ground.
    #[func]
    fn pick_up(&self, bag: i64, slot: i64) {
        self.send(Command::PickUp {
            bag: bag.max(0) as u32,
            slot: slot.max(0) as u8,
        });
    }

    /// Drops a carried item at the player's feet.
    #[func]
    fn drop_item(&self, slot: i64) {
        self.send(Command::Drop {
            slot: slot.max(0) as u16,
        });
    }

    /// Fires in the given direction, in radians.
    ///
    /// Only the aim is sent. Whether the weapon is ready, where the shot travels and what it hits
    /// are all the server's, and there is no way for a client to report a hit at all.
    #[func]
    fn shoot(&self, angle: f32) {
        self.send(Command::Shoot { angle });
    }

    #[func]
    fn send_chat(&self, text: GString) {
        self.send(Command::Chat(text.to_string()));
    }

    #[func]
    fn use_portal(&self, entity: i64) {
        self.send(Command::UsePortal(entity.max(0) as u32));
    }

    fn send(&self, command: Command) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(command);
        }
    }

    fn with_world<T: Default>(&self, read: impl FnOnce(&WorldView) -> T) -> T {
        self.shared
            .world
            .lock()
            .map(|world| read(&world))
            .unwrap_or_default()
    }
}

/// The worker: owns the connection, decodes everything, publishes the result.
async fn run(
    shared: Arc<Shared>,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<Command>,
    host: String,
    port: u16,
    token: String,
    character: u32,
    trust: Trust,
) {
    let address = match resolve(&host, port) {
        Some(address) => address,
        None => {
            shared.set_status(Status::Failed);
            shared.push(Event::Disconnected(format!("cannot resolve {host}:{port}")));
            return;
        }
    };

    let mut link = match connect(address, &host, trust).await {
        Ok(link) => link,
        Err(err) => {
            shared.set_status(Status::Failed);
            shared.push(Event::Disconnected(err.to_string()));
            return;
        }
    };

    shared.set_status(Status::Connected);
    shared.push(Event::Connected);

    // The opening message. Reliable, because a lost hello is a connection that never starts.
    let mut scratch = Vec::new();
    ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        token: &token,
        character,
    }
    .encode(&mut Writer::new(&mut scratch));

    if let Err(err) = link.send(Delivery::Stream, &scratch).await {
        shared.set_status(Status::Failed);
        shared.push(Event::Disconnected(err.to_string()));
        return;
    }

    // What the client holds, so a snapshot naming a baseline can be resolved against the right one.
    let mut history: BaselineRing<WorldSnapshot> = BaselineRing::new();
    let mut newest: Option<Tick> = None;

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                if !handle(&mut link, command, newest, &mut scratch).await {
                    break;
                }
            }

            received = link.recv() => {
                let Some(received) = received else {
                    shared.push(Event::Disconnected("the server closed the connection".into()));
                    break;
                };

                shared.rtt_ms.store(
                    link.rtt().as_millis().min(255) as u8,
                    Ordering::Relaxed,
                );

                apply(&shared, received, &mut history, &mut newest);
            }
        }
    }

    shared.set_status(Status::Closed);
    link.close("client disconnecting");
}

/// Sends one command. Returns false when the connection should end.
async fn handle(
    link: &mut hendra_transport::Link,
    command: Command,
    newest: Option<Tick>,
    scratch: &mut Vec<u8>,
) -> bool {
    scratch.clear();
    let ack = match newest {
        Some(tick) => Acknowledgement::of(tick),
        None => Acknowledgement::NONE,
    };

    let (delivery, message) = match command {
        Command::Disconnect => return false,
        Command::Input { x, y, time_ms } => (
            // Input is worthless once it is late, so it never waits for a retransmission.
            Delivery::Datagram,
            ClientMessage::Input(Input {
                ack,
                client_time_ms: time_ms,
                x,
                y,
            }),
        ),
        Command::Shoot { angle } => {
            let mut buf = Vec::new();
            ClientMessage::Shoot {
                angle,
                client_time_ms: 0,
            }
            .encode(&mut Writer::new(&mut buf));
            return link.send(Delivery::Stream, &buf).await.is_ok();
        }

        Command::MoveItem { from, to } => {
            let place = |(container, slot): (u8, u16)| match container {
                1 => hendra_net::message::SlotLocation::Equipment { slot: slot as u8 },
                2 => hendra_net::message::SlotLocation::Vault { slot },
                _ => hendra_net::message::SlotLocation::Inventory { slot: slot as u8 },
            };

            let mut buf = Vec::new();
            ClientMessage::MoveItem {
                from: place(from),
                to: place(to),
            }
            .encode(&mut Writer::new(&mut buf));
            return link.send(Delivery::Stream, &buf).await.is_ok();
        }

        Command::PickUp { bag, slot } => {
            let mut buf = Vec::new();
            ClientMessage::MoveItem {
                from: hendra_net::message::SlotLocation::Bag {
                    entity: EntityId(bag),
                    slot,
                },
                // The server chooses the slot; naming one here would only be a guess.
                to: hendra_net::message::SlotLocation::Inventory { slot: 0 },
            }
            .encode(&mut Writer::new(&mut buf));
            return link.send(Delivery::Stream, &buf).await.is_ok();
        }

        Command::Drop { slot } => {
            let mut buf = Vec::new();
            ClientMessage::MoveItem {
                from: hendra_net::message::SlotLocation::Inventory { slot: slot as u8 },
                to: hendra_net::message::SlotLocation::Ground,
            }
            .encode(&mut Writer::new(&mut buf));
            return link.send(Delivery::Stream, &buf).await.is_ok();
        }

        Command::Chat(text) => {
            let mut buf = Vec::new();
            ClientMessage::Chat { text: &text }.encode(&mut Writer::new(&mut buf));
            return link.send(Delivery::Stream, &buf).await.is_ok();
        }
        Command::UsePortal(entity) => (
            Delivery::Stream,
            ClientMessage::UsePortal {
                entity: EntityId(entity),
            },
        ),
    };

    message.encode(&mut Writer::new(scratch));
    link.send(delivery, scratch).await.is_ok()
}

/// Decodes one arriving payload and publishes whatever it means.
fn apply(
    shared: &Arc<Shared>,
    received: Received,
    history: &mut BaselineRing<WorldSnapshot>,
    newest: &mut Option<Tick>,
) {
    let payload = received.into_payload();
    let mut reader = Reader::new(&payload);

    let message = match ServerMessage::decode(&mut reader) {
        Ok(message) => message,
        Err(err) => {
            tracing::warn!(%err, "undecodable message from the server");
            return;
        }
    };

    match message {
        ServerMessage::Welcome {
            player,
            tick,
            world,
        } => shared.push(Event::Welcome {
            player: player.0,
            tick: tick.0,
            world: world.to_owned(),
        }),

        ServerMessage::Rejected { reason } => shared.push(Event::Rejected(reason)),

        ServerMessage::Chat { from, text } => shared.push(Event::Chat {
            from: from.to_owned(),
            text: text.to_owned(),
        }),

        ServerMessage::Ping { .. } => {}

        ServerMessage::Container { container, slots } => shared.push(Event::Container {
            container: container as u8,
            slots,
        }),

        ServerMessage::Refused { message } => shared.push(Event::Refused(message.to_owned())),

        ServerMessage::Shot {
            projectile,
            owner,
            object_type,
            x,
            y,
            angle,
            speed,
            lifetime_ms,
        } => shared.push(Event::Shot {
            projectile: projectile.0,
            owner: owner.0,
            object_type,
            x,
            y,
            angle,
            speed,
            lifetime_ms,
        }),

        ServerMessage::Snapshot { body } => {
            let mut body = Reader::new(body);
            let Ok(header) = read_header(&mut body) else {
                return;
            };

            // Stale on arrival: datagrams reorder, and an older snapshot must not rewind the world.
            if newest.is_some_and(|held| !header.tick.is_newer_than(held)) {
                return;
            }

            let world = {
                let baseline = match header.baseline {
                    Some(tick) => match history.get(tick) {
                        Some(world) => Some(world),
                        // Encoded against something that never arrived. The server notices when the
                        // acknowledgement stops advancing and sends a full snapshot.
                        None => return,
                    },
                    None => None,
                };

                match decode_body(header, baseline, &mut body) {
                    Ok(world) => world,
                    Err(err) => {
                        tracing::warn!(%err, "undecodable snapshot");
                        return;
                    }
                }
            };

            if let Ok(mut view) = shared.world.lock() {
                view.replace_with(&world);
            }

            history.store(header.tick, world);
            *newest = Some(header.tick);
        }
    }
}

/// Resolves a host and port, preferring IPv4 when both are offered.
fn resolve(host: &str, port: u16) -> Option<std::net::SocketAddr> {
    use std::net::ToSocketAddrs;

    let candidates: Vec<std::net::SocketAddr> = (host, port).to_socket_addrs().ok()?.collect();
    candidates
        .iter()
        .find(|address| address.is_ipv4())
        .or_else(|| candidates.first())
        .copied()
}
