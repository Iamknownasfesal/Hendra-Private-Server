//! Measures what a snapshot actually costs, against the encoding it replaces.
//!
//! Run with: `cargo run --release --example bandwidth`
//!
//! The comparison is against a fixed record per visible entity, which is what the legacy server
//! sends: `NewTick` carries every stat of every entity in sight whether or not it changed. Twenty
//! fields at four bytes plus an id is a conservative reading of it. The real packet also spends a
//! byte on each stat's type tag.

use hendra_net::{
    EntityId, EntityState, SnapshotEncoder, Tick, WorldSnapshot, Writer, decode_snapshot,
    snapshot::{Acknowledgement, BaselineRing},
};

/// What the legacy encoding spends on one entity, every tick, unconditionally.
const LEGACY_BYTES_PER_ENTITY: usize = 4 + 20 * 4;

/// A sight radius of 20 tiles holds roughly this many entities in a busy dungeon.
const VISIBLE: u32 = 120;

const TICKS: u32 = 200;

fn spawn(id: u32) -> (EntityId, EntityState) {
    let angle = id as f32 * 0.37;
    (
        EntityId(id),
        EntityState {
            object_type: 0x0a00 + (id % 40) as u16,
            x: 100.0 + angle.cos() * 12.0,
            y: 100.0 + angle.sin() * 12.0,
            hp: 500,
            max_hp: 800,
            mp: 100,
            max_mp: 200,
            conditions: 0,
            size: 100,
            name: None,
            texture: 0,
            stats: [0; 8],
        },
    )
}

/// Advances the world one tick. `moving` is the share of entities that actually do something.
fn step(world: &mut WorldSnapshot, tick: u32, moving: u32) -> WorldSnapshot {
    let mut next: Vec<(EntityId, EntityState)> = world
        .iter()
        .map(|(id, state)| (id, state.clone()))
        .collect();

    for (id, state) in next.iter_mut() {
        if id.0 % VISIBLE >= moving {
            continue;
        }
        let phase = (tick as f32 + id.0 as f32) * 0.11;
        state.x += phase.cos() * 0.12;
        state.y += phase.sin() * 0.12;

        // A quarter of the movers are also taking damage.
        if id.0 % 4 == 0 {
            state.hp = (state.hp - 3).max(1);
        }
    }

    WorldSnapshot::from_unsorted(next)
}

fn run(label: &str, moving: u32) {
    let mut world = WorldSnapshot::from_unsorted((0..VISIBLE).map(spawn).collect());

    let mut encoder = SnapshotEncoder::new();
    let mut history: BaselineRing<WorldSnapshot> = BaselineRing::new();
    let mut client: Option<WorldSnapshot> = None;

    let mut total = 0usize;
    let mut delta_peak = 0usize;
    let mut full_peak = 0usize;
    let mut full_count = 0usize;
    let mut over_mtu = 0usize;
    let mut buf = Vec::new();

    for tick in 0..TICKS {
        let ack = client
            .as_ref()
            .map(|_| Acknowledgement::of(Tick(tick.saturating_sub(1))))
            .unwrap_or(Acknowledgement::NONE);

        let baseline = match history.baseline_for(ack) {
            hendra_net::Baseline::Delta { tick, state } => Some((tick, state)),
            hendra_net::Baseline::Full => None,
        };

        buf.clear();
        let delivery = encoder.encode(Tick(tick), &world, baseline, &mut Writer::new(&mut buf));
        if delivery == hendra_net::Delivery::Stream && baseline.is_some() {
            over_mtu += 1;
        }

        // The client reconstructs from the same baseline, proving the bytes are sufficient.
        let (_, seen) =
            decode_snapshot(baseline.map(|(_, w)| w), &mut hendra_net::Reader::new(&buf))
                .expect("a snapshot we just encoded must decode");
        assert_eq!(seen.len(), world.len(), "tick {tick} lost entities");
        client = Some(seen);

        total += buf.len();
        if baseline.is_none() {
            full_count += 1;
            full_peak = full_peak.max(buf.len());
        } else {
            delta_peak = delta_peak.max(buf.len());
        }

        history.store(Tick(tick), world.clone());
        world = step(&mut world, tick, moving);
    }

    let legacy = LEGACY_BYTES_PER_ENTITY * VISIBLE as usize;
    let mean = total / TICKS as usize;
    let fits = if over_mtu == 0 {
        "fits".to_string()
    } else {
        format!("{over_mtu} OVER")
    };

    println!(
        "{label:<20} mean {mean:>5} B   delta peak {delta_peak:>5} B  [{fits}]   \
         {:>5.1} KB/s/player   vs legacy {:>5.1} KB/s",
        mean as f64 * TPS / 1024.0,
        legacy as f64 * TPS / 1024.0,
    );

    // The full snapshot is the same size in every scenario, so report it once.
    if full_count > 0 && moving == 0 {
        println!(
            "{:<20} full snapshot {full_peak} B, {} the {MTU} B datagram limit\n",
            "",
            if full_peak > MTU { "EXCEEDS" } else { "within" }
        );
    }
}

/// The usable payload of a QUIC datagram on a typical path.
const MTU: usize = 1200;

/// The rate both encodings are compared at. The legacy server actually runs at 6, but comparing
/// the encodings at the same rate is the only way to isolate the encoding from the tick rate.
const TPS: f64 = 20.0;

fn main() {
    println!("{VISIBLE} entities in sight, {TICKS} ticks, compared at {TPS} TPS\n");

    run("all idle", 0);
    run("10% moving", VISIBLE / 10);
    run("half moving", VISIBLE / 2);
    run("everything moving", VISIBLE);

    println!(
        "\nlegacy figure is {LEGACY_BYTES_PER_ENTITY} B per visible entity per tick, sent\n\
         unconditionally. At its real 6 TPS that is {:.1} KB/s per player.",
        (LEGACY_BYTES_PER_ENTITY * VISIBLE as usize) as f64 * 6.0 / 1024.0
    );
}
