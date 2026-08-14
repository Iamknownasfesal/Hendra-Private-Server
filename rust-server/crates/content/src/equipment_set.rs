//! Sets of equipment that do something extra when all of them are worn.
//!
//! Follows `EquipmentSetDesc` in the original, and `BoostStatManager.ApplySetBonus`, which is where
//! the checking happens: a set gives nothing for three of its four pieces, and everything for the
//! fourth. That is the whole shape of it, and it is why a set is worth chasing.
//!
//! # Why a set is not just four items
//!
//! Because what it gives is not on any of them. The pieces carry their own stats as usual; the set
//! carries a skin, a bullet and a handful of stat increments that exist nowhere else. An item read
//! on its own can never say what wearing it alongside three others is worth.
//!
//! # Empty slots count
//!
//! A setpiece naming item type `0xFFFF` asks for that slot to be *empty*, which is how the original
//! spells a three-piece set. Reading it as "wear item 65535" would make such a set unwearable.

use crate::xml::Node;

/// The item type that means "nothing in this slot".
pub const EMPTY_SLOT: u16 = 0xFFFF;

/// One piece of a set: which worn slot, and what has to be in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    pub slot: u16,

    /// What must be worn there, or `None` where the slot must be empty.
    pub item: Option<crate::ObjectType>,
}

/// A set of equipment, and what wearing all of it gives.
#[derive(Debug, Clone, PartialEq)]
pub struct EquipmentSet {
    pub set_type: u16,
    pub id: String,

    /// Every slot the set has an opinion about.
    pub pieces: Vec<Piece>,

    /// What it gives when every piece is in place, read as ordinary activates: the same skin
    /// changes and stat increments an item can carry, which is what the original reuses.
    pub gives: Vec<crate::ActivateDesc>,
}

impl EquipmentSet {
    pub fn parse(node: &Node) -> Option<EquipmentSet> {
        let set_type = node
            .attr("type")
            .and_then(crate::xml::parse_int)
            .map(|value| value as u16)?;

        let pieces = node
            .children_named("Setpiece")
            .filter_map(|piece| {
                let slot = piece
                    .attr("slot")
                    .and_then(crate::xml::parse_int)
                    .map(|value| value as u16)?;

                let item = piece
                    .attr("itemtype")
                    .and_then(crate::xml::parse_int)
                    .map(|value| value as u16)?;

                Some(Piece {
                    slot,
                    // `0xFFFF` is how the original spells "this slot must be empty", which is a
                    // three-piece set. Reading it as an item would make one unwearable.
                    item: (item != EMPTY_SLOT).then_some(crate::ObjectType(item)),
                })
            })
            .collect();

        Some(EquipmentSet {
            set_type,
            id: node.attr("id").unwrap_or_default().to_string(),
            pieces,
            gives: node
                .children_named("ActivateOnEquipAll")
                .map(crate::ActivateDesc::parse)
                .collect(),
        })
    }

    /// Whether what somebody is wearing completes this set.
    ///
    /// `worn` answers what is in a slot, or `None` for an empty one. Every piece has to agree,
    /// which is what makes three of four worth nothing.
    pub fn worn_by(&self, worn: &dyn Fn(u16) -> Option<crate::ObjectType>) -> bool {
        !self.pieces.is_empty()
            && self
                .pieces
                .iter()
                .all(|piece| worn(piece.slot) == piece.item)
    }
}

/// Reads every set in a file.
pub fn parse_all(node: &Node) -> Vec<EquipmentSet> {
    node.children_named("EquipmentSet")
        .filter_map(EquipmentSet::parse)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ObjectType;

    const FIXTURE: &str = r#"<EquipmentSets>
        <EquipmentSet type="0x0001" id="Two Piece">
            <Setpiece slot="0" itemtype="0x1111">Equipment</Setpiece>
            <Setpiece slot="1" itemtype="0x2222">Equipment</Setpiece>
            <ActivateOnEquipAll stat="0" amount="50">IncrementStat</ActivateOnEquipAll>
        </EquipmentSet>
        <EquipmentSet type="0x0002" id="Bare Handed">
            <Setpiece slot="0" itemtype="0x1111">Equipment</Setpiece>
            <Setpiece slot="1" itemtype="0xFFFF">Equipment</Setpiece>
            <ActivateOnEquipAll skinType="0x0401" size="70">ChangeSkin</ActivateOnEquipAll>
        </EquipmentSet>
      </EquipmentSets>"#;

    fn sets() -> Vec<EquipmentSet> {
        let document = Node::parse(FIXTURE).expect("the fixture parses");
        parse_all(&document)
    }

    #[test]
    fn a_set_is_read_with_its_pieces_and_what_it_gives() {
        let sets = sets();
        assert_eq!(sets.len(), 2);

        let two = &sets[0];
        assert_eq!(two.id, "Two Piece");
        assert_eq!(two.pieces.len(), 2);
        assert_eq!(two.pieces[0].item, Some(ObjectType(0x1111)));
        assert_eq!(two.gives.len(), 1);
    }

    #[test]
    fn three_of_four_is_worth_nothing() {
        // The whole shape of a set: it gives nothing until the last piece is in place.
        let sets = sets();
        let two = &sets[0];

        let half = |slot: u16| (slot == 0).then_some(ObjectType(0x1111));
        assert!(!two.worn_by(&half));

        let both = |slot: u16| match slot {
            0 => Some(ObjectType(0x1111)),
            1 => Some(ObjectType(0x2222)),
            _ => None,
        };
        assert!(two.worn_by(&both));
    }

    #[test]
    fn a_piece_that_asks_for_an_empty_slot_asks_for_an_empty_slot() {
        // `0xFFFF` is how the original spells a three-piece set. Read as an item it would be a set
        // nobody could ever wear.
        let sets = sets();
        let bare = &sets[1];

        assert_eq!(bare.pieces[1].item, None);

        let holding = |slot: u16| match slot {
            0 => Some(ObjectType(0x1111)),
            1 => Some(ObjectType(0x3333)),
            _ => None,
        };
        assert!(
            !bare.worn_by(&holding),
            "something in the slot completed it"
        );

        let empty = |slot: u16| (slot == 0).then_some(ObjectType(0x1111));
        assert!(bare.worn_by(&empty));
    }

    #[test]
    fn wearing_the_right_things_in_the_wrong_slots_is_not_a_set() {
        let sets = sets();
        let two = &sets[0];

        let swapped = |slot: u16| match slot {
            0 => Some(ObjectType(0x2222)),
            1 => Some(ObjectType(0x1111)),
            _ => None,
        };
        assert!(!two.worn_by(&swapped));
    }

    #[test]
    fn every_set_in_the_content_reads() {
        let Ok(text) = std::fs::read_to_string(
            "../../../godot-client/assets/xml/EmbeddedData_EquipmentSetsCXML.xml",
        ) else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        let document = Node::parse(&text).expect("the content parses");
        let sets = parse_all(&document);

        assert_eq!(sets.len(), 7, "the content ships seven");

        let mut with_stats = 0;
        for set in &sets {
            assert!(!set.pieces.is_empty(), "{} has no pieces", set.id);
            assert!(!set.gives.is_empty(), "{} gives nothing", set.id);

            // Every piece has to name a real slot, or the set can never be completed.
            for piece in &set.pieces {
                assert!(piece.slot < 4, "{} wants slot {}", set.id, piece.slot);
            }

            // `IncrementStat` on a set is a boost rather than the permanent rise it means on a
            // potion, which is what `ApplySetBonus` does with it. Both spellings count here.
            if set.gives.iter().any(|activate| {
                matches!(
                    crate::Effect::of(activate),
                    crate::Effect::IncrementStat { .. } | crate::Effect::StatBoost { .. }
                )
            }) {
                with_stats += 1;
            }
        }

        // Most of them raise stats, which is the part that reaches the simulation. A run where none
        // did would mean the effect names had drifted and nothing was being applied.
        assert!(with_stats >= 5, "only {with_stats} sets raise a stat");
    }
}

#[cfg(test)]
mod applied {

    #[test]
    fn a_real_set_from_the_content_raises_a_real_stat() {
        // The end of the chain: the file parses, the catalog holds it, and what it gives is a stat
        // the simulation understands. A set that parsed and gave nothing would look implemented.
        let Ok((catalog, _)) =
            crate::Catalog::load_dir(std::path::Path::new("../../../godot-client/assets/xml"))
        else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        let sets = catalog.equipment_sets();
        assert_eq!(sets.len(), 7, "the catalog did not keep them");

        let geb = sets
            .iter()
            .find(|set| set.id.contains("Priest of Geb"))
            .expect("the Geb set");

        // Every piece it asks for has to be an item the content actually has, or the set is one
        // nobody can ever complete.
        for piece in &geb.pieces {
            let Some(item) = piece.item else { continue };
            assert!(
                catalog.object(item).is_some(),
                "{} wants an item the content does not have",
                geb.id
            );
        }

        let raised: i32 = geb
            .gives
            .iter()
            .filter_map(|activate| match crate::Effect::of(activate) {
                crate::Effect::IncrementStat { amount, .. } => Some(amount),
                crate::Effect::StatBoost { amount, .. } => Some(amount),
                _ => None,
            })
            .sum();

        assert!(raised > 0, "{} raises nothing", geb.id);
    }

    #[test]
    fn wearing_a_real_set_is_recognised_by_the_catalog() {
        let Ok((catalog, _)) =
            crate::Catalog::load_dir(std::path::Path::new("../../../godot-client/assets/xml"))
        else {
            return;
        };

        let geb = catalog
            .equipment_sets()
            .iter()
            .find(|set| set.id.contains("Priest of Geb"))
            .expect("the Geb set")
            .clone();

        let worn = |slot: u16| {
            geb.pieces
                .iter()
                .find(|piece| piece.slot == slot)
                .and_then(|piece| piece.item)
        };

        let found = catalog.sets_worn(&worn);
        assert!(
            found.iter().any(|set| set.id == geb.id),
            "wearing every piece did not complete it"
        );

        // And three of four is worth nothing, which is the whole shape of a set.
        let nearly = |slot: u16| if slot == 0 { None } else { worn(slot) };
        assert!(
            !catalog
                .sets_worn(&nearly)
                .iter()
                .any(|set| set.id == geb.id),
            "three of four completed it"
        );
    }
}
