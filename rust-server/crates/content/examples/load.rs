//! Loads the real content directory and reports what came out of it.
//!
//! Run with: `cargo run --example load -- <dir>`

use std::path::PathBuf;
use std::time::Instant;

use hendra_content::Catalog;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("content/xmls"));

    let started = Instant::now();
    let (catalog, report) = match Catalog::load_dir(&dir) {
        Ok(loaded) => loaded,
        Err(err) => {
            eprintln!("could not read {}: {err}", dir.display());
            std::process::exit(1);
        }
    };
    let elapsed = started.elapsed();

    println!("read {} files in {:.1?}", report.files_read, elapsed);
    println!("  objects   {}", report.objects);
    println!("    items   {}", report.items);
    println!("    enemies {}", catalog.enemies().count());
    println!("  tiles     {}", report.tiles);

    if report.problems.is_empty() {
        println!("  problems  none");
    } else {
        println!("  problems  {}", report.problems.len());
        for problem in report.problems.iter().take(40) {
            println!("    - {problem}");
        }
        if report.problems.len() > 40 {
            println!("    ... and {} more", report.problems.len() - 40);
        }
    }

    // Named spot checks. A count alone cannot tell you the right things loaded, and these four
    // exercise the interesting cases: two stackable consumables, a weapon whose projectile has to
    // resolve across a file boundary, and the projectile object it resolves to.
    for id in [
        "Health Potion",
        "Magic Potion",
        "Sword of Acclaim",
        "Purple Bolt",
    ] {
        match catalog.by_name(id) {
            Some(desc) => println!(
                "  {id:<20} type 0x{:04x}  item={}  shots={}",
                desc.object_type.0,
                desc.is_item(),
                desc.projectiles.len()
            ),
            None => println!("  {id:<20} MISSING"),
        }
    }
}
