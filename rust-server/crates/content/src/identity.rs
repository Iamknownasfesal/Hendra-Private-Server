//! What a piece of content is, independently of how it is numbered.
//!
//! # Three names for one thing
//!
//! A piece of content has a readable name (`id="Wand of Dark Magic"`), a durable identity (a
//! UUID), and a runtime number (`ObjectType`, a `u16`). They are different because they answer
//! different questions.
//!
//! The name is for people. It changes when someone decides a wand should be called something else,
//! and nothing durable may depend on it.
//!
//! The UUID is what a saved inventory and a database row refer to. It never changes, so an item
//! survives being renamed, being renumbered, and being moved between content files.
//!
//! The `ObjectType` is what the simulation and the wire use. It indexes arrays directly and costs
//! two bytes rather than sixteen, which matters when it appears in every snapshot for every entity
//! twenty times a second. It is assigned at load and is not durable.
//!
//! # Why content no longer writes hex
//!
//! The legacy files number everything by hand: `type="0x0a00"`. That means picking a free number
//! before writing an item, keeping a ledger of which are taken, and never reusing one. Content now
//! only needs a name; the number is assigned at load, deterministically, from the identity.
//!
//! An author who wants an identity that survives a rename writes one:
//!
//! ```xml
//! <Object uuid="0f8fad5b-d9cb-469f-a165-70867728950e" id="Wand of Dark Magic">
//! ```
//!
//! and `cargo run -p hendra-content --example new_id` prints a fresh one. Without it, the identity
//! is derived from the name, which is stable across runs and machines but changes if the name does.

use uuid::Uuid;

/// The namespace every derived identity is drawn from.
///
/// A fixed value rather than a random one, so the same name yields the same identity on every
/// machine and every run. Changing it would renumber the entire catalog.
pub const NAMESPACE: Uuid = Uuid::from_bytes([
    0x68, 0x65, 0x6e, 0x64, 0x72, 0x61, 0x2d, 0x63, 0x6f, 0x6e, 0x74, 0x65, 0x6e, 0x74, 0x21, 0x21,
]);

/// The identity of one piece of content.
pub fn identity(written: Option<&str>, name: &str) -> Uuid {
    // An explicit identity wins, and a malformed one falls back rather than refusing: a typo in a
    // UUID should cost that object its rename-safety, not the whole file.
    written
        .and_then(|text| Uuid::parse_str(text.trim()).ok())
        .unwrap_or_else(|| Uuid::new_v5(&NAMESPACE, name.as_bytes()))
}

/// A fresh identity, for an author writing new content.
pub fn fresh() -> Uuid {
    Uuid::new_v4()
}

/// The first runtime number that may be assigned.
///
/// The legacy files number everything below this, so anything assigned here cannot collide with
/// content that still writes its own.
pub const FIRST_ASSIGNED: u16 = 0x1000;

/// Where an identity would like to sit in the runtime numbering.
///
/// Derived from the identity rather than from a counter, so the same content gets the same number
/// whatever order the files are read in. Collisions are resolved by the caller, which is the only
/// part that needs to see the whole catalog.
pub fn preferred_slot(identity: Uuid) -> u16 {
    let bytes = identity.as_bytes();
    let mixed = u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]) ^ u64::from_le_bytes([
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    ]);

    let span = (u16::MAX - FIRST_ASSIGNED) as u64 + 1;
    FIRST_ASSIGNED + (mixed % span) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_identity_is_used_as_written() {
        let written = "0f8fad5b-d9cb-469f-a165-70867728950e";
        assert_eq!(
            identity(Some(written), "anything at all"),
            Uuid::parse_str(written).unwrap()
        );
    }

    #[test]
    fn a_derived_identity_is_the_same_every_time() {
        // Stable across runs and machines, or a saved inventory would stop resolving after a
        // restart.
        assert_eq!(
            identity(None, "Wand of Dark Magic"),
            identity(None, "Wand of Dark Magic")
        );
    }

    #[test]
    fn different_names_derive_different_identities() {
        assert_ne!(identity(None, "Wand of Dark Magic"), identity(None, "Robe"));
    }

    #[test]
    fn a_malformed_identity_costs_that_object_its_rename_safety_and_nothing_else() {
        // A typo in a UUID should not refuse the file. It falls back to the derived one, which is
        // exactly what the object would have had if nothing were written.
        assert_eq!(identity(Some("not-a-uuid"), "Robe"), identity(None, "Robe"));
    }

    #[test]
    fn an_identity_survives_a_rename_and_a_derived_one_does_not() {
        let written = Some("0f8fad5b-d9cb-469f-a165-70867728950e");

        assert_eq!(identity(written, "Old Name"), identity(written, "New Name"));
        assert_ne!(identity(None, "Old Name"), identity(None, "New Name"));
    }

    #[test]
    fn assigned_numbers_stay_clear_of_what_the_legacy_files_use() {
        for name in ["Wand", "Robe", "Slime", "Oryx the Mad God", ""] {
            assert!(preferred_slot(identity(None, name)) >= FIRST_ASSIGNED);
        }
    }

    #[test]
    fn the_preferred_slot_is_a_function_of_the_identity_alone() {
        let one = identity(None, "Wand of Dark Magic");
        assert_eq!(preferred_slot(one), preferred_slot(one));
    }

    #[test]
    fn a_thousand_names_spread_across_the_range_rather_than_clustering() {
        // A hash that clustered would make assignment quadratic through probing, and would do it
        // silently.
        use std::collections::HashSet;

        let slots: HashSet<u16> = (0..1_000)
            .map(|n| preferred_slot(identity(None, &format!("Item {n}"))))
            .collect();

        assert!(
            slots.len() > 950,
            "only {} distinct slots for a thousand names",
            slots.len()
        );
    }

    #[test]
    fn a_fresh_identity_is_not_the_same_twice() {
        assert_ne!(fresh(), fresh());
    }
}
