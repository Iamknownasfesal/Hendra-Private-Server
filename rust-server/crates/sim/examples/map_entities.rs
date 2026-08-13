//! What every shipped map puts in a world, and what it leaves on the ground.
//!
//! Scenery is not an entity, and the line between the two is drawn by a rule rather than by hand.
//! This runs that rule over every map and shows the result, so a map that quietly lost the object a
//! player was meant to walk up to shows here rather than in a bug report.
//!
//! A second argument names one map to list object by object, for when a count is not enough:
//!
//!     cargo run -p hendra-sim --example map_entities
//!     cargo run -p hendra-sim --example map_entities ../godot-client/assets/xml nexustry

use std::collections::BTreeMap;
use std::path::Path;

use hendra_content::Catalog;
use hendra_sim::World;

fn main() {
    let content = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../godot-client/assets/xml".to_string());

    let Ok((catalog, _)) = Catalog::load_dir(Path::new(&content)) else {
        eprintln!("cannot read the content at {content}");
        std::process::exit(1);
    };

    let directory = Path::new("content/maps");
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("hmap"))
        .collect();
    paths.sort();

    println!(
        "{:<36} {:>10} {:>10} {:>8}",
        "map", "scenery", "entities", "over"
    );

    let mut kept: BTreeMap<String, usize> = BTreeMap::new();
    let mut over = 0;

    for path in paths {
        let Ok(raw) = std::fs::read(&path) else {
            continue;
        };
        let Ok(map) = hendra_content::Map::read(&raw) else {
            continue;
        };

        let (mut scenery, mut entities) = (0usize, 0usize);
        for (_, _, square) in map.objects() {
            if World::is_scenery(&catalog, square) {
                scenery += 1;
            } else {
                entities += 1;
                if let Some(desc) = catalog.object(square.object) {
                    *kept.entry(desc.class.clone()).or_default() += 1;
                }
            }
        }

        // A map whose entities alone fill a world has nowhere left to put an enemy, a player or a
        // loot bag, which is the failure this rule exists to prevent.
        let full = entities > hendra_sim::MAX_ENTITIES;
        if full {
            over += 1;
        }

        println!(
            "{:<36} {:>10} {:>10} {:>8}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            scenery,
            entities,
            if full { "FULL" } else { "" }
        );
    }

    if let Some(name) = std::env::args().nth(2) {
        let path = directory.join(format!("{name}.hmap"));
        let raw = std::fs::read(&path).expect("that map");
        let map = hendra_content::Map::read(&raw).expect("that map");

        println!();
        println!("{name}:");
        for (x, y, square) in map.objects() {
            let id = catalog
                .object(square.object)
                .map(|desc| desc.id.as_str())
                .unwrap_or("?");
            println!(
                "  {:>5},{:<5} {:<28} {}",
                x,
                y,
                id,
                if World::is_scenery(&catalog, square) {
                    "scenery"
                } else {
                    "entity"
                }
            );
        }
    }

    println!();
    println!("what stays an entity, by class:");
    for (class, count) in &kept {
        println!("  {class}: {count}");
    }

    if over > 0 {
        println!();
        println!("{over} maps fill a world with their own objects");
        std::process::exit(1);
    }
}
