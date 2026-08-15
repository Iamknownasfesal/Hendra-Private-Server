//! The three ways a player's slot is numbered, and how each turns into the next.
//!
//! # Flat, as the client counts
//!
//! The client lays a character's slots out as one run: worn first with room for eight, carried from
//! eight, and the backpack from sixteen. Those numbers are the original client's, where
//! `NUM_EQUIPMENT_SLOTS` is eight (`Client-Side/src/kabam/rotmg/constants/GeneralConstants.as:5`),
//! the inventory grid is built at offset eight and the backpack grid at sixteen
//! (`ui/panels/itemgrids/InventoryGrid.as:37`, given its offset by
//! `game/view/components/InventoryTabContent.as:29` and `BackpackTabContent.as:28`). A drag names
//! the tile it started on, and a tile's id is that flat number
//! (`ui/panels/mediators/ItemGridMediator.as:202`).
//!
//! # Wire, as [`SlotLocation`] says it
//!
//! Structural: which container, and which slot within that container. A client cannot name another
//! player's pack this way, and the server bounds each container separately.
//!
//! # Durable, as the table holds it
//!
//! One row per occupied slot, worn and carried in the same numbering: four worn and then everything
//! else. The original stored twenty-four slots per character with the client's own numbering
//! (`InventorySize` defaults to 24, `common/resources/AppSettings.cs:69-70`) and indexed them by
//! the flat number the client sent (`wServer/networking/handlers/InvSwapHandler.cs:93-96`,
//! `wServer/realm/entities/player/Player.UseItem.cs:152`), leaving slots four to seven drawn by
//! nothing and typed by nothing (`Player.cs:464-466` resizes a class's four slot types to
//! twenty-four). Ours packs them instead, so the carried run sits four below where the client
//! counts it and the four the client leaves room for are not slots at all.

use crate::message::SlotLocation;

/// How many worn slots a character has.
///
/// Four, which is what every class declares: a weapon, an ability, armour and a ring.
pub const EQUIPPED_SLOTS: u16 = 4;

/// Where the carried run begins in the client's flat numbering.
pub const FIRST_CARRIED_SLOT: u16 = 8;

/// Where the backpack begins in the client's flat numbering.
pub const FIRST_BACKPACK_SLOT: u16 = 16;

/// The flat slot past the last one a character could ever have.
pub const FLAT_SLOTS: u16 = 24;

/// Turns a flat player slot into the place on the wire that names it.
///
/// `None` for the four the client leaves room for and this game does not have, and for anything
/// past the backpack: letting either through would put an item in a carried square by another name.
pub fn located(flat: u16) -> Option<SlotLocation> {
    if flat < EQUIPPED_SLOTS {
        return Some(SlotLocation::Equipment { slot: flat as u8 });
    }

    if !(FIRST_CARRIED_SLOT..FLAT_SLOTS).contains(&flat) {
        return None;
    }

    Some(SlotLocation::Inventory {
        slot: (flat - FIRST_CARRIED_SLOT) as u8,
    })
}

/// Turns a place on the wire into the slot the character's table holds it in.
///
/// `None` for anything that is not one of the character's own slots: a vault, a gift and a bag are
/// all kept elsewhere and numbered on their own.
pub fn durable(at: SlotLocation) -> Option<i16> {
    match at {
        SlotLocation::Equipment { slot } if u16::from(slot) < EQUIPPED_SLOTS => Some(slot as i16),

        // A worn slot beyond what exists is not a slot at all.
        SlotLocation::Equipment { .. } => None,

        // Carried slots sit above the worn ones in the same table.
        SlotLocation::Inventory { slot } => {
            Some((slot as i16).saturating_add(EQUIPPED_SLOTS as i16))
        }

        SlotLocation::Vault { .. }
        | SlotLocation::Gift { .. }
        | SlotLocation::Bag { .. }
        | SlotLocation::Ground => None,
    }
}

/// Turns a flat player slot straight into the durable one it means.
///
/// The two steps above in one, for the places that are handed a flat number and hold the table:
/// the vault panel, which addresses the player's pack as a chest of flat slots, and using an item.
pub fn flat_to_durable(flat: u16) -> Option<i16> {
    durable(located(flat)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worn_slots_keep_their_numbers() {
        // The first four are the same in all three numberings, which is why a bug in the rest of
        // this went unnoticed for as long as it did: abilities and armour worked.
        for flat in 0..EQUIPPED_SLOTS {
            assert_eq!(
                located(flat),
                Some(SlotLocation::Equipment { slot: flat as u8 })
            );
            assert_eq!(flat_to_durable(flat), Some(flat as i16));
        }
    }

    #[test]
    fn the_carried_run_starts_at_eight_and_lands_on_four() {
        // The one that matters: the first square of the pack the player drags in is flat eight, and
        // the table holds it at four.
        assert_eq!(located(8), Some(SlotLocation::Inventory { slot: 0 }));
        assert_eq!(flat_to_durable(8), Some(4));
        assert_eq!(located(15), Some(SlotLocation::Inventory { slot: 7 }));
        assert_eq!(flat_to_durable(15), Some(11));
    }

    #[test]
    fn the_backpack_carries_on_where_the_pack_stops() {
        assert_eq!(flat_to_durable(16), Some(12));
        assert_eq!(flat_to_durable(23), Some(19));
    }

    #[test]
    fn the_room_left_for_worn_slots_we_do_not_have_is_not_a_slot() {
        for flat in EQUIPPED_SLOTS..FIRST_CARRIED_SLOT {
            assert_eq!(located(flat), None, "flat {flat}");
            assert_eq!(flat_to_durable(flat), None, "flat {flat}");
        }
    }

    #[test]
    fn nothing_past_the_backpack_is_addressable() {
        // Which is what keeps the stack slots -- 254 and 255 -- from being read as squares in the
        // pack by a path that took them for ordinary slots.
        for flat in [FLAT_SLOTS, 100, 254, 255, u16::MAX] {
            assert_eq!(located(flat), None, "flat {flat}");
        }
    }

    #[test]
    fn every_flat_slot_maps_to_a_durable_one_and_no_two_share() {
        // The property the bug broke: the mapping is injective across the whole run, so no drag can
        // ever land on the square another drag owns.
        let mut seen = Vec::new();
        for flat in 0..FLAT_SLOTS {
            if let Some(durable) = flat_to_durable(flat) {
                assert!(!seen.contains(&durable), "flat {flat} collides");
                seen.push(durable);
            }
        }
        assert_eq!(seen.len(), 20, "four worn, eight carried, eight backpack");
    }

    #[test]
    fn what_is_not_the_players_own_pack_has_no_durable_slot() {
        assert_eq!(durable(SlotLocation::Vault { slot: 0 }), None);
        assert_eq!(durable(SlotLocation::Gift { slot: 0 }), None);
        assert_eq!(durable(SlotLocation::Ground), None);
        assert_eq!(
            durable(SlotLocation::Bag {
                entity: crate::entity::EntityId(7),
                slot: 0,
            }),
            None
        );
        assert_eq!(durable(SlotLocation::Equipment { slot: 4 }), None);
    }
}
