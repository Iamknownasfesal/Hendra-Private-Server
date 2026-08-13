//! What abilities the content actually uses, ranked by how often each appears.

use std::collections::BTreeMap;
use std::path::Path;

use hendra_content::Catalog;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../godot-client/assets/xml".to_string());
    let (catalog, report) = Catalog::load_dir(Path::new(&dir)).expect("a content directory");

    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
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
            *kinds.entry(activate.name.clone()).or_default() += 1;
        }
    }

    let total: usize = kinds.values().sum();
    println!(
        "{} objects, {items} items, {with_ability} with an ability, {total} activates\n",
        report.objects
    );

    let mut ranked: Vec<_> = kinds.into_iter().collect();
    ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n));

    let mut running = 0usize;
    for (name, count) in &ranked {
        running += count;
        println!(
            "{count:>5}  {name:<26} {:>5.1}% cumulative",
            100.0 * running as f64 / total as f64
        );
    }
}
