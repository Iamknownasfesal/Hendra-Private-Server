//! A real QUIC connection over loopback, carrying real protocol messages.
//!
//! Everything below this point has been tested against an in-memory pipe. These tests put an actual
//! handshake, an actual congestion controller and an actual UDP socket underneath, which is where
//! assumptions about datagram limits and stream framing stop being assumptions.

use std::net::SocketAddr;
use std::time::Duration;

use hendra_net::message::{ClientMessage, Input, PROTOCOL_VERSION, ServerMessage};
use hendra_net::snapshot::Acknowledgement;
use hendra_net::{
    Delivery, EntityId, EntityState, Reader, SnapshotEncoder, Tick, WorldSnapshot, Writer,
    decode_snapshot,
};
use hendra_transport::{Link, Listener, Received, ServerIdentity, TransportError, Trust, connect};

/// Brings up a listener on a loopback port and connects a client to it.
async fn pair() -> (Link, Link) {
    let identity = ServerIdentity::self_signed(&["localhost"]).expect("a test certificate");
    let listener = Listener::bind("127.0.0.1:0".parse().unwrap(), identity).expect("a listener");
    let address = listener.local_address().expect("a bound address");

    let accepting = tokio::spawn(async move {
        listener
            .accept()
            .await
            .expect("a connection")
            .expect("a successful handshake")
    });

    let client = connect(address, "localhost", Trust::AnyCertificate)
        .await
        .expect("the client should connect");
    let server = accepting.await.expect("the accept task should finish");

    (server, client)
}

/// Waits for the next payload, failing the test rather than hanging if none arrives.
async fn expect_next(link: &mut Link) -> Received {
    tokio::time::timeout(Duration::from_secs(5), link.recv())
        .await
        .expect("timed out waiting for a payload")
        .expect("the connection closed unexpectedly")
}

fn encoded<F: FnOnce(&mut Writer<'_>)>(write: F) -> Vec<u8> {
    let mut buf = Vec::new();
    write(&mut Writer::new(&mut buf));
    buf
}

fn world_of(count: u32) -> WorldSnapshot {
    WorldSnapshot::from_unsorted(
        (0..count)
            .map(|n| {
                (
                    EntityId(n),
                    EntityState {
                        object_type: 0x0a00 + n as u16,
                        x: n as f32,
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
            })
            .collect(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_handshake_completes_over_a_real_connection() {
    let (server, client) = pair().await;

    assert_eq!(client.remote_address().ip().to_string(), "127.0.0.1");
    assert!(
        server.remote_address().port() > 0,
        "the server should see the client's address"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hello_travels_reliably_and_is_answered() {
    let (mut server, mut client) = pair().await;

    let hello = encoded(|w| {
        ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            token: "a session token from the app server",
            character: 4,
        }
        .encode(w)
    });
    client.send(Delivery::Stream, &hello).await.unwrap();

    let arrived = expect_next(&mut server).await;
    assert!(
        matches!(arrived, Received::Reliable(_)),
        "a hello must arrive on the stream"
    );

    match ClientMessage::decode(&mut Reader::new(arrived.payload())).unwrap() {
        ClientMessage::Hello {
            protocol,
            token,
            character,
        } => {
            assert_eq!(protocol, PROTOCOL_VERSION);
            assert_eq!(token, "a session token from the app server");
            assert_eq!(character, 4);
        }
        other => panic!("expected a hello, got {other:?}"),
    }

    let welcome = encoded(|w| {
        ServerMessage::Welcome {
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
        }
        .encode(w)
    });
    server.send(Delivery::Stream, &welcome).await.unwrap();

    let arrived = expect_next(&mut client).await;
    assert_eq!(
        ServerMessage::decode(&mut Reader::new(arrived.payload())).unwrap(),
        ServerMessage::Welcome {
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
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reliable_messages_keep_their_order() {
    let (mut server, client) = pair().await;

    for n in 0..64u32 {
        let chat = encoded(|w| {
            ClientMessage::Chat {
                text: &format!("message {n}"),
            }
            .encode(w)
        });
        client.send(Delivery::Stream, &chat).await.unwrap();
    }

    for n in 0..64u32 {
        let arrived = expect_next(&mut server).await;
        match ClientMessage::decode(&mut Reader::new(arrived.payload())).unwrap() {
            ClientMessage::Chat { text } => assert_eq!(text, format!("message {n}")),
            other => panic!("expected chat, got {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_snapshot_delta_travels_as_a_datagram_and_decodes() {
    let (server, mut client) = pair().await;

    let before = world_of(30);
    let mut after = before.clone();
    // Nudge one entity so the delta is non-empty.
    let moved: Vec<(EntityId, EntityState)> = after
        .iter()
        .map(|(id, state)| {
            let mut next = state.clone();
            if id == EntityId(5) {
                next.x += 0.25;
            }
            (id, next)
        })
        .collect();
    after = WorldSnapshot::from_unsorted(moved);

    let mut encoder = SnapshotEncoder::new();
    let mut buf = Vec::new();
    let mut writer = Writer::new(&mut buf);
    hendra_net::begin_snapshot(&mut writer);
    let delivery = encoder.encode(Tick(1), &after, Some((Tick(0), &before)), &mut writer);

    assert_eq!(delivery, Delivery::Datagram, "a small delta should fit");
    server.send(delivery, &buf).await.unwrap();

    let arrived = expect_next(&mut client).await;
    assert!(
        matches!(arrived, Received::Unreliable(_)),
        "a delta must arrive as a datagram"
    );

    let mut reader = Reader::new(arrived.payload());
    let ServerMessage::Snapshot { body } = ServerMessage::decode(&mut reader).unwrap() else {
        panic!("expected a snapshot");
    };

    let (tick, world) = decode_snapshot(Some(&before), &mut Reader::new(body)).unwrap();
    assert_eq!(tick, Tick(1));
    assert_eq!(
        hendra_net::quantize(world.get(EntityId(5)).unwrap().x),
        hendra_net::quantize(5.25)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_snapshot_is_routed_to_the_stream_and_arrives_whole() {
    let (server, mut client) = pair().await;

    // 120 entities: measured at roughly 2,600 bytes, over twice the datagram floor.
    let world = world_of(120);

    let mut encoder = SnapshotEncoder::new();
    let mut buf = Vec::new();
    let mut writer = Writer::new(&mut buf);
    hendra_net::begin_snapshot(&mut writer);
    let delivery = encoder.encode(Tick(0), &world, None, &mut writer);

    assert_eq!(delivery, Delivery::Stream);
    assert!(
        buf.len() > client.max_datagram_size().unwrap_or(1200),
        "this snapshot should be too large for a datagram"
    );

    server.send(delivery, &buf).await.unwrap();

    let arrived = expect_next(&mut client).await;
    assert!(matches!(arrived, Received::Reliable(_)));

    let mut reader = Reader::new(arrived.payload());
    let ServerMessage::Snapshot { body } = ServerMessage::decode(&mut reader).unwrap() else {
        panic!("expected a snapshot");
    };
    let (_, decoded) = decode_snapshot(None, &mut Reader::new(body)).unwrap();
    assert_eq!(decoded.len(), 120, "the whole world should survive");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_datagram_is_refused_rather_than_silently_dropped() {
    let (server, _client) = pair().await;

    // The limit is per-endpoint, so ask the side that is about to send.
    let limit = server
        .max_datagram_size()
        .expect("datagrams should be available");
    let too_big = vec![0u8; limit + 1];

    match server.try_send(Delivery::Datagram, &too_big) {
        Err(TransportError::DatagramTooLarge { len, limit: seen }) => {
            assert_eq!(len, limit + 1);
            assert_eq!(seen, limit);
        }
        other => panic!("expected a size refusal, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_negotiated_datagram_size_covers_our_budget() {
    let (server, client) = pair().await;

    // Both directions must carry a full-size snapshot, and the two limits are negotiated
    // separately. The server's is the smaller of the pair here, which is exactly why the default
    // budget cannot be the 1200-byte MTU floor: QUIC's own packet overhead comes out of it first.
    for (side, link) in [("server", &server), ("client", &client)] {
        let limit = link
            .max_datagram_size()
            .expect("datagrams should be available");
        assert!(
            limit >= hendra_net::DATAGRAM_BUDGET,
            "{side} allows {limit} bytes, below the {} we encode to",
            hendra_net::DATAGRAM_BUDGET
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_encoder_can_take_the_budget_from_the_live_connection() {
    let (server, _client) = pair().await;

    let negotiated = server.max_datagram_size().expect("datagrams available");
    let encoder = SnapshotEncoder::with_budget(negotiated);

    assert_eq!(encoder.budget(), negotiated);

    // The negotiated limit is not a constant: it starts conservative and rises as MTU discovery
    // learns what the path carries, which is the whole reason this is read rather than assumed.
    assert!(
        negotiated >= hendra_net::DATAGRAM_BUDGET,
        "a live connection should carry at least the default budget, got {negotiated}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tick_loop_runs_over_the_real_connection() {
    let (server, mut client) = pair().await;

    let mut world = world_of(40);
    let mut encoder = SnapshotEncoder::new();
    let mut previous: Option<(Tick, WorldSnapshot)> = None;
    let mut client_world: Option<WorldSnapshot> = None;

    for tick in 0..30u32 {
        let mut buf = Vec::new();
        let mut writer = Writer::new(&mut buf);
        hendra_net::begin_snapshot(&mut writer);

        let baseline = previous.as_ref().map(|(t, w)| (*t, w));
        let delivery = encoder.encode(Tick(tick), &world, baseline, &mut writer);
        server.send(delivery, &buf).await.unwrap();

        let arrived = expect_next(&mut client).await;
        let mut reader = Reader::new(arrived.payload());
        let ServerMessage::Snapshot { body } = ServerMessage::decode(&mut reader).unwrap() else {
            panic!("expected a snapshot");
        };

        let (_, seen) = decode_snapshot(client_world.as_ref(), &mut Reader::new(body)).unwrap();
        client_world = Some(seen);

        // The client answers with input, as it would at tick rate.
        let input = encoded(|w| {
            ClientMessage::Input(Input {
                ack: Acknowledgement::of(Tick(tick)),
                client_time_ms: tick * 50,
                x: 10.0,
                y: 10.0,
            })
            .encode(w)
        });
        client.send(Delivery::Datagram, &input).await.unwrap();

        previous = Some((Tick(tick), world.clone()));

        let moved: Vec<(EntityId, EntityState)> = world
            .iter()
            .map(|(id, state)| {
                let mut next = state.clone();
                next.x += 0.125;
                (id, next)
            })
            .collect();
        world = WorldSnapshot::from_unsorted(moved);
    }

    let seen = client_world.expect("the client should hold a world");
    let truth = previous.expect("the server should have a last snapshot").1;
    for (id, state) in truth.iter() {
        assert_eq!(
            hendra_net::quantize(seen.get(id).unwrap().x),
            hendra_net::quantize(state.x),
            "{id:?} drifted over a real connection"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_self_signed_certificate_is_refused_when_roots_are_required() {
    let identity = ServerIdentity::self_signed(&["localhost"]).unwrap();
    let listener = Listener::bind("127.0.0.1:0".parse().unwrap(), identity).unwrap();
    let address: SocketAddr = listener.local_address().unwrap();

    tokio::spawn(async move {
        let _ = listener.accept().await;
    });

    // This is the check that matters for a public server: verification must actually run, and a
    // certificate no authority issued must not be accepted.
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        connect(address, "localhost", Trust::Roots),
    )
    .await
    .expect("the attempt should not hang");

    assert!(
        matches!(outcome, Err(TransportError::Handshake(_))),
        "an unverifiable certificate must fail the handshake, got {outcome:?}"
    );
}
