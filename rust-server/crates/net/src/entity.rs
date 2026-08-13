//! Entity state and the field mask that keeps snapshots small.
//!
//! An entity has a dozen or so transmittable fields, and on any given tick almost none of them
//! change. A monster that is walking changes its position and nothing else; one that is standing
//! still changes nothing at all. Writing a fixed record per entity spends bytes in proportion to
//! how many entities exist; writing a mask and only the fields behind it spends bytes in proportion
//! to how much actually happened.
//!
//! The legacy protocol did the former, which is why its tick packets grew with crowd size even when
//! the crowd was idle.

use crate::codec::{CodecError, Reader, Writer, quantize};

/// A server-assigned entity handle, unique within a world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct EntityId(pub u32);

/// Which fields a record carries.
///
/// Written as a varint, so the common case of an entity that only moved costs a single byte
/// regardless of how many fields exist in total. Adding a field later costs nothing until something
/// sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FieldMask(pub u32);

impl FieldMask {
    pub const EMPTY: FieldMask = FieldMask(0);

    pub const POSITION: FieldMask = FieldMask(1 << 0);
    pub const HP: FieldMask = FieldMask(1 << 1);
    pub const MAX_HP: FieldMask = FieldMask(1 << 2);
    pub const MP: FieldMask = FieldMask(1 << 3);
    pub const MAX_MP: FieldMask = FieldMask(1 << 4);
    pub const CONDITIONS: FieldMask = FieldMask(1 << 5);
    pub const SIZE: FieldMask = FieldMask(1 << 6);
    pub const NAME: FieldMask = FieldMask(1 << 7);
    pub const OBJECT_TYPE: FieldMask = FieldMask(1 << 8);

    /// Every field, for an entity the receiver has never seen.
    pub const ALL: FieldMask = FieldMask(0x1ff);

    pub fn has(self, field: FieldMask) -> bool {
        self.0 & field.0 != 0
    }

    pub fn with(self, field: FieldMask) -> FieldMask {
        FieldMask(self.0 | field.0)
    }

    pub fn set(&mut self, field: FieldMask, when: bool) {
        if when {
            self.0 |= field.0;
        }
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Everything about an entity that travels to a client.
///
/// Deliberately a flat struct of plain fields rather than a map of typed values: comparing two of
/// these is a handful of integer comparisons, and encoding one touches no allocation. The name is
/// the only heap field, and it is the only one that essentially never changes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntityState {
    pub object_type: u16,
    pub x: f32,
    pub y: f32,
    pub hp: i32,
    pub max_hp: i32,
    pub mp: i32,
    pub max_mp: i32,

    /// Condition effects, as the bitmask described in `hendra-content`. Kept as a bare `u128` so
    /// this crate stays free of a content dependency. The client's extension has no use for loot
    /// tables or spawn rules.
    pub conditions: u128,

    /// Rendered size in percent, where 100 is the object's natural size.
    pub size: u16,

    pub name: Option<Box<str>>,
}

impl EntityState {
    /// Which fields differ from `baseline`.
    ///
    /// Positions are compared *after* quantisation. Two coordinates that differ by less than a
    /// transmission step encode to identical bytes, so treating them as a change would spend a byte
    /// to tell the client something it already believes. Floating-point drift in the simulation
    /// makes that case common rather than rare.
    pub fn changes_from(&self, baseline: &EntityState) -> FieldMask {
        let mut mask = FieldMask::EMPTY;

        mask.set(
            FieldMask::POSITION,
            quantize(self.x) != quantize(baseline.x) || quantize(self.y) != quantize(baseline.y),
        );
        mask.set(FieldMask::HP, self.hp != baseline.hp);
        mask.set(FieldMask::MAX_HP, self.max_hp != baseline.max_hp);
        mask.set(FieldMask::MP, self.mp != baseline.mp);
        mask.set(FieldMask::MAX_MP, self.max_mp != baseline.max_mp);
        mask.set(
            FieldMask::CONDITIONS,
            self.conditions != baseline.conditions,
        );
        mask.set(FieldMask::SIZE, self.size != baseline.size);
        mask.set(FieldMask::NAME, self.name != baseline.name);
        mask.set(
            FieldMask::OBJECT_TYPE,
            self.object_type != baseline.object_type,
        );

        mask
    }

    /// Writes the fields named by `mask`.
    ///
    /// When `baseline` is `Some`, the position is written as a delta against it; otherwise it is
    /// absolute. The decoder makes the same choice from the same information, which is what keeps
    /// the two in step without a flag on the wire.
    pub fn encode(&self, mask: FieldMask, baseline: Option<&EntityState>, w: &mut Writer<'_>) {
        w.varint(mask.0 as u64);

        if mask.has(FieldMask::OBJECT_TYPE) {
            w.varint(self.object_type as u64);
        }
        if mask.has(FieldMask::POSITION) {
            match baseline {
                Some(from) => {
                    w.position_delta(self.x, from.x);
                    w.position_delta(self.y, from.y);
                }
                None => {
                    w.position(self.x);
                    w.position(self.y);
                }
            }
        }
        if mask.has(FieldMask::HP) {
            w.varint_signed(self.hp as i64);
        }
        if mask.has(FieldMask::MAX_HP) {
            w.varint_signed(self.max_hp as i64);
        }
        if mask.has(FieldMask::MP) {
            w.varint_signed(self.mp as i64);
        }
        if mask.has(FieldMask::MAX_MP) {
            w.varint_signed(self.max_mp as i64);
        }
        if mask.has(FieldMask::CONDITIONS) {
            w.condition_mask(self.conditions);
        }
        if mask.has(FieldMask::SIZE) {
            w.varint(self.size as u64);
        }
        if mask.has(FieldMask::NAME) {
            match &self.name {
                Some(name) => {
                    w.bool(true);
                    w.string(name);
                }
                None => w.bool(false),
            }
        }
    }

    /// Reads a record, starting from `baseline` and overwriting only what the mask names.
    ///
    /// Fields absent from the mask keep their baseline values, which is the whole point: the sender
    /// omitted them precisely because they had not changed.
    pub fn decode(
        baseline: Option<&EntityState>,
        r: &mut Reader<'_>,
    ) -> Result<EntityState, CodecError> {
        let mask = FieldMask(r.varint_u32()?);
        let mut state = baseline.cloned().unwrap_or_default();

        if mask.has(FieldMask::OBJECT_TYPE) {
            state.object_type =
                u16::try_from(r.varint()?).map_err(|_| CodecError::InvalidValue {
                    what: "object type",
                    value: 0,
                })?;
        }
        if mask.has(FieldMask::POSITION) {
            match baseline {
                Some(from) => {
                    state.x = r.position_delta(from.x)?;
                    state.y = r.position_delta(from.y)?;
                }
                None => {
                    state.x = r.position_value()?;
                    state.y = r.position_value()?;
                }
            }
        }
        if mask.has(FieldMask::HP) {
            state.hp = r.varint_signed()? as i32;
        }
        if mask.has(FieldMask::MAX_HP) {
            state.max_hp = r.varint_signed()? as i32;
        }
        if mask.has(FieldMask::MP) {
            state.mp = r.varint_signed()? as i32;
        }
        if mask.has(FieldMask::MAX_MP) {
            state.max_mp = r.varint_signed()? as i32;
        }
        if mask.has(FieldMask::CONDITIONS) {
            state.conditions = r.condition_mask()?;
        }
        if mask.has(FieldMask::SIZE) {
            state.size = u16::try_from(r.varint()?).unwrap_or(u16::MAX);
        }
        if mask.has(FieldMask::NAME) {
            state.name = if r.bool()? {
                Some(r.string()?.into())
            } else {
                None
            };
        }

        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walking() -> EntityState {
        EntityState {
            object_type: 0x0a22,
            x: 103.5,
            y: 88.25,
            hp: 480,
            max_hp: 800,
            mp: 120,
            max_mp: 250,
            conditions: 0,
            size: 100,
            name: Some("Hobbit Mage".into()),
        }
    }

    fn round_trip(current: &EntityState, baseline: Option<&EntityState>) -> (EntityState, usize) {
        let mask = match baseline {
            Some(from) => current.changes_from(from),
            None => FieldMask::ALL,
        };

        let mut buf = Vec::new();
        current.encode(mask, baseline, &mut Writer::new(&mut buf));

        let decoded = EntityState::decode(baseline, &mut Reader::new(&buf)).unwrap();
        (decoded, buf.len())
    }

    #[test]
    fn a_first_sighting_carries_every_field() {
        let entity = walking();
        let (decoded, _) = round_trip(&entity, None);
        assert_eq!(decoded, entity);
    }

    #[test]
    fn an_unchanged_entity_costs_a_single_byte() {
        let entity = walking();
        let (decoded, bytes) = round_trip(&entity, Some(&entity));
        assert_eq!(decoded, entity);
        assert_eq!(bytes, 1, "an empty mask is the whole record");
    }

    #[test]
    fn an_entity_that_only_moved_costs_three_bytes() {
        let before = walking();
        let mut after = before.clone();
        after.x += 0.125;
        after.y -= 0.125;

        let (decoded, bytes) = round_trip(&after, Some(&before));
        assert_eq!(quantize(decoded.x), quantize(after.x));
        assert_eq!(quantize(decoded.y), quantize(after.y));

        // One byte of mask plus one per axis.
        assert_eq!(bytes, 3);
    }

    #[test]
    fn sub_quantum_jitter_is_not_a_change() {
        let before = walking();
        let mut after = before.clone();

        // Far below one transmission step: the client already holds this value.
        after.x += 0.0001;
        after.y -= 0.0001;

        assert_eq!(after.changes_from(&before), FieldMask::EMPTY);
        let (_, bytes) = round_trip(&after, Some(&before));
        assert_eq!(bytes, 1);
    }

    #[test]
    fn unmentioned_fields_keep_their_baseline_values() {
        let before = walking();
        let mut after = before.clone();
        after.hp = 200;

        let (decoded, _) = round_trip(&after, Some(&before));
        assert_eq!(decoded.hp, 200);

        // Everything the mask did not name survived.
        assert_eq!(decoded.name, before.name);
        assert_eq!(decoded.max_hp, before.max_hp);
        assert_eq!(decoded.object_type, before.object_type);
        assert_eq!(quantize(decoded.x), quantize(before.x));
    }

    #[test]
    fn conditions_round_trip_at_full_width() {
        let before = walking();
        let mut after = before.clone();
        after.conditions = 1u128 << 120;

        let (decoded, _) = round_trip(&after, Some(&before));
        assert_eq!(decoded.conditions, 1u128 << 120);
    }

    #[test]
    fn a_name_can_be_added_and_cleared() {
        let mut anonymous = walking();
        anonymous.name = None;

        let named = walking();
        let (decoded, _) = round_trip(&named, Some(&anonymous));
        assert_eq!(decoded.name, named.name);

        let (decoded, _) = round_trip(&anonymous, Some(&named));
        assert_eq!(decoded.name, None);
    }

    #[test]
    fn a_busy_entity_still_beats_a_fixed_record() {
        let before = walking();
        let mut after = before.clone();
        after.x += 0.25;
        after.y += 0.25;
        after.hp -= 137;
        after.mp -= 40;
        after.conditions = 0b1010;

        let (_, bytes) = round_trip(&after, Some(&before));

        // Five changed fields. A fixed record would spend four bytes on each of the
        // twenty-odd fields an entity has, whether they changed or not.
        assert!(bytes < 16, "{bytes} bytes for five changed fields");
    }

    #[test]
    fn a_truncated_record_errors_rather_than_panicking() {
        let before = walking();
        let mut after = before.clone();
        after.hp = 1;
        after.name = Some("a considerably longer name".into());

        let mask = after.changes_from(&before);
        let mut buf = Vec::new();
        after.encode(mask, Some(&before), &mut Writer::new(&mut buf));

        for cut in 0..buf.len() {
            let mut reader = Reader::new(&buf[..cut]);
            let _ = EntityState::decode(Some(&before), &mut reader);
        }
    }
}
