use hendra_behavior::compile::compile;
use hendra_behavior::parse::parse;
use hendra_behavior::transpile::{Report, transpile};
use std::collections::BTreeMap;

/// Every loot kind the original's tables use, and whether this server rolls it.
///
/// Counted because nothing counted it: `threshold` was dropped by the converter and again by the
/// compiler, and two hundred of the content's soulbound drops went nowhere in silence.
const LOOT_KINDS: &[(&str, bool)] = &[("item", true), ("tier", true), ("threshold", true)];

fn main() {
    let source = std::path::PathBuf::from("../Server-Side/wServer/logic/db");
    let mut files: Vec<_> = std::fs::read_dir(&source)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("cs"))
        .collect();
    files.sort();

    let mut behaviours: BTreeMap<String, usize> = BTreeMap::new();
    let mut conditions: BTreeMap<String, usize> = BTreeMap::new();
    let mut total_b = 0usize;
    let mut total_c = 0usize;

    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut report = Report::default();
        let converted = transpile(&text, &mut report);
        if converted.trim().is_empty() {
            continue;
        }
        let Ok(parsed) = parse(&converted) else {
            continue;
        };
        let (programs, _) = compile(&parsed);
        for program in &programs.programs {
            for state in &program.states {
                for b in &state.behaviours {
                    total_b += 1;
                    if let hendra_behavior::Primitive::Unsupported { name } = b {
                        *behaviours.entry(name.clone()).or_default() += 1;
                    }
                }
                for t in &state.transitions {
                    total_c += 1;
                    if let hendra_behavior::Condition::Unsupported { name } = &t.condition {
                        *conditions.entry(name.clone()).or_default() += 1;
                    }
                }
            }
        }
    }

    let mut b: Vec<_> = behaviours.into_iter().collect();
    b.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let missing_b: usize = b.iter().map(|(_, n)| n).sum();
    println!(
        "BEHAVIOURS: {missing_b} of {total_b} uses unimplemented ({:.0}% covered)",
        100.0 * (total_b - missing_b) as f64 / total_b as f64
    );
    for (name, n) in &b {
        println!("  {n:>4}  {name}");
    }

    let unrolled: Vec<&str> = LOOT_KINDS
        .iter()
        .filter(|(_, rolled)| !rolled)
        .map(|(name, _)| *name)
        .collect();
    println!(
        "\nLOOT: {} of {} kinds rolled{}",
        LOOT_KINDS.len() - unrolled.len(),
        LOOT_KINDS.len(),
        if unrolled.is_empty() {
            String::new()
        } else {
            format!(", missing {}", unrolled.join(", "))
        }
    );

    let mut c: Vec<_> = conditions.into_iter().collect();
    c.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let missing_c: usize = c.iter().map(|(_, n)| n).sum();
    println!(
        "\nCONDITIONS: {missing_c} of {total_c} uses unimplemented ({:.0}% covered)",
        100.0 * (total_c - missing_c) as f64 / total_c as f64
    );
    for (name, n) in &c {
        println!("  {n:>4}  {name}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_loot_kind_the_content_uses_is_rolled() {
        // `threshold` was dropped twice over, by the converter and then by the compiler, and two
        // hundred of the content's soulbound drops went nowhere in silence. This is what would have
        // said so.
        let unrolled: Vec<&str> = LOOT_KINDS
            .iter()
            .filter(|(_, rolled)| !rolled)
            .map(|(name, _)| *name)
            .collect();

        assert!(unrolled.is_empty(), "nothing rolls: {unrolled:?}");
    }

    #[test]
    fn a_threshold_survives_being_converted_and_compiled() {
        // The end-to-end check the gap needed: the C# says who is eligible and what they get, and
        // both have to still be there after two translations.
        let source = r#".Init("Boss", new State(),
            new Threshold(0.05,
                new ItemLoot("Health Potion", 0.5),
                new TierLoot(7, ItemType.Weapon, 0.2)
            ))"#;

        let mut report = Report::default();
        let converted = transpile(source, &mut report);
        assert!(
            converted.contains("threshold(0.05)"),
            "the share was lost: {converted}"
        );
        assert!(
            converted.contains("Health Potion"),
            "the children were lost: {converted}"
        );

        let behaviours = hendra_behavior::parse(&converted).expect("it parses");
        let programs = hendra_behavior::compile(&behaviours);

        let boss = programs
            .0
            .programs
            .iter()
            .find(|program| program.name == "Boss")
            .expect("the boss");

        let held = boss
            .loot
            .iter()
            .find_map(|entry| match entry {
                hendra_behavior::program::LootEntry::Threshold { share, children } => {
                    Some((share, children.len()))
                }
                _ => None,
            })
            .expect("a threshold");

        assert!((held.0 - 0.05).abs() < 0.001, "share is {}", held.0);
        assert_eq!(held.1, 2, "both children should survive");
    }
}
