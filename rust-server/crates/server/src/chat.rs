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

    /// The last line said, stripped of everything that makes two sayings of the same thing look
    /// different, and when it was said.
    last: Option<(String, Instant)>,

    /// How far the line before that one was from the one before it.
    ///
    /// Two in a row rather than one, from `CompareAndCheckSpam`: saying the same thing twice is
    /// somebody making sure they were heard, and saying it three times is spam.
    last_deviation: usize,

    /// Whether the previous line was already a repeat, so the next one is refused rather than the
    /// first offence being.
    repeating: bool,
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

    /// How long a line is remembered for the purpose of noticing a repeat.
    ///
    /// Ten seconds, from the original. Long enough that saying the same thing over and over is
    /// caught, short enough that answering the same question twice in an evening is not.
    pub const REPEAT_WINDOW: Duration = Duration::from_secs(10);

    /// How different two lines have to be to count as different things.
    ///
    /// Nothing at all for a short line, since "ok" and "no" are two characters apart and both are
    /// worth saying. Three edits for anything longer, which is what catches a line retyped with a
    /// character changed to get past a check on the exact text.
    fn far_enough(length: usize) -> usize {
        if length > 4 { 3 } else { 0 }
    }

    pub fn new() -> Limit {
        Limit {
            recent: Vec::with_capacity(ALLOWED),
            last: None,
            last_deviation: usize::MAX,
            repeating: false,
        }
    }

    /// Whether this is the same thing being said again, recording it either way.
    ///
    /// Follows `Player.CompareAndCheckSpam`. Two things make it work: the comparison ignores
    /// punctuation, case and runs of a repeated character, so "hi!!!" and "HIII" are the same line;
    /// and it takes two repeats rather than one, so somebody making sure they were heard is not
    /// refused.
    pub fn repeats(&mut self, line: &str, now: Instant) -> bool {
        let stripped = strip(line);

        let Some((last, at)) = self.last.take() else {
            self.last = Some((stripped, now));
            self.repeating = false;
            return false;
        };

        // Faster than anybody types, which is not a person saying something twice.
        if now.duration_since(at) < Duration::from_millis(500) {
            self.last = Some((stripped, now));
            let was = self.repeating;
            self.repeating = true;
            return was;
        }

        if now.duration_since(at) > Limit::REPEAT_WINDOW {
            self.last_deviation = distance(&last, &stripped);
            self.last = Some((stripped, now));
            self.repeating = false;
            return false;
        }

        let deviation = distance(&last, &stripped);
        let same = self.last_deviation <= Limit::far_enough(stripped.len())
            && deviation <= Limit::far_enough(line.chars().count());

        self.last_deviation = deviation;
        self.last = Some((stripped, now));

        if !same {
            self.repeating = false;
            return false;
        }

        let was = self.repeating;
        self.repeating = true;
        was
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

/// A line reduced to what it is saying, so two sayings of the same thing look the same.
///
/// Punctuation and case go, and so does every repeat of a character: "hi!!!" and "HIII" both become
/// "hi". Without that, a spammer changes one exclamation mark and starts again.
fn strip(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut previous = None;

    for held in line.chars().filter(|held| held.is_alphanumeric()) {
        let held = held.to_ascii_lowercase();
        if previous != Some(held) {
            out.push(held);
        }
        previous = Some(held);
    }

    out
}

/// How many single-character edits turn one line into the other.
///
/// The plain Levenshtein distance, as the original computes it. Two rows rather than the whole
/// table, since only the previous row is ever read.
fn distance(from: &str, to: &str) -> usize {
    let from: Vec<char> = from.chars().collect();
    let to: Vec<char> = to.chars().collect();

    if from.is_empty() {
        return to.len();
    }
    if to.is_empty() {
        return from.len();
    }

    let mut previous: Vec<usize> = (0..=to.len()).collect();
    let mut current = vec![0; to.len() + 1];

    for (i, held) in from.iter().enumerate() {
        current[0] = i + 1;

        for (j, other) in to.iter().enumerate() {
            let cost = usize::from(held != other);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
        }

        std::mem::swap(&mut previous, &mut current);
    }

    previous[to.len()]
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

#[cfg(test)]
mod repetition {
    use super::*;

    /// Says a line `count` times, a second apart, and answers how many were refused.
    fn said(lines: &[&str]) -> Vec<bool> {
        let mut limit = Limit::new();
        let mut now = Instant::now();

        lines
            .iter()
            .map(|line| {
                now += Duration::from_secs(1);
                limit.repeats(line, now)
            })
            .collect()
    }

    #[test]
    fn saying_something_again_is_making_sure_you_were_heard() {
        // Three of the same line pass and the fourth does not, which is what the original does and
        // for a reason worth keeping: it takes two lines to establish that a line is being repeated
        // at all, and one more before the repeating is treated as deliberate.
        let refused = said(&["is anyone there", "is anyone there", "is anyone there"]);
        assert_eq!(refused, [false, false, false]);
    }

    #[test]
    fn saying_it_over_and_over_is_spam() {
        let refused = said(&[
            "is anyone there",
            "is anyone there",
            "is anyone there",
            "is anyone there",
            "is anyone there",
        ]);

        assert_eq!(refused, [false, false, false, true, true]);
    }

    #[test]
    fn a_conversation_is_never_refused() {
        let refused = said(&[
            "has anyone seen the tomb",
            "there is one in the realm",
            "which realm",
            "the third one down",
        ]);
        assert!(!refused.iter().any(|refused| *refused));
    }

    #[test]
    fn changing_the_punctuation_is_saying_the_same_thing() {
        // Otherwise a spammer adds an exclamation mark and starts again.
        let refused = said(&[
            "buy gold now",
            "buy gold now!",
            "BUY GOLD NOW!!!",
            "buy gold now...",
        ]);

        assert!(refused[3], "punctuation got round the check");

        // And the same line typed out plainly is refused at the same point, so the stripping is
        // what made those three count as one line rather than the timing.
        let plain = said(&[
            "buy gold now",
            "buy gold now",
            "buy gold now",
            "buy gold now",
        ]);
        assert_eq!(refused, plain);
    }

    #[test]
    fn a_repeated_letter_is_the_same_letter() {
        assert_eq!(strip("hiii!!!"), strip("HI"));
    }

    #[test]
    fn two_short_lines_are_never_the_same_line() {
        // "ok" and "no" are two edits apart and both are worth saying, so a short line is only a
        // repeat when it is the very same line.
        let refused = said(&["ok", "no", "ok", "no"]);
        assert!(!refused.iter().any(|refused| *refused));
    }

    #[test]
    fn a_gap_forgets_what_was_said() {
        // Answering the same question twice in an evening is not spam.
        let mut limit = Limit::new();
        let mut now = Instant::now();

        for _ in 0..5 {
            assert!(!limit.repeats("the tomb is that way", now));
            now += Limit::REPEAT_WINDOW + Duration::from_secs(1);
        }
    }

    #[test]
    fn typing_faster_than_a_person_can_is_not_a_person() {
        let mut limit = Limit::new();
        let now = Instant::now();

        assert!(!limit.repeats("first", now));
        assert!(!limit.repeats("second", now + Duration::from_millis(100)));
        assert!(
            limit.repeats("third", now + Duration::from_millis(200)),
            "three different lines in a fifth of a second was accepted"
        );
    }

    #[test]
    fn the_distance_is_the_number_of_edits() {
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("abc", ""), 3);
        assert_eq!(distance("abc", "abc"), 0);
        assert_eq!(distance("kitten", "sitting"), 3);
    }
}
