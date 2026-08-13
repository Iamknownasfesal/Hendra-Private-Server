//! What each playable class is.
//!
//! A class is the difference between one avatar and fourteen: where its equipment slots are, what
//! it can put in them, what it starts with, how it grows, and what has to be done before it can be
//! played at all. None of that was read before this — every character was created with the same
//! hardcoded health and the same kit, which made the class a sprite.

use crate::desc::ObjectType;
use crate::xml::Node;

/// The eight stats, in the order the game has always numbered them.
///
/// The order is not an implementation detail: it is the order the stats appear in on the wire and
/// in every saved character, so it is fixed by the format rather than chosen here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Stat {
    MaxHitPoints = 0,
    MaxMagicPoints = 1,
    Attack = 2,
    Defense = 3,
    Speed = 4,
    Dexterity = 5,
    HpRegen = 6,
    MpRegen = 7,
}

pub const STATS: [Stat; 8] = [
    Stat::MaxHitPoints,
    Stat::MaxMagicPoints,
    Stat::Attack,
    Stat::Defense,
    Stat::Speed,
    Stat::Dexterity,
    Stat::HpRegen,
    Stat::MpRegen,
];

impl Stat {
    /// The element name this stat is written under.
    ///
    /// `HpRegen` and `MpRegen` are what the files call vitality and wisdom; the names here follow
    /// the files rather than the interface, because the files are what has to be read.
    pub fn element(self) -> &'static str {
        match self {
            Stat::MaxHitPoints => "MaxHitPoints",
            Stat::MaxMagicPoints => "MaxMagicPoints",
            Stat::Attack => "Attack",
            Stat::Defense => "Defense",
            Stat::Speed => "Speed",
            Stat::Dexterity => "Dexterity",
            Stat::HpRegen => "HpRegen",
            Stat::MpRegen => "MpRegen",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// One stat's starting value, ceiling, and how much a level adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatGrowth {
    pub starting: i32,
    pub maximum: i32,

    /// The inclusive range a level-up adds. Both zero for stats a class does not gain.
    pub min_increase: i32,
    pub max_increase: i32,
}

impl StatGrowth {
    fn parse(node: &Node, stat: Stat) -> StatGrowth {
        let element = stat.element();

        let (starting, maximum) = node
            .children_named(element)
            .next()
            .map(|found| {
                let starting = found.as_int().unwrap_or(0) as i32;
                // A stat with no ceiling in the file cannot be raised past where it starts, which
                // is safer than treating "absent" as "unbounded".
                let maximum = found
                    .attr_int("max")
                    .map(|max| max as i32)
                    .unwrap_or(starting);
                (starting, maximum)
            })
            .unwrap_or((0, 0));

        // Level increases are listed as repeated elements whose text names the stat, so finding
        // this one means scanning them rather than looking it up.
        let mut min_increase = 0;
        let mut max_increase = 0;
        for increase in node.children_named("LevelIncrease") {
            if increase.text.trim() == element {
                min_increase = increase.attr_int("min").unwrap_or(0) as i32;
                max_increase = increase.attr_int("max").unwrap_or(0) as i32;
                break;
            }
        }

        StatGrowth {
            starting,
            maximum,
            min_increase,
            max_increase,
        }
    }
}

/// What has to be done before a class can be played.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Unlock {
    /// The class that has to be levelled, and how far.
    pub after: Option<(ObjectType, i32)>,

    /// What it costs to skip that.
    pub cost: Option<u32>,
}

impl Unlock {
    /// Whether this class is available from the start.
    pub fn free(&self) -> bool {
        self.after.is_none() && self.cost.is_none()
    }
}

/// A playable class.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerDesc {
    pub object_type: ObjectType,

    /// What each slot accepts, by slot type. The first four are the equipped ones — weapon,
    /// ability, armour, ring — and the rest are the backpack.
    pub slot_types: Vec<i32>,

    /// What the class starts holding, in slot order. `None` where it starts with nothing.
    pub equipment: Vec<Option<ObjectType>>,

    pub stats: [StatGrowth; 8],
    pub unlock: Unlock,
}

impl PlayerDesc {
    /// Reads a class, or `None` if the object is not one.
    pub fn parse(node: &Node, object_type: ObjectType) -> Option<PlayerDesc> {
        if !node.has("Player") {
            return None;
        }

        let slot_types = comma_ints(node.field("SlotTypes").unwrap_or_default())
            .into_iter()
            .map(|value| value as i32)
            .collect();

        // Equipment is written as -1 for an empty slot, which is not a type but a hole. Turning it
        // into `None` here means nothing downstream has to know that -1 was ever the convention.
        let equipment = comma_ints(node.field("Equipment").unwrap_or_default())
            .into_iter()
            .map(|value| {
                (value >= 0 && value <= u16::MAX as i64).then_some(ObjectType(value as u16))
            })
            .collect();

        let mut stats = [StatGrowth::default(); 8];
        for stat in STATS {
            stats[stat.index()] = StatGrowth::parse(node, stat);
        }

        let unlock = Unlock {
            after: node.children_named("UnlockLevel").next().and_then(|level| {
                Some((
                    ObjectType(level.attr_int("type")? as u16),
                    level.attr_int("level")? as i32,
                ))
            }),
            cost: node.int("UnlockCost").map(|cost| cost.max(0) as u32),
        };

        Some(PlayerDesc {
            object_type,
            slot_types,
            equipment,
            stats,
            unlock,
        })
    }

    pub fn stat(&self, stat: Stat) -> StatGrowth {
        self.stats[stat.index()]
    }

    /// The health a freshly made character of this class has.
    pub fn starting_hp(&self) -> i32 {
        self.stat(Stat::MaxHitPoints).starting
    }

    pub fn starting_mp(&self) -> i32 {
        self.stat(Stat::MaxMagicPoints).starting
    }

    /// What a slot accepts, or `None` past the end of the class's slots.
    pub fn slot_type(&self, slot: usize) -> Option<i32> {
        self.slot_types.get(slot).copied()
    }
}

/// Why a class cannot be played yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locked {
    /// Another class has to be levelled first.
    NeedsLevel { class: ObjectType, level: i32 },
}

impl PlayerDesc {
    /// Whether an account may make a character of this class.
    ///
    /// `best_level` answers "how far has this account taken that class", and `purchased` is the
    /// set bought outright. Progress is looked up by class rather than passed as one number
    /// because an unlock names a specific class — levelling a wizard does not open the knight.
    ///
    /// A class whose prerequisite is not in the files at all is treated as open. The alternative
    /// is a class nobody can ever play because of a typo in content, which fails silently and is
    /// found by a player rather than by a test.
    pub fn locked_for(
        &self,
        best_level: &impl Fn(ObjectType) -> i32,
        purchased: &[ObjectType],
    ) -> Option<Locked> {
        if purchased.contains(&self.object_type) {
            return None;
        }

        let (after, level) = self.unlock.after?;
        if best_level(after) >= level {
            return None;
        }

        Some(Locked::NeedsLevel {
            class: after,
            level,
        })
    }
}

/// Splits a comma-separated list, skipping anything that is not a number.
///
/// The files space these inconsistently — `2, 13, 6, 9, 52, 53 , 54 ,55` is one real line — so
/// trimming each piece is required rather than tidy.
fn comma_ints(text: &str) -> Vec<i64> {
    text.split(',')
        .filter_map(|piece| {
            let piece = piece.trim();
            if let Some(hex) = piece.strip_prefix("0x") {
                i64::from_str_radix(hex, 16).ok()
            } else {
                piece.parse().ok()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::Node as XmlNode;

    const ROGUE: &str = r#"
<Objects>
   <Object type="0x0300" id="Rogue">
      <Class>Player</Class>
      <Player/>
      <SlotTypes>2, 13, 6, 9, 52, 53 , 54 ,55 ,  0, 0, 0, 0</SlotTypes>
      <Equipment>0x0a00, -1, 0x0a01, -1, -1, -1, -1, -1</Equipment>
      <MaxHitPoints max="720">150</MaxHitPoints>
      <MaxMagicPoints max="252">100</MaxMagicPoints>
      <Attack max="50">10</Attack>
      <Defense max="25">0</Defense>
      <Speed max="75">15</Speed>
      <Dexterity max="75">15</Dexterity>
      <HpRegen max="60">15</HpRegen>
      <MpRegen max="70">10</MpRegen>
      <LevelIncrease min="20" max="30">MaxHitPoints</LevelIncrease>
      <LevelIncrease min="2" max="8">MaxMagicPoints</LevelIncrease>
      <LevelIncrease min="0" max="2">Attack</LevelIncrease>
      <LevelIncrease min="0" max="0">Defense</LevelIncrease>
      <LevelIncrease min="1" max="2">Speed</LevelIncrease>
      <LevelIncrease min="1" max="2">Dexterity</LevelIncrease>
      <LevelIncrease min="0" max="1">HpRegen</LevelIncrease>
      <LevelIncrease min="0" max="2">MpRegen</LevelIncrease>
      <UnlockLevel level="5" type="0x0307">Archer</UnlockLevel>
      <UnlockCost>199</UnlockCost>
   </Object>
   <Object type="0x0500" id="Sheep">
      <Class>Character</Class>
      <MaxHitPoints max="100">100</MaxHitPoints>
   </Object>
</Objects>"#;

    fn rogue() -> PlayerDesc {
        let document = XmlNode::parse(ROGUE).unwrap();
        let node = document
            .children_named("Object")
            .find(|node| node.attr("id") == Some("Rogue"))
            .unwrap();
        PlayerDesc::parse(node, ObjectType(0x0300)).unwrap()
    }

    #[test]
    fn a_class_reads_its_stats_with_their_ceilings() {
        let rogue = rogue();

        assert_eq!(rogue.starting_hp(), 150);
        assert_eq!(rogue.stat(Stat::MaxHitPoints).maximum, 720);
        assert_eq!(rogue.stat(Stat::Defense).starting, 0);
        assert_eq!(rogue.stat(Stat::Defense).maximum, 25);
    }

    #[test]
    fn level_increases_are_matched_to_the_stat_they_name() {
        // They are repeated elements whose text names the stat rather than attributes of it, so
        // this is the part that would silently pair the wrong numbers together.
        let rogue = rogue();

        assert_eq!(rogue.stat(Stat::MaxHitPoints).min_increase, 20);
        assert_eq!(rogue.stat(Stat::MaxHitPoints).max_increase, 30);
        assert_eq!(rogue.stat(Stat::Dexterity).min_increase, 1);
        assert_eq!(rogue.stat(Stat::Dexterity).max_increase, 2);

        // A stat a rogue never gains, which is different from one that was not listed.
        assert_eq!(rogue.stat(Stat::Defense).min_increase, 0);
        assert_eq!(rogue.stat(Stat::Defense).max_increase, 0);
    }

    #[test]
    fn slot_types_survive_the_spacing_in_the_files() {
        let rogue = rogue();

        assert_eq!(rogue.slot_type(0), Some(2), "weapon");
        assert_eq!(rogue.slot_type(1), Some(13), "ability");
        assert_eq!(rogue.slot_type(7), Some(55));
        assert_eq!(rogue.slot_type(99), None);
    }

    #[test]
    fn an_empty_equipment_slot_is_nothing_rather_than_minus_one() {
        let rogue = rogue();

        assert_eq!(rogue.equipment[0], Some(ObjectType(0x0a00)));
        assert_eq!(rogue.equipment[1], None);
        assert_eq!(rogue.equipment[2], Some(ObjectType(0x0a01)));
        assert!(rogue.equipment[3..].iter().all(Option::is_none));
    }

    #[test]
    fn an_unlock_names_the_class_and_the_level() {
        let rogue = rogue();

        assert_eq!(rogue.unlock.after, Some((ObjectType(0x0307), 5)));
        assert_eq!(rogue.unlock.cost, Some(199));
        assert!(!rogue.unlock.free());
    }

    #[test]
    fn a_class_is_locked_until_its_prerequisite_is_levelled() {
        let rogue = rogue();
        let archer = ObjectType(0x0307);

        assert_eq!(
            rogue.locked_for(&|_| 0, &[]),
            Some(Locked::NeedsLevel {
                class: archer,
                level: 5
            })
        );
        assert!(
            rogue.locked_for(&|_| 4, &[]).is_some(),
            "one short is still short"
        );
        assert_eq!(
            rogue.locked_for(&|_| 5, &[]),
            None,
            "exactly the level is enough"
        );
        assert_eq!(rogue.locked_for(&|_| 20, &[]), None);
    }

    #[test]
    fn only_the_named_class_counts_toward_an_unlock() {
        // Levelling anything at all would make every unlock the same unlock.
        let rogue = rogue();
        let archer = ObjectType(0x0307);

        let levelled_something_else = |class: ObjectType| if class == archer { 0 } else { 20 };
        assert!(rogue.locked_for(&levelled_something_else, &[]).is_some());

        let levelled_the_archer = |class: ObjectType| if class == archer { 20 } else { 0 };
        assert!(rogue.locked_for(&levelled_the_archer, &[]).is_none());
    }

    #[test]
    fn a_purchased_class_needs_no_levelling() {
        let rogue = rogue();

        assert!(rogue.locked_for(&|_| 0, &[]).is_some());
        assert!(rogue.locked_for(&|_| 0, &[ObjectType(0x0300)]).is_none());
        assert!(
            rogue.locked_for(&|_| 0, &[ObjectType(0x0999)]).is_some(),
            "buying a different class should not open this one"
        );
    }

    #[test]
    fn a_class_with_no_prerequisite_is_open_from_the_start() {
        let document =
            XmlNode::parse(r#"<Objects><Object type="0x1" id="Free"><Player/></Object></Objects>"#)
                .unwrap();
        let node = document.children_named("Object").next().unwrap();
        let free = PlayerDesc::parse(node, ObjectType(1)).unwrap();

        assert_eq!(free.locked_for(&|_| 0, &[]), None);
    }

    #[test]
    fn something_that_is_not_a_player_is_not_a_class() {
        let document = XmlNode::parse(ROGUE).unwrap();
        let sheep = document
            .children_named("Object")
            .find(|node| node.attr("id") == Some("Sheep"))
            .unwrap();

        assert!(PlayerDesc::parse(sheep, ObjectType(0x0500)).is_none());
    }

    #[test]
    fn a_class_with_nothing_written_down_still_parses() {
        // A half-converted content directory should give a class with no stats rather than no
        // catalog at all.
        let document =
            XmlNode::parse(r#"<Objects><Object type="0x1" id="Bare"><Player/></Object></Objects>"#)
                .unwrap();
        let node = document.children_named("Object").next().unwrap();
        let bare = PlayerDesc::parse(node, ObjectType(1)).unwrap();

        assert_eq!(bare.starting_hp(), 0);
        assert!(bare.slot_types.is_empty());
        assert!(bare.equipment.is_empty());
        assert!(bare.unlock.free());
    }
}
