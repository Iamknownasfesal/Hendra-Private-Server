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

    /// Something the server should act on rather than repeat.
    Command { name: String, rest: String },

    /// Nothing worth sending.
    Nothing,
}

/// The longest line the server will look at.
///
/// `PlayerTextHandler` drops anything longer than this without answering, so a line over the limit
/// is not shortened, refused or repeated: it never happened.
pub const MAX_LENGTH: usize = 512;

/// Reads what a player typed.
///
/// A line beginning with a slash is a command whatever else it looks like, and it is split on the
/// first space with the whole remainder handed over as one string, exactly as
/// `CommandManager.Execute` splits it. Whispering is `/tell` like any other command rather than
/// something read here, so its refusals are the original's.
pub fn read(line: &str) -> Said {
    // Counted in UTF-16 code units, which is what `text.Length` counts: a line of astral characters
    // reaches the limit at half as many of them as a line of Latin ones.
    if line.is_empty() || line.encode_utf16().count() > MAX_LENGTH {
        return Said::Nothing;
    }

    let Some(rest) = line.strip_prefix('/') else {
        if line.trim().is_empty() {
            return Said::Nothing;
        }
        return Said::Say(line.to_string());
    };

    // Everything after the first space, untrimmed: a command that reads a name and a reason reads
    // the reason exactly as it was typed.
    let (word, tail) = rest.split_once(' ').unwrap_or((rest, ""));

    // A lone slash is a command with no name, which the original answers with `Unknown command!`
    // rather than with silence.
    Said::Command {
        name: word.to_string(),
        rest: tail.to_string(),
    }
}

/// How often one player may say the same thing.
///
/// The only thing standing between a player and the chat, because it is the only thing the original
/// puts there: `PlayerTextHandler` checks the name, the mute and `CompareAndCheckSpam`, and nothing
/// counts lines per second. A player answering six people in five seconds is saying six different
/// things, and refusing that would silence a conversation the original allows.
#[derive(Debug, Clone)]
pub struct Limit {
    /// The last line said, stripped of everything that makes two sayings of the same thing look
    /// different, and when it was said.
    last: Option<(String, Instant)>,

    /// The distance measured between the previous pair of lines, kept so that a repeat is only
    /// called one when two consecutive pairs were both close.
    ///
    /// `LastMessageDeviation`, and it starts at the largest number there is: until two lines have
    /// been said there is nothing for the next one to be a repeat of.
    last_deviation: usize,

    /// Whether the previous line was already a repeat, so the next one is refused rather than the
    /// first offence being.
    repeating: bool,
}

impl Default for Limit {
    fn default() -> Limit {
        Limit::new()
    }
}

impl Limit {
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
        let Some((last, at)) = self.last.take() else {
            // The first line of the connection. `LastMessageTime` starts at zero against a clock
            // that is the server's uptime, so the first line is always older than the ten seconds
            // that forget what was said: it takes that branch, against an empty previous line.
            let stripped = strip(line);
            self.last_deviation = stripped.len();
            self.last = Some((stripped, now));
            self.repeating = false;
            return false;
        };

        // Faster than anybody types, which is not a person saying something twice. Only the time is
        // recorded: the original never strips this line and never stores it, so the next line is
        // still measured against the last one said at a human pace, and the deviation carried over
        // from that pair is the one the next comparison reads.
        if now.duration_since(at) < Duration::from_millis(500) {
            self.last = Some((last, now));
            let was = self.repeating;
            self.repeating = true;
            return was;
        }

        let stripped = strip(line);

        if now.duration_since(at) > Limit::REPEAT_WINDOW {
            self.last_deviation = distance(&last, &stripped);
            self.last = Some((stripped, now));
            self.repeating = false;
            return false;
        }

        let deviation = distance(&last, &stripped);

        // Both thresholds are the original's, and they read different lengths. The first reads the
        // line just said, because `LastMessage` has already been reassigned by the time the check
        // runs; stripping leaves only ASCII, so its byte length is the `string.Length` the original
        // sees. The second reads the raw line in UTF-16 code units, which is what `message.Length`
        // counts: an astral character counts twice there and once as a `char` here.
        let same = self.last_deviation <= Limit::far_enough(stripped.len())
            && deviation <= Limit::far_enough(line.encode_utf16().count());

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
}

/// A line reduced to what it is saying, so two sayings of the same thing look the same.
///
/// Two passes, as `nonAlphaNum` and `repetition` do them. The first keeps only `[a-zA-Z0-9 ]` and
/// lowercases what is left, so punctuation, case and every character outside plain ASCII go — a
/// line of Cyrillic reduces to nothing at all, which is what the original's character class does to
/// it. Spaces survive, so the words stay separated and the length the thresholds read is the length
/// of a sentence rather than of one long word.
///
/// The second drops any character equal to the one before it, so "hi!!!" and "HIII" both become
/// "hi". Without that, a spammer changes one exclamation mark and starts again.
fn strip(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut previous = None;

    for held in line
        .chars()
        .filter(|held| held.is_ascii_alphanumeric() || *held == ' ')
    {
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
    fn a_whisper_is_a_command_like_any_other() {
        // `/tell` is a command in the original, so what it refuses and what it says about it come
        // from the command rather than from here.
        assert_eq!(
            read("/tell Fesal are you there"),
            Said::Command {
                name: "tell".to_string(),
                rest: "Fesal are you there".to_string()
            }
        );
    }

    #[test]
    fn anything_beginning_with_a_slash_is_a_command() {
        assert_eq!(
            read("/who is here"),
            Said::Command {
                name: "who".to_string(),
                rest: "is here".to_string()
            }
        );
    }

    #[test]
    fn the_words_are_split_on_the_first_space_and_nothing_else() {
        // `text.IndexOf(' ')`, and the remainder handed over whole. A tab is part of the word, and
        // the spaces inside a reason are part of the reason.
        assert_eq!(
            read("/ban Bob  because he asked "),
            Said::Command {
                name: "ban".to_string(),
                rest: "Bob  because he asked ".to_string()
            }
        );
    }

    #[test]
    fn a_lone_slash_is_a_command_with_no_name() {
        // Which the original answers with `Unknown command!` rather than with silence.
        assert_eq!(
            read("/"),
            Said::Command {
                name: String::new(),
                rest: String::new()
            }
        );
        assert_eq!(read("   "), Said::Nothing);
    }

    #[test]
    fn a_line_over_the_limit_never_happened() {
        // `PlayerTextHandler` drops anything longer than 512 characters without answering it.
        let long = "a".repeat(MAX_LENGTH + 1);
        assert_eq!(read(&long), Said::Nothing);

        let allowed = "a".repeat(MAX_LENGTH);
        assert_eq!(read(&allowed), Said::Say(allowed));

        // `text.Length` counts UTF-16 code units, so a character outside the basic plane counts
        // twice and the limit arrives at half as many of them.
        let astral = "🙂".repeat(MAX_LENGTH / 2);
        assert_eq!(read(&astral), Said::Say(astral));
        assert_eq!(read(&"🙂".repeat(MAX_LENGTH / 2 + 1)), Said::Nothing);
    }

    #[test]
    fn nothing_counts_lines_per_second() {
        // `PlayerTextHandler` checks the name, the mute and `CompareAndCheckSpam` and stops there,
        // so a burst of different lines is a conversation and goes through. A limit of so many
        // lines in so many seconds would silence somebody answering several people at once.
        let mut limit = Limit::new();
        let start = Instant::now();

        for (line, text) in ["on my way", "which one", "the top left", "coming", "wait", "ok here"]
            .iter()
            .enumerate()
        {
            let now = start + Duration::from_millis(line as u64 * 600);
            assert!(!limit.repeats(text, now), "line {line} was refused");
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
    fn the_words_stay_separated() {
        // `[^a-zA-Z0-9 ]` keeps the space, and a run of spaces collapses like a run of anything
        // else. Dropping them instead would make a short sentence one short word and put it under
        // the four characters where the threshold falls to nothing.
        assert_eq!(strip("Buy  my   gold!!!"), "buy my gold");
    }

    #[test]
    fn anything_outside_plain_ascii_is_not_there_at_all() {
        // The original's character class is `a-zA-Z0-9 ` and nothing else, so a line with no ASCII
        // letters in it reduces to the empty string and two such lines are the same line.
        assert_eq!(strip("Привет!"), "");
        assert_eq!(strip("café"), "caf");
    }

    #[test]
    fn a_sentence_is_measured_as_a_sentence() {
        // Five characters with the space, four without, and the threshold falls to nothing at four.
        // Two lines one edit apart are the same line at the original's length and different lines
        // at the shorter one, so this is what keeping the space decides.
        let refused = said(&["ab cd", "ab ce", "ab cd", "ab ce"]);
        assert_eq!(refused, [false, false, false, true]);
    }

    #[test]
    fn a_line_sent_too_fast_is_not_the_line_that_is_remembered() {
        // The sub-500ms branch records the time and returns; it never strips the line and never
        // stores it. So the burst does not become the thing the next line is compared against, and
        // the repeat that straddles it is still caught.
        let mut limit = Limit::new();
        let start = Instant::now();
        let line = "buy my gold cheap";

        assert!(!limit.repeats(line, start));
        assert!(!limit.repeats(line, start + Duration::from_secs(1)));

        // Too fast to be a person, and a different line entirely.
        assert!(!limit.repeats("z", start + Duration::from_millis(1200)));

        assert!(
            limit.repeats(line, start + Duration::from_secs(2)),
            "the fast line replaced the one being repeated"
        );
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
