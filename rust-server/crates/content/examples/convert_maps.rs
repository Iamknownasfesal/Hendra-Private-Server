//! Converts every legacy map into our format, and reports what it found.
//!
//! Run with: `cargo run --release --example convert_maps -- <worlds-dir> [out-dir]`
//!
//! Without an output directory it converts in memory and reports only, which is the useful mode for
//! checking whether the whole corpus still round-trips after a format change.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use hendra_content::legacy::{self, UnresolvedNames};
use hendra_content::world::WorldDef;
use hendra_content::{Catalog, Map};

fn main() {
    let mut args = std::env::args().skip(1);
    let worlds = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("content/worlds"));
    let out = args.next().map(PathBuf::from);

    let content = PathBuf::from("content/xmls");
    let (catalog, report) = match Catalog::load_dir(&content) {
        Ok(loaded) => loaded,
        Err(err) => {
            eprintln!("could not read {}: {err}", content.display());
            std::process::exit(1);
        }
    };
    println!(
        "catalog: {} objects, {} tiles, {} problems\n",
        report.objects,
        report.tiles,
        report.problems.len()
    );

    let mut files: Vec<PathBuf> = match std::fs::read_dir(&worlds) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("jm") | Some("wmap")
                )
            })
            .collect(),
        Err(err) => {
            eprintln!("could not read {}: {err}", worlds.display());
            std::process::exit(1);
        }
    };
    files.sort();

    if let Some(dir) = &out {
        std::fs::create_dir_all(dir).expect("output directory");
    }

    let started = Instant::now();
    let mut converted = 0usize;
    let mut failed = 0usize;

    // Output names already used, so a second source cannot quietly replace the first.
    let mut written_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut collisions: Vec<(String, String)> = Vec::new();
    let mut legacy_bytes = 0usize;
    let mut ours_bytes = 0usize;
    let mut squares = 0usize;
    let mut missing = UnresolvedNames::default();
    let mut failures: Vec<(String, String)> = Vec::new();

    for path in &files {
        let raw = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                failures.push((name_of(path), err.to_string()));
                failed += 1;
                continue;
            }
        };

        let outcome = if path.extension().and_then(|e| e.to_str()) == Some("jm") {
            match std::str::from_utf8(&raw) {
                Ok(text) => legacy::from_jm(text, &catalog),
                Err(err) => {
                    failures.push((name_of(path), err.to_string()));
                    failed += 1;
                    continue;
                }
            }
        } else {
            legacy::from_wmap(&raw, &catalog)
        };

        let (map, unresolved) = match outcome {
            Ok(result) => result,
            Err(err) => {
                failures.push((name_of(path), err.to_string()));
                failed += 1;
                continue;
            }
        };

        let mut encoded = Vec::new();
        if let Err(err) = map.write(&mut encoded) {
            failures.push((name_of(path), err.to_string()));
            failed += 1;
            continue;
        }

        // Every conversion is verified by reading it straight back. A format that writes files it
        // cannot read is worse than one that fails outright.
        match Map::read(&encoded) {
            Ok(reloaded) if reloaded == map => {}
            Ok(_) => {
                failures.push((name_of(path), "round trip changed the map".into()));
                failed += 1;
                continue;
            }
            Err(err) => {
                failures.push((name_of(path), format!("round trip failed: {err}")));
                failed += 1;
                continue;
            }
        }

        if let Some(dir) = &out {
            // Two of these exist as both `.jm` and `.wmap`, snakepit and tomb, and dropping the
            // extension made them the same output file, so one silently overwrote the other and
            // which one won depended on the order the directory happened to be read in. The first
            // writer keeps the plain name and the rest are qualified, so both survive and the
            // choice is deterministic.
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let plain = format!("{stem}.hmap");

            let name = if written_names.insert(plain.clone()) {
                plain
            } else {
                let extension = path.extension().unwrap_or_default().to_string_lossy();
                let qualified = format!("{stem}.{extension}.hmap");
                collisions.push((name_of(path), qualified.clone()));
                qualified
            };

            let target = dir.join(name);
            std::fs::write(&target, &encoded).expect("writing the converted map");
        }

        converted += 1;
        legacy_bytes += raw.len();
        ours_bytes += encoded.len();
        squares += (map.width() as usize) * (map.height() as usize);

        merge(&mut missing.grounds, unresolved.grounds);
        merge(&mut missing.objects, unresolved.objects);
        merge(&mut missing.regions, unresolved.regions);
    }

    let elapsed = started.elapsed();

    println!("maps      {converted} converted, {failed} failed, in {elapsed:.1?}");
    println!(
        "squares   {squares} total ({:.1} million)",
        squares as f64 / 1_000_000.0
    );
    if !collisions.is_empty() {
        println!(
            "\n{} maps exist under two source formats and were kept apart:",
            collisions.len()
        );
        for (source, written) in &collisions {
            println!("  {source} -> {written}");
        }
    }

    println!(
        "size      {:.1} KB legacy -> {:.1} KB ours  ({:+.0}%)",
        legacy_bytes as f64 / 1024.0,
        ours_bytes as f64 / 1024.0,
        (ours_bytes as f64 / legacy_bytes.max(1) as f64 - 1.0) * 100.0
    );

    for (label, names) in [
        ("grounds", &missing.grounds),
        ("objects", &missing.objects),
        ("regions", &missing.regions),
    ] {
        if names.is_empty() {
            continue;
        }
        println!("\nunknown {label} ({}):", names.len());
        for name in names.iter().take(12) {
            println!("  - {name}");
        }
        if names.len() > 12 {
            println!("  ... and {} more", names.len() - 12);
        }
    }

    if !failures.is_empty() {
        println!("\nfailures:");
        for (file, reason) in failures.iter().take(20) {
            println!("  {file}: {reason}");
        }
    }

    // World definitions travel alongside the maps.
    let mut worlds_read = 0usize;
    let mut world_failures = Vec::new();
    let mut referenced: BTreeSet<String> = BTreeSet::new();

    if let Ok(entries) = std::fs::read_dir(&worlds) {
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("jw"))
            .collect();
        paths.sort();

        for path in paths {
            match std::fs::read_to_string(&path).map(|text| WorldDef::parse(&text)) {
                Ok(Ok(world)) => {
                    worlds_read += 1;
                    referenced.extend(world.maps.iter().cloned());
                }
                Ok(Err(err)) => world_failures.push((name_of(&path), err.to_string())),
                Err(err) => world_failures.push((name_of(&path), err.to_string())),
            }
        }
    }

    println!(
        "\nworlds    {worlds_read} parsed, {} failed, referencing {} maps",
        world_failures.len(),
        referenced.len()
    );
    for (file, reason) in world_failures.iter().take(10) {
        println!("  {file}: {reason}");
    }
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn merge(into: &mut Vec<String>, from: Vec<String>) {
    for name in from {
        if !into.contains(&name) {
            into.push(name);
        }
    }
}
