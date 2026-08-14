//! Keeping count of what a connection has been refused, and cutting it once the refusals stop
//! looking like accidents.
//!
//! Follows `Player.Verify.Strike`. The world already refuses each thing on its own: a move too far
//! is clamped, a shot too soon is dropped, a hit is decided by the server whatever the client says.
//! None of that needs a record to be correct. What a record adds is the difference between one
//! refusal and forty in ten seconds, which is the difference between a bad connection and a client
//! that has been changed.
//!
//! # Why the account is left alone
//!
//! Deliberately. Everything counted here is a judgement made from timings and distances measured
//! over a network, and a network can produce every one of these honestly. The connection is cut and
//! the reason is logged for somebody to read; nothing durable happens to the player. A server that
//! banned on arithmetic would eventually ban somebody on a train.
//!
//! # Why a window
//!
//! Because the count that matters is a rate. Twelve refusals over an evening is a bad line; twelve
//! in ten seconds is not a line at all. The window resets rather than decaying, which is coarser
//! than a decay and enough: what is being separated differs by an order of magnitude, not a little.

use std::time::{Duration, Instant};

/// How many refusals inside one window end the connection.
///
/// Twelve, from the original. High enough that a bad few seconds does not reach it and low enough
/// that a client refusing constantly does so quickly.
pub const LIMIT: u32 = 12;

/// How long the window is.
pub const WINDOW: Duration = Duration::from_secs(10);

/// What a connection has been refused lately.
#[derive(Debug, Clone)]
pub struct Strikes {
    count: u32,

    /// When the current window ends. `None` before the first strike.
    until: Option<Instant>,
}

impl Default for Strikes {
    fn default() -> Strikes {
        Strikes::new()
    }
}

/// What to do about a strike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Counted, and nothing else. Almost always this.
    Noted,

    /// Enough in one window. Cut the connection.
    Cut,
}

impl Strikes {
    pub fn new() -> Strikes {
        Strikes {
            count: 0,
            until: None,
        }
    }

    /// Notes one refusal and says whether it was the last straw.
    ///
    /// `now` is passed in rather than read, so a test can move time without waiting for it.
    pub fn note(&mut self, now: Instant) -> Verdict {
        // Expired rather than decayed: what is being separated is a bad few seconds from a client
        // that never stops, and those differ by an order of magnitude rather than a little.
        if self.until.is_some_and(|until| now >= until) {
            self.count = 0;
        }

        self.until = Some(now + WINDOW);
        self.count += 1;

        if self.count >= LIMIT {
            // Reset on the way out, so a connection that survives being told off does not carry the
            // count into whatever comes next.
            self.count = 0;
            return Verdict::Cut;
        }

        Verdict::Noted
    }

    /// How many are held, for a log line.
    pub fn held(&self) -> u32 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_refusal_is_not_an_accusation() {
        let mut strikes = Strikes::new();
        assert_eq!(strikes.note(Instant::now()), Verdict::Noted);
        assert_eq!(strikes.held(), 1);
    }

    #[test]
    fn enough_in_one_window_ends_the_connection() {
        let mut strikes = Strikes::new();
        let now = Instant::now();

        for _ in 0..(LIMIT - 1) {
            assert_eq!(strikes.note(now), Verdict::Noted);
        }
        assert_eq!(strikes.note(now), Verdict::Cut);
    }

    #[test]
    fn refusals_spread_out_never_add_up() {
        // A bad line produces these honestly. Twelve over an evening is somebody on a train; twelve
        // in ten seconds is not a line at all.
        let mut strikes = Strikes::new();
        let mut now = Instant::now();

        for _ in 0..100 {
            assert_eq!(strikes.note(now), Verdict::Noted);
            now += WINDOW + Duration::from_millis(1);
        }
    }

    #[test]
    fn the_window_runs_from_the_last_strike_rather_than_the_first() {
        // A client refusing steadily just under the limit should still be cut eventually, which it
        // is not if the window is anchored to the first strike and keeps expiring.
        let mut strikes = Strikes::new();
        let mut now = Instant::now();

        for _ in 0..(LIMIT - 1) {
            assert_eq!(strikes.note(now), Verdict::Noted);
            now += Duration::from_millis(500);
        }

        assert_eq!(strikes.note(now), Verdict::Cut);
    }

    #[test]
    fn being_cut_clears_the_count() {
        // Or a connection that survives being told off carries the count into whatever comes next
        // and is cut again immediately.
        let mut strikes = Strikes::new();
        let now = Instant::now();

        for _ in 0..LIMIT {
            strikes.note(now);
        }
        assert_eq!(strikes.held(), 0);
        assert_eq!(strikes.note(now), Verdict::Noted);
    }

    #[test]
    fn a_window_that_has_expired_starts_the_count_again_rather_than_continuing_it() {
        let mut strikes = Strikes::new();
        let now = Instant::now();

        for _ in 0..(LIMIT - 1) {
            strikes.note(now);
        }
        assert_eq!(strikes.held(), LIMIT - 1);

        let later = now + WINDOW + Duration::from_millis(1);
        assert_eq!(strikes.note(later), Verdict::Noted);
        assert_eq!(strikes.held(), 1, "the old window carried over");
    }
}
