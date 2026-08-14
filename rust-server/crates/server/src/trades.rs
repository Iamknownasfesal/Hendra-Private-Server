//! Who is trading with whom, and what each side is offering.
//!
//! Follows `wServer/realm/entities/player/Player.Trade.cs` and its handlers.
//!
//! # How a trade begins
//!
//! By both sides asking. There is no separate accept: asking somebody who has already asked you is
//! the agreement, and a request that is not answered expires after twenty seconds. That is what the
//! original does, and it means one message rather than two does the work.
//!
//! # Why this is here and not in the world
//!
//! A trade is two connections talking to each other, and the world knows about bodies rather than
//! connections. So this holds each player's sender and writes to the other side directly, the same
//! way the world writes snapshots. Who is trading with whom is kept apart from how to reach them,
//! which is what lets the agreement be checked without a connection to check it over.
//!
//! # What this does not decide
//!
//! Whether the items actually move. That is [`hendra_store::Store::trade`], which locks both
//! inventories, checks every offered item is still there and commits the whole exchange or none of
//! it. This layer only agrees what the exchange is; a trade agreed here can still be refused there,
//! and the refusal is the correct answer rather than a failure.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hendra_net::{Delivery, ServerMessage, Writer};
use hendra_transport::LinkSender;

/// How long an unanswered request stands.
pub const REQUEST_EXPIRES: Duration = Duration::from_secs(20);

/// How many inventory slots a trade covers: the four worn and the eight carried.
pub const TRADE_SLOTS: usize = 12;

/// The first slot that may be offered. The four before it are worn.
pub const FIRST_TRADEABLE: usize = 4;

/// Somebody who could be traded with.
struct Party {
    character_id: i64,

    /// Which world they are in, for `/visit` and for anybody looking for them.
    world: String,
}

/// One side of a trade in progress.
#[derive(Debug, Clone, Default)]
struct Side {
    offer: Vec<bool>,
    accepted: bool,
}

/// A trade in progress. Both sides are held, each naming the other, so either can be looked up by
/// the name of whoever sent the message.
struct Active {
    partner: String,
    side: Side,
}

#[derive(Default)]
struct State {
    /// Everybody who is online, by name.
    present: HashMap<String, Party>,

    /// How to reach each of them.
    ///
    /// Kept apart from who is present so that agreeing a trade needs only the names and the state,
    /// and can be checked without a connection to check it over.
    senders: HashMap<String, LinkSender>,

    /// Who has asked whom, and when. Keyed by the player who was asked.
    asked: HashMap<String, Vec<(String, Instant)>>,

    /// Who is trading, by name. Both sides appear, each naming the other.
    active: HashMap<String, Active>,
}

/// What the caller should do about a trade message.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Nothing more to do. The message has already been sent to whoever needed it.
    Done,

    /// Say this to the player who asked.
    Say(String),

    /// Both sides have agreed. Move the items.
    Settle {
        /// The other player's character.
        partner_character: i64,
        partner_name: String,

        /// Which slots each side is giving up.
        mine: Vec<i16>,
        theirs: Vec<i16>,
    },
}

/// Who is trading with whom.
#[derive(Default)]
pub struct Trades {
    inner: Mutex<State>,
}

impl Trades {
    pub fn new() -> Trades {
        Trades::default()
    }

    /// Notes that somebody is online and could be traded with.
    pub fn arrived(&self, name: &str, character_id: i64, sender: LinkSender) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };

        state.present.insert(
            name.to_string(),
            Party {
                character_id,
                world: String::new(),
            },
        );
        state.senders.insert(name.to_string(), sender);
    }

    /// Notes that somebody has gone, cancelling whatever they were doing.
    ///
    /// A trade left half-agreed by a disconnection is the shape a duplication is built on, so it
    /// ends here rather than waiting to be noticed.
    pub fn left(&self, name: &str) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };

        state.present.remove(name);
        state.senders.remove(name);
        state.asked.remove(name);
        for asked in state.asked.values_mut() {
            asked.retain(|(who, _)| who != name);
        }

        if let Some(active) = state.active.remove(name) {
            state.active.remove(&active.partner);
            tell(
                &state,
                &active.partner,
                &ServerMessage::TradeDone {
                    code: 1,
                    message: format!("{name} left."),
                },
            );
        }
    }

    /// Asks somebody to trade, or accepts their standing request.
    pub fn request(&self, from: &str, to: &str) -> Step {
        let Ok(mut state) = self.inner.lock() else {
            return Step::Say("trading is unavailable".to_string());
        };

        if from == to {
            return Step::Say("You cannot trade with yourself.".to_string());
        }
        if !state.present.contains_key(to) {
            return Step::Say(format!("{to} is not here."));
        }
        if state.active.contains_key(from) {
            return Step::Say("You are already trading.".to_string());
        }
        if state.active.contains_key(to) {
            return Step::Say(format!("{to} is already trading."));
        }

        // Their request stands only if it has not expired. Expiry is checked when it is looked at
        // rather than on a timer, because nothing else needs to know.
        let theirs = state.asked.entry(from.to_string()).or_default();
        theirs.retain(|(_, when)| when.elapsed() < REQUEST_EXPIRES);
        let mutual = theirs.iter().any(|(who, _)| who == to);

        if !mutual {
            let mine = state.asked.entry(to.to_string()).or_default();
            mine.retain(|(_, when)| when.elapsed() < REQUEST_EXPIRES);
            if !mine.iter().any(|(who, _)| who == from) {
                mine.push((from.to_string(), Instant::now()));
            }

            tell(
                &state,
                to,
                &ServerMessage::TradeRequested {
                    name: from.to_string(),
                },
            );
            return Step::Say(format!("You have sent a trade request to {to}."));
        }

        // Both have asked, so the trade begins. Every standing request from either of them is
        // dropped: they are busy now, and a request answered later would start a second trade.
        state.asked.remove(from);
        state.asked.remove(to);

        state.active.insert(
            from.to_string(),
            Active {
                partner: to.to_string(),
                side: Side {
                    offer: vec![false; TRADE_SLOTS],
                    accepted: false,
                },
            },
        );
        state.active.insert(
            to.to_string(),
            Active {
                partner: from.to_string(),
                side: Side {
                    offer: vec![false; TRADE_SLOTS],
                    accepted: false,
                },
            },
        );

        Step::Done
    }

    /// Notes which world somebody has moved to.
    pub fn moved(&self, name: &str, world: &str) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };

        if let Some(party) = state.present.get_mut(name) {
            party.world = world.to_string();
        }
    }

    /// Which world somebody is in.
    pub fn world_of(&self, name: &str) -> Option<String> {
        let state = self.inner.lock().ok()?;
        state
            .present
            .iter()
            .find(|(held, _)| held.eq_ignore_ascii_case(name))
            .map(|(_, party)| party.world.clone())
            .filter(|world| !world.is_empty())
    }

    /// How many players are in each world.
    ///
    /// From the roster because this is what already knows where everybody is; the alternative is
    /// asking every world in turn, which is a message per world per refresh for a number the
    /// roster is holding anyway.
    pub fn counts(&self) -> std::collections::HashMap<String, usize> {
        let Ok(state) = self.inner.lock() else {
            return std::collections::HashMap::new();
        };

        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for party in state.present.values() {
            if party.world.is_empty() {
                continue;
            }
            *counts.entry(party.world.clone()).or_default() += 1;
        }

        counts
    }

    /// Everybody online, by name.
    ///
    /// The roster is here because this is what already knows who is connected and how to reach
    /// them; `/who` and `/online` need exactly that and nothing else.
    pub fn present(&self) -> Vec<String> {
        let Ok(state) = self.inner.lock() else {
            return Vec::new();
        };

        let mut names: Vec<String> = state.present.keys().cloned().collect();
        names.sort();
        names
    }

    /// Ends somebody's connection.
    ///
    /// Closing the sender is what a kick is: the session sees its link go and shuts down the same
    /// way it does when somebody quits, so a kick and a disconnection leave the same state behind.
    pub fn kick(&self, name: &str) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };

        if let Some(sender) = state.senders.remove(name) {
            sender.close("kicked");
        }
    }

    /// Who somebody is trading with, if anybody.
    pub fn partner(&self, name: &str) -> Option<String> {
        let state = self.inner.lock().ok()?;
        state.active.get(name).map(|active| active.partner.clone())
    }

    /// Changes what somebody is offering.
    ///
    /// Any change unagrees both sides. Otherwise one player could accept, wait for the other to
    /// accept, and then quietly take an item back out.
    pub fn change(&self, name: &str, offer: &[bool]) -> Step {
        let Ok(mut state) = self.inner.lock() else {
            return Step::Done;
        };

        let Some(partner) = state.active.get(name).map(|active| active.partner.clone()) else {
            return Step::Say("You are not trading.".to_string());
        };

        let offer = tidy(offer);

        if let Some(active) = state.active.get_mut(name) {
            active.side.offer = offer.clone();
            active.side.accepted = false;
        }
        if let Some(active) = state.active.get_mut(&partner) {
            active.side.accepted = false;
        }

        tell(&state, &partner, &ServerMessage::TradeChanged { offer });
        Step::Done
    }

    /// Agrees to a trade.
    ///
    /// `theirs` is what the accepting player believes the other side is offering. An accept that
    /// names a stale offer is ignored: a player agrees to what they were looking at, and what they
    /// were looking at is no longer what is on the table.
    pub fn accept(&self, name: &str, mine: &[bool], theirs: &[bool]) -> Step {
        let Ok(mut state) = self.inner.lock() else {
            return Step::Done;
        };

        let Some(partner) = state.active.get(name).map(|active| active.partner.clone()) else {
            return Step::Say("You are not trading.".to_string());
        };

        let (mine, theirs) = (tidy(mine), tidy(theirs));

        let on_the_table = state
            .active
            .get(&partner)
            .map(|active| active.side.offer.clone())
            .unwrap_or_default();

        if on_the_table != theirs {
            return Step::Say("Their offer changed.".to_string());
        }

        if let Some(active) = state.active.get_mut(name) {
            active.side.offer = mine.clone();
            active.side.accepted = true;
        }

        tell(
            &state,
            &partner,
            &ServerMessage::TradeAccepted {
                mine: theirs.clone(),
                theirs: mine.clone(),
            },
        );

        let both = state
            .active
            .get(&partner)
            .is_some_and(|active| active.side.accepted);
        if !both {
            return Step::Done;
        }

        let Some(partner_character) = state.present.get(&partner).map(|party| party.character_id)
        else {
            return Step::Say("They are no longer here.".to_string());
        };

        // Cleared before the items move. The exchange is the store's to make or refuse, and leaving
        // an agreed trade standing while it runs is how the same offer gets settled twice.
        state.active.remove(name);
        state.active.remove(&partner);

        Step::Settle {
            partner_character,
            partner_name: partner,
            mine: offered_slots(&mine),
            theirs: offered_slots(&on_the_table),
        }
    }

    /// Ends a trade, telling both sides.
    pub fn cancel(&self, name: &str, why: &str) -> Step {
        let Ok(mut state) = self.inner.lock() else {
            return Step::Done;
        };

        let Some(active) = state.active.remove(name) else {
            return Step::Done;
        };
        state.active.remove(&active.partner);

        let done = ServerMessage::TradeDone {
            code: 1,
            message: why.to_string(),
        };
        tell(&state, &active.partner, &done);
        tell(&state, name, &done);

        Step::Done
    }

    /// Tells both sides how a trade ended.
    pub fn finished(&self, name: &str, partner: &str, code: u32, message: &str) {
        let Ok(state) = self.inner.lock() else {
            return;
        };

        let done = ServerMessage::TradeDone {
            code,
            message: message.to_string(),
        };
        tell(&state, name, &done);
        tell(&state, partner, &done);
    }

    /// Sends one message to a named player.
    pub fn send(&self, name: &str, message: &ServerMessage<'_>) {
        let Ok(state) = self.inner.lock() else {
            return;
        };
        tell(&state, name, message);
    }
}

/// An offer at exactly the length a trade covers, with the worn slots forced off.
///
/// A client that sends a longer offer, a shorter one, or one claiming a worn slot gets the same
/// answer as one that behaves: what it asked for, cut down to what is allowed.
fn tidy(offer: &[bool]) -> Vec<bool> {
    let mut tidied = vec![false; TRADE_SLOTS];

    for (slot, included) in offer.iter().enumerate().take(TRADE_SLOTS) {
        tidied[slot] = *included && slot >= FIRST_TRADEABLE;
    }

    tidied
}

/// Which slots an offer names.
fn offered_slots(offer: &[bool]) -> Vec<i16> {
    offer
        .iter()
        .enumerate()
        .filter(|(_, included)| **included)
        .map(|(slot, _)| slot as i16)
        .collect()
}

/// Sends a message to a named player, if they are still here.
fn tell(state: &State, name: &str, message: &ServerMessage<'_>) {
    let Some(sender) = state.senders.get(name) else {
        return;
    };

    let mut buffer = Vec::new();
    message.encode(&mut Writer::new(&mut buffer));
    let _ = sender.try_send(Delivery::Stream, &buffer);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trades() -> Trades {
        Trades::new()
    }

    /// Present without a connection. What reaches the other side is the transport's business; what
    /// is being checked here is who may trade with whom and what they agreed to.
    fn arrive(trades: &Trades, name: &str, character_id: i64) {
        let Ok(mut state) = trades.inner.lock() else {
            return;
        };
        state.present.insert(
            name.to_string(),
            Party {
                character_id,
                world: String::new(),
            },
        );
    }

    fn offering(slots: &[usize]) -> Vec<bool> {
        let mut offer = vec![false; TRADE_SLOTS];
        for slot in slots {
            offer[*slot] = true;
        }
        offer
    }

    #[test]
    fn a_trade_begins_when_both_have_asked() {
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);

        assert!(matches!(trades.request("Ana", "Bo"), Step::Say(_)));
        assert_eq!(trades.partner("Ana"), None, "one ask is not a trade");

        assert_eq!(trades.request("Bo", "Ana"), Step::Done);
        assert_eq!(trades.partner("Ana").as_deref(), Some("Bo"));
        assert_eq!(trades.partner("Bo").as_deref(), Some("Ana"));
    }

    #[test]
    fn a_request_that_is_not_answered_expires() {
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);

        trades.request("Ana", "Bo");

        // Aged past its life rather than waited out.
        if let Ok(mut state) = trades.inner.lock()
            && let Some(asked) = state.asked.get_mut("Bo")
        {
            for (_, when) in asked.iter_mut() {
                *when = Instant::now() - REQUEST_EXPIRES - Duration::from_secs(1);
            }
        }

        assert!(matches!(trades.request("Bo", "Ana"), Step::Say(_)));
        assert_eq!(trades.partner("Ana"), None, "a stale ask started a trade");
    }

    #[test]
    fn nobody_trades_with_two_people_at_once() {
        let trades = trades();
        for (name, id) in [("Ana", 1), ("Bo", 2), ("Cy", 3)] {
            arrive(&trades, name, id);
        }

        trades.request("Ana", "Bo");
        trades.request("Bo", "Ana");

        assert!(
            matches!(trades.request("Cy", "Ana"), Step::Say(said) if said.contains("already")),
            "a third player joined a trade"
        );
    }

    #[test]
    fn a_worn_slot_cannot_be_offered() {
        // The four worn slots are not somewhere a trade may take from: what is worn is what keeps
        // a character alive, and the original's loop starts at four for the same reason.
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);
        trades.request("Ana", "Bo");
        trades.request("Bo", "Ana");

        trades.change("Ana", &offering(&[0, 1, 2, 3, 5]));

        let held = trades
            .inner
            .lock()
            .unwrap()
            .active
            .get("Ana")
            .unwrap()
            .side
            .offer
            .clone();
        assert_eq!(held, offering(&[5]));
    }

    #[test]
    fn changing_an_offer_unagrees_both_sides() {
        // Otherwise one player accepts, waits for the other, and quietly takes an item back out.
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);
        trades.request("Ana", "Bo");
        trades.request("Bo", "Ana");

        trades.change("Ana", &offering(&[5]));
        trades.accept("Bo", &offering(&[6]), &offering(&[5]));

        assert!(
            trades
                .inner
                .lock()
                .unwrap()
                .active
                .get("Bo")
                .unwrap()
                .side
                .accepted,
            "Bo agreed"
        );

        trades.change("Ana", &offering(&[7]));
        assert!(
            !trades
                .inner
                .lock()
                .unwrap()
                .active
                .get("Bo")
                .unwrap()
                .side
                .accepted,
            "and should have been asked again"
        );
    }

    #[test]
    fn accepting_an_offer_that_has_changed_does_nothing() {
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);
        trades.request("Ana", "Bo");
        trades.request("Bo", "Ana");

        trades.change("Ana", &offering(&[5]));

        // Bo agrees to something Ana never offered.
        let step = trades.accept("Bo", &offering(&[6]), &offering(&[5, 7]));

        assert!(matches!(step, Step::Say(_)));
        assert!(
            !trades
                .inner
                .lock()
                .unwrap()
                .active
                .get("Bo")
                .unwrap()
                .side
                .accepted
        );
    }

    #[test]
    fn both_accepting_the_same_offer_settles_it() {
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);
        trades.request("Ana", "Bo");
        trades.request("Bo", "Ana");

        trades.change("Ana", &offering(&[5, 6]));
        trades.change("Bo", &offering(&[7]));

        assert_eq!(
            trades.accept("Ana", &offering(&[5, 6]), &offering(&[7])),
            Step::Done,
            "one side is not enough"
        );

        let step = trades.accept("Bo", &offering(&[7]), &offering(&[5, 6]));

        assert_eq!(
            step,
            Step::Settle {
                partner_character: 1,
                partner_name: "Ana".to_string(),
                mine: vec![7],
                theirs: vec![5, 6],
            }
        );
    }

    #[test]
    fn a_settled_trade_is_no_longer_a_trade() {
        // Leaving it standing while the items move is how the same offer gets settled twice.
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);
        trades.request("Ana", "Bo");
        trades.request("Bo", "Ana");

        trades.change("Ana", &offering(&[5]));
        trades.accept("Ana", &offering(&[5]), &offering(&[]));
        trades.accept("Bo", &offering(&[]), &offering(&[5]));

        assert_eq!(trades.partner("Ana"), None);
        assert_eq!(trades.partner("Bo"), None);
    }

    #[test]
    fn leaving_ends_the_trade_rather_than_leaving_it_half_agreed() {
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);
        trades.request("Ana", "Bo");
        trades.request("Bo", "Ana");

        trades.change("Ana", &offering(&[5]));
        trades.accept("Ana", &offering(&[5]), &offering(&[]));

        trades.left("Ana");

        assert_eq!(trades.partner("Bo"), None);
    }

    #[test]
    fn cancelling_ends_it_for_both() {
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);
        trades.request("Ana", "Bo");
        trades.request("Bo", "Ana");

        trades.cancel("Ana", "Trade cancelled.");

        assert_eq!(trades.partner("Ana"), None);
        assert_eq!(trades.partner("Bo"), None);
    }

    #[test]
    fn nobody_trades_with_themselves_or_with_somebody_who_is_not_here() {
        let trades = trades();
        arrive(&trades, "Ana", 1);

        assert!(matches!(trades.request("Ana", "Ana"), Step::Say(_)));
        assert!(matches!(trades.request("Ana", "Nobody"), Step::Say(_)));
        assert_eq!(trades.partner("Ana"), None);
    }
}
