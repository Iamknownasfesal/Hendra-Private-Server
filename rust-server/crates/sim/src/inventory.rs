//! Containers, and moving items between them.
//!
//! # Every move is read, validate, write
//!
//! Duplication bugs almost always come from a move that gets halfway: an item added to the
//! destination before it is removed from the source, or a failure partway through that leaves it in
//! both. The C# server guarded against that with a hand-rolled transaction object over several
//! containers, and the guarantee lived in whether every call site remembered to use it.
//!
//! Here it is structural. A move reads both slots into local values, decides whether it is allowed,
//! and only then writes both. There is no window in which an item exists twice, because between the
//! read and the write nothing is stored anywhere, and no call site can opt out of that, because it
//! is the only way to move an item.
//!
//! Items are [`ObjectType`], which is `Copy` and sixteen bits wide. That is what makes the pattern
//! free rather than merely correct.

use hendra_content::{Catalog, ObjectType};

/// What a container is for, which decides what may be taken out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    /// A player's backpack.
    Inventory,

    /// A player's worn equipment, whose slots are typed.
    Equipment,

    /// A loot bag on the ground.
    Bag,

    /// Persistent storage.
    Vault,

    /// A shop's stock, or a gift chest. Items may be taken but not put back.
    ///
    /// The original calls this a `OneWayContainer`, and the gift chest is one: a gift is something
    /// the server put there, and a chest you could also put things into would be extra vault space
    /// that nobody paid for.
    Merchant,
}

impl ContainerKind {
    /// Whether an item may be placed into this kind of container.
    pub fn accepts_deposits(self) -> bool {
        !matches!(self, ContainerKind::Merchant)
    }
}

/// One slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub item: ObjectType,

    /// The item type this slot accepts, or `None` for any.
    ///
    /// Equipment slots are typed: a wand does not go in the armour slot. Backpack slots are not.
    pub accepts: Option<i32>,
}

impl Slot {
    pub const EMPTY: Slot = Slot {
        item: ObjectType::NONE,
        accepts: None,
    };

    pub fn typed(accepts: i32) -> Slot {
        Slot {
            item: ObjectType::NONE,
            accepts: Some(accepts),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.item.is_none()
    }
}

/// A set of slots.
#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    pub kind: ContainerKind,
    slots: Vec<Slot>,
}

impl Container {
    /// A container of untyped slots.
    pub fn new(kind: ContainerKind, size: usize) -> Container {
        Container {
            kind,
            slots: vec![Slot::EMPTY; size],
        }
    }

    /// Equipment, whose slots each accept one item type.
    pub fn equipment(accepts: &[i32]) -> Container {
        Container {
            kind: ContainerKind::Equipment,
            slots: accepts.iter().map(|kind| Slot::typed(*kind)).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<Slot> {
        self.slots.get(index).copied()
    }

    pub fn item(&self, index: usize) -> ObjectType {
        self.slots
            .get(index)
            .map(|slot| slot.item)
            .unwrap_or(ObjectType::NONE)
    }

    pub fn set(&mut self, index: usize, item: ObjectType) -> bool {
        match self.slots.get_mut(index) {
            Some(slot) => {
                slot.item = item;
                true
            }
            None => false,
        }
    }

    /// How many slots hold something.
    pub fn occupied(&self) -> usize {
        self.slots.iter().filter(|slot| !slot.is_empty()).count()
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, ObjectType)> {
        self.slots
            .iter()
            .enumerate()
            .map(|(index, slot)| (index, slot.item))
    }

    /// The first empty slot that would accept an item.
    pub fn room_for(&self, item: ObjectType, catalog: &Catalog) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.is_empty() && slot_accepts(slot, item, catalog))
    }

    /// Puts an item in the first slot that will take it.
    pub fn insert(&mut self, item: ObjectType, catalog: &Catalog) -> Option<usize> {
        let index = self.room_for(item, catalog)?;
        self.slots[index].item = item;
        Some(index)
    }
}

/// Whether a slot will take an item.
fn slot_accepts(slot: &Slot, item: ObjectType, catalog: &Catalog) -> bool {
    if item.is_none() {
        return true;
    }
    let Some(wanted) = slot.accepts else {
        return true;
    };
    catalog
        .object(item)
        .and_then(|desc| desc.item.as_ref())
        .is_some_and(|desc| desc.slot_type == wanted)
}

/// Why a move was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MoveError {
    #[error("no such slot")]
    NoSuchSlot,

    #[error("that slot does not take this item")]
    WrongSlotType,

    #[error("that container does not accept deposits")]
    NoDeposits,

    #[error("this item is soulbound and cannot leave your possession")]
    Soulbound,

    #[error("moving an item onto itself")]
    SameSlot,
}

/// Moves or swaps the contents of two slots.
///
/// The two containers are passed separately because a move usually spans two; see
/// [`swap_within`] for the case where it does not.
///
/// Nothing is written until everything has been checked, so a refusal leaves both containers
/// exactly as they were and a success moves each item exactly once.
pub fn swap(
    left: &mut Container,
    left_index: usize,
    right: &mut Container,
    right_index: usize,
    catalog: &Catalog,
) -> Result<(), MoveError> {
    // Read.
    let Some(from) = left.get(left_index) else {
        return Err(MoveError::NoSuchSlot);
    };
    let Some(to) = right.get(right_index) else {
        return Err(MoveError::NoSuchSlot);
    };

    // Validate. Every check happens here, before anything is written.
    check(left, right, from.item, catalog)?;
    check(right, left, to.item, catalog)?;

    if !slot_accepts(&to, from.item, catalog) || !slot_accepts(&from, to.item, catalog) {
        return Err(MoveError::WrongSlotType);
    }

    // Write. Both, unconditionally.
    left.set(left_index, to.item);
    right.set(right_index, from.item);
    Ok(())
}

/// Moves or swaps two slots of one container.
pub fn swap_within(
    container: &mut Container,
    left_index: usize,
    right_index: usize,
    catalog: &Catalog,
) -> Result<(), MoveError> {
    if left_index == right_index {
        return Err(MoveError::SameSlot);
    }

    let Some(from) = container.get(left_index) else {
        return Err(MoveError::NoSuchSlot);
    };
    let Some(to) = container.get(right_index) else {
        return Err(MoveError::NoSuchSlot);
    };

    if !slot_accepts(&to, from.item, catalog) || !slot_accepts(&from, to.item, catalog) {
        return Err(MoveError::WrongSlotType);
    }

    container.set(left_index, to.item);
    container.set(right_index, from.item);
    Ok(())
}

/// Whether an item may leave `from` and enter `into`.
fn check(
    from: &Container,
    into: &Container,
    item: ObjectType,
    catalog: &Catalog,
) -> Result<(), MoveError> {
    if item.is_none() {
        return Ok(());
    }

    if !into.kind.accepts_deposits() {
        return Err(MoveError::NoDeposits);
    }

    // Soulbound items may be worn and stored, but never handed to anyone else. The check is on
    // where it is going rather than where it came from, because that is the rule: a bag on the
    // ground is how an item reaches another player.
    let soulbound = catalog
        .object(item)
        .and_then(|desc| desc.item.as_ref())
        .is_some_and(|desc| desc.soulbound);

    if soulbound && matches!(into.kind, ContainerKind::Bag) {
        return Err(MoveError::Soulbound);
    }

    let _ = from;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"<Objects>
        <Object type="0x900" id="Wand">
          <Class>Equipment</Class><Item/><SlotType>8</SlotType>
        </Object>
        <Object type="0x901" id="Robe">
          <Class>Equipment</Class><Item/><SlotType>14</SlotType>
        </Object>
        <Object type="0x902" id="Potion">
          <Class>Equipment</Class><Item/><SlotType>0</SlotType><Consumable/>
        </Object>
        <Object type="0x903" id="Heirloom">
          <Class>Equipment</Class><Item/><SlotType>8</SlotType><Soulbound/>
        </Object>
      </Objects>"#;

    const WAND: ObjectType = ObjectType(0x900);
    const ROBE: ObjectType = ObjectType(0x901);
    const POTION: ObjectType = ObjectType(0x902);
    const HEIRLOOM: ObjectType = ObjectType(0x903);

    fn catalog() -> Catalog {
        Catalog::load_str(&[FIXTURE]).0
    }

    fn backpack(items: &[ObjectType]) -> Container {
        let mut container = Container::new(ContainerKind::Inventory, 8);
        for (index, item) in items.iter().enumerate() {
            container.set(index, *item);
        }
        container
    }

    #[test]
    fn an_item_moves_into_an_empty_slot() {
        let catalog = catalog();
        let mut from = backpack(&[WAND]);
        let mut to = backpack(&[]);

        swap(&mut from, 0, &mut to, 3, &catalog).unwrap();

        assert!(from.item(0).is_none(), "it left the source");
        assert_eq!(to.item(3), WAND, "and arrived at the destination");
    }

    #[test]
    fn two_items_swap_places() {
        let catalog = catalog();
        let mut left = backpack(&[WAND]);
        let mut right = backpack(&[ROBE]);

        swap(&mut left, 0, &mut right, 0, &catalog).unwrap();

        assert_eq!(left.item(0), ROBE);
        assert_eq!(right.item(0), WAND);
    }

    #[test]
    fn nothing_is_ever_in_two_places() {
        // The property the whole design exists for. Every move is checked for it, including the
        // ones that fail.
        let catalog = catalog();
        let mut left = backpack(&[WAND, ROBE, POTION]);
        let mut right = Container::equipment(&[8, 14]);

        let count = |left: &Container, right: &Container, item: ObjectType| {
            left.iter().filter(|(_, held)| *held == item).count()
                + right.iter().filter(|(_, held)| *held == item).count()
        };

        for (a, b) in [(0usize, 0usize), (1, 1), (2, 0), (0, 1), (2, 1), (1, 0)] {
            let _ = swap(&mut left, a, &mut right, b, &catalog);

            for item in [WAND, ROBE, POTION] {
                assert!(
                    count(&left, &right, item) <= 1,
                    "{item:?} exists more than once after moving slot {a} to {b}"
                );
            }
        }
    }

    #[test]
    fn a_refused_move_changes_nothing() {
        let catalog = catalog();
        let mut backpack = backpack(&[WAND]);
        let mut equipment = Container::equipment(&[14]); // armour only

        let before = (backpack.clone(), equipment.clone());
        let outcome = swap(&mut backpack, 0, &mut equipment, 0, &catalog);

        assert_eq!(outcome, Err(MoveError::WrongSlotType));
        assert_eq!(backpack, before.0, "the source is untouched");
        assert_eq!(equipment, before.1, "and so is the destination");
    }

    #[test]
    fn equipment_slots_only_take_what_they_are_for() {
        let catalog = catalog();
        let mut backpack = backpack(&[WAND, ROBE]);
        let mut equipment = Container::equipment(&[8, 14]);

        swap(&mut backpack, 0, &mut equipment, 0, &catalog).expect("a wand in the wand slot");
        assert_eq!(equipment.item(0), WAND);

        let outcome = swap(&mut backpack, 1, &mut equipment, 0, &catalog);
        assert_eq!(
            outcome,
            Err(MoveError::WrongSlotType),
            "a robe is not a wand"
        );

        swap(&mut backpack, 1, &mut equipment, 1, &catalog).expect("a robe in the armour slot");
        assert_eq!(equipment.item(1), ROBE);
    }

    #[test]
    fn a_soulbound_item_cannot_be_put_in_a_bag() {
        // A bag on the ground is how an item reaches another player, so that is where the check is.
        let catalog = catalog();
        let mut backpack = backpack(&[HEIRLOOM]);
        let mut bag = Container::new(ContainerKind::Bag, 8);

        assert_eq!(
            swap(&mut backpack, 0, &mut bag, 0, &catalog),
            Err(MoveError::Soulbound)
        );
        assert_eq!(backpack.item(0), HEIRLOOM, "and it stays where it was");
    }

    #[test]
    fn a_soulbound_item_may_still_be_worn_and_stored() {
        let catalog = catalog();
        let mut backpack = backpack(&[HEIRLOOM]);
        let mut equipment = Container::equipment(&[8]);
        let mut vault = Container::new(ContainerKind::Vault, 8);

        swap(&mut backpack, 0, &mut equipment, 0, &catalog).expect("worn");
        swap(&mut equipment, 0, &mut vault, 0, &catalog).expect("stored");
        assert_eq!(vault.item(0), HEIRLOOM);
    }

    #[test]
    fn a_merchant_gives_but_does_not_take() {
        let catalog = catalog();
        let mut shop = Container::new(ContainerKind::Merchant, 4);
        shop.set(0, WAND);
        let mut backpack = backpack(&[ROBE]);

        // Taking from the shop into an empty slot is fine.
        swap(&mut shop, 0, &mut backpack, 1, &catalog).expect("buying");
        assert_eq!(backpack.item(1), WAND);

        // Putting something back is not.
        assert_eq!(
            swap(&mut backpack, 0, &mut shop, 0, &catalog),
            Err(MoveError::NoDeposits)
        );
    }

    #[test]
    fn moving_within_one_container_works_and_refuses_itself() {
        let catalog = catalog();
        let mut backpack = backpack(&[WAND, ROBE]);

        swap_within(&mut backpack, 0, 5, &catalog).unwrap();
        assert!(backpack.item(0).is_none());
        assert_eq!(backpack.item(5), WAND);

        assert_eq!(
            swap_within(&mut backpack, 1, 1, &catalog),
            Err(MoveError::SameSlot)
        );
    }

    #[test]
    fn a_slot_that_does_not_exist_is_refused() {
        let catalog = catalog();
        let mut left = backpack(&[WAND]);
        let mut right = backpack(&[]);

        assert_eq!(
            swap(&mut left, 99, &mut right, 0, &catalog),
            Err(MoveError::NoSuchSlot)
        );
        assert_eq!(
            swap(&mut left, 0, &mut right, 99, &catalog),
            Err(MoveError::NoSuchSlot)
        );
        assert_eq!(left.item(0), WAND, "and nothing moved");
    }

    #[test]
    fn inserting_finds_the_first_slot_that_fits() {
        let catalog = catalog();
        let mut equipment = Container::equipment(&[14, 8, 8]);

        assert_eq!(
            equipment.insert(WAND, &catalog),
            Some(1),
            "the first wand slot"
        );
        assert_eq!(equipment.insert(WAND, &catalog), Some(2), "then the next");
        assert_eq!(equipment.insert(WAND, &catalog), None, "and then no room");

        assert_eq!(equipment.insert(ROBE, &catalog), Some(0));
        assert_eq!(equipment.occupied(), 3);
    }

    #[test]
    fn a_full_backpack_has_no_room() {
        let catalog = catalog();
        let mut backpack = Container::new(ContainerKind::Inventory, 2);
        assert!(backpack.insert(WAND, &catalog).is_some());
        assert!(backpack.insert(ROBE, &catalog).is_some());
        assert_eq!(backpack.insert(POTION, &catalog), None);
        assert_eq!(backpack.occupied(), 2);
    }

    #[test]
    fn a_long_run_of_random_moves_conserves_every_item() {
        // The strongest statement available without a database: whatever the sequence, the multiset
        // of items across every container is exactly what it started as.
        let catalog = catalog();
        let mut backpack = backpack(&[WAND, ROBE, POTION, HEIRLOOM]);
        let mut equipment = Container::equipment(&[8, 14]);
        let mut vault = Container::new(ContainerKind::Vault, 6);
        let mut bag = Container::new(ContainerKind::Bag, 4);

        let tally = |containers: [&Container; 4]| {
            let mut items: Vec<ObjectType> = containers
                .iter()
                .flat_map(|container| container.iter().map(|(_, item)| item))
                .filter(|item| !item.is_none())
                .collect();
            items.sort();
            items
        };

        let before = tally([&backpack, &equipment, &vault, &bag]);

        let mut seed = 0x5eed_1234u32;
        let mut next = |limit: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as usize) % limit
        };

        for _ in 0..5_000 {
            let a = next(4);
            let b = next(4);
            let (i, j) = (next(8), next(8));

            // Same container, or two different ones.
            if a == b {
                let container = match a {
                    0 => &mut backpack,
                    1 => &mut equipment,
                    2 => &mut vault,
                    _ => &mut bag,
                };
                let _ = swap_within(container, i, j, &catalog);
                continue;
            }

            let outcome = match (a.min(b), a.max(b)) {
                (0, 1) => swap(&mut backpack, i, &mut equipment, j, &catalog),
                (0, 2) => swap(&mut backpack, i, &mut vault, j, &catalog),
                (0, 3) => swap(&mut backpack, i, &mut bag, j, &catalog),
                (1, 2) => swap(&mut equipment, i, &mut vault, j, &catalog),
                (1, 3) => swap(&mut equipment, i, &mut bag, j, &catalog),
                _ => swap(&mut vault, i, &mut bag, j, &catalog),
            };
            let _ = outcome;

            assert_eq!(
                tally([&backpack, &equipment, &vault, &bag]),
                before,
                "an item was created or destroyed"
            );
        }
    }
}
