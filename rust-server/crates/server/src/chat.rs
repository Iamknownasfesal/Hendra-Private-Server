//! What a line of chat means, and who hears it.
//!
//! Chat is the one thing a player sends that reaches other players verbatim, so everything that
//! makes it safe lives here: how fast it may be sent, how long it may be, and which of it is a
//! command rather than something to repeat.

use std::time::{Duration, Instant};

/// Who a line is meant for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    /// Everyone nearby.
    Say(String),

    /// One named player.
    Tell { to: String, text: String },

    /// Something the server should act on rather than repeat.
    Command { name: String, rest: String },

    /// Nothing worth sending.
    Nothing,
}

/// The longest line that will be repeated.
///
/// A line nobody can read is not communication, and an unbounded one is a way to spend everyone
/// else's bandwidth.
pub const MAX_LENGTH: usize = 256;

/// Reads what a player typed.
pub fn read(line: &str) -> Said {
    let line = line.trim();
    if line.is_empty() {
        return Said::Nothing;
    }

    // Truncated on a character boundary rather than a byte one, or a line ending in an accent
    // would be cut in half and stop being text at all.
    let line: String = line.chars().take(MAX_LENGTH).collect();

    let Some(rest) = line.strip_prefix('/') else {
        return Said::Say(line);
    };

    let (word, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let tail = tail.trim();

    match word.to_ascii_lowercase().as_str() {
        "tell" | "t" | "w" | "whisper" => match tail.split_once(char::is_whitespace) {
            Some((to, text)) if !text.trim().is_empty() => Said::Tell {
                to: to.to_string(),
                text: text.trim().to_string(),
            },
            _ => Said::Nothing,
        },
        "" => Said::Nothing,
        other => Said::Command {
            name: other.to_string(),
            rest: tail.to_string(),
        },
    }
}

/// How fast one player may speak.
///
/// A sliding window rather than a fixed one: a fixed window lets someone send a burst at the end
/// of one and another at the start of the next, which is twice the limit in an instant.
#[derive(Debug, Clone)]
pub struct Limit {
    recent: Vec<Instant>,
}

/// How many lines may be sent inside [`Limit::WINDOW`].
pub const ALLOWED: usize = 5;

impl Default for Limit {
    fn default() -> Limit {
        Limit::new()
    }
}

impl Limit {
    /// How long lines are remembered.
    pub const WINDOW: Duration = Duration::from_secs(5);

    pub fn new() -> Limit {
        Limit {
            recent: Vec::with_capacity(ALLOWED),
        }
    }

    /// Whether another line may be sent, recording it if so.
    pub fn allow(&mut self, now: Instant) -> bool {
        self.recent
            .retain(|at| now.duration_since(*at) < Limit::WINDOW);

        if self.recent.len() >= ALLOWED {
            return false;
        }

        self.recent.push(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_line_is_said_to_everyone() {
        assert_eq!(read("hello"), Said::Say("hello".to_string()));
    }

    #[test]
    fn a_tell_names_who_it_is_for() {
        assert_eq!(
            read("/tell Fesal are you there"),
            Said::Tell {
                to: "Fesal".to_string(),
                text: "are you there".to_string()
            }
        );
    }

    #[test]
    fn the_short_spellings_of_tell_all_work() {
        for spelling in ["/t", "/w", "/whisper", "/TELL"] {
            assert!(
                matches!(read(&format!("{spelling} Fesal hi")), Said::Tell { .. }),
                "{spelling}"
            );
        }
    }

    #[test]
    fn a_tell_with_nothing_to_say_is_nothing() {
        // Otherwise naming someone would send them an empty line.
        assert_eq!(read("/tell Fesal"), Said::Nothing);
        assert_eq!(read("/tell"), Said::Nothing);
    }

    #[test]
    fn anything_else_beginning_with_a_slash_is_a_command() {
        assert_eq!(
            read("/who is here"),
            Said::Command {
                name: "who".to_string(),
                rest: "is here".to_string()
            }
        );
    }

    #[test]
    fn a_lone_slash_is_not_a_command() {
        assert_eq!(read("/"), Said::Nothing);
        assert_eq!(read("   "), Said::Nothing);
    }

    #[test]
    fn a_long_line_is_cut_rather_than_refused() {
        // Refusing would lose the whole message over a mistake at the end of it.
        let long = "a".repeat(MAX_LENGTH * 2);
        match read(&long) {
            Said::Say(text) => assert_eq!(text.chars().count(), MAX_LENGTH),
            other => panic!("expected something said, got {other:?}"),
        }
    }

    #[test]
    fn a_line_is_cut_on_a_character_rather_than_a_byte() {
        // Cutting mid-character would stop it being text at all.
        let long: String = std::iter::repeat_n('é', MAX_LENGTH * 2).collect();
        match read(&long) {
            Said::Say(text) => assert_eq!(text.chars().count(), MAX_LENGTH),
            other => panic!("expected something said, got {other:?}"),
        }
    }

    #[test]
    fn a_burst_is_allowed_and_then_refused() {
        let mut limit = Limit::new();
        let now = Instant::now();

        for line in 0..ALLOWED {
            assert!(limit.allow(now), "line {line} should be allowed");
        }
        assert!(!limit.allow(now), "and the next should not");
    }

    #[test]
    fn the_window_slides_rather_than_resetting() {
        // A fixed window lets someone send a burst at the end of one and another at the start of
        // the next, which is twice the limit in an instant.
        let mut limit = Limit::new();
        let start = Instant::now();

        for line in 0..ALLOWED {
            assert!(limit.allow(start + Duration::from_millis(line as u64 * 100)));
        }

        // The first line ages out, and exactly one more becomes available.
        let freed = start + Limit::WINDOW;
        assert!(limit.allow(freed));
        assert!(!limit.allow(freed));
    }

    #[test]
    fn waiting_out_the_window_allows_a_full_burst_again() {
        let mut limit = Limit::new();
        let start = Instant::now();

        for _ in 0..ALLOWED {
            limit.allow(start);
        }

        let later = start + Limit::WINDOW + Duration::from_millis(1);
        for line in 0..ALLOWED {
            assert!(limit.allow(later), "line {line} after the window");
        }
    }
}
