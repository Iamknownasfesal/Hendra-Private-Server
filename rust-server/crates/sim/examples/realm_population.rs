//! Populates the real realm map with the real content and says what it got.
//!
//! The unit tests use a field of one terrain, which proves the rules and nothing about whether the
//! shipped map and the shipped objects actually meet. This loads both: if the map carries no
//! terrain, or an enemy Oryx names is not in the catalog, or a terrain has squares but nowhere
//! walkable on them, this is where it shows rather than as a realm nobody ever fights in.
//!
//!     cargo run -p hendra-sim --example realm_population

use std::path::Path;

use hendra_content::{Catalog, TERRAIN_COUNT, Terrain as TerrainKind};
use hendra_sim::{Terrain, World, realm};

fn main() {
    let content = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../godot-client/assets/xml".to_string());
    let content = Path::new(&content);
    let map_path = Path::new("content/maps/world1.hmap");

    let Ok((catalog, problems)) = Catalog::load_dir(content) else {
        eprintln!("cannot read the content at {}", content.display());
        std::process::exit(1);
    };
    if !problems.problems.is_empty() {
        println!("{} content problems", problems.problems.len());
    }

    let (spawnable, missing) = realm::spawnable_reporting(&catalog);
    for name in &missing {
        println!("MISSING from the content: {name}");
    }
    println!("{} spawns resolved", spawnable.len());

    let Ok(raw) = std::fs::read(map_path) else {
        eprintln!("cannot read the map at {}", map_path.display());
        std::process::exit(1);
    };
    let map = match hendra_content::Map::read(&raw) {
        Ok(map) => map,
        Err(err) => {
            eprintln!("cannot load the map: {err}");
            std::process::exit(1);
        }
    };

    // What the map's objects are, before anything is built from them: the original makes an entity
    // of an object only when it is not static or it is an enemy, and leaves the rest on the tile.
    let (mut statics, mut living) = (0usize, 0usize);
    let mut classes: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (_, _, square) in map.objects() {
        if hendra_sim::World::is_scenery(&catalog, square) {
            statics += 1;
            if let Some(desc) = catalog.object(square.object) {
                *classes.entry(desc.class.clone()).or_default() += 1;
            }
        } else {
            living += 1;
        }
    }
    println!("{statics} scenery, {living} that have to be entities");
    for (class, count) in &classes {
        println!("  {class}: {count}");
    }

    let terrain = Terrain::build(map, &catalog);
    println!(
        "map {}x{}, {} squares walkable",
        terrain.width(),
        terrain.height(),
        terrain.walkable_count()
    );

    let census = terrain.terrain_census();
    let targets = realm::targets(&census);

    let mut world = World::new("Realm", terrain, &catalog);

    // Setpieces first, then enemies, as `Realm.Init` does.
    let mut dice = hendra_sim::setpiece::Dice::new(0x5eed_beef);
    let places = {
        let ground = |x: u32, y: u32| world.terrain().terrain_at(x, y);
        hendra_sim::setpiece::scatter(
            world.terrain().width(),
            world.terrain().height(),
            &ground,
            &mut dice,
        )
    };

    let mut by_kind: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut missing: Vec<&str> = Vec::new();
    for place in &places {
        *by_kind.entry(format!("{:?}", place.kind)).or_default() += 1;

        let drawing = place.kind.draw(&mut dice);
        if drawing.prefab.is_some() {
            continue;
        }
        missing.extend(world.draw(&catalog, &drawing, (place.x, place.y)));
    }

    missing.sort_unstable();
    missing.dedup();
    for name in &missing {
        println!("MISSING from the content: {name}");
    }

    if world.refused_squares() > 0 {
        println!(
            "{} squares refused: the map cannot describe any more kinds",
            world.refused_squares()
        );
    }

    println!();
    println!("setpieces drawn:");
    for (kind, count) in &by_kind {
        println!("  {kind}: {count}");
    }
    println!("  {} entities now in the world", world.len());
    println!();
    println!(
        "{} entities came from the map itself, of a ceiling of {}",
        world.len(),
        hendra_sim::MAX_ENTITIES
    );
    world.realm_mut().measure(&census);

    let opening = world.realm().opening();
    let (placed, _) = world.populate(&catalog, &spawnable, &opening);

    let alive = world.alive_by_terrain();

    println!();
    println!(
        "{:<14} {:>10} {:>8} {:>8}",
        "terrain", "squares", "wanted", "placed"
    );

    let mut short = 0;
    for index in 0..TERRAIN_COUNT {
        if census[index] == 0 && targets[index] == 0 {
            continue;
        }
        let name = TerrainKind::from_index(index as u8)
            .map(|terrain| format!("{terrain:?}"))
            .unwrap_or_else(|| index.to_string());

        println!(
            "{:<14} {:>10} {:>8} {:>8}",
            name, census[index], targets[index], alive[index]
        );

        if targets[index] > 0 && alive[index] * 4 < targets[index] * 3 {
            short += 1;
        }
    }

    println!();
    println!(
        "{placed} placed against a target of {}",
        world.realm().population()
    );

    if !missing.is_empty() || short > 0 {
        println!("{} terrains came up short", short);
        std::process::exit(1);
    }
}
