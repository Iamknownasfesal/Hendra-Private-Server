//! Making characters, and deciding which classes may be made.
//!
//! This sits between the content and the database because both servers need it and neither owns
//! it. The app server needs it to draw a character-select screen and to create from one; the game
//! server needs it because a player who connects with no living character is given one, and an
//! unlock enforced only at the front door is one a direct connection walks past.
//!
//! It lives in one place because two implementations of "which classes are open" is how a
//! class ends up playable through one route and locked through the other.

use std::collections::HashMap;

use hendra_content::player::Locked;
use hendra_content::{Catalog, ObjectType};
use hendra_store::{Character, Store, StoreError};

/// What every class carries on top of the gear its own slots decide.
#[derive(Debug, Clone)]
pub struct CommonItems {
    /// Catalog ids, in the order they should be carried.
    pub names: Vec<String>,
}

impl CommonItems {
    pub fn new<I, S>(names: I) -> CommonItems
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        CommonItems {
            names: names.into_iter().map(Into::into).collect(),
        }
    }
}

/// How many slots are worn rather than carried.
///
/// Weapon, ability, armour and ring, the four the game has always had. Everything past them is the
/// backpack, held in the same table so a move between them is one statement.
pub const EQUIPPED_SLOTS: i16 = 4;

/// One class, as a character-select screen needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassOffer {
    pub object_type: ObjectType,
    pub id: String,
    pub description: Option<String>,
    pub starting_hp: i32,
    pub starting_mp: i32,

    /// `None` when the class can be played now.
    pub locked: Option<Locked>,

    /// What it costs to unlock outright, when there is a price.
    pub cost: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    #[error("no such class")]
    NoSuchClass,

    #[error("that class is locked")]
    Locked(Locked),

    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Every class, with whether this account may play it.
///
/// One database round trip for all of them rather than one each: deciding what to show on a
/// character-select screen is a single question.
pub async fn offers(
    store: &Store,
    catalog: &Catalog,
    account_id: i64,
) -> Result<Vec<ClassOffer>, StoreError> {
    let unlocks = Unlocks::load(store, account_id).await?;

    Ok(catalog
        .classes()
        .iter()
        .map(|class| {
            let desc = catalog.object(class.object_type);
            ClassOffer {
                object_type: class.object_type,
                id: desc.map(|d| d.id.clone()).unwrap_or_default(),
                description: desc.and_then(|d| d.display_id.clone()),
                starting_hp: class.starting_hp(),
                starting_mp: class.starting_mp(),
                locked: unlocks.locked(catalog, class),
                cost: class.unlock.cost,
            }
        })
        .collect())
}

/// What an account has unlocked, loaded once and asked many times.
pub struct Unlocks {
    /// Keyed by identity rather than runtime number, because this outlives a load.
    progress: HashMap<uuid::Uuid, (i16, i32)>,
    purchased: Vec<uuid::Uuid>,
}

impl Unlocks {
    pub async fn load(store: &Store, account_id: i64) -> Result<Unlocks, StoreError> {
        Ok(Unlocks {
            progress: store.class_progress(account_id).await?,
            purchased: store.purchased_classes(account_id).await?,
        })
    }

    /// How many stars this account has earned.
    ///
    /// Every class contributes what its best fame is worth, which is why it is a record of an
    /// account rather than of whichever character is being looked at: somebody standing there on a
    /// new wizard still has the five stars their knight earned.
    pub fn stars(&self) -> u8 {
        let total: i32 = self
            .progress
            .values()
            .map(|(_, best_fame)| hendra_sim::leveling::stars(*best_fame))
            .sum();

        total.clamp(0, u8::MAX as i32) as u8
    }

    /// Why this class cannot be played, if it cannot.
    ///
    /// Takes the catalog because progress is keyed by identity and the unlock names a runtime
    /// number: the two have to be brought together somewhere, and here is the only place that has
    /// both.
    pub fn locked(&self, catalog: &Catalog, class: &hendra_content::PlayerDesc) -> Option<Locked> {
        let best_level = |needed: ObjectType| {
            catalog
                .object(needed)
                .and_then(|desc| self.progress.get(&desc.uuid))
                .map(|(level, _)| *level as i32)
                .unwrap_or(0)
        };

        let purchased: Vec<ObjectType> = self
            .purchased
            .iter()
            .filter_map(|bought| catalog.type_of_uuid(*bought))
            .collect();

        class.locked_for(&best_level, &purchased)
    }
}

/// The class a new account starts on.
///
/// The first class the content offers for free rather than a name written here, so adding or
/// reordering classes is a content change. In the shipped files that is the wizard.
pub fn default_class(catalog: &Catalog) -> Option<ObjectType> {
    catalog
        .classes()
        .iter()
        .find(|class| class.unlock.free())
        .or_else(|| catalog.classes().first())
        .map(|class| class.object_type)
}

/// Makes a character, if the account may play that class.
pub async fn create(
    store: &Store,
    catalog: &Catalog,
    common: &CommonItems,
    account_id: i64,
    class: ObjectType,
    name: &str,
) -> Result<Character, CreateError> {
    let desc = catalog.class(class).ok_or(CreateError::NoSuchClass)?;
    let desc_uuid = catalog
        .object(class)
        .map(|object| object.uuid)
        .ok_or(CreateError::NoSuchClass)?;

    let unlocks = Unlocks::load(store, account_id).await?;
    if let Some(locked) = unlocks.locked(catalog, desc) {
        return Err(CreateError::Locked(locked));
    }

    // Health comes from the class, so a warrior is not a wizard with a different sprite.
    let max_hp = desc.starting_hp().max(1);
    let character = store
        .create_character(account_id, desc_uuid, name, max_hp)
        .await?;

    let mut slots = starting_slots(catalog, desc, common);
    slots.sort_by_key(|(slot, _)| *slot);

    if !slots.is_empty() {
        store.set_inventory(character.id, &slots).await?;
    }

    Ok(store.character(character.id).await?)
}

/// What a fresh character of this class holds.
///
/// Nothing is written down: every class in the files lists its equipment as empty, so a character
/// built from them alone would arrive with nothing to shoot. The lowest tier that fits each worn
/// slot is what the game has always handed out, and deriving it means a new class needs no new
/// configuration to be playable.
pub fn starting_slots(
    catalog: &Catalog,
    class: &hendra_content::PlayerDesc,
    common: &CommonItems,
) -> Vec<(i16, uuid::Uuid)> {
    let mut slots: Vec<(i16, uuid::Uuid)> = Vec::new();

    for worn in 0..EQUIPPED_SLOTS as usize {
        // A class listing fewer worn slots than four gets fewer, rather than a slot filled with
        // whatever happened to match nothing.
        let Some(slot_type) = class.slot_type(worn) else {
            continue;
        };
        let Some(object_type) = catalog.lowest_tier_for_slot(slot_type) else {
            continue;
        };
        let Some(item) = catalog.object(object_type) else {
            continue;
        };
        slots.push((worn as i16, item.uuid));
    }

    let mut next_carried = EQUIPPED_SLOTS;
    for name in &common.names {
        // Anything the catalog does not have is skipped rather than fatal: a half-converted content
        // directory should still let someone play.
        let Some(item) = catalog
            .type_of(name)
            .and_then(|found| catalog.object(found))
        else {
            tracing::warn!(item = %name, "common kit names an item the catalog does not have");
            continue;
        };

        slots.push((next_carried, item.uuid));
        next_carried += 1;
    }

    slots
}

/// Whether a character may put an item in one of its own slots.
///
/// The class decides. A wizard's first slot takes staves and wands; a warrior's takes swords. The
/// old server did not check this at all, so a wizard could equip a sword and shoot with it, which
/// is not a balance problem so much as fourteen classes quietly collapsing into one.
///
/// Slots past the worn four are the backpack and take anything. An empty item is always allowed,
/// that is not putting something somewhere, it is taking it away.
pub fn slot_accepts(
    catalog: &Catalog,
    class: &hendra_content::PlayerDesc,
    slot: i16,
    item: ObjectType,
) -> bool {
    if item.is_none() || slot >= EQUIPPED_SLOTS {
        return slot >= 0;
    }

    let Some(wanted) = class.slot_type(slot as usize) else {
        return false;
    };

    catalog
        .object(item)
        .and_then(|desc| desc.item.as_ref())
        .is_some_and(|item| item.slot_type == wanted)
}

/// Whether an item may be activated from the slot it is sitting in.
///
/// Follows the last line of `Player.UseItem`, which activates an item only when it is consumable or
/// the slot it is in is the kind of slot it belongs to. Without it a player carries four tomes in
/// the pack and uses each in turn, which is four abilities rather than one.
///
/// Consumables are exempt, as they are in the original: drinking a potion out of the pack is how a
/// potion is drunk. So is anything with no slot type of its own, which is what a carried item is.
pub fn activates_from_slot(
    catalog: &Catalog,
    class: &hendra_content::PlayerDesc,
    slot: i16,
    item: ObjectType,
) -> bool {
    let Some(desc) = catalog.object(item).and_then(|desc| desc.item.as_ref()) else {
        return false;
    };

    if desc.consumable || desc.slot_type == 0 {
        return true;
    }

    slot < EQUIPPED_SLOTS && slot_accepts(catalog, class, slot, item)
}

/// Records how far a character got, so its class counts toward unlocking the next.
///
/// Called when a character dies or is saved rather than only at death: an account whose best
/// warrior is still alive has still levelled a warrior, and making them die for it would be a rule
/// nobody expects.
pub async fn record_progress(
    store: &Store,
    account_id: i64,
    character: &Character,
) -> Result<(), StoreError> {
    store
        .record_class_progress(account_id, character.class, character.level, character.fame)
        .await
}

#[cfg(test)]
mod slots {
    use super::*;

    /// A wizard, a tome that only fits the ability slot, and a potion that fits no worn slot at
    /// all.
    ///
    /// The slot types are the ones the content actually uses, which matters for the potion: every
    /// consumable in the files has a slot type of its own, so a potion is exempt because it is
    /// consumable and not because it fits nowhere.
    const CONTENT: &str = r#"<Objects>
        <Object id="Wizard" type="0x030e">
            <Class>Wizard</Class>
            <Player/>
            <SlotTypes>1, 4, 6, 9</SlotTypes>
            <Equipment>-1, -1, -1, -1</Equipment>
        </Object>
        <Object id="Tome of Purification">
            <Class>Equipment</Class><Item/><SlotType>4</SlotType>
        </Object>
        <Object id="Wand of Sparks">
            <Class>Equipment</Class><Item/><SlotType>1</SlotType>
        </Object>
        <Object id="Health Potion">
            <Class>Equipment</Class><Item/><SlotType>10</SlotType><Consumable/>
        </Object>
    </Objects>"#;

    fn content() -> Catalog {
        Catalog::load_str(&[CONTENT]).0
    }

    fn wizard(catalog: &Catalog) -> &hendra_content::PlayerDesc {
        let kind = catalog.type_of("Wizard").expect("the class");
        catalog.class(kind).expect("the class description")
    }

    #[test]
    fn an_ability_works_from_the_slot_it_is_worn_in() {
        let catalog = content();
        let tome = catalog.type_of("Tome of Purification").expect("the tome");

        assert!(activates_from_slot(&catalog, wizard(&catalog), 1, tome));
    }

    #[test]
    fn an_ability_carried_in_the_pack_does_nothing() {
        // Or a player carries four tomes and uses each in turn, which is four abilities rather
        // than one.
        let catalog = content();
        let tome = catalog.type_of("Tome of Purification").expect("the tome");

        assert!(!activates_from_slot(&catalog, wizard(&catalog), 5, tome));
        assert!(!activates_from_slot(&catalog, wizard(&catalog), 11, tome));
    }

    #[test]
    fn an_ability_in_the_wrong_worn_slot_does_nothing_either() {
        let catalog = content();
        let tome = catalog.type_of("Tome of Purification").expect("the tome");

        assert!(!activates_from_slot(&catalog, wizard(&catalog), 0, tome));
        assert!(!activates_from_slot(&catalog, wizard(&catalog), 2, tome));
        assert!(!activates_from_slot(&catalog, wizard(&catalog), 3, tome));
    }

    #[test]
    fn a_potion_is_drunk_from_wherever_it_is() {
        // The exemption the original has: drinking out of the pack is how a potion is drunk.
        let catalog = content();
        let potion = catalog.type_of("Health Potion").expect("the potion");

        assert!(activates_from_slot(&catalog, wizard(&catalog), 7, potion));
    }

    #[test]
    fn the_rule_makes_nothing_in_the_content_unusable() {
        // The risk in refusing an activation from the pack is refusing one that was fine. Every
        // item the game lets you use from the pack is consumable, and everything else with an
        // activate is equipment: a shield that shoots, a ring that boosts. If that ever stops being
        // true, something usable has quietly become unusable.
        let Ok((catalog, _)) =
            Catalog::load_dir(std::path::Path::new("../../../godot-client/assets/xml"))
        else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        let worn: std::collections::HashSet<i32> = catalog
            .classes()
            .iter()
            .flat_map(|class| (0..EQUIPPED_SLOTS).filter_map(|slot| class.slot_type(slot as usize)))
            .collect();

        let mut checked = 0;
        let mut stranded = Vec::new();
        for number in 0..u16::MAX {
            let Some(desc) = catalog.object(ObjectType(number)) else {
                continue;
            };
            let Some(item) = desc.item.as_ref() else {
                continue;
            };
            if item.activate.is_empty() || item.consumable || item.slot_type == 0 {
                continue;
            }
            checked += 1;

            if !worn.contains(&item.slot_type) {
                stranded.push(desc);
            }
        }

        assert!(checked > 100, "only {checked} items were judged");

        // What is left names the potion slot type and so belongs in no worn slot at all. The
        // original refuses these from the pack by the same arithmetic, so it is the shape of the
        // content rather than a hole this rule opened. All of them summon a pet, which this server
        // does not have; anything else appearing here would be a usable item quietly made unusable.
        for desc in &stranded {
            let item = desc.item.as_ref().expect("an item");
            assert!(
                item.activate.iter().all(|activate| matches!(
                    hendra_content::Effect::of(activate),
                    hendra_content::Effect::Pet { .. }
                )),
                "{} can be activated but no class can wear it",
                desc.id
            );
        }
    }

    #[test]
    fn a_worn_slot_takes_only_what_the_class_wears_in_it() {
        let catalog = content();
        let class = wizard(&catalog);
        let wand = catalog.type_of("Wand of Sparks").expect("the wand");
        let tome = catalog.type_of("Tome of Purification").expect("the tome");

        assert!(slot_accepts(&catalog, class, 0, wand));
        assert!(!slot_accepts(&catalog, class, 0, tome));
        assert!(slot_accepts(&catalog, class, 1, tome));
        assert!(!slot_accepts(&catalog, class, 1, wand));

        // A carried slot has no opinion, which is what makes it carried.
        assert!(slot_accepts(&catalog, class, 6, tome));
    }
}

#[cfg(test)]
mod stars {
    use super::*;

    fn earned(best_fames: &[i32]) -> u8 {
        let progress = best_fames
            .iter()
            .enumerate()
            .map(|(index, fame)| (uuid::Uuid::from_u128(index as u128 + 1), (20i16, *fame)))
            .collect();

        Unlocks {
            progress,
            purchased: Vec::new(),
        }
        .stars()
    }

    #[test]
    fn an_account_that_has_done_nothing_has_no_stars() {
        assert_eq!(earned(&[]), 0);
        assert_eq!(earned(&[0, 19]), 0);
    }

    #[test]
    fn every_class_contributes_what_its_best_fame_is_worth() {
        // A record of the account rather than of whichever character is being looked at: somebody
        // standing there on a new wizard still has the five stars their knight earned.
        assert_eq!(earned(&[2000]), 5);
        assert_eq!(earned(&[2000, 800]), 9);
        assert_eq!(earned(&[20, 150, 400, 800, 2000]), 1 + 2 + 3 + 4 + 5);
    }

    #[test]
    fn a_class_counts_once_at_its_best_rather_than_per_character() {
        // The store keeps one row per class holding the best, which is what makes this true. Ten
        // knights that each reached four hundred fame are three stars, not thirty.
        assert_eq!(earned(&[400]), 3);
    }

    #[test]
    fn a_count_too_large_to_show_is_held_at_the_largest_that_fits() {
        // Fourteen classes at five stars is seventy, so this cannot arise from the content as it
        // stands. It is here because the wire field is one byte and the arithmetic is not.
        let many: Vec<i32> = (0..300).map(|_| 2000).collect();
        assert_eq!(earned(&many), u8::MAX);
    }
}
