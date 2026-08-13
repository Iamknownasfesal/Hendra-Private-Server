//! A readable listing of converted maps, since HMAP is binary and cannot be eyeballed.

use std::path::PathBuf;

use hendra_content::Map;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("content/maps"));

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("a maps directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("hmap"))
        .collect();
    files.sort();

    println!(
        "{:<34} {:>9} {:>10} {:>7} {:>8} {:>7}",
        "map", "size", "squares", "kinds", "objects", "bytes"
    );

    let (mut squares, mut bytes) = (0usize, 0usize);
    for path in &files {
        let raw = std::fs::read(path).expect("readable");
        let Ok(map) = Map::read(&raw) else {
            println!("{:<34} unreadable", name(path));
            continue;
        };

        let objects = map.objects().count();

        println!(
            "{:<34} {:>4}x{:<4} {:>10} {:>7} {:>8} {:>7}",
            name(path),
            map.width(),
            map.height(),
            map.width() as usize * map.height() as usize,
            map.distinct_squares(),
            objects,
            raw.len()
        );

        squares += map.width() as usize * map.height() as usize;
        bytes += raw.len();
    }

    println!(
        "\n{} maps, {squares} squares, {:.1} KB",
        files.len(),
        bytes as f64 / 1024.0
    );
}

fn name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into()
}
