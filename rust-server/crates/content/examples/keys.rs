//! Which per-entity settings the shipped maps carry, and whether anything reads them.
//!
//! A census, in the way the others are. The original reads a dozen keys off each placed object and
//! this server reads two, so the question worth answering is not "which keys exist" but "which keys
//! appear in the maps we actually ship". A key that turns up here and is not in the list below is a
//! map asking for something nobody is listening for.

/// What we read, and what we have looked at and decided not to.
///
/// `conn` picks which sprite a wall or fence draws, from how its neighbours join up; nothing about
/// it reaches the simulation. `xOffset` and `yOffset` nudge a placed object by a fraction of a tile,
/// which moves where it is drawn and not what it blocks, since a static object blocks the square it
/// stands on either way.
const KNOWN: &[(&str, &str)] = &[
    ("conn", "presentational: which way a wall piece joins up"),
    ("name", "read: overrides the object's displayed name"),
    ("size", "read: a percentage of the object's natural size"),
    ("xOffset", "presentational: nudges where it is drawn"),
    ("yOffset", "presentational: nudges where it is drawn"),
];

fn main() {
    let counts = census();

    println!("{:>8}  {:<10}  what it is", "uses", "key");
    for (key, count) in &counts {
        let known = KNOWN.iter().find(|(known, _)| known == key);
        println!(
            "{count:>8}  {key:<10}  {}",
            known.map(|(_, what)| *what).unwrap_or("NOBODY READS THIS")
        );
    }

    let unread = counts
        .iter()
        .filter(|(key, _)| !KNOWN.iter().any(|(known, _)| known == key))
        .count();

    println!();
    println!(
        "SETTINGS: {} kinds, {unread} that nothing reads",
        counts.len()
    );
}

/// Every setting key in every shipped map, with how often it appears.
fn census() -> std::collections::BTreeMap<String, usize> {
    let dir = ["content/maps", "../../content/maps"]
        .into_iter()
        .map(std::path::Path::new)
        .find(|dir| dir.exists())
        .expect("the maps directory");

    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();

    for entry in std::fs::read_dir(dir)
        .expect("the maps directory")
        .flatten()
    {
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Ok(map) = hendra_content::Map::read(&bytes) else {
            continue;
        };

        for (_, _, square) in map.objects() {
            for (key, _) in square.settings() {
                *counts.entry(key.to_string()).or_default() += 1;
            }
        }
    }

    counts
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_setting_the_maps_ask_for_is_one_somebody_has_looked_at() {
        let counts = super::census();
        assert!(!counts.is_empty(), "no maps were read");

        let unread: Vec<&String> = counts
            .keys()
            .filter(|key| !super::KNOWN.iter().any(|(known, _)| *known == key.as_str()))
            .collect();

        assert!(
            unread.is_empty(),
            "the maps ask for settings nothing reads: {unread:?}"
        );
    }
}
