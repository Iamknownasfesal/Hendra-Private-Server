//! What abilities the content actually uses, ranked by how often each appears, and how many of
//! those uses the runtime implements.

use std::collections::BTreeMap;
use std::path::Path;

use hendra_content::{Catalog, Effect};

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../godot-client/assets/xml".to_string());
    let (catalog, report) = Catalog::load_dir(Path::new(&dir)).expect("a content directory");

    let mut kinds: BTreeMap<String, (usize, bool)> = BTreeMap::new();
    let (mut items, mut with_ability) = (0usize, 0usize);

    for desc in catalog.items() {
        let Some(item) = desc.item.as_ref() else {
            continue;
        };
        items += 1;
        if !item.activate.is_empty() || !item.activate_on_equip.is_empty() {
            with_ability += 1;
        }
        for activate in item.activate.iter().chain(&item.activate_on_equip) {
            let effect = Effect::of(activate);
            let entry = kinds.entry(activate.name.clone()).or_insert((0, false));
            entry.0 += 1;
            entry.1 = effect.is_supported();
        }
    }

    let total: usize = kinds.values().map(|(count, _)| count).sum();
    let done: usize = kinds
        .values()
        .filter(|(_, supported)| *supported)
        .map(|(count, _)| count)
        .sum();

    println!(
        "{} objects, {items} items, {with_ability} with an ability, {total} activates",
        report.objects
    );
    println!(
        "IMPLEMENTED: {done} of {total} uses ({:.0}% covered), {} of {} kinds\n",
        100.0 * done as f64 / total.max(1) as f64,
        kinds.values().filter(|(_, s)| *s).count(),
        kinds.len()
    );

    let mut ranked: Vec<_> = kinds.into_iter().collect();
    ranked.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));

    let mut running = 0usize;
    for (name, (count, supported)) in &ranked {
        running += count;
        println!(
            "{} {count:>5}  {name:<28} {:>5.1}% cumulative",
            if *supported { "ok " } else { "TODO" },
            100.0 * running as f64 / total.max(1) as f64
        );
    }
}
