//! Ticks a real map with real player counts, and reports whether it fits the budget.
//!
//! Run with: `cargo run --release -p hendra-sim --example tick_budget`
//!
//! The C# server ran at six ticks per second. This exists to establish what the Rust one can
//! actually sustain, measured on the game's own maps rather than argued from first principles.

use std::time::Instant;

use hendra_content::{Catalog, Map, legacy};
use hendra_sim::metrics::TickMetrics;
use hendra_sim::world::{Entity, SIGHT_RADIUS};
use hendra_sim::{Terrain, World};

const TICKS_PER_SECOND: u32 = 20;
const TICKS: u32 = 400;

fn main() {
    let content = std::path::PathBuf::from("../Server-Side/XmlDatas/xmls/client");
    let (catalog, report) = match Catalog::load_dir(&content) {
        Ok(loaded) => loaded,
        Err(err) => {
            eprintln!("could not read {}: {err}", content.display());
            std::process::exit(1);
        }
    };
    println!("catalog: {} objects, {} tiles\n", report.objects, report.tiles);

    let worlds = std::path::PathBuf::from("../Server-Side/XmlDatas/worlds");
    for (file, players) in [
        ("Nexus.jm", 60),
        ("Nexus.jm", 200),
        ("OryxCastle.jm", 120),
        ("Losthall.jm", 80),
    ] {
        let path = worlds.join(file);
        let Ok(raw) = std::fs::read(&path) else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(&raw) else {
            continue;
        };
        let Ok((map, _)) = legacy::from_jm(text, &catalog) else {
            continue;
        };

        measure(file, map, players, &catalog);
    }
}

/// The weapon players carry in the measurement, so projectile load is real rather than assumed.
const WEAPON: &str = "Wand of Dark Magic";

fn measure(label: &str, map: Map, players: u32, catalog: &Catalog) {
    let (width, height) = (map.width(), map.height());
    let terrain = Terrain::build(map, catalog);
    let walkable = terrain.walkable_count();

    let mut world = World::new(label, terrain, catalog);
    let fixtures = world.len();

    // Spread players over walkable ground, clustered enough that sight radii overlap the way they
    // do in a busy nexus.
    let mut placed = Vec::new();
    let mut seed = 0x1234_5678u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };

    let centre_x = width as f32 / 2.0;
    let centre_y = height as f32 / 2.0;
    while (placed.len() as u32) < players {
        let x = centre_x + ((next() % 3000) as f32 / 100.0) - 15.0;
        let y = centre_y + ((next() % 3000) as f32 / 100.0) - 15.0;
        if !world.terrain().walkable_at(x, y) {
            continue;
        }
        let mut player = Entity::player(hendra_content::ObjectType(0x0300), x, y, 800);
        player.weapon = catalog.type_of(WEAPON);
        if let Some(handle) = world.spawn(player) {
            placed.push(handle);
        }
    }

    let mut metrics = TickMetrics::for_rate(TICKS_PER_SECOND);
    let elapsed_ms = 1000 / TICKS_PER_SECOND;
    let mut snapshot_entities = 0usize;
    let mut peak_projectiles = 0usize;

    for tick in 0..TICKS {
        let started = Instant::now();

        // What a real tick does: resolve every player's claimed position, advance the world, then
        // build a snapshot for each of them.
        for (index, handle) in placed.iter().enumerate() {
            let drift = ((tick + index as u32) % 8) as f32 * 0.03;
            let Some(entity) = world.get(*handle) else {
                continue;
            };
            let (claim_x, claim_y) = (entity.x + drift, entity.y + drift * 0.5);

            if let Some(outcome) = world.resolve_move(*handle, catalog, claim_x, claim_y, elapsed_ms)
            {
                world.place(*handle, outcome);
            }
        }

        // Everyone fires as fast as the server lets them, which is the worst case for projectile
        // load: the cooldown refuses most of these, and the ones it allows all stay in flight.
        for (index, handle) in placed.iter().enumerate() {
            let angle = (tick + index as u32) as f32 * 0.21;
            world.shoot(*handle, catalog, angle);
        }

        world.advance(catalog, elapsed_ms);
        peak_projectiles = peak_projectiles.max(world.projectile_count());

        for handle in &placed {
            snapshot_entities += world.snapshot_for(*handle, SIGHT_RADIUS).len();
        }

        metrics.record(started.elapsed());
    }

    let per_tick = if TICKS > 0 {
        snapshot_entities / TICKS as usize
    } else {
        0
    };

    println!("{label} — {width}×{height}, {walkable} walkable, {fixtures} fixtures");
    println!(
        "  {players} players, {per_tick} entities encoded per tick, {peak_projectiles} projectiles at peak"
    );
    println!("  {metrics}");
    println!(
        "  headroom: {:.0}× budget at p99{}\n",
        metrics.budget().as_secs_f64() / metrics.percentile(0.99).as_secs_f64().max(1e-9),
        if metrics.healthy() { "" } else { "  ** OVER **" }
    );
}
