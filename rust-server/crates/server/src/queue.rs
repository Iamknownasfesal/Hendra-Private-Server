//! Who is waiting to get in, when there is no room.
//!
//! Follows `ConnectionQueue` and the check in `ConnectManager.Tick`: while the server is full, an
//! arriving connection waits rather than being refused, and one is let in each time somebody leaves.
//!
//! # Why a queue rather than a refusal
//!
//! A refusal makes everybody retry, and everybody retrying is a server that is hardest to get into
//! exactly when it is busiest. A queue turns that into a line: one connection each, in an order the
//! server chooses, and the people in it can be told where they are.
//!
//! # The order
//!
//! By rank, with anybody reconnecting put ahead of everybody. Somebody who was already playing a
//! minute ago and lost their connection is not a new arrival competing for a place; they are
//! somebody the server dropped, and putting them at the back of a queue it caused would be its own
//! kind of unfair. The original adds a hundred and one to their sort value, which is more than any
//! rank, so the effect is exactly that.
//!
//! Ties keep their arrival order, so equal ranks are first come first served.

use std::collections::VecDeque;

/// Somebody waiting for a place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub account_id: i64,

    /// What their rank buys them. Higher goes first.
    pub rank: i16,

    /// Whether they were playing until a moment ago.
    pub reconnecting: bool,

    /// Where they came in the arrival order, which breaks ties.
    arrived: u64,
}

/// The line.
#[derive(Debug, Default)]
pub struct Queue {
    waiting: VecDeque<Waiting>,

    /// Counts up forever, so two people who arrive in the same instant keep their order.
    next: u64,
}

/// What a rank is worth against a reconnection.
///
/// More than any rank, which is what makes somebody reconnecting go ahead of everybody regardless.
const RECONNECTING_IS_WORTH: i32 = 101;

impl Queue {
    pub fn new() -> Queue {
        Queue::default()
    }

    /// Puts somebody in the line, or moves them if they are already in it.
    ///
    /// Already in it matters: a client that retries while waiting should not take two places, and
    /// the second attempt is the one that knows whether they are reconnecting.
    ///
    /// Returns where they now stand, counting from one.
    pub fn join(&mut self, account_id: i64, rank: i16, reconnecting: bool) -> usize {
        self.waiting.retain(|held| held.account_id != account_id);

        let arrived = self.next;
        self.next += 1;

        self.waiting.push_back(Waiting {
            account_id,
            rank,
            reconnecting,
            arrived,
        });

        self.sort();
        self.place_of(account_id).unwrap_or(self.waiting.len())
    }

    /// Takes whoever is first, or nothing when nobody is waiting.
    pub fn next_in(&mut self) -> Option<Waiting> {
        self.waiting.pop_front()
    }

    /// Takes somebody out, for a connection that gave up before its turn.
    pub fn leave(&mut self, account_id: i64) {
        self.waiting.retain(|held| held.account_id != account_id);
    }

    /// Where somebody stands, counting from one.
    pub fn place_of(&self, account_id: i64) -> Option<usize> {
        self.waiting
            .iter()
            .position(|held| held.account_id == account_id)
            .map(|index| index + 1)
    }

    pub fn len(&self) -> usize {
        self.waiting.len()
    }

    /// Sorts the line: reconnections first, then rank, then arrival order.
    fn sort(&mut self) {
        self.waiting.make_contiguous().sort_by(|a, b| {
            let worth = |who: &Waiting| {
                who.rank as i32
                    + if who.reconnecting {
                        RECONNECTING_IS_WORTH
                    } else {
                        0
                    }
            };

            // Higher worth first, and then whoever has been waiting longer.
            //
            // The second half is belt and braces: `sort_by` is stable and the line is only ever
            // appended to, so equal ranks already keep their order. It is written out because a
            // queue whose fairness rests on a documented property of the sort is one that quietly
            // stops being fair the day somebody reaches for `sort_unstable_by`. No test can tell
            // the two apart, which is the reason to say it here instead.
            worth(b).cmp(&worth(a)).then(a.arrived.cmp(&b.arrived))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_queue_has_nobody_in_it() {
        let mut queue = Queue::new();
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.next_in(), None);
    }

    #[test]
    fn equal_ranks_are_first_come_first_served() {
        let mut queue = Queue::new();

        assert_eq!(queue.join(1, 0, false), 1);
        assert_eq!(queue.join(2, 0, false), 2);
        assert_eq!(queue.join(3, 0, false), 3);

        assert_eq!(queue.next_in().unwrap().account_id, 1);
        assert_eq!(queue.next_in().unwrap().account_id, 2);
    }

    #[test]
    fn a_higher_rank_goes_ahead() {
        let mut queue = Queue::new();

        queue.join(1, 0, false);
        queue.join(2, 50, false);

        assert_eq!(queue.next_in().unwrap().account_id, 2);
    }

    #[test]
    fn somebody_reconnecting_goes_ahead_of_every_rank() {
        // Somebody who was playing a minute ago is not a new arrival competing for a place. They
        // are somebody the server dropped, and the back of a queue it caused is its own unfairness.
        let mut queue = Queue::new();

        queue.join(1, 100, false);
        queue.join(2, 0, true);

        assert_eq!(queue.next_in().unwrap().account_id, 2);
    }

    #[test]
    fn two_reconnections_are_still_ordered_by_rank() {
        let mut queue = Queue::new();

        queue.join(1, 0, true);
        queue.join(2, 40, true);

        assert_eq!(queue.next_in().unwrap().account_id, 2);
    }

    #[test]
    fn retrying_while_waiting_does_not_take_a_second_place() {
        let mut queue = Queue::new();

        queue.join(1, 0, false);
        queue.join(2, 0, false);
        queue.join(1, 0, false);

        assert_eq!(queue.len(), 2, "one client took two places");
    }

    #[test]
    fn retrying_moves_somebody_behind_those_who_waited_through_it() {
        // The retry is a new arrival at the same rank, so it goes behind whoever was already there.
        // Not a punishment: it is what first come first served means when somebody leaves and
        // comes back.
        let mut queue = Queue::new();

        queue.join(1, 0, false);
        queue.join(2, 0, false);
        queue.join(1, 0, false);

        assert_eq!(queue.next_in().unwrap().account_id, 2);
    }

    #[test]
    fn a_retry_can_say_it_is_now_a_reconnection() {
        // The second attempt is the one that knows, which is why the whole entry is replaced rather
        // than the first one kept.
        let mut queue = Queue::new();

        queue.join(1, 0, false);
        queue.join(2, 0, false);
        queue.join(1, 0, true);

        assert_eq!(queue.next_in().unwrap().account_id, 1);
    }

    #[test]
    fn somebody_who_gives_up_leaves_the_line() {
        let mut queue = Queue::new();

        queue.join(1, 0, false);
        queue.join(2, 0, false);
        queue.leave(1);

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.next_in().unwrap().account_id, 2);
    }

    #[test]
    fn a_place_is_counted_from_one_because_nobody_is_zeroth_in_a_queue() {
        let mut queue = Queue::new();

        queue.join(1, 0, false);
        assert_eq!(queue.place_of(1), Some(1));
        assert_eq!(queue.place_of(99), None);
    }

    #[test]
    fn joining_says_where_you_now_stand_rather_than_where_you_arrived() {
        // Somebody important arriving pushes everybody else back, and the number they were told is
        // stale the moment it is said. What matters is that the number is right when it is said.
        let mut queue = Queue::new();

        queue.join(1, 0, false);
        queue.join(2, 0, false);

        assert_eq!(
            queue.join(3, 90, false),
            1,
            "a high rank joins at the front"
        );
        assert_eq!(queue.place_of(1), Some(2));
    }
}
