//! A whole session, both ends, with no transport underneath.
//!
//! The unit tests check each layer in isolation. These drive the layers together the way a real
//! connection does, being a handshake and then a tick loop where the server encodes against
//! whatever the client last acknowledged, and then start dropping and reordering packets, which is what the
//! acknowledged baseline design exists for and what a transport would otherwise have to reproduce
//! to test.
//!
//! Both halves keep a [`BaselineRing`], and the symmetry is what makes the test meaningful: the server records what it
//! sent, the client records what it received, and a snapshot names which entry it was measured
//! against so neither side has to guess.

use hendra_net::message::{ClientMessage, Input, PROTOCOL_VERSION, RejectReason, ServerMessage};
use hendra_net::snapshot::{Acknowledgement, Baseline, BaselineRing};
use hendra_net::{
    Delivery, EntityId, EntityState, Reader, SnapshotEncoder, Tick, WorldSnapshot, Writer,
    decode_body, quantize, read_header,
};

/// The server's half of a connection.
struct Server {
    encoder: SnapshotEncoder,
    history: BaselineRing<WorldSnapshot>,
    world: WorldSnapshot,
    tick: Tick,
}

/// The client's half.
#[derive(Default)]
struct Client {
    history: BaselineRing<WorldSnapshot>,
    world: WorldSnapshot,
    newest: Option<Tick>,

    /// Snapshots discarded because their baseline was never received.
    unusable: usize,
}

impl Server {
    fn new(world: WorldSnapshot) -> Server {
        Server {
            encoder: SnapshotEncoder::new(),
            history: BaselineRing::new(),
            world,
            tick: Tick::ZERO,
        }
    }

    /// Produces the snapshot message for this tick, given what the client says it holds.
    fn snapshot(&mut self, ack: Acknowledgement) -> (Vec<u8>, Delivery) {
        let baseline = match self.history.baseline_for(ack) {
            Baseline::Delta { tick, state } => Some((tick, state)),
            Baseline::Full => None,
        };

        let mut buf = Vec::new();
        let mut writer = Writer::new(&mut buf);
        hendra_net::begin_snapshot(&mut writer);
        let delivery = self
            .encoder
            .encode(self.tick, &self.world, baseline, &mut writer);

        self.history.store(self.tick, self.world.clone());
        (buf, delivery)
    }

    fn advance(&mut self, by: f32) {
        self.tick = self.tick.next();
        let moved: Vec<(EntityId, EntityState)> = self
            .world
            .iter()
            .map(|(id, state)| {
                let mut next = state.clone();
                next.x += by;
                (id, next)
            })
            .collect();
        self.world = WorldSnapshot::from_unsorted(moved);
    }
}

impl Client {
    fn ack(&self) -> Acknowledgement {
        match self.newest {
            Some(tick) => Acknowledgement::of(tick),
            None => Acknowledgement::NONE,
        }
    }

    /// Applies a snapshot message. Returns false if it was discarded.
    fn receive(&mut self, packet: &[u8]) -> bool {
        let mut reader = Reader::new(packet);
        let ServerMessage::Snapshot { body } =
            ServerMessage::decode(&mut reader).expect("a snapshot message")
        else {
            panic!("expected a snapshot");
        };

        let mut body = Reader::new(body);
        let header = read_header(&mut body).expect("a snapshot header");

        // Look up exactly the baseline the sender named, never "the newest one I hold".
        let world = {
            let baseline = match header.baseline {
                Some(tick) => match self.history.get(tick) {
                    Some(world) => Some(world),
                    None => {
                        // Encoded against something that never reached us. Nothing to do but wait
                        // for the server to notice and send a full snapshot.
                        self.unusable += 1;
                        return false;
                    }
                },
                None => None,
            };

            match decode_body(header, baseline, &mut body) {
                Ok(world) => world,
                Err(_) => return false,
            }
        };

        // Datagrams reorder, so a snapshot older than what we already hold is stale on arrival.
        if self
            .newest
            .is_some_and(|newest| !header.tick.is_newer_than(newest))
        {
            return false;
        }

        self.history.store(header.tick, world.clone());
        self.world = world;
        self.newest = Some(header.tick);
        true
    }

    fn sees(&self, id: EntityId) -> Option<&EntityState> {
        self.world.get(id)
    }
}

fn entity(id: u32, x: f32) -> (EntityId, EntityState) {
    (
        EntityId(id),
        EntityState {
            object_type: 0x0a00 + id as u16,
            x,
            y: 50.0,
            hp: 500,
            max_hp: 800,
            mp: 100,
            max_mp: 200,
            conditions: 0,
            size: 100,
            name: None,
            texture: 0,
            stats: [0; hendra_net::STAT_COUNT],
            stars: 0,
            oxygen: 100,
            ..Default::default()
        },
    )
}

fn world_of(count: u32) -> WorldSnapshot {
    WorldSnapshot::from_unsorted((0..count).map(|n| entity(n, n as f32)).collect())
}

/// Runs a tick: the server sends, the client may or may not receive, then the world moves on.
fn exchange(server: &mut Server, client: &mut Client, deliver: bool) {
    // Most tests do not care which transport the snapshot would take; `exchange_on` is for the
    // ones that do.
    let _ = exchange_on(server, client, deliver);
}

/// As [`exchange`], but reports which transport the snapshot needed.
fn exchange_on(server: &mut Server, client: &mut Client, deliver: bool) -> Delivery {
    let (packet, delivery) = server.snapshot(client.ack());
    if deliver {
        assert!(client.receive(&packet), "a delivered snapshot must apply");
    }
    server.advance(0.125);
    delivery
}

/// Delivers one final snapshot without advancing, so the two can be compared.
fn settle(server: &mut Server, client: &mut Client) {
    let (packet, _) = server.snapshot(client.ack());
    assert!(client.receive(&packet), "the settling snapshot must apply");
}

fn assert_converged(server: &Server, client: &Client) {
    for (id, truth) in server.world.iter() {
        let seen = client
            .sees(id)
            .unwrap_or_else(|| panic!("{id:?} unknown to client"));
        assert_eq!(quantize(seen.x), quantize(truth.x), "{id:?} drifted on x");
        assert_eq!(seen.hp, truth.hp, "{id:?} drifted on hp");
    }
    assert_eq!(
        client.world.len(),
        server.world.len(),
        "entity count differs"
    );
}

#[test]
fn a_handshake_is_accepted_and_answered() {
    let mut buf = Vec::new();
    ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        token: "session-token",
        character: 4,
    }
    .encode(&mut Writer::new(&mut buf));

    let reply = match ClientMessage::decode(&mut Reader::new(&buf)).unwrap() {
        ClientMessage::Hello {
            protocol, token, ..
        } if protocol == PROTOCOL_VERSION && !token.is_empty() => ServerMessage::Welcome {
            player: EntityId(1),
            tick: Tick::ZERO,
            world: "Nexus",
            width: 64,
            height: 64,
            background: 0,
            difficulty: 0,
            allow_teleport: true,
            show_displays: true,
            music: "Nexus",
        },
        _ => ServerMessage::Rejected {
            reason: RejectReason::BadToken,
        },
    };

    let mut out = Vec::new();
    reply.encode(&mut Writer::new(&mut out));

    match ServerMessage::decode(&mut Reader::new(&out)).unwrap() {
        ServerMessage::Welcome { player, world, .. } => {
            assert_eq!(player, EntityId(1));
            assert_eq!(world, "Nexus");
        }
        other => panic!("expected a welcome, got {other:?}"),
    }
}

#[test]
fn an_outdated_client_is_told_why() {
    let mut buf = Vec::new();
    ClientMessage::Hello {
        protocol: PROTOCOL_VERSION + 1,
        token: "session-token",
        character: 4,
    }
    .encode(&mut Writer::new(&mut buf));

    let ClientMessage::Hello { protocol, .. } =
        ClientMessage::decode(&mut Reader::new(&buf)).unwrap()
    else {
        panic!("expected a hello");
    };
    assert_ne!(protocol, PROTOCOL_VERSION);

    let mut out = Vec::new();
    ServerMessage::Rejected {
        reason: RejectReason::VersionMismatch,
    }
    .encode(&mut Writer::new(&mut out));

    assert_eq!(
        ServerMessage::decode(&mut Reader::new(&out)).unwrap(),
        ServerMessage::Rejected {
            reason: RejectReason::VersionMismatch
        }
    );
}

#[test]
fn the_first_snapshot_is_full_and_takes_the_stream() {
    let mut server = Server::new(world_of(40));
    let mut client = Client::default();

    let (packet, delivery) = server.snapshot(client.ack());
    assert_eq!(
        delivery,
        Delivery::Stream,
        "a client with no baseline needs a reliable full snapshot"
    );

    assert!(client.receive(&packet));
    assert_eq!(client.sees(EntityId(7)).unwrap().object_type, 0x0a07);
    assert_converged(&server, &client);
}

#[test]
fn a_steady_session_stays_converged() {
    let mut server = Server::new(world_of(30));
    let mut client = Client::default();

    for _ in 0..100 {
        exchange(&mut server, &mut client, true);

        // The client answers with its input, carrying the acknowledgement the server encodes from.
        let mut buf = Vec::new();
        ClientMessage::Input(Input {
            ack: client.ack(),
            client_time_ms: 0,
            x: 0.0,
            y: 0.0,
        })
        .encode(&mut Writer::new(&mut buf));
        assert!(ClientMessage::decode(&mut Reader::new(&buf)).is_ok());
    }

    settle(&mut server, &mut client);
    assert_converged(&server, &client);
}

#[test]
fn only_the_first_snapshot_needs_the_stream() {
    let mut server = Server::new(world_of(30));
    let mut client = Client::default();

    assert_eq!(
        exchange_on(&mut server, &mut client, true),
        Delivery::Stream
    );

    for _ in 0..50 {
        assert_eq!(
            exchange_on(&mut server, &mut client, true),
            Delivery::Datagram,
            "a delta should fit a datagram"
        );
    }
}

#[test]
fn a_dropped_snapshot_costs_no_accuracy_once_one_arrives() {
    let mut server = Server::new(world_of(30));
    let mut client = Client::default();

    exchange(&mut server, &mut client, true);
    for _ in 0..10 {
        exchange(&mut server, &mut client, true);
    }
    let before_loss = client.sees(EntityId(3)).unwrap().x;

    // Three ticks vanish. The acknowledgement stops advancing, so the server keeps encoding
    // against the last snapshot it knows arrived.
    for _ in 0..3 {
        exchange(&mut server, &mut client, false);
    }
    assert_eq!(
        client.sees(EntityId(3)).unwrap().x,
        before_loss,
        "the client cannot have moved on from packets it never saw"
    );

    // One delivery closes the whole gap.
    settle(&mut server, &mut client);
    assert_converged(&server, &client);
}

#[test]
fn a_gap_longer_than_the_history_falls_back_to_a_full_snapshot() {
    let mut server = Server::new(world_of(30));
    let mut client = Client::default();

    exchange(&mut server, &mut client, true);
    let before = server.history.full_sends();

    // Longer than the ring, so the acknowledged tick is overwritten and can no longer be a
    // baseline for either side.
    for _ in 0..(hendra_net::SNAPSHOT_HISTORY + 5) {
        exchange(&mut server, &mut client, false);
    }

    let (packet, delivery) = server.snapshot(client.ack());
    assert_eq!(
        delivery,
        Delivery::Stream,
        "a full snapshot must be reliable"
    );
    assert!(
        server.history.full_sends() > before,
        "the server should have counted a fallback"
    );

    assert!(client.receive(&packet));
    assert_converged(&server, &client);
}

#[test]
fn a_snapshot_whose_baseline_never_arrived_is_discarded_not_misread() {
    let mut server = Server::new(world_of(20));
    let mut client = Client::default();

    exchange(&mut server, &mut client, true);

    // Take a snapshot the client will never see, and let the server believe it landed by feeding
    // it an acknowledgement for that tick.
    server.advance(0.125);
    let stranded = server.tick;
    let (_never_delivered, _) = server.snapshot(Acknowledgement::of(stranded));

    // The next snapshot is encoded against the one that vanished.
    server.advance(0.125);
    let (packet, _) = server.snapshot(Acknowledgement::of(stranded));

    assert!(
        !client.receive(&packet),
        "a snapshot naming an unknown baseline must be refused"
    );
    assert_eq!(client.unusable, 1);

    // Nothing was corrupted by the refusal; the client still holds what it had.
    assert!(client.sees(EntityId(3)).is_some());
}

#[test]
fn a_reordered_snapshot_is_ignored_rather_than_rewinding_the_world() {
    let mut server = Server::new(world_of(20));
    let mut client = Client::default();

    exchange(&mut server, &mut client, true);

    // Capture two consecutive snapshots, then deliver them out of order.
    server.advance(0.125);
    let (first, _) = server.snapshot(client.ack());
    let ack_after_first = Acknowledgement::of(server.tick);
    server.advance(0.125);
    let (second, _) = server.snapshot(ack_after_first);

    assert!(client.receive(&first));
    assert!(client.receive(&second));
    let current = client.sees(EntityId(3)).unwrap().x;

    // The stale one arrives late. It must not drag the world backwards.
    assert!(
        !client.receive(&first),
        "an out-of-order snapshot must be ignored"
    );
    assert_eq!(client.sees(EntityId(3)).unwrap().x, current);
}

#[test]
fn sustained_loss_never_desynchronises() {
    let mut server = Server::new(world_of(25));
    let mut client = Client::default();

    // Drop roughly one packet in three, for two hundred ticks.
    let mut seed = 0x51ed_2701u64;
    let mut delivered = 0usize;

    for _ in 0..200 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let (packet, _) = server.snapshot(client.ack());
        if !seed.is_multiple_of(3) && client.receive(&packet) {
            delivered += 1;
        }
        server.advance(0.0625);
    }

    assert!(
        delivered > 50,
        "the test should have delivered most packets"
    );

    settle(&mut server, &mut client);
    assert_converged(&server, &client);
}
