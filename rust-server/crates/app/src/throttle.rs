//! Limiting how fast passwords can be guessed, and how much work guessing costs the server.
//!
//! Two separate problems share this file because the same request causes both.
//!
//! The first is guessing. Argon2 makes one attempt expensive for the attacker, but nothing about
//! it makes a million attempts impossible — it makes them slow, and slow is a budget rather than a
//! wall. A limit per account name turns that budget into a hard ceiling.
//!
//! The second is that the expense lands on this server too. A few hundred concurrent registrations
//! will spend every core hashing, and the endpoint that does it needs no account at all. That is
//! bounded separately, because it is not about any one attacker's persistence.
//!
//! # Why per name and not per address
//!
//! There is no trustworthy address here. The server is meant to sit behind a proxy, so the socket
//! address is the proxy's, and the forwarded header is whatever the client wrote unless something
//! in front rewrites it. Keying on a value the attacker controls is a limiter that lifts itself.
//! A name is not attacker-controlled in the way that matters: guessing one account's password
//! means sending that account's name, whatever address it comes from.
//!
//! The cost is that someone can lock a name they do not own out of logging in. That is why the
//! window is short — minutes, not hours — and why the limit counts failures rather than attempts,
//! so a player typing their own password correctly is never locked out by someone else's guessing.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How many failures a name may accumulate before it is refused outright.
pub const FAILURES_ALLOWED: usize = 8;

/// How long failures are remembered.
///
/// Eight failures in five minutes is far above what anyone mistyping their own password produces,
/// and far below what guessing needs.
pub const WINDOW: Duration = Duration::from_secs(5 * 60);

/// How many passwords may be hashed at once.
///
/// Argon2 is meant to saturate a core, so this is what stops the login endpoint from being a way
/// to take the whole process down. Requests past it wait rather than fail: a player waiting an
/// extra moment during a rush is better than one refused.
pub const CONCURRENT_HASHES: usize = 4;

/// How many failures are recorded between sweeps of expired entries.
///
/// The map is filled by unauthenticated requests naming whatever they like, so something has to
/// shrink it. Doing it on a count rather than a timer keeps the whole limiter free of background
/// tasks, and bounds the overshoot: at most this many dead entries can accumulate before they go.
const SWEEP_EVERY: usize = 1024;

/// Failed attempts, keyed by the name they were made against.
pub struct Throttle {
    failures: Mutex<HashMap<String, Vec<Instant>>>,
    since_sweep: Mutex<usize>,
}

impl Default for Throttle {
    fn default() -> Self {
        Throttle::new()
    }
}

impl Throttle {
    pub fn new() -> Throttle {
        Throttle {
            failures: Mutex::new(HashMap::new()),
            since_sweep: Mutex::new(0),
        }
    }

    /// Whether this name has spent its attempts, and how long until it has one back.
    ///
    /// `now` is passed in so the window is testable without sleeping through it.
    pub fn locked_out(&self, name: &str, now: Instant) -> Option<Duration> {
        let mut failures = self.failures.lock().unwrap_or_else(|err| err.into_inner());
        let recent = failures.get_mut(&key(name))?;
        recent.retain(|at| now.duration_since(*at) < WINDOW);

        if recent.len() < FAILURES_ALLOWED {
            return None;
        }

        // The oldest failure still counted is the one whose expiry frees a slot.
        let oldest = recent.first().copied()?;
        Some(WINDOW.saturating_sub(now.duration_since(oldest)))
    }

    /// Records a failure.
    pub fn failed(&self, name: &str, now: Instant) {
        {
            let mut failures = self.failures.lock().unwrap_or_else(|err| err.into_inner());
            let recent = failures.entry(key(name)).or_default();
            recent.retain(|at| now.duration_since(*at) < WINDOW);
            recent.push(now);
        }

        let mut since = self
            .since_sweep
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        *since += 1;
        if *since >= SWEEP_EVERY {
            *since = 0;
            drop(since);
            self.forget_old(now);
        }
    }

    /// Forgets a name's failures, which a correct password does.
    pub fn succeeded(&self, name: &str) {
        self.failures
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&key(name));
    }

    /// Drops names whose failures have all expired.
    ///
    /// Without this the map only grows, and it is filled by unauthenticated requests naming
    /// whatever they like — which makes it a way to spend the server's memory.
    pub fn forget_old(&self, now: Instant) {
        let mut failures = self.failures.lock().unwrap_or_else(|err| err.into_inner());
        failures.retain(|_, recent| {
            recent.retain(|at| now.duration_since(*at) < WINDOW);
            !recent.is_empty()
        });
    }

    /// How many names are being tracked.
    pub fn tracked(&self) -> usize {
        self.failures
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .len()
    }
}

/// Names are matched the way the database matches them, so that varying the case is not a way to
/// get a fresh set of attempts.
fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_locked_out_after_enough_failures() {
        let throttle = Throttle::new();
        let start = Instant::now();

        for _ in 0..FAILURES_ALLOWED - 1 {
            throttle.failed("Fesal", start);
        }
        assert!(throttle.locked_out("Fesal", start).is_none());

        throttle.failed("Fesal", start);
        assert!(throttle.locked_out("Fesal", start).is_some());
    }

    #[test]
    fn the_lockout_lifts_when_the_window_passes() {
        let throttle = Throttle::new();
        let start = Instant::now();

        for _ in 0..FAILURES_ALLOWED {
            throttle.failed("Fesal", start);
        }

        let remaining = throttle.locked_out("Fesal", start).unwrap();
        assert!(remaining <= WINDOW && remaining > WINDOW - Duration::from_secs(1));

        assert!(throttle.locked_out("Fesal", start + WINDOW).is_none());
    }

    #[test]
    fn failures_expire_one_at_a_time_rather_than_all_at_once() {
        // A window that cleared wholesale would hand back the full allowance at a predictable
        // moment, which is a rhythm to guess in rather than a limit.
        let throttle = Throttle::new();
        let start = Instant::now();

        for second in 0..FAILURES_ALLOWED as u64 {
            throttle.failed("Fesal", start + Duration::from_secs(second));
        }

        let locked = start + Duration::from_secs(FAILURES_ALLOWED as u64);
        assert!(throttle.locked_out("Fesal", locked).is_some());

        // Once the first failure ages out there is room for exactly one more, and no more.
        let freed = start + WINDOW;
        assert!(throttle.locked_out("Fesal", freed).is_none());
        throttle.failed("Fesal", freed);
        assert!(throttle.locked_out("Fesal", freed).is_some());
    }

    #[test]
    fn a_correct_password_clears_the_count() {
        let throttle = Throttle::new();
        let start = Instant::now();

        for _ in 0..FAILURES_ALLOWED - 1 {
            throttle.failed("Fesal", start);
        }
        throttle.succeeded("Fesal");

        for _ in 0..FAILURES_ALLOWED - 1 {
            throttle.failed("Fesal", start);
        }
        assert!(
            throttle.locked_out("Fesal", start).is_none(),
            "the earlier failures should not still be counted"
        );
    }

    #[test]
    fn one_name_locking_out_does_not_lock_out_another() {
        let throttle = Throttle::new();
        let start = Instant::now();

        for _ in 0..FAILURES_ALLOWED * 2 {
            throttle.failed("Fesal", start);
        }

        assert!(throttle.locked_out("Fesal", start).is_some());
        assert!(throttle.locked_out("Someone", start).is_none());
    }

    #[test]
    fn case_is_not_a_way_to_get_a_fresh_allowance() {
        let throttle = Throttle::new();
        let start = Instant::now();

        for name in [
            "fesal", "FESAL", "FeSaL", " Fesal ", "fESAL", "FEsal", "feSAL", "FesaL",
        ] {
            throttle.failed(name, start);
        }

        assert_eq!(throttle.tracked(), 1);
        assert!(throttle.locked_out("Fesal", start).is_some());
    }

    #[test]
    fn names_that_stopped_failing_are_forgotten() {
        // The map is filled by unauthenticated requests naming whatever they like, so it has to
        // shrink on its own.
        let throttle = Throttle::new();
        let start = Instant::now();

        for i in 0..1_000 {
            throttle.failed(&format!("name{i}"), start);
        }
        // Deliberately under SWEEP_EVERY, so this test is about forget_old rather than about the
        // automatic sweep that the next one covers.
        assert_eq!(throttle.tracked(), 1_000);

        throttle.forget_old(start + Duration::from_secs(1));
        assert_eq!(throttle.tracked(), 1_000, "these are still recent");

        throttle.forget_old(start + WINDOW);
        assert_eq!(throttle.tracked(), 0);
    }

    #[test]
    fn the_map_sweeps_itself_without_anyone_asking() {
        // Nothing calls forget_old in the running server, so the limiter has to bound its own
        // memory or it becomes the thing it was added to prevent.
        let throttle = Throttle::new();
        let start = Instant::now();

        for i in 0..SWEEP_EVERY {
            throttle.failed(&format!("name{i}"), start);
        }
        assert_eq!(throttle.tracked(), SWEEP_EVERY, "all still recent");

        // Every one of those is now stale, and the next batch of failures clears them.
        let later = start + WINDOW;
        for i in 0..SWEEP_EVERY {
            throttle.failed(&format!("later{i}"), later);
        }

        assert_eq!(
            throttle.tracked(),
            SWEEP_EVERY,
            "the stale batch should be gone and only the new one left"
        );
    }

    #[test]
    fn an_unknown_name_is_tracked_the_same_as_a_real_one() {
        // Otherwise the limiter answers a question the login endpoint deliberately does not:
        // whether the account exists.
        let throttle = Throttle::new();
        let start = Instant::now();

        for _ in 0..FAILURES_ALLOWED {
            throttle.failed("NobodyHasThisName", start);
        }
        assert!(throttle.locked_out("NobodyHasThisName", start).is_some());
    }
}
