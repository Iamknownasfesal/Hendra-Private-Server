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

    /// Which account they belong to.
    ///
    /// One account plays one character at a time. Two sessions on one account is two copies of the
    /// same vault and the same gold, and every durable rule this server has assumes one writer.
    account_id: i64,

    /// The account's rank, which decides whether it may trade at all.
    ///
    /// Held here rather than read when it is needed, because both sides' ranks are wanted at once
    /// and one of them belongs to a session this one cannot ask.
    rank: i16,

    /// Which room they are in, for `/visit` and for anybody looking for them.
    ///
    /// The registry's instance key rather than the world's name, because there can be ten Undead
    /// Lairs running and a name names all of them at once. What that costs when it is a name: the
    /// nexus labels a realm portal with a crowd made of every copy of that world added together,
    /// and two players in two different Undead Lairs go on trading with each other because leaving
    /// one for the other looks like standing still. Where a person would want the world's name,
    /// [`crate::worlds::world_of_key`] reads it back off this.
    world: String,
}

/// The rank at and above which an account may not trade, from `Player.Trade.cs:37` and `:52`.
const NO_TRADING_FROM: i16 = 40;

/// The rank at and above which an account counts as an administrator.
///
/// `RankedCommands.cs:1369` sets the account's `Admin` flag to `rank >= 80`, and
/// `AcceptTradeHandler.cs:42` compares that flag across the two sides.
const ADMINISTRATOR: i16 = 80;

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

    /// How to ask each of them to re-read what they hold.
    refreshes: HashMap<String, tokio::sync::mpsc::Sender<()>>,

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
    pub fn arrived(
        &self,
        name: &str,
        account_id: i64,
        character_id: i64,
        rank: i16,
        sender: LinkSender,
        refresh: tokio::sync::mpsc::Sender<()>,
    ) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };

        state.present.insert(
            name.to_string(),
            Party {
                character_id,
                account_id,
                rank,
                world: String::new(),
            },
        );
        state.senders.insert(name.to_string(), sender);
        state.refreshes.insert(name.to_string(), refresh);
    }

    /// Claims an account for a new session, ending whatever else was playing on it.
    ///
    /// One account plays one character at a time. The original takes a lock and disconnects the
    /// other client; this does the same, and for the same reason: two sessions on one account are
    /// two writers to one vault, and every durable rule here is written for one.
    ///
    /// The old session is ended rather than the new one refused, because the common case is not
    /// somebody cheating but somebody whose connection dropped and who is trying to get back in. A
    /// server that refused them would hold them out until a timeout they cannot see.
    ///
    /// Returns how many were ended.
    pub fn claim(&self, account_id: i64) -> usize {
        let Ok(mut state) = self.inner.lock() else {
            return 0;
        };

        let others: Vec<String> = state
            .present
            .iter()
            .filter(|(_, party)| party.account_id == account_id)
            .map(|(name, _)| name.clone())
            .collect();

        for name in &others {
            // Closing the sender is what ends a session: the one that owns the link sees it go and
            // shuts down the same way it does when somebody quits, so a claim and a disconnection
            // leave the same state behind.
            if let Some(sender) = state.senders.remove(name) {
                sender.close("that account is playing somewhere else");
            }
            state.present.remove(name);
            state.refreshes.remove(name);

            if let Some(active) = state.active.remove(name) {
                state.active.remove(&active.partner);
            }
        }

        others.len()
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
        state.refreshes.remove(name);
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
    ///
    /// The target is named the way a player types it, so it is matched without regard to case and
    /// answered under the name the server holds. `World.GetUniqueNamedPlayer` (`World.cs:432-447`)
    /// does the same, and looks only at the players in the asking player's own world -- so somebody
    /// standing in another world is not found, however exactly their name is spelled.
    pub fn request(&self, from: &str, to: &str) -> Step {
        let Ok(mut state) = self.inner.lock() else {
            return Step::Say("trading is unavailable".to_string());
        };

        if from.eq_ignore_ascii_case(to) {
            return Step::Say("You can't trade with yourself!".to_string());
        }

        // Rank first, as `Player.Trade.cs:37-41` has it: an account trusted to conjure items is not
        // allowed to hand them out through a trade, in either direction.
        if state
            .present
            .get(from)
            .is_some_and(|party| party.rank >= NO_TRADING_FROM)
        {
            return Step::Say(
                "Your rank is too high to give/trade items to this person!".to_string(),
            );
        }

        let here = state
            .present
            .get(from)
            .map(|party| party.world.clone())
            .unwrap_or_default();

        // Only the players in this world, under the name the server holds for them.
        let Some(found) = state
            .present
            .iter()
            .find(|(held, party)| held.eq_ignore_ascii_case(to) && party.world == here)
            .map(|(held, _)| held.clone())
        else {
            return Step::Say(format!("{to} not found!"));
        };
        let to = found.as_str();

        if state
            .present
            .get(to)
            .is_some_and(|party| party.rank >= NO_TRADING_FROM)
        {
            return Step::Say("Your rank is too low to give/trade items to this person.".to_string());
        }

        if state.active.contains_key(from) {
            return Step::Say("Already trading!".to_string());
        }
        if state.active.contains_key(to) {
            return Step::Say(format!("{to} is already trading!"));
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
            return Step::Say(format!("You have sent a trade request to {to}!"));
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

    /// Notes which room somebody has moved to, ending any trade they were in.
    ///
    /// A trade is between two people standing in the same room, and walking out of it ends the
    /// trade rather than carrying it along: `World.LeaveWorld` (`World.cs:373-375`) cancels the
    /// leaving player's trade before anything else it does. Without this a player could agree an
    /// exchange, step through a portal, and settle it from another world against a partner who can
    /// no longer see them.
    ///
    /// `room` is the registry's instance key. A name would make two people in two copies of one
    /// dungeon look like two people who had not moved, which is exactly the case this guards.
    pub fn moved(&self, name: &str, room: &str) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };

        let elsewhere = state
            .present
            .get(name)
            .is_some_and(|party| party.world != room);

        if let Some(party) = state.present.get_mut(name) {
            party.world = room.to_string();
        }

        if !elsewhere {
            return;
        }

        let Some(active) = state.active.remove(name) else {
            return;
        };
        state.active.remove(&active.partner);

        let done = ServerMessage::TradeDone {
            code: 1,
            message: "Trade canceled!".to_string(),
        };
        tell(&state, name, &done);
        tell(&state, &active.partner, &done);
    }

    /// Which character somebody is playing.
    ///
    /// A name belongs to an account and every character on that account answers to it, so the name
    /// alone cannot pick one out of the database. The roster can: one account plays one character
    /// at a time, and this is the one it is playing.
    pub fn character_of(&self, name: &str) -> Option<i64> {
        let state = self.inner.lock().ok()?;
        state
            .present
            .iter()
            .find(|(held, _)| held.eq_ignore_ascii_case(name))
            .map(|(_, party)| party.character_id)
    }

    /// What the world somebody is in is called.
    ///
    /// The name rather than the key, because this answers a person: "they are in the Undead Lair"
    /// is what was asked, not which of the four Undead Lairs.
    pub fn world_of(&self, name: &str) -> Option<String> {
        self.room_of(name)
            .map(|key| crate::worlds::world_of_key(&key).to_string())
    }

    /// Which room somebody is in, as the registry keys it.
    pub fn room_of(&self, name: &str) -> Option<String> {
        let state = self.inner.lock().ok()?;
        state
            .present
            .iter()
            .find(|(held, _)| held.eq_ignore_ascii_case(name))
            .map(|(_, party)| party.world.clone())
            .filter(|world| !world.is_empty())
    }

    /// How many players are standing in one room.
    ///
    /// Asked by whoever is about to open a door into it and needs to know whether it is empty,
    /// which is the question `GuildHall.GetInstance` asks with `world.Players.Count > 0`
    /// (`GuildHall.cs:58`).
    pub fn in_world(&self, key: &str) -> usize {
        let Ok(state) = self.inner.lock() else {
            return 0;
        };

        state
            .present
            .values()
            .filter(|party| party.world == key)
            .count()
    }

    /// How many players are in each room.
    ///
    /// From the roster because this is what already knows where everybody is; the alternative is
    /// asking every world in turn, which is a message per world per refresh for a number the
    /// roster is holding anyway.
    ///
    /// Keyed by the instance key, so ten parties in ten copies of one dungeon are ten crowds rather
    /// than one.
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

        // An administrator and an ordinary player do not exchange items with each other.
        // `AcceptTradeHandler.cs:42-47` compares the two accounts' `Admin` flags the moment both
        // have agreed and cancels the trade outright when they differ, rather than refusing it
        // earlier -- so it is checked here, on the agreement, and not on the request.
        let administrator = |who: &str| {
            state
                .present
                .get(who)
                .is_some_and(|party| party.rank >= ADMINISTRATOR)
        };

        if administrator(name) != administrator(&partner) {
            state.active.remove(name);
            state.active.remove(&partner);

            let done = ServerMessage::TradeDone {
                code: 1,
                message: "Trade canceled!".to_string(),
            };
            tell(&state, name, &done);
            tell(&state, &partner, &done);
            return Step::Done;
        }

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

    /// Asks a named player's session to re-read what it holds.
    ///
    /// Somebody else's doing: a trade that completed, an item sold on the market, a gift from an
    /// administrator. The session that made the change refreshes itself; this is for the one that
    /// did not and would otherwise be looking at a vault, a pack or a purse that is no longer what
    /// the database says.
    ///
    /// Asked rather than sent, because only a session can read its own account and only it knows
    /// what it has already shown.
    pub fn refresh(&self, name: &str) {
        let Ok(state) = self.inner.lock() else {
            return;
        };

        if let Some(asking) = state.refreshes.get(name) {
            // A full queue means one is already waiting, and one is enough: what it will read is
            // whatever is true when it reads it, not whatever was true when this was asked.
            let _ = asking.try_send(());
        }
    }

    /// The name a connected player is registered under, however it was typed.
    ///
    /// A player has one spelling and clients have several: `/invite bob` and `/invite Bob` name the
    /// same person, and everything keyed by name here is keyed by that one spelling.
    pub fn named(&self, name: &str) -> Option<String> {
        let state = self.inner.lock().ok()?;
        state
            .present
            .keys()
            .find(|held| held.eq_ignore_ascii_case(name))
            .cloned()
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
        arrive_on(trades, name, character_id, character_id);
    }

    /// Present on a named account, for the tests that care which.
    fn arrive_on(trades: &Trades, name: &str, account_id: i64, character_id: i64) {
        arrive_ranked(trades, name, account_id, character_id, 0);
    }

    /// Present at a named rank, for the tests that care what it is.
    fn arrive_ranked(trades: &Trades, name: &str, account_id: i64, character_id: i64, rank: i16) {
        let Ok(mut state) = trades.inner.lock() else {
            return;
        };
        state.present.insert(
            name.to_string(),
            Party {
                character_id,
                account_id,
                rank,
                world: "Nexus".to_string(),
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
    fn a_refresh_reaches_the_session_that_did_not_make_the_change() {
        let trades = trades();
        let (to_them, mut asked) = tokio::sync::mpsc::channel(1);

        {
            let Ok(mut state) = trades.inner.lock() else {
                panic!("the lock");
            };
            state.refreshes.insert("Bo".to_string(), to_them);
        }

        trades.refresh("Bo");
        assert!(asked.try_recv().is_ok(), "Bo was never asked");
    }

    #[test]
    fn a_second_refresh_waiting_behind_the_first_is_dropped() {
        // What a refresh reads is whatever is true when it reads it, so two waiting would read the
        // same thing twice and the second would say nothing new.
        let trades = trades();
        let (to_them, mut asked) = tokio::sync::mpsc::channel(1);

        {
            let Ok(mut state) = trades.inner.lock() else {
                panic!("the lock");
            };
            state.refreshes.insert("Bo".to_string(), to_them);
        }

        for _ in 0..10 {
            trades.refresh("Bo");
        }

        assert!(asked.try_recv().is_ok());
        assert!(asked.try_recv().is_err(), "ten asks queued up");
    }

    #[test]
    fn refreshing_somebody_who_is_not_here_does_nothing() {
        let trades = trades();
        trades.refresh("Nobody");
    }

    #[test]
    fn one_account_plays_one_character_at_a_time() {
        // Two sessions on one account are two writers to one vault, and every durable rule here is
        // written for one.
        let trades = trades();
        arrive_on(&trades, "First", 7, 1);
        arrive_on(&trades, "Second", 7, 2);
        arrive_on(&trades, "Somebody Else", 8, 3);

        let ended = trades.claim(7);

        assert_eq!(ended, 2, "both sessions on that account should end");
        assert!(trades.present().contains(&"Somebody Else".to_string()));
        assert!(!trades.present().contains(&"First".to_string()));
    }

    #[test]
    fn claiming_an_account_nobody_is_playing_ends_nothing() {
        let trades = trades();
        arrive_on(&trades, "Somebody", 8, 1);

        assert_eq!(trades.claim(7), 0);
        assert_eq!(trades.present().len(), 1);
    }

    #[test]
    fn taking_over_an_account_ends_the_trade_it_was_in() {
        // A trade left half-agreed by a takeover is the same shape a duplication is built on as one
        // left by a disconnection.
        let trades = trades();
        arrive_on(&trades, "Ana", 7, 1);
        arrive_on(&trades, "Bo", 8, 2);

        trades.request("Ana", "Bo");
        trades.request("Bo", "Ana");
        assert_eq!(trades.partner("Bo").as_deref(), Some("Ana"));

        trades.claim(7);

        assert_eq!(
            trades.partner("Bo"),
            None,
            "Bo is still trading with a ghost"
        );
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
    fn nobody_is_found_in_another_world() {
        // A trade is between two people standing in the same room. `GetUniqueNamedPlayer` looks at
        // the asking player's own world and nowhere else.
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);
        trades.moved("Bo", "Vault");

        assert!(
            matches!(trades.request("Ana", "Bo"), Step::Say(said) if said.contains("not found")),
            "Bo was reachable from another world"
        );
    }

    #[test]
    fn a_name_is_matched_whatever_its_capitals() {
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);

        trades.request("Ana", "bO");
        assert_eq!(trades.request("Bo", "ANA"), Step::Done);
        assert_eq!(trades.partner("Ana").as_deref(), Some("Bo"));
    }

    #[test]
    fn walking_out_of_the_world_ends_the_trade() {
        // Otherwise an agreed exchange settles from another world against a partner who can no
        // longer see the player they agreed with.
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);
        trades.request("Ana", "Bo");
        trades.request("Bo", "Ana");

        trades.moved("Ana", "Vault");

        assert_eq!(trades.partner("Ana"), None);
        assert_eq!(trades.partner("Bo"), None, "Bo is trading with a ghost");
    }

    #[test]
    fn staying_where_you_are_does_not_end_the_trade() {
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);
        trades.request("Ana", "Bo");
        trades.request("Bo", "Ana");

        trades.moved("Ana", "Nexus");

        assert_eq!(trades.partner("Ana").as_deref(), Some("Bo"));
    }

    #[test]
    fn a_ranked_account_neither_gives_nor_receives() {
        let trades = trades();
        arrive_ranked(&trades, "Ana", 1, 1, 0);
        arrive_ranked(&trades, "Staff", 2, 2, NO_TRADING_FROM);

        assert!(
            matches!(trades.request("Staff", "Ana"), Step::Say(said) if said.contains("too high")),
            "a ranked account started a trade"
        );
        assert!(
            matches!(trades.request("Ana", "Staff"), Step::Say(said) if said.contains("too low")),
            "a ranked account was asked to trade"
        );
        assert_eq!(trades.partner("Ana"), None);
    }

    #[test]
    fn an_administrator_and_a_player_do_not_settle() {
        // The rank check on the request is what usually stops this, so the pair here are both under
        // it on one side of the comparison and either side of the other -- which is the state the
        // original guards against at the moment both have agreed.
        let trades = trades();
        arrive_ranked(&trades, "Ana", 1, 1, 0);
        arrive_ranked(&trades, "Mod", 2, 2, 0);
        trades.request("Ana", "Mod");
        trades.request("Mod", "Ana");

        if let Ok(mut state) = trades.inner.lock()
            && let Some(party) = state.present.get_mut("Mod")
        {
            party.rank = ADMINISTRATOR;
        }

        trades.change("Ana", &offering(&[5]));
        trades.accept("Ana", &offering(&[5]), &offering(&[]));
        let step = trades.accept("Mod", &offering(&[]), &offering(&[5]));

        assert_eq!(step, Step::Done, "the items moved across the rank line");
        assert_eq!(trades.partner("Ana"), None, "and the trade was cancelled");
        assert_eq!(trades.partner("Mod"), None);
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

    #[test]
    fn two_copies_of_one_dungeon_are_two_crowds() {
        // Keyed by name, ten parties in ten Undead Lairs are reported as one crowd of ten -- which
        // is also the number a realm portal's label would carry.
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);
        arrive(&trades, "Cy", 3);

        trades.moved("Ana", "UndeadLair#Realm:11");
        trades.moved("Bo", "UndeadLair#Realm:11");
        trades.moved("Cy", "UndeadLair#Realm:12");

        let counts = trades.counts();
        assert_eq!(counts.get("UndeadLair#Realm:11").copied(), Some(2));
        assert_eq!(counts.get("UndeadLair#Realm:12").copied(), Some(1));
        assert_eq!(counts.get("UndeadLair"), None);

        assert_eq!(trades.in_world("UndeadLair#Realm:11"), 2);
        assert_eq!(trades.in_world("UndeadLair#Realm:12"), 1);
        assert_eq!(trades.in_world("GuildHall#guild4"), 0);
    }

    #[test]
    fn a_room_is_named_by_its_world_when_a_person_asks() {
        let trades = trades();
        arrive(&trades, "Ana", 1);
        trades.moved("Ana", "UndeadLair#Realm:11");

        assert_eq!(trades.world_of("Ana").as_deref(), Some("UndeadLair"));
        assert_eq!(
            trades.room_of("Ana").as_deref(),
            Some("UndeadLair#Realm:11")
        );
    }

    #[test]
    fn walking_into_the_other_copy_of_a_dungeon_ends_the_trade() {
        // Two Undead Lairs share a name, so a trade agreed in one used to survive a walk into the
        // other and settle against a partner who could no longer see it.
        let trades = trades();
        arrive(&trades, "Ana", 1);
        arrive(&trades, "Bo", 2);
        trades.moved("Ana", "UndeadLair#Realm:11");
        trades.moved("Bo", "UndeadLair#Realm:11");
        trades.request("Ana", "Bo");
        trades.request("Bo", "Ana");

        trades.moved("Ana", "UndeadLair#Realm:12");

        assert_eq!(trades.partner("Ana"), None);
        assert_eq!(trades.partner("Bo"), None, "Bo is trading with a ghost");
    }
}
