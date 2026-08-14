//! Every command the original accepts, and what answers it here.
//!
//! Counted for the same reason the packets and the endpoints are: ninety-one commands went
//! unnoticed for as long as nothing listed them, and the only thing that has ever found a gap in
//! this server is counting something rather than reasoning about it.
//!
//!     cargo run -p hendra-server --example commands

// The same file the server compiles, rather than a copy of its table. A copy is how a census stops
// matching what it is counting.
#[path = "../src/commands.rs"]
#[allow(dead_code)]
mod commands;

fn main() {
    println!(
        "{} commands, {} of them for moderators or administrators",
        commands::ALL.len(),
        commands::ALL
            .iter()
            .filter(|command| command.needs != commands::Needs::Nobody)
            .count()
    );
    println!();

    for command in commands::ALL {
        let rank = match command.needs {
            commands::Needs::Nobody => "",
            commands::Needs::Moderator => " (moderator)",
            commands::Needs::Administrator => " (administrator)",
        };

        let spellings = if command.aliases.is_empty() {
            String::new()
        } else {
            format!("  [{}]", command.aliases.join(", "))
        };

        println!("/{:<14} {}{rank}{spellings}", command.name, command.summary);
    }
}
