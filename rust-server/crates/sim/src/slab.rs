//! Entity storage: a dense slab addressed by generational handles.
//!
//! # Why generations
//!
//! The C# server keyed entities by a plain integer that was reused as soon as an entity died. That
//! is the shape behind the hit-validation bug: a client reports hitting bullet 47, the bullet that
//! was 47 has expired, slot 47 now holds an unrelated bullet, and the lookup succeeds against the
//! wrong thing. Nothing detects it, because nothing can: a bare index carries no evidence of which
//! occupant it meant.
//!
//! A handle here carries the slot *and* how many times that slot has been reused. Looking up a
//! handle whose generation no longer matches returns `None` rather than the current occupant, so a
//! stale reference is a miss instead of a case of mistaken identity.
//!
//! # Layout
//!
//! Slots live in one contiguous `Vec`, so iterating every entity is a linear walk rather than
//! chasing pointers around the heap the way `Dictionary<int, Entity>` did. Freed slots are recycled
//! through a free list, so a world that spawns and kills constantly does not grow without bound.

use hendra_net::EntityId;

/// How many bits of a handle address the slot. The rest count reuses.
const INDEX_BITS: u32 = 16;
const INDEX_MASK: u32 = (1 << INDEX_BITS) - 1;

/// The most entities one world can hold at once.
pub const MAX_ENTITIES: usize = 1 << INDEX_BITS;

/// A reference to an entity, valid only while that entity lives.
///
/// This is also the identifier on the wire: packing the generation into it means a client cannot
/// name a dead entity by accident, and the server does not have to keep a separate table to notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(pub u32);

impl Handle {
    /// A handle that never resolves.
    pub const NONE: Handle = Handle(u32::MAX);

    fn new(index: u16, generation: u16) -> Handle {
        Handle(((generation as u32) << INDEX_BITS) | index as u32)
    }

    pub fn index(self) -> u16 {
        (self.0 & INDEX_MASK) as u16
    }

    pub fn generation(self) -> u16 {
        (self.0 >> INDEX_BITS) as u16
    }

    pub fn is_none(self) -> bool {
        self == Handle::NONE
    }

    /// The form this takes on the wire.
    pub fn to_entity_id(self) -> EntityId {
        EntityId(self.0)
    }

    pub fn from_entity_id(id: EntityId) -> Handle {
        Handle(id.0)
    }
}

struct Slot<T> {
    /// Incremented every time the slot is freed, so a handle to the previous occupant stops
    /// matching.
    generation: u16,
    value: Option<T>,
}

/// A slab of entities.
pub struct Slab<T> {
    slots: Vec<Slot<T>>,

    /// Indices of slots ready for reuse, most recently freed first.
    free: Vec<u16>,

    live: usize,
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Slab::new()
    }
}

impl<T> Slab<T> {
    pub fn new() -> Slab<T> {
        Slab {
            slots: Vec::new(),
            free: Vec::new(),
            live: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Slab<T> {
        Slab {
            slots: Vec::with_capacity(capacity.min(MAX_ENTITIES)),
            free: Vec::new(),
            live: 0,
        }
    }

    /// How many entities are alive.
    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// How many slots exist, live or not. Iteration walks all of these.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Adds an entity, returning the handle that names it.
    ///
    /// Returns `None` when the world is full rather than growing past what a handle can address.
    pub fn insert(&mut self, value: T) -> Option<Handle> {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            debug_assert!(slot.value.is_none(), "a free slot held a value");
            slot.value = Some(value);
            self.live += 1;
            return Some(Handle::new(index, slot.generation));
        }

        if self.slots.len() >= MAX_ENTITIES {
            return None;
        }

        let index = self.slots.len() as u16;
        self.slots.push(Slot {
            generation: 0,
            value: Some(value),
        });
        self.live += 1;
        Some(Handle::new(index, 0))
    }

    /// Whether a handle still names a living entity.
    pub fn contains(&self, handle: Handle) -> bool {
        self.get(handle).is_some()
    }

    #[inline]
    pub fn get(&self, handle: Handle) -> Option<&T> {
        let slot = self.slots.get(handle.index() as usize)?;
        if slot.generation != handle.generation() {
            return None;
        }
        slot.value.as_ref()
    }

    #[inline]
    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut T> {
        let slot = self.slots.get_mut(handle.index() as usize)?;
        if slot.generation != handle.generation() {
            return None;
        }
        slot.value.as_mut()
    }

    /// Removes an entity and returns it, if the handle still named one.
    pub fn remove(&mut self, handle: Handle) -> Option<T> {
        let slot = self.slots.get_mut(handle.index() as usize)?;
        if slot.generation != handle.generation() {
            return None;
        }

        let value = slot.value.take()?;
        // Wrapping is intended. After 65,536 reuses a handle could alias again, which is
        // astronomically unlikely to be held that long, and refusing to reuse the slot instead
        // leaks it forever.
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(handle.index());
        self.live -= 1;
        Some(value)
    }

    /// Every living entity, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (Handle, &T)> {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            let value = slot.value.as_ref()?;
            Some((Handle::new(index as u16, slot.generation), value))
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle, &mut T)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(index, slot)| {
                let generation = slot.generation;
                let value = slot.value.as_mut()?;
                Some((Handle::new(index as u16, generation), value))
            })
    }

    /// Every living handle, collected into a caller-owned buffer.
    ///
    /// The buffer is reused between ticks so this allocates nothing once warm. It exists because a
    /// tick usually needs to iterate entities while mutating them, which a borrowing iterator
    /// cannot do.
    pub fn handles_into(&self, out: &mut Vec<Handle>) {
        out.clear();
        out.extend(self.iter().map(|(handle, _)| handle));
    }

    /// Removes every entity for which `keep` returns false.
    pub fn retain(&mut self, mut keep: impl FnMut(Handle, &mut T) -> bool) {
        for index in 0..self.slots.len() {
            let slot = &mut self.slots[index];
            let generation = slot.generation;
            let Some(value) = slot.value.as_mut() else {
                continue;
            };

            if keep(Handle::new(index as u16, generation), value) {
                continue;
            }

            slot.value = None;
            slot.generation = generation.wrapping_add(1);
            self.free.push(index as u16);
            self.live -= 1;
        }
    }

    pub fn clear(&mut self) {
        self.slots.clear();
        self.free.clear();
        self.live = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_resolves_to_what_it_named() {
        let mut slab = Slab::new();
        let first = slab.insert("goblin").unwrap();
        let second = slab.insert("archer").unwrap();

        assert_eq!(slab.get(first), Some(&"goblin"));
        assert_eq!(slab.get(second), Some(&"archer"));
        assert_eq!(slab.len(), 2);
    }

    #[test]
    fn a_stale_handle_does_not_resolve_to_the_new_occupant() {
        // This is the whole reason generations exist. The C# server's plain integer key would have
        // resolved the dead handle to the living entity that took its slot.
        let mut slab = Slab::new();
        let dead = slab.insert("first").unwrap();

        assert_eq!(slab.remove(dead), Some("first"));

        let reused = slab.insert("second").unwrap();
        assert_eq!(
            reused.index(),
            dead.index(),
            "the slot should have been recycled"
        );
        assert_ne!(reused, dead, "but the handle must differ");

        assert_eq!(slab.get(dead), None, "a stale handle must miss");
        assert_eq!(slab.get(reused), Some(&"second"));
        assert!(!slab.contains(dead));
    }

    #[test]
    fn removing_twice_is_harmless() {
        let mut slab = Slab::new();
        let handle = slab.insert(1).unwrap();

        assert_eq!(slab.remove(handle), Some(1));
        assert_eq!(slab.remove(handle), None);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn a_handle_from_another_slab_does_not_resolve_by_luck() {
        let mut slab = Slab::new();
        let handle = slab.insert("real").unwrap();

        // Far outside anything allocated.
        assert_eq!(slab.get(Handle(0xffff_0001)), None);
        assert_eq!(slab.get(Handle::NONE), None);
        assert!(Handle::NONE.is_none());
        assert!(slab.contains(handle));
    }

    #[test]
    fn slots_are_recycled_rather_than_growing() {
        let mut slab = Slab::new();

        let handles: Vec<Handle> = (0..100).map(|n| slab.insert(n).unwrap()).collect();
        assert_eq!(slab.capacity(), 100);

        for handle in &handles {
            slab.remove(*handle);
        }
        assert_eq!(slab.len(), 0);

        for n in 0..100 {
            slab.insert(n).unwrap();
        }
        assert_eq!(
            slab.capacity(),
            100,
            "reusing freed slots should not allocate more"
        );
        assert_eq!(slab.len(), 100);
    }

    #[test]
    fn iteration_visits_every_living_entity_and_no_holes() {
        let mut slab = Slab::new();
        let handles: Vec<Handle> = (0..10).map(|n| slab.insert(n).unwrap()).collect();

        // Punch holes.
        slab.remove(handles[2]);
        slab.remove(handles[5]);
        slab.remove(handles[9]);

        let seen: Vec<i32> = slab.iter().map(|(_, value)| *value).collect();
        assert_eq!(seen, vec![0, 1, 3, 4, 6, 7, 8]);
        assert_eq!(slab.len(), 7);

        // And the handles iteration yields must still resolve.
        for (handle, _) in slab.iter() {
            assert!(slab.contains(handle));
        }
    }

    #[test]
    fn mutation_through_a_handle_sticks() {
        let mut slab = Slab::new();
        let handle = slab.insert(10).unwrap();

        *slab.get_mut(handle).unwrap() += 5;
        assert_eq!(slab.get(handle), Some(&15));

        for (_, value) in slab.iter_mut() {
            *value *= 2;
        }
        assert_eq!(slab.get(handle), Some(&30));
    }

    #[test]
    fn retain_frees_slots_and_invalidates_their_handles() {
        let mut slab = Slab::new();
        let handles: Vec<Handle> = (0..10).map(|n| slab.insert(n).unwrap()).collect();

        slab.retain(|_, value| *value % 2 == 0);

        assert_eq!(slab.len(), 5);
        assert!(slab.contains(handles[0]));
        assert!(!slab.contains(handles[1]), "an odd entry should be gone");

        // The freed slots are reusable, and reuse invalidates the old handle.
        let fresh = slab.insert(99).unwrap();
        assert!(slab.contains(fresh));
        assert!(!slab.contains(handles[1]));
    }

    #[test]
    fn handles_survive_a_round_trip_through_the_wire_form() {
        let mut slab = Slab::new();
        let handle = slab.insert("player").unwrap();

        let on_the_wire = handle.to_entity_id();
        assert_eq!(Handle::from_entity_id(on_the_wire), handle);
        assert_eq!(
            slab.get(Handle::from_entity_id(on_the_wire)),
            Some(&"player")
        );
    }

    #[test]
    fn the_world_refuses_to_exceed_what_a_handle_can_address() {
        let mut slab = Slab::with_capacity(4);
        for n in 0..MAX_ENTITIES {
            assert!(slab.insert(n).is_some(), "should fit {n}");
        }
        assert_eq!(slab.len(), MAX_ENTITIES);
        assert!(
            slab.insert(0).is_none(),
            "one past the limit must be refused, not silently wrapped"
        );
    }

    #[test]
    fn a_generation_wrap_is_the_only_way_a_stale_handle_can_alias() {
        let mut slab = Slab::new();
        let first = slab.insert(0).unwrap();
        let stale = first;

        // Cycle the slot all the way around. Each remove-and-reinsert advances the generation by
        // one, so returning to where it started takes a full 2^16 of them.
        let mut handle = first;
        for n in 1..=(u16::MAX as u32 + 1) {
            slab.remove(handle);
            handle = slab.insert(n as i32).unwrap();
        }

        assert_eq!(
            handle, stale,
            "after 65,536 reuses the handle repeats, which is the documented limit"
        );
    }
}
