//! Converts the C# behaviour database into this language, and reports what it found.
//!
//! Run with: `cargo run --release -p hendra-behavior --example convert -- [out-dir]`
//!
//! Without an output directory it converts in memory and reports only, which is the useful mode for
//! checking the whole corpus still round-trips after a change to the emitter.

use std::path::PathBuf;
use std::time::Instant;

use hendra_behavior::compile::compile;
use hendra_behavior::parse::parse;
use hendra_behavior::transpile::{Report, transpile};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().map(PathBuf::from);

    let source = PathBuf::from("../Server-Side/wServer/logic/db");
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&source) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("cs"))
            .collect(),
        Err(err) => {
            eprintln!("could not read {}: {err}", source.display());
            std::process::exit(1);
        }
    };
    files.sort();

    if let Some(directory) = &out {
        std::fs::create_dir_all(directory).expect("output directory");
    }

    let started = Instant::now();
    let mut report = Report::default();
    let mut lines_in = 0usize;
    let mut lines_out = 0usize;

    let mut failed_to_parse = Vec::new();
    let mut diagnostics = 0usize;
    let mut compiled = 0usize;

    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        lines_in += text.lines().count();

        let converted = transpile(&text, &mut report);
        if converted.trim().is_empty() {
            continue;
        }
        lines_out += converted.lines().count();

        // Every file is parsed back. A transpiler whose output does not parse has converted
        // nothing, however convincing the output looks.
        match parse(&converted) {
            Ok(parsed) => {
                let (programs, found) = compile(&parsed);
                compiled += programs.len();
                diagnostics += found.len();
            }
            Err(err) => failed_to_parse.push((name_of(path), err.to_string())),
        }

        if let Some(directory) = &out {
            let target = directory.join(format!(
                "{}.beh",
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .replace("BehaviorDb.", "")
                    .to_lowercase()
            ));
            std::fs::write(&target, &converted).expect("writing the converted behaviours");
        }
    }

    let elapsed = started.elapsed();

    println!("read      {} files, {lines_in} lines of C#", files.len());
    println!(
        "converted {} enemies into {lines_out} lines, in {elapsed:.1?}",
        report.enemies
    );
    println!("compiled  {compiled} programs, {diagnostics} diagnostics");

    if !report.skipped.is_empty() {
        println!("\nskipped {} entries:", report.skipped.len());
        for (name, reason) in report.skipped.iter().take(8) {
            println!("  {name}: {reason}");
        }
    }

    if !report.renamed.is_empty() {
        println!(
            "\nrenamed {} shadowed states (the C# keeps the last of a repeated name):",
            report.renamed.len()
        );
        for (enemy, change) in report.renamed.iter().take(6) {
            println!("  {enemy}: {change}");
        }
    }

    if !report.dangling.is_empty() {
        println!(
            "\ndropped {} transitions naming a state that does not exist.",
            report.dangling.len()
        );
        println!("  These are bugs in the original: the C# looks the target up in a dictionary");
        println!("  with no fallback, so each one throws when its enemy spawns.");
        for (enemy, detail) in report.dangling.iter().take(8) {
            println!("    {enemy}: {detail}");
        }
    }

    if !failed_to_parse.is_empty() {
        println!("\n{} files did not parse back:", failed_to_parse.len());
        for (file, err) in failed_to_parse.iter().take(8) {
            println!("  {file}: {err}");
        }
    }

    // The reason to run this early: it says which primitives are worth implementing, measured
    // rather than guessed.
    let counts = report.by_use();
    let total: usize = counts.iter().map(|(_, seen)| seen).sum();
    println!(
        "\n{} distinct primitives, {total} uses in total",
        counts.len()
    );

    let implemented = [
        "shoot",
        "wander",
        "follow",
        "orbit",
        "stay_back",
        "stay_close_to_spawn",
        "heal_self",
        "spawn",
        "suicide",
        "prioritize",
        "timed",
        "player_within",
        "no_player_within",
        "hp_below",
    ];

    let covered: usize = counts
        .iter()
        .filter(|(name, _)| implemented.contains(&name.as_str()))
        .map(|(_, seen)| seen)
        .sum();

    println!(
        "{covered} of them ({:.0}%) are primitives the runtime already implements\n",
        covered as f64 / total.max(1) as f64 * 100.0
    );

    println!("most used:");
    for (name, seen) in counts.iter().take(24) {
        let mark = if implemented.contains(&name.as_str()) {
            "done"
        } else {
            "    "
        };
        println!("  {mark}  {seen:>5}  {name}");
    }
}

fn name_of(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
