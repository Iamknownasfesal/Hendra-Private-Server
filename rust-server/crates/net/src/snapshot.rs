//! Acknowledged baselines: deciding what a snapshot may safely be a delta against.
//!
//! Snapshots ride unreliable datagrams, so the server cannot assume the last thing it sent
//! arrived. Diffing against "what I sent last" would desynchronise permanently the first time a
//! packet dropped. Instead the client reports the newest tick it actually holds, the server keeps a
//! short history of what each connection has been sent, and the next snapshot is encoded against
//! the acknowledged one.
//!
//! The failure mode is bounded and self-healing: if the acknowledgement is missing or has aged out
//! of the ring, the server sends one full snapshot and the connection re-converges on the next
//! tick.

use std::cell::Cell;

use crate::codec::{CodecError, Reader, Writer};

/// How many snapshots of history each connection keeps.
///
/// At 20 ticks per second this is 1.6 seconds of tolerance — comfortably longer than any round
/// trip that is still worth encoding a delta for. A client quiet for longer than this gets a full
/// snapshot, which is the correct answer anyway.
pub const SNAPSHOT_HISTORY: usize = 32;

/// A simulation tick number.
///
/// Comparison is wrapping-aware. At 20 TPS a `u32` lasts about seven years, so this is not a
/// practical concern, but wrapping comparison costs nothing and removes the need to reason about
/// it at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tick(pub u32);

impl Tick {
    pub const ZERO: Tick = Tick(0);

    pub fn next(self) -> Tick {
        Tick(self.0.wrapping_add(1))
    }

    /// Whether this tick comes after `other`, accounting for wraparound.
    pub fn is_newer_than(self, other: Tick) -> bool {
        let forward = self.0.wrapping_sub(other.0);
        forward != 0 && forward < 0x8000_0000
    }

    /// How many ticks this is ahead of `other`. Meaningless if it is behind.
    pub fn since(self, other: Tick) -> u32 {
        self.0.wrapping_sub(other.0)
    }
}

/// What a client reports having received.
///
/// Carried on every input packet, which is why it costs nothing: the client is already sending
/// movement at tick rate, and this rides along in a byte or two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Acknowledgement {
    /// The newest tick the client holds, or `None` before it has received any.
    pub latest: Option<Tick>,
}

impl Acknowledgement {
    pub const NONE: Acknowledgement = Acknowledgement { latest: None };

    pub fn of(tick: Tick) -> Acknowledgement {
        Acknowledgement { latest: Some(tick) }
    }

    /// Encoded as a presence byte plus the tick, so "nothing yet" costs one byte.
    pub fn encode(self, writer: &mut Writer<'_>) {
        match self.latest {
            Some(tick) => {
                writer.bool(true);
                writer.varint(tick.0 as u64);
            }
            None => writer.bool(false),
        }
    }

    pub fn decode(reader: &mut Reader<'_>) -> Result<Acknowledgement, CodecError> {
        if !reader.bool()? {
            return Ok(Acknowledgement::NONE);
        }
        Ok(Acknowledgement::of(Tick(reader.varint_u32()?)))
    }
}

/// What the encoder should do for the next snapshot.
///
/// An enum rather than an `Option` so a caller cannot quietly forget the full-snapshot path — the
/// case that only shows up under packet loss, and therefore the one that never gets tested if it
/// is easy to skip.
#[derive(Debug, PartialEq, Eq)]
pub enum Baseline<'a, T> {
    /// Encode against this acknowledged state.
    Delta { tick: Tick, state: &'a T },

    /// No usable baseline. Send the whole world.
    Full,
}

impl<T> Baseline<'_, T> {
    pub fn is_full(&self) -> bool {
        matches!(self, Baseline::Full)
    }
}

/// Per-connection history of what has been sent.
///
/// The ring is allocated once when a connection opens and never grows, so storing a snapshot
/// costs an index and a move.
pub struct BaselineRing<T> {
    slots: Vec<Option<(Tick, T)>>,
    newest: Option<Tick>,

    /// How many times this connection has needed a full snapshot.
    ///
    /// A [`Cell`] so that choosing a baseline is a `&self` operation. Counting is bookkeeping, not
    /// mutation of the history itself, and keeping it out of the signature means the encoder can
    /// hold the ring immutably while it reads from the baseline it was handed.
    full_sends: Cell<u64>,
}

impl<T> BaselineRing<T> {
    pub fn new() -> BaselineRing<T> {
        BaselineRing::with_capacity(SNAPSHOT_HISTORY)
    }

    pub fn with_capacity(capacity: usize) -> BaselineRing<T> {
        let capacity = capacity.max(1);
        let mut slots = Vec::with_capacity(capacity);
        slots.resize_with(capacity, || None);
        BaselineRing {
            slots,
            newest: None,
            full_sends: Cell::new(0),
        }
    }

    fn index_of(&self, tick: Tick) -> usize {
        tick.0 as usize % self.slots.len()
    }

    /// Records a snapshot as sent.
    pub fn store(&mut self, tick: Tick, state: T) {
        let index = self.index_of(tick);
        self.slots[index] = Some((tick, state));
        if self.newest.is_none_or(|newest| tick.is_newer_than(newest)) {
            self.newest = Some(tick);
        }
    }

    /// The stored snapshot for a tick, if it has not been overwritten.
    ///
    /// The tick is checked as well as the slot: a ring position holds whichever tick landed there
    /// most recently, and an aged-out tick maps to the same index as a much newer one.
    pub fn get(&self, tick: Tick) -> Option<&T> {
        match &self.slots[self.index_of(tick)] {
            Some((stored, state)) if *stored == tick => Some(state),
            _ => None,
        }
    }

    /// The newest tick stored.
    pub fn newest(&self) -> Option<Tick> {
        self.newest
    }

    /// How many full snapshots this connection has needed.
    pub fn full_sends(&self) -> u64 {
        self.full_sends.get()
    }

    /// Decides what the next snapshot may be encoded against.
    ///
    /// Three things disqualify a baseline, and all of them mean the same thing to the caller: send
    /// the whole world once and let the next acknowledgement restore the delta stream.
    pub fn baseline_for(&self, ack: Acknowledgement) -> Baseline<'_, T> {
        let Some(tick) = ack.latest else {
            return self.needs_full();
        };

        // A client cannot acknowledge a tick we never sent. Accepting one would let it name its own
        // baseline: the ring slot for a far-future tick holds some unrelated older snapshot, and
        // encoding against that would produce deltas neither side could reconstruct.
        if self.newest.is_none_or(|newest| tick.is_newer_than(newest)) {
            return self.needs_full();
        }

        match self.get(tick) {
            Some(state) => Baseline::Delta { tick, state },
            None => self.needs_full(),
        }
    }

    fn needs_full(&self) -> Baseline<'_, T> {
        self.full_sends.set(self.full_sends.get() + 1);
        Baseline::Full
    }

    /// Forgets everything, for a connection that has changed world.
    pub fn clear(&mut self) {
        for slot in self.slots.iter_mut() {
            *slot = None;
        }
        self.newest = None;
    }
}

impl<T> Default for BaselineRing<T> {
    fn default() -> Self {
        BaselineRing::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_order_correctly_across_a_wrap() {
        assert!(Tick(5).is_newer_than(Tick(4)));
        assert!(!Tick(4).is_newer_than(Tick(5)));
        assert!(!Tick(4).is_newer_than(Tick(4)));

        // Straddling the u32 boundary.
        let before = Tick(u32::MAX - 2);
        let after = before.next().next().next().next();
        assert!(after.is_newer_than(before));
        assert!(!before.is_newer_than(after));
        assert_eq!(after.since(before), 4);
    }

    #[test]
    fn an_acknowledged_tick_becomes_the_baseline() {
        let mut ring = BaselineRing::new();
        ring.store(Tick(1), "one");
        ring.store(Tick(2), "two");

        match ring.baseline_for(Acknowledgement::of(Tick(1))) {
            Baseline::Delta { tick, state } => {
                assert_eq!(tick, Tick(1));
                assert_eq!(*state, "one");
            }
            Baseline::Full => panic!("tick 1 is still in the ring"),
        }
        assert_eq!(ring.full_sends(), 0);
    }

    #[test]
    fn a_client_with_nothing_yet_gets_a_full_snapshot() {
        let mut ring: BaselineRing<&str> = BaselineRing::new();
        ring.store(Tick(1), "one");
        assert!(ring.baseline_for(Acknowledgement::NONE).is_full());
        assert_eq!(ring.full_sends(), 1);
    }

    #[test]
    fn a_tick_that_has_aged_out_falls_back_to_full() {
        let mut ring = BaselineRing::with_capacity(4);
        for n in 0..8 {
            ring.store(Tick(n), n);
        }

        // Tick 0 shares a slot with tick 4, which overwrote it.
        assert!(ring.get(Tick(0)).is_none());
        assert!(ring.baseline_for(Acknowledgement::of(Tick(0))).is_full());

        // The most recent four are still there.
        assert_eq!(ring.get(Tick(7)), Some(&7));
        assert!(!ring.baseline_for(Acknowledgement::of(Tick(7))).is_full());
    }

    #[test]
    fn a_client_cannot_acknowledge_a_tick_we_never_sent() {
        let mut ring = BaselineRing::new();
        ring.store(Tick(3), "three");

        // Claiming a future tick must not select a stale ring slot as the baseline.
        assert!(ring.baseline_for(Acknowledgement::of(Tick(9999))).is_full());
        assert!(
            ring.baseline_for(Acknowledgement::of(Tick(u32::MAX)))
                .is_full()
        );
    }

    #[test]
    fn loss_costs_one_full_snapshot_then_re_converges() {
        let mut ring = BaselineRing::new();
        let mut acked = Acknowledgement::NONE;

        // Tick 0: nothing acknowledged, so a full send.
        ring.store(Tick(0), 0);
        assert!(ring.baseline_for(acked).is_full());

        // The client receives it and acknowledges.
        acked = Acknowledgement::of(Tick(0));
        for n in 1..20 {
            ring.store(Tick(n), n);
            assert!(!ring.baseline_for(acked).is_full(), "tick {n} should delta");
            acked = Acknowledgement::of(Tick(n));
        }

        // One full send at the start, none since.
        assert_eq!(ring.full_sends(), 1);
    }

    #[test]
    fn clearing_forces_a_full_snapshot_after_a_world_change() {
        let mut ring = BaselineRing::new();
        ring.store(Tick(5), "five");
        ring.clear();
        assert!(ring.baseline_for(Acknowledgement::of(Tick(5))).is_full());
        assert_eq!(ring.newest(), None);
    }

    #[test]
    fn acknowledgements_round_trip_and_cost_one_byte_when_empty() {
        let mut buf = Vec::new();
        Acknowledgement::NONE.encode(&mut Writer::new(&mut buf));
        assert_eq!(buf.len(), 1);
        assert_eq!(
            Acknowledgement::decode(&mut Reader::new(&buf)).unwrap(),
            Acknowledgement::NONE
        );

        for tick in [0u32, 1, 127, 128, 100_000, u32::MAX] {
            let mut buf = Vec::new();
            Acknowledgement::of(Tick(tick)).encode(&mut Writer::new(&mut buf));
            assert_eq!(
                Acknowledgement::decode(&mut Reader::new(&buf)).unwrap(),
                Acknowledgement::of(Tick(tick)),
                "tick {tick}"
            );
        }
    }
}
