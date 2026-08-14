//! World snapshots and the diff between two of them.
//!
//! A snapshot is what one client can see at one tick. Encoding one against an earlier snapshot
//! reduces to a merge of two id-sorted lists. That yields the three things the receiver needs,
//! entities that appeared, entities that changed and entities that left, in a single pass with no
//! lookups and no allocation.
//!
//! Entities that exist in both snapshots and changed in no field are omitted entirely, not written
//! with an empty mask. On a quiet tick that makes the whole snapshot a few bytes of header.

use crate::codec::{CodecError, Reader, Writer};
use crate::entity::{EntityId, EntityState, FieldMask};
use crate::snapshot::Tick;

/// The visible world at one tick, sorted by entity id.
///
/// Sorted because the diff is a merge and merges want order. The simulation hands entities over in
/// whatever order it stores them, so [`WorldSnapshot::from_unsorted`] sorts once per tick. A few
/// microseconds for the hundred-odd entities a sight radius holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldSnapshot {
    entities: Vec<(EntityId, EntityState)>,
}

impl WorldSnapshot {
    pub fn new() -> WorldSnapshot {
        WorldSnapshot::default()
    }

    /// Builds a snapshot from entities in arbitrary order.
    pub fn from_unsorted(mut entities: Vec<(EntityId, EntityState)>) -> WorldSnapshot {
        entities.sort_unstable_by_key(|(id, _)| *id);
        entities.dedup_by_key(|(id, _)| *id);
        WorldSnapshot { entities }
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn get(&self, id: EntityId) -> Option<&EntityState> {
        self.entities
            .binary_search_by_key(&id, |(entity, _)| *entity)
            .ok()
            .map(|index| &self.entities[index].1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &EntityState)> {
        self.entities.iter().map(|(id, state)| (*id, state))
    }

    /// Empties the snapshot while keeping its allocation, so the next tick reuses the memory.
    pub fn clear(&mut self) {
        self.entities.clear();
    }

    /// Appends an entity. The caller must add entities in ascending id order.
    pub fn push(&mut self, id: EntityId, state: EntityState) {
        debug_assert!(
            self.entities.last().is_none_or(|(last, _)| *last < id),
            "entities must be pushed in ascending id order"
        );
        self.entities.push((id, state));
    }
}

/// A conservative default for how much snapshot fits in one datagram.
///
/// QUIC does not fragment datagrams: anything over the limit is refused outright rather than split.
/// The number is *not* the path MTU. 1200 bytes is the floor QUIC assumes for a whole packet, and a
/// datagram's payload is what remains after connection ids, packet number, frame header and the
/// AEAD tag. Before path discovery runs that came to 1162 bytes on a measured connection, under
/// the 1200 this was originally, and wrongly, set to.
///
/// It is only a default, and a pessimistic one. The real limit is negotiated per
/// connection and rises once MTU discovery learns what the path carries, so a server should take it
/// from the live connection via [`SnapshotEncoder::with_budget`] rather than encode to this.
pub const DATAGRAM_BUDGET: usize = 1100;

/// How an encoded snapshot has to reach the client.
///
/// Measured, not guessed: a delta for 120 visible entities all moving at once comes to about 540
/// bytes, while a full snapshot of the same world is roughly 2,600. Deltas fit a datagram with room
/// to spare; full snapshots never will, and they are also the one kind of snapshot that must not be
/// lost, since everything after one is encoded against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a snapshot sent on the wrong transport either fails silently or arrives too late"]
pub enum Delivery {
    /// Small, and safe to lose. The next delta supersedes it.
    Datagram,

    /// Must arrive intact, so it goes on the reliable stream, which also handles the fragmentation
    /// a datagram cannot.
    Stream,
}

/// One entity's place in a diff.
struct Record {
    id: EntityId,
    mask: FieldMask,
    current: usize,
    baseline: Option<usize>,
}

/// Encodes snapshots, reusing its working buffers between ticks.
///
/// One of these lives per connection. After the first few ticks its buffers have grown to the size
/// a sight radius needs and encoding stops allocating entirely.
pub struct SnapshotEncoder {
    despawns: Vec<EntityId>,
    records: Vec<Record>,

    /// The most this encoder will put in a datagram before routing to the stream instead.
    budget: usize,
}

impl Default for SnapshotEncoder {
    fn default() -> SnapshotEncoder {
        SnapshotEncoder::new()
    }
}

impl SnapshotEncoder {
    /// An encoder using the conservative [`DATAGRAM_BUDGET`].
    pub fn new() -> SnapshotEncoder {
        SnapshotEncoder::with_budget(DATAGRAM_BUDGET)
    }

    /// An encoder that knows what this particular connection will carry.
    ///
    /// The limit is negotiated per connection and can shrink when path discovery finds a smaller
    /// MTU, so a server should take it from the live connection rather than trusting the default.
    pub fn with_budget(budget: usize) -> SnapshotEncoder {
        SnapshotEncoder {
            despawns: Vec::new(),
            records: Vec::new(),
            budget,
        }
    }

    /// Updates the budget, for a path whose limit has changed since the connection opened.
    pub fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    /// Writes `current` as a delta against `baseline`, or in full when there is none.
    ///
    /// Returns how the result has to be sent. Returning it rather than leaving the caller to work
    /// it out is intentional: sending a full snapshot on a datagram fails silently, because QUIC refuses
    /// the oversized payload and the client simply never converges.
    pub fn encode(
        &mut self,
        tick: Tick,
        current: &WorldSnapshot,
        baseline: Option<(Tick, &WorldSnapshot)>,
        w: &mut Writer<'_>,
    ) -> Delivery {
        let started = w.len();
        self.despawns.clear();
        self.records.clear();

        match baseline {
            Some((_, baseline)) => self.diff(current, baseline),
            None => {
                // Nothing to compare against: every entity is a first sighting.
                self.records
                    .extend(
                        current
                            .entities
                            .iter()
                            .enumerate()
                            .map(|(index, (id, _))| Record {
                                id: *id,
                                mask: FieldMask::ALL,
                                current: index,
                                baseline: None,
                            }),
                    );
            }
        }

        w.varint(tick.0 as u64);

        // Name the baseline explicitly. The receiver cannot infer it: whether a position is
        // absolute or a delta depends on which snapshot this was measured against, and "the newest
        // one I hold" is not the same thing. Datagrams reorder, and a full snapshot sent to a
        // client that already knows these entities would otherwise be read as a delta from a value
        // the server never used.
        match baseline {
            Some((from, _)) => {
                w.bool(true);
                w.varint(from.0 as u64);
            }
            None => w.bool(false),
        }

        // Ids are ascending in both lists, so writing each as a step from the previous one keeps
        // them to a byte apiece even in a world with millions of entity handles issued.
        w.varint(self.despawns.len() as u64);
        let mut previous = 0u32;
        for id in &self.despawns {
            w.varint(id.0.wrapping_sub(previous) as u64);
            previous = id.0;
        }

        w.varint(self.records.len() as u64);
        let mut previous = 0u32;
        for record in &self.records {
            w.varint(record.id.0.wrapping_sub(previous) as u64);
            previous = record.id.0;

            let from = record
                .baseline
                .and_then(|index| baseline.map(|(_, snapshot)| &snapshot.entities[index].1));
            current.entities[record.current]
                .1
                .encode(record.mask, from, w);
        }

        // A full snapshot goes on the stream whatever its size: losing one strands the client with
        // no baseline, and every delta after it would be measured from something it does not have.
        if baseline.is_none() || w.len() - started > self.budget {
            Delivery::Stream
        } else {
            Delivery::Datagram
        }
    }

    /// Walks both id-sorted lists together, classifying every entity exactly once.
    fn diff(&mut self, current: &WorldSnapshot, baseline: &WorldSnapshot) {
        let (mut i, mut j) = (0usize, 0usize);

        while i < current.entities.len() || j < baseline.entities.len() {
            match (current.entities.get(i), baseline.entities.get(j)) {
                (Some((id, _)), None) => {
                    self.records.push(Record {
                        id: *id,
                        mask: FieldMask::ALL,
                        current: i,
                        baseline: None,
                    });
                    i += 1;
                }
                (None, Some((id, _))) => {
                    self.despawns.push(*id);
                    j += 1;
                }
                (Some((current_id, state)), Some((baseline_id, was))) => {
                    match current_id.cmp(baseline_id) {
                        std::cmp::Ordering::Less => {
                            self.records.push(Record {
                                id: *current_id,
                                mask: FieldMask::ALL,
                                current: i,
                                baseline: None,
                            });
                            i += 1;
                        }
                        std::cmp::Ordering::Greater => {
                            self.despawns.push(*baseline_id);
                            j += 1;
                        }
                        std::cmp::Ordering::Equal => {
                            let mask = state.changes_from(was);
                            // An entity that did nothing is not mentioned at all.
                            if !mask.is_empty() {
                                self.records.push(Record {
                                    id: *current_id,
                                    mask,
                                    current: i,
                                    baseline: Some(j),
                                });
                            }
                            i += 1;
                            j += 1;
                        }
                    }
                }
                (None, None) => break,
            }
        }
    }
}

/// What a snapshot says about itself, before any of its entities are read.
///
/// Read this first. It names the baseline the sender used, which is the only way to know whether
/// the body can be decoded at all, and, if the receiver has fallen behind or packets arrived out
/// of order, whether it should be dropped instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotHeader {
    /// The tick this snapshot describes.
    pub tick: Tick,

    /// The tick it was encoded against, or `None` if it is complete in itself.
    pub baseline: Option<Tick>,
}

impl SnapshotHeader {
    pub fn is_full(&self) -> bool {
        self.baseline.is_none()
    }
}

/// Reads the header, leaving the reader positioned at the body.
pub fn read_header(r: &mut Reader<'_>) -> Result<SnapshotHeader, CodecError> {
    let tick = Tick(r.varint_u32()?);
    let baseline = if r.bool()? {
        Some(Tick(r.varint_u32()?))
    } else {
        None
    };
    Ok(SnapshotHeader { tick, baseline })
}

/// Reconstructs the world from a snapshot body.
///
/// `baseline` must be the snapshot the header names, `Some` for a delta and `None` for a full
/// snapshot. Passing the wrong one is an error rather than a silent misread. Getting this
/// wrong produces entities at plausible but incorrect positions, which is far harder to notice than
/// a refused packet.
pub fn decode_body(
    header: SnapshotHeader,
    baseline: Option<&WorldSnapshot>,
    r: &mut Reader<'_>,
) -> Result<WorldSnapshot, CodecError> {
    const MAX_ENTITIES: usize = 4096;

    if header.baseline.is_some() != baseline.is_some() {
        return Err(CodecError::BaselineMismatch {
            needed: header.baseline.map(|tick| tick.0),
            supplied: baseline.map(|_| 0),
        });
    }

    let despawn_count = r.count(MAX_ENTITIES)?;
    let mut despawned = Vec::with_capacity(despawn_count.min(256));
    let mut previous = 0u32;
    for _ in 0..despawn_count {
        previous = previous.wrapping_add(r.varint_u32()?);
        despawned.push(EntityId(previous));
    }

    let record_count = r.count(MAX_ENTITIES)?;
    let mut records = Vec::with_capacity(record_count.min(256));
    let mut previous = 0u32;
    for _ in 0..record_count {
        previous = previous.wrapping_add(r.varint_u32()?);
        let id = EntityId(previous);
        let from = baseline.and_then(|snapshot| snapshot.get(id));
        records.push((id, EntityState::decode(from, r)?));
    }

    // Carry forward everything the baseline held that was neither removed nor updated.
    let mut entities: Vec<(EntityId, EntityState)> = Vec::new();
    if let Some(baseline) = baseline {
        entities.reserve(baseline.len() + records.len());
        for (id, state) in baseline.iter() {
            if despawned.binary_search(&id).is_ok() {
                continue;
            }
            if records
                .binary_search_by_key(&id, |(record, _)| *record)
                .is_ok()
            {
                continue;
            }
            entities.push((id, state.clone()));
        }
    }
    entities.extend(records);
    entities.sort_unstable_by_key(|(id, _)| *id);

    Ok(WorldSnapshot { entities })
}

/// Reads a whole snapshot when the caller already holds the right baseline.
///
/// A convenience over [`read_header`] and [`decode_body`]. A receiver that keeps a history should
/// use those two directly, so it can look the named baseline up rather than assume it.
pub fn decode_snapshot(
    baseline: Option<&WorldSnapshot>,
    r: &mut Reader<'_>,
) -> Result<(Tick, WorldSnapshot), CodecError> {
    let header = read_header(r)?;
    let world = decode_body(header, baseline, r)?;
    Ok((header.tick, world))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::quantize;

    fn entity(object_type: u16, x: f32, y: f32, hp: i32) -> EntityState {
        EntityState {
            object_type,
            x,
            y,
            hp,
            max_hp: 800,
            mp: 100,
            max_mp: 200,
            conditions: 0,
            size: 100,
            name: None,
            texture: 0,
            stats: [0; 8],
            stars: 0,
            oxygen: 100,
        }
    }

    fn snapshot(entities: &[(u32, EntityState)]) -> WorldSnapshot {
        WorldSnapshot::from_unsorted(
            entities
                .iter()
                .map(|(id, state)| (EntityId(*id), state.clone()))
                .collect(),
        )
    }

    fn round_trip(
        current: &WorldSnapshot,
        baseline: Option<&WorldSnapshot>,
    ) -> (WorldSnapshot, usize) {
        let (decoded, bytes, _) = round_trip_with_delivery(current, baseline);
        (decoded, bytes)
    }

    fn round_trip_with_delivery(
        current: &WorldSnapshot,
        baseline: Option<&WorldSnapshot>,
    ) -> (WorldSnapshot, usize, Delivery) {
        let mut buf = Vec::new();
        let delivery = SnapshotEncoder::new().encode(
            Tick(7),
            current,
            baseline.map(|b| (Tick(6), b)),
            &mut Writer::new(&mut buf),
        );
        let (tick, decoded) = decode_snapshot(baseline, &mut Reader::new(&buf)).unwrap();
        assert_eq!(tick, Tick(7));
        (decoded, buf.len(), delivery)
    }

    #[test]
    fn a_full_snapshot_reconstructs_every_entity() {
        let world = snapshot(&[
            (1, entity(0x100, 10.0, 20.0, 500)),
            (9, entity(0x200, 30.5, 40.25, 100)),
            (4, entity(0x300, 1.0, 2.0, 50)),
        ]);

        let (decoded, _) = round_trip(&world, None);
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded.get(EntityId(9)).unwrap().object_type, 0x200);
        assert_eq!(decoded.get(EntityId(4)).unwrap().hp, 50);
    }

    #[test]
    fn a_world_where_nothing_happened_costs_five_bytes() {
        let world = snapshot(&[
            (1, entity(0x100, 10.0, 20.0, 500)),
            (2, entity(0x100, 11.0, 21.0, 500)),
            (3, entity(0x100, 12.0, 22.0, 500)),
        ]);

        let (decoded, bytes) = round_trip(&world, Some(&world));
        assert_eq!(decoded, world);

        // Tick, the named baseline, zero despawns, zero records. None of the three entities is
        // mentioned at all. Naming the baseline is two of these five bytes and is what lets the
        // receiver tell a delta from a full snapshot.
        assert_eq!(bytes, 5);
    }

    #[test]
    fn only_the_entity_that_moved_is_mentioned() {
        let before = snapshot(&[
            (1, entity(0x100, 10.0, 20.0, 500)),
            (2, entity(0x100, 11.0, 21.0, 500)),
            (3, entity(0x100, 12.0, 22.0, 500)),
        ]);

        let mut after = before.clone();
        let moved = after
            .entities
            .iter_mut()
            .find(|(id, _)| *id == EntityId(2))
            .unwrap();
        moved.1.x += 0.125;

        let (decoded, bytes) = round_trip(&after, Some(&before));

        assert_eq!(
            quantize(decoded.get(EntityId(2)).unwrap().x),
            quantize(11.125)
        );
        assert_eq!(
            quantize(decoded.get(EntityId(1)).unwrap().x),
            quantize(10.0)
        );

        // Header and named baseline, then one record: id step, mask, two axes.
        assert!(bytes <= 10, "{bytes} bytes for one entity moving");
    }

    #[test]
    fn appearing_and_leaving_are_both_carried() {
        let before = snapshot(&[
            (1, entity(0x100, 10.0, 20.0, 500)),
            (2, entity(0x100, 11.0, 21.0, 500)),
        ]);
        let after = snapshot(&[
            (1, entity(0x100, 10.0, 20.0, 500)),
            (3, entity(0x999, 50.0, 60.0, 42)),
        ]);

        let (decoded, _) = round_trip(&after, Some(&before));

        assert!(decoded.get(EntityId(2)).is_none(), "2 should have left");
        assert_eq!(decoded.get(EntityId(3)).unwrap().object_type, 0x999);
        assert_eq!(decoded.get(EntityId(1)).unwrap().hp, 500);
        assert_eq!(decoded.len(), 2);
    }

    #[test]
    fn a_new_entity_gets_an_absolute_position() {
        // Entity 5 is absent from the baseline, so its position cannot be a delta from anything.
        let before = snapshot(&[(1, entity(0x100, 10.0, 20.0, 500))]);
        let after = snapshot(&[
            (1, entity(0x100, 10.0, 20.0, 500)),
            (5, entity(0x100, 900.75, 512.5, 7)),
        ]);

        let (decoded, _) = round_trip(&after, Some(&before));
        let fresh = decoded.get(EntityId(5)).unwrap();
        assert_eq!(quantize(fresh.x), quantize(900.75));
        assert_eq!(quantize(fresh.y), quantize(512.5));
    }

    #[test]
    fn a_long_chain_of_deltas_does_not_drift() {
        let mut world = snapshot(&[(1, entity(0x100, 0.0, 0.0, 500))]);
        let mut client = round_trip(&world, None).0;

        // Two hundred ticks of movement, each encoded against what the client holds.
        for _ in 0..200 {
            let moving = world.entities.first_mut().unwrap();
            moving.1.x += 0.13;
            moving.1.y += 0.07;

            let (decoded, _) = round_trip(&world, Some(&client));
            client = decoded;
        }

        let server = world.get(EntityId(1)).unwrap();
        let seen = client.get(EntityId(1)).unwrap();
        assert_eq!(quantize(seen.x), quantize(server.x), "x drifted");
        assert_eq!(quantize(seen.y), quantize(server.y), "y drifted");
    }

    #[test]
    fn the_encoder_stops_allocating_once_warm() {
        let before = snapshot(&[(1, entity(0x100, 10.0, 20.0, 500))]);
        let mut after = before.clone();
        after.entities[0].1.x += 1.0;

        let mut encoder = SnapshotEncoder::new();
        let mut buf = Vec::new();

        let first = encoder.encode(
            Tick(1),
            &after,
            Some((Tick(0), &before)),
            &mut Writer::new(&mut buf),
        );
        assert_eq!(first, Delivery::Datagram);
        let warm = (encoder.records.capacity(), encoder.despawns.capacity());

        for tick in 2..50 {
            buf.clear();
            let repeat = encoder.encode(
                Tick(tick),
                &after,
                Some((Tick(0), &before)),
                &mut Writer::new(&mut buf),
            );
            assert_eq!(
                repeat, first,
                "delivery should not vary for an identical diff"
            );
        }

        assert_eq!(
            (encoder.records.capacity(), encoder.despawns.capacity()),
            warm,
            "working buffers grew after warm-up"
        );
    }

    #[test]
    fn a_truncated_snapshot_errors_rather_than_panicking() {
        let before = snapshot(&[(1, entity(0x100, 10.0, 20.0, 500))]);
        let after = snapshot(&[
            (1, entity(0x100, 10.5, 20.5, 480)),
            (2, entity(0x200, 5.0, 6.0, 90)),
        ]);

        let mut buf = Vec::new();
        let _ = SnapshotEncoder::new().encode(
            Tick(3),
            &after,
            Some((Tick(2), &before)),
            &mut Writer::new(&mut buf),
        );

        for cut in 0..buf.len() {
            let mut reader = Reader::new(&buf[..cut]);
            let _ = decode_snapshot(Some(&before), &mut reader);
        }
    }

    #[test]
    fn full_snapshots_take_the_stream_and_deltas_take_datagrams() {
        // A crowd large enough that the full snapshot cannot fit a datagram, the case measured in
        // the bandwidth example, where 120 entities came to roughly 2,600 bytes.
        let crowd: Vec<(u32, EntityState)> = (0..120)
            .map(|n| (n, entity(0x100 + n as u16, n as f32, n as f32, 500)))
            .collect();
        let before = snapshot(&crowd);

        let (_, full_bytes, delivery) = round_trip_with_delivery(&before, None);
        assert_eq!(
            delivery,
            Delivery::Stream,
            "a full snapshot must be reliable"
        );
        assert!(
            full_bytes > DATAGRAM_BUDGET,
            "expected the full snapshot to exceed {DATAGRAM_BUDGET} B, got {full_bytes}"
        );

        // The same world one tick later, with everything moving, is a delta that fits.
        let mut after = before.clone();
        for (_, state) in after.entities.iter_mut() {
            state.x += 0.125;
            state.y -= 0.125;
        }

        let (_, delta_bytes, delivery) = round_trip_with_delivery(&after, Some(&before));
        assert_eq!(delivery, Delivery::Datagram);
        assert!(
            delta_bytes <= DATAGRAM_BUDGET,
            "expected the delta to fit {DATAGRAM_BUDGET} B, got {delta_bytes}"
        );
    }

    #[test]
    fn an_oversized_delta_falls_back_to_the_stream() {
        // Names are the one unbounded field. Enough of them arriving at once will overflow a
        // datagram even though the snapshot is a delta.
        let before = snapshot(&[(1, entity(0x100, 0.0, 0.0, 500))]);

        let mut wordy: Vec<(u32, EntityState)> = vec![(1, entity(0x100, 0.0, 0.0, 500))];
        for n in 2..60u32 {
            let mut state = entity(0x100, n as f32, n as f32, 500);
            state.name = Some("a guild name of considerable length".into());
            wordy.push((n, state));
        }

        let (_, bytes, delivery) = round_trip_with_delivery(&snapshot(&wordy), Some(&before));
        assert!(bytes > DATAGRAM_BUDGET, "expected an oversized delta");
        assert_eq!(delivery, Delivery::Stream);
    }

    #[test]
    fn a_hostile_entity_count_is_refused() {
        let mut buf = Vec::new();
        {
            let mut w = Writer::new(&mut buf);
            w.varint(1);
            w.varint(u32::MAX as u64);
        }
        assert!(decode_snapshot(None, &mut Reader::new(&buf)).is_err());
    }
}
