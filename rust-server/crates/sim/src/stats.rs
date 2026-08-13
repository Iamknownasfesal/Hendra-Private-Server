//! The eight stats, and what they are worth.
//!
//! Every class declares a starting value, a ceiling and a per-level range for each of the eight.
//! Together they decide how fast a character moves, how hard it hits and how often it can fire,
//! which is what separates a warrior from a wizard.
//!
//! # Three layers, kept apart
//!
//! A stat is a base, plus what equipment adds, plus what temporary boosts add. They are held
//! separately rather than summed into one number because taking a ring off has to subtract exactly
//! what it added, and a single total cannot say what that was. The alternative, recomputing from
//! the inventory on every change, is what the old server did and is why its stats drifted.
//!
//! # What a stat is worth
//!
//! The conversions here are the game's, not inventions: attack and dexterity fold into damage and
//! rate of fire through the formulas the client has always drawn with, and a server that used
//! different ones would disagree with every number a player can see.

use hendra_content::{PlayerDesc, Stat, STATS};

/// One character's stats, in three layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// What levelling has produced. The only layer that persists.
    base: [i32; 8],

    /// What worn equipment adds. Rebuilt whenever equipment changes.
    equipment: [i32; 8],

    /// What temporary effects add.
    boosts: [i32; 8],
}

impl Stats {
    /// A character at the start of its class.
    pub fn starting(class: &PlayerDesc) -> Stats {
        let mut base = [0i32; 8];
        for stat in STATS {
            base[stat.index()] = class.stat(stat).starting;
        }

        Stats {
            base,
            equipment: [0; 8],
            boosts: [0; 8],
        }
    }

    /// The base value, before equipment or boosts.
    pub fn base(&self, stat: Stat) -> i32 {
        self.base[stat.index()]
    }

    /// The total, which is what everything else asks for.
    ///
    /// Only the base is capped. Equipment and boosts reach past a class's maximum, which is what
    /// late-game equipment is for; capping the total would make a ring worthless to exactly the
    /// characters who earned it.
    pub fn total(&self, stat: Stat) -> i32 {
        let index = stat.index();
        (self.base[index] + self.equipment[index] + self.boosts[index]).max(0)
    }

    /// Raises the base, refusing to go past the class's ceiling.
    pub fn raise(&mut self, class: &PlayerDesc, stat: Stat, by: i32) {
        let index = stat.index();
        let ceiling = class.stat(stat).maximum;
        self.base[index] = (self.base[index] + by).clamp(0, ceiling.max(0));
    }

    /// Replaces what equipment contributes.
    ///
    /// Replaced wholesale rather than adjusted. Tracking which item added what and undoing it on
    /// removal is the bookkeeping that drifts.
    pub fn set_equipment(&mut self, boosts: [i32; 8]) {
        self.equipment = boosts;
    }

    /// Adds a temporary boost.
    pub fn boost(&mut self, stat: Stat, amount: i32) {
        self.boosts[stat.index()] += amount;
    }

    pub fn clear_boosts(&mut self) {
        self.boosts = [0; 8];
    }

    /// What each stat is at, for the wire and for a character sheet.
    pub fn totals(&self) -> [i32; 8] {
        let mut out = [0i32; 8];
        for stat in STATS {
            out[stat.index()] = self.total(stat);
        }
        out
    }

    // -- what the stats are worth -------------------------------------------------------------

    /// Multiplies weapon damage.
    ///
    /// Attack scales damage rather than adding to it, so a stronger weapon benefits more from the
    /// same attack, which is what makes both worth pursuing.
    pub fn damage_multiplier(&self) -> f32 {
        (self.total(Stat::Attack) as f32) * ATTACK_PER_POINT + BASE_MULTIPLIER
    }

    /// Multiplies weapon cooldown. Below one is faster.
    pub fn rate_of_fire(&self) -> f32 {
        1.0 / ((self.total(Stat::Dexterity) as f32) * DEXTERITY_PER_POINT + BASE_MULTIPLIER)
    }

    /// Tiles per second.
    pub fn movement_speed(&self) -> f32 {
        (self.total(Stat::Speed) as f32) * SPEED_PER_POINT + BASE_SPEED
    }

    /// Health regained per second while not recently hurt.
    pub fn health_regen(&self) -> f32 {
        BASE_REGEN + (self.total(Stat::HpRegen) as f32) * REGEN_PER_POINT
    }

    /// Magic regained per second.
    pub fn magic_regen(&self) -> f32 {
        BASE_REGEN + (self.total(Stat::MpRegen) as f32) * REGEN_PER_POINT
    }

    pub fn max_hp(&self) -> i32 {
        self.total(Stat::MaxHitPoints)
    }

    pub fn max_mp(&self) -> i32 {
        self.total(Stat::MaxMagicPoints)
    }

    pub fn defence(&self) -> i32 {
        self.total(Stat::Defense)
    }
}

/// The multiplier a character with nothing in a stat still has.
const BASE_MULTIPLIER: f32 = 0.5;

/// What one point of attack adds to the damage multiplier.
const ATTACK_PER_POINT: f32 = 0.02;

/// What one point of dexterity adds to the rate-of-fire multiplier.
const DEXTERITY_PER_POINT: f32 = 0.02;

/// Tiles per second with no speed at all.
const BASE_SPEED: f32 = 4.0;

/// What one point of speed adds, in tiles per second.
const SPEED_PER_POINT: f32 = 0.04;

/// Health or magic per second with no vitality or wisdom.
const BASE_REGEN: f32 = 1.0;

/// What one point of vitality or wisdom adds, per second.
const REGEN_PER_POINT: f32 = 0.12;

/// What a set of worn items contributes, summed.
///
/// Taken from what is worn rather than accumulated as items move, so the answer never depends on
/// having seen every change. A missed equip cannot leave a stat permanently wrong.
pub fn equipment_boosts<'a>(
    worn: impl Iterator<Item = &'a hendra_content::ItemDesc>,
) -> [i32; 8] {
    let mut out = [0i32; 8];
    for item in worn {
        for boost in &item.stat_boosts {
            if let Some(slot) = out.get_mut(boost.stat as usize) {
                *slot += boost.amount;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hendra_content::Node;

    const WIZARD: &str = r#"
<Objects>
   <Object type="0x030e" id="Wizard">
      <Player/>
      <MaxHitPoints max="670">100</MaxHitPoints>
      <MaxMagicPoints max="385">100</MaxMagicPoints>
      <Attack max="75">12</Attack>
      <Defense max="25">0</Defense>
      <Speed max="50">12</Speed>
      <Dexterity max="75">15</Dexterity>
      <HpRegen max="40">10</HpRegen>
      <MpRegen max="60">10</MpRegen>
      <LevelIncrease min="20" max="30">MaxHitPoints</LevelIncrease>
   </Object>
</Objects>"#;

    fn wizard() -> PlayerDesc {
        let document = Node::parse(WIZARD).unwrap();
        let node = document.children_named("Object").next().unwrap();
        PlayerDesc::parse(node, hendra_content::ObjectType(0x030e)).unwrap()
    }

    #[test]
    fn a_fresh_character_starts_where_its_class_says() {
        let stats = Stats::starting(&wizard());

        assert_eq!(stats.total(Stat::MaxHitPoints), 100);
        assert_eq!(stats.total(Stat::Attack), 12);
        assert_eq!(stats.total(Stat::Defense), 0);
    }

    #[test]
    fn a_stat_cannot_be_levelled_past_its_class_ceiling() {
        let class = wizard();
        let mut stats = Stats::starting(&class);

        for _ in 0..500 {
            stats.raise(&class, Stat::Attack, 5);
        }

        assert_eq!(stats.base(Stat::Attack), 75, "the wizard's maximum");
    }

    #[test]
    fn equipment_reaches_past_the_ceiling_that_levelling_cannot() {
        // The whole point of late-game equipment. Capping the total would make a ring worthless to
        // exactly the characters who earned it.
        let class = wizard();
        let mut stats = Stats::starting(&class);
        for _ in 0..500 {
            stats.raise(&class, Stat::Attack, 5);
        }

        let mut boosts = [0i32; 8];
        boosts[Stat::Attack.index()] = 6;
        stats.set_equipment(boosts);

        assert_eq!(stats.total(Stat::Attack), 81);
        assert_eq!(stats.base(Stat::Attack), 75, "the base is still capped");
    }

    #[test]
    fn taking_equipment_off_returns_every_stat_to_exactly_where_it_was() {
        // Drift is the risk: a stat that creeps up by a point each time something is equipped and
        // unequipped, which nobody notices until a character is wrong.
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let before = stats.totals();

        let mut seed = 0x1234_5678u32;
        for round in 0..1_000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);

            let mut boosts = [0i32; 8];
            for slot in boosts.iter_mut() {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                *slot = (seed % 21) as i32 - 10;
            }
            stats.set_equipment(boosts);
            stats.set_equipment([0; 8]);

            assert_eq!(stats.totals(), before, "drifted on round {round}");
        }
    }

    #[test]
    fn two_items_boosting_the_same_stat_stack() {
        let ring = hendra_content::ItemDesc {
            stat_boosts: vec![hendra_content::StatBoost {
                stat: Stat::Dexterity as u8,
                amount: 6,
            }],
            ..Default::default()
        };

        let boosts = equipment_boosts([&ring, &ring].into_iter());
        assert_eq!(boosts[Stat::Dexterity.index()], 12);
    }

    #[test]
    fn a_stat_boost_naming_something_that_is_not_a_stat_is_ignored() {
        // The content has a few of these. Indexing an array with it would panic, and taking the
        // modulus would silently boost the wrong stat.
        let odd = hendra_content::ItemDesc {
            stat_boosts: vec![hendra_content::StatBoost {
                stat: 200,
                amount: 50,
            }],
            ..Default::default()
        };

        assert_eq!(equipment_boosts([&odd].into_iter()), [0; 8]);
    }

    #[test]
    fn more_attack_means_more_damage_and_more_dexterity_means_faster() {
        let class = wizard();
        let weak = Stats::starting(&class);

        let mut strong = weak;
        strong.boost(Stat::Attack, 50);
        assert!(strong.damage_multiplier() > weak.damage_multiplier());

        let mut quick = weak;
        quick.boost(Stat::Dexterity, 50);
        assert!(
            quick.rate_of_fire() < weak.rate_of_fire(),
            "a lower cooldown multiplier is faster"
        );
    }

    #[test]
    fn a_character_with_nothing_in_a_stat_is_not_useless() {
        // Zero attack must not mean zero damage, or a fresh character could not kill anything.
        let empty = Stats::default();

        assert!(empty.damage_multiplier() > 0.0);
        assert!(empty.rate_of_fire() > 0.0 && empty.rate_of_fire().is_finite());
        assert!(empty.movement_speed() > 0.0);
    }

    #[test]
    fn boosts_can_be_cleared_without_touching_the_other_layers() {
        let class = wizard();
        let mut stats = Stats::starting(&class);

        let mut worn = [0i32; 8];
        worn[Stat::Speed.index()] = 5;
        stats.set_equipment(worn);
        stats.boost(Stat::Speed, 20);

        assert_eq!(stats.total(Stat::Speed), 12 + 5 + 20);

        stats.clear_boosts();
        assert_eq!(stats.total(Stat::Speed), 17, "equipment and base remain");
    }

    #[test]
    fn a_negative_boost_cannot_take_a_stat_below_zero() {
        let mut stats = Stats::starting(&wizard());
        stats.boost(Stat::Attack, -1_000);

        assert_eq!(stats.total(Stat::Attack), 0);
    }
}
