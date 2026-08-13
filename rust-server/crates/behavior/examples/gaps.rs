use hendra_behavior::compile::compile;
use hendra_behavior::parse::parse;
use hendra_behavior::transpile::{Report, transpile};
use std::collections::BTreeMap;

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
