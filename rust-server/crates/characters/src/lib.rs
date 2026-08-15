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

/// Whether this install hands every class to every account.
///
/// `NewAccounts/ClassesUnlocked` in the original's settings, which ships as `1`
/// (`XmlDatas/data/init.xml:34`). `Database.Verify` (`common/Database.cs:140-145`) acts on it at
/// every single login, writing a class-stats row for every class in the game; registration
/// (`:406-409`) and guest creation (`:104-111`) do the same. An account on such an install
/// therefore has an entry — and so an unlock — for every class before it has played anything,
/// which is why that server's character list offers all fourteen as `unrestricted` and reports a
/// `<ClassStats>` for each.
///
/// Read from the environment because the content here carries no settings file of its own. Unset
/// means off: an install that says nothing keeps its unlock ladder rather than silently opening
/// every class.
fn classes_all_unlocked() -> bool {
    static SETTING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

    *SETTING.get_or_init(|| {
        matches!(
            std::env::var("HENDRA_CLASSES_UNLOCKED").as_deref(),
            Ok("1") | Ok("true")
        )
    })
}

/// What an account has unlocked, loaded once and asked many times.
pub struct Unlocks {
    /// Keyed by identity rather than runtime number, because this outlives a load.
    progress: HashMap<uuid::Uuid, (i16, i32)>,
    purchased: Vec<uuid::Uuid>,

    /// Set when the install opens every class to everyone. See [`classes_all_unlocked`].
    everything: bool,
}

impl Unlocks {
    pub async fn load(store: &Store, account_id: i64) -> Result<Unlocks, StoreError> {
        Ok(Unlocks {
            progress: store.class_progress(account_id).await?,
            purchased: store.purchased_classes(account_id).await?,
            everything: classes_all_unlocked(),
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

    /// The best level and fame this account has ever reached with one class.
    ///
    /// Zeroes for a class never played, which is what a class-select screen shows before anybody
    /// has taken it anywhere. Read from the same map the unlock decision uses, so the level a
    /// screen displays and the level a lock is measured against cannot drift apart.
    pub fn best(&self, catalog: &Catalog, object_type: ObjectType) -> (i32, i32) {
        catalog
            .object(object_type)
            .and_then(|desc| self.progress.get(&desc.uuid))
            .map(|(level, fame)| (*level as i32, *fame))
            .unwrap_or((0, 0))
    }

    /// Why this class cannot be played, if it cannot.
    ///
    /// Takes the catalog because progress is keyed by identity and the unlock names a runtime
    /// number: the two have to be brought together somewhere, and here is the only place that has
    /// both.
    pub fn locked(&self, catalog: &Catalog, class: &hendra_content::PlayerDesc) -> Option<Locked> {
        // An install that unlocks everything has already written the class-stats row the ladder
        // would be measured against, so the ladder never refuses anything. Answered here rather
        // than at each caller, so the list, the picker and character creation cannot disagree about
        // which classes are open.
        if self.everything {
            return None;
        }

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
    account_id: i64,
    class: ObjectType,
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
        .create_character(account_id, desc_uuid, max_hp)
        .await?;

    let mut slots = starting_slots(catalog, desc);
    slots.sort_by_key(|(slot, _)| *slot);

    if !slots.is_empty() {
        store.set_inventory(character.id, &slots).await?;
    }

    Ok(store.character(character.id).await?)
}

/// What a fresh character of this class holds.
///
/// The class says so itself, in the `Equipment` list it declares: item types in slot order, with a
/// hole where the class starts with nothing. A class that declares an empty list starts empty, and
/// no tier is derived to cover for it — handing out a weapon the class never listed is how a wizard
/// ends up wearing a ring nobody granted. This is `Database.CreateCharacter`'s
/// `Items = InitInventory(playerDesc.Equipment)` (`common/Database.cs:998`), whose `InitInventory`
/// (`:937-944`) likewise only pads the tail with empties and invents nothing.
///
/// So the code here has no say in whether a new character is armed; the content does, and ours
/// differs from the 2020 import deliberately. In that import every one of the fourteen classes
/// declares `Equipment` as twenty-four `-1`, and the engine has no fallback, so a new character
/// spawns with no weapon — and with no weapon it cannot kill, so it cannot get a drop, so it can
/// never obtain one. That is not a quirk of the game to be preserved but a stripped content dump:
/// the same import also ships an unclosed `<Region>` that stops the world server booting at all.
/// Our `EmbeddedData_PlayersCXML.xml` therefore gives each class the tier-zero item for the first
/// three slot types it declares — weapon, ability, armour — and leaves the ring empty, which is
/// what the retail game does. It is a content decision of this fork, recorded here because it is
/// the one place someone asks why a wizard starts holding a wand; it is not a claim about what the
/// reference server does.
pub fn starting_slots(
    catalog: &Catalog,
    class: &hendra_content::PlayerDesc,
) -> Vec<(i16, uuid::Uuid)> {
    class
        .equipment
        .iter()
        .enumerate()
        .filter_map(|(slot, declared)| {
            let object_type = (*declared)?;

            // An item the catalog does not have is skipped rather than fatal, so a half-converted
            // content directory still lets someone play.
            let Some(item) = catalog.object(object_type) else {
                tracing::warn!(
                    class = ?class.object_type,
                    ?object_type,
                    "a class starts with an item the catalog does not have"
                );
                return None;
            };

            Some((slot as i16, item.uuid))
        })
        .collect()
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
            everything: false,
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
