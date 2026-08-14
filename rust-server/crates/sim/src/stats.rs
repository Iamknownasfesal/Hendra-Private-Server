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
//! Every formula and constant below is read from the original server's `StatsManager`. The
//! condition effects that change them are applied inside the same expression that reads the stat,
//! as they are there: Weak does not halve damage, it holds attack at its minimum.

use hendra_content::{PlayerDesc, STATS, Stat};

use crate::effects::Rules;

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
    /// Something that cannot move under its own power, such as a wall or a sign.
    pub fn still() -> Stats {
        let mut stats = Stats::default();
        stats.base[Stat::Speed.index()] = NO_MOVEMENT;
        stats
    }

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

    /// Sets what temporary boosts come to, having already been stacked.
    ///
    /// Set rather than added, because the answer is a function of what is held: recomputing it from
    /// the list every time a boost is given or lapses is what makes a lapse take the right amount
    /// away rather than whatever was added last.
    pub fn set_boosts(&mut self, boosts: [i32; 8]) {
        self.boosts = boosts;
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
    /// `MinAttackMult + (attack / 75) * (MaxAttackMult - MinAttackMult)`, held at the minimum by
    /// Weak and multiplied by Damaging.
    pub fn damage_multiplier(&self, rules: &Rules) -> f32 {
        if rules.weak {
            return MIN_ATTACK_MULT;
        }

        let attack = self.total(Stat::Attack) as f32;
        let mult = MIN_ATTACK_MULT + (attack / STAT_SCALE) * (MAX_ATTACK_MULT - MIN_ATTACK_MULT);

        if rules.damaging { mult * 1.5 } else { mult }
    }

    /// Shots per millisecond.
    ///
    /// A frequency rather than a cooldown, matching `GetAttackFrequency`. Dazed holds it at the
    /// minimum and Berserk multiplies it.
    pub fn attack_frequency(&self, rules: &Rules) -> f32 {
        if rules.dazed {
            return MIN_ATTACK_FREQ;
        }

        let dexterity = self.total(Stat::Dexterity) as f32;
        let frequency =
            MIN_ATTACK_FREQ + (dexterity / STAT_SCALE) * (MAX_ATTACK_FREQ - MIN_ATTACK_FREQ);

        if rules.berserk {
            frequency * 1.5
        } else {
            frequency
        }
    }

    /// Milliseconds between shots from a weapon of a given rate of fire.
    ///
    /// The weapon's own rate is a multiplier on the character's frequency, so a fast weapon in a
    /// dextrous hand compounds rather than replacing it.
    pub fn shot_cooldown_ms(&self, rules: &Rules, weapon_rate: f32) -> u32 {
        let frequency = self.attack_frequency(rules) * weapon_rate.max(0.01);
        if frequency <= 0.0 {
            return u32::MAX;
        }
        (1.0 / frequency).clamp(1.0, 60_000.0) as u32
    }

    /// Tiles per second.
    ///
    /// `4 + 5.6 * (speed / 75)`. Slowed holds it at the base rather than scaling it, and Paralyzed
    /// is handled by the caller refusing the move outright.
    pub fn movement_speed(&self, rules: &Rules) -> f32 {
        if rules.rooted {
            return 0.0;
        }
        if rules.slowed {
            return BASE_SPEED;
        }

        let speed = (self.base(Stat::Speed) + self.layers(Stat::Speed)) as f32;
        let ret = BASE_SPEED + SPEED_RANGE * (speed / STAT_SCALE);

        if rules.speedy {
            (ret * 1.5).max(0.0)
        } else {
            ret.max(0.0)
        }
    }

    /// Equipment plus boosts, which may be negative.
    fn layers(&self, stat: Stat) -> i32 {
        self.equipment[stat.index()] + self.boosts[stat.index()]
    }

    /// Health regained per second.
    ///
    /// `6 + vitality * 0.12`. Sick zeroes the vitality but not the base, so a sick character still
    /// recovers slowly rather than not at all.
    pub fn health_regen(&self, rules: &Rules) -> f32 {
        let vitality = if rules.sick {
            0.0
        } else {
            self.total(Stat::HpRegen) as f32
        };
        BASE_HP_REGEN + vitality * HP_REGEN_PER_POINT
    }

    /// Magic regained per second. Quiet stops it entirely.
    pub fn magic_regen(&self, rules: &Rules) -> f32 {
        if rules.no_magic_regen {
            return 0.0;
        }
        BASE_MP_REGEN + (self.total(Stat::MpRegen) as f32) * MP_REGEN_PER_POINT
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

/// The stat value every formula is scaled against.
const STAT_SCALE: f32 = 75.0;

/// The damage multiplier at zero attack, and at the scale value.
const MIN_ATTACK_MULT: f32 = 0.5;
const MAX_ATTACK_MULT: f32 = 2.0;

/// Shots per millisecond at zero dexterity, and at the scale value.
const MIN_ATTACK_FREQ: f32 = 0.0015;
const MAX_ATTACK_FREQ: f32 = 0.008;

/// Tiles per second at zero speed, and how much the scale value adds.
const BASE_SPEED: f32 = 4.0;
const SPEED_RANGE: f32 = 5.6;

/// Health per second at zero vitality, and what one point adds.
const BASE_HP_REGEN: f32 = 6.0;
const HP_REGEN_PER_POINT: f32 = 0.12;

/// Magic per second at zero wisdom, and what one point adds.
const BASE_MP_REGEN: f32 = 0.5;
const MP_REGEN_PER_POINT: f32 = 0.06;

/// The speed value that means "does not move".
///
/// Below the point where the formula returns zero, so a wall cannot drift.
const NO_MOVEMENT: i32 = -((BASE_SPEED / SPEED_RANGE * STAT_SCALE) as i32) - 1;

/// What a set of worn items contributes, summed.
///
/// Taken from what is worn rather than accumulated as items move, so the answer never depends on
/// having seen every change. A missed equip cannot leave a stat permanently wrong.
pub fn equipment_boosts<'a>(worn: impl Iterator<Item = &'a hendra_content::ItemDesc>) -> [i32; 8] {
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

/// What a set of temporary boosts on one stat comes to.
///
/// Follows `ActivateBoost.GetBoost`. Boosts do not simply add: sorted, the largest counts in full,
/// the next at a half, the next at a quarter, and so on. Two rings of eight attack are worth twelve
/// rather than sixteen, which is why stacking the same buff is worth less each time and why a
/// second one is worth having at all.
///
/// Non-stacking boosts are separate and only the largest of them counts, which is what stops two
/// copies of a buff that says it does not stack from stacking.
pub fn stacked(stacking: &[i32], separate: &[i32]) -> i32 {
    let mut sorted: Vec<i32> = stacking.to_vec();

    // Largest first, because the discount is applied by position and the largest is meant to be the
    // one that counts in full.
    sorted.sort_unstable_by(|a, b| b.cmp(a));

    let mut total = 0i32;
    for (index, amount) in sorted.iter().enumerate() {
        // Halving each time. Past a handful this is zero, which is the point: piling on more of the
        // same buff stops being worth anything.
        let share = 0.5f64.powi(index as i32);
        total += (*amount as f64 * share) as i32;
    }

    total + separate.iter().copied().max().unwrap_or(0)
}

#[cfg(test)]
mod stacking {
    use super::stacked;

    #[test]
    fn one_boost_counts_in_full() {
        assert_eq!(stacked(&[8], &[]), 8);
    }

    #[test]
    fn each_boost_after_the_largest_counts_for_half_of_the_last() {
        // Two rings of eight attack are worth twelve rather than sixteen, which is why stacking the
        // same buff is worth less each time and why a second one is worth having at all.
        assert_eq!(stacked(&[8, 8], &[]), 12);
        assert_eq!(stacked(&[8, 8, 8], &[]), 14);
        assert_eq!(stacked(&[8, 8, 8, 8], &[]), 15);
    }

    #[test]
    fn the_largest_is_the_one_that_counts_in_full_whatever_order_they_arrived_in() {
        // Sorted rather than taken as given, or a small boost arriving first would take the full
        // share and the large one behind it would be halved.
        assert_eq!(stacked(&[2, 20], &[]), stacked(&[20, 2], &[]));
        assert_eq!(stacked(&[2, 20], &[]), 21);
    }

    #[test]
    fn piling_on_more_of_the_same_stops_being_worth_anything() {
        let many = vec![10; 20];
        let few = vec![10; 6];

        // Past a handful the halving reaches zero, which is the point of the rule.
        assert_eq!(stacked(&many, &[]), stacked(&few, &[]));
    }

    #[test]
    fn the_kind_that_does_not_stack_takes_only_its_largest() {
        assert_eq!(stacked(&[], &[5, 5, 5]), 5);
        assert_eq!(stacked(&[], &[3, 9, 1]), 9);
    }

    #[test]
    fn the_two_kinds_are_added_to_each_other() {
        // They are separate rules rather than one pool: a stacking buff and an aura are both worth
        // having at once.
        assert_eq!(stacked(&[8, 8], &[10]), 22);
    }

    #[test]
    fn nothing_held_is_nothing() {
        assert_eq!(stacked(&[], &[]), 0);
    }

    #[test]
    fn a_negative_boost_still_counts_against_you() {
        // Some content lowers a stat, and sorting largest-first means the least bad one counts in
        // full. That is what the original does with them, and it is the merciful reading.
        assert_eq!(stacked(&[-10], &[]), -10);
        assert_eq!(stacked(&[-10, -10], &[]), -15);
    }
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
        let none = Rules::NONE;
        let class = wizard();
        let plain = Stats::starting(&class);

        let mut strong = plain;
        strong.boost(Stat::Attack, 50);
        assert!(strong.damage_multiplier(&none) > plain.damage_multiplier(&none));

        let mut quick = plain;
        quick.boost(Stat::Dexterity, 50);
        assert!(
            quick.shot_cooldown_ms(&none, 1.0) < plain.shot_cooldown_ms(&none, 1.0),
            "a shorter wait between shots is faster"
        );
    }

    #[test]
    fn the_damage_multiplier_spans_the_range_the_original_uses() {
        let none = Rules::NONE;
        let mut stats = Stats::default();

        assert!((stats.damage_multiplier(&none) - MIN_ATTACK_MULT).abs() < 0.001);

        stats.boost(Stat::Attack, STAT_SCALE as i32);
        assert!((stats.damage_multiplier(&none) - MAX_ATTACK_MULT).abs() < 0.001);
    }

    #[test]
    fn being_weak_holds_attack_at_the_minimum_rather_than_halving_it() {
        // `return MinAttackMult`, not a multiplier. On a character with high attack the difference
        // is large, and halving would leave it far above where the original puts it.
        let mut stats = Stats::default();
        stats.boost(Stat::Attack, 75);

        let weakened = Rules {
            weak: true,
            ..Rules::NONE
        };
        assert!((stats.damage_multiplier(&weakened) - MIN_ATTACK_MULT).abs() < 0.001);
        assert!(stats.damage_multiplier(&Rules::NONE) > MIN_ATTACK_MULT * 2.0);
    }

    #[test]
    fn being_dazed_holds_rate_of_fire_at_the_minimum() {
        let mut stats = Stats::default();
        stats.boost(Stat::Dexterity, 75);

        let dazed = Rules {
            dazed: true,
            ..Rules::NONE
        };
        assert!(stats.shot_cooldown_ms(&dazed, 1.0) > stats.shot_cooldown_ms(&Rules::NONE, 1.0));
        assert_eq!(
            stats.shot_cooldown_ms(&dazed, 1.0),
            (1.0 / MIN_ATTACK_FREQ) as u32
        );
    }

    #[test]
    fn being_slowed_holds_speed_at_the_base_rather_than_scaling_it() {
        // `ret = 4`, not `ret *= 0.5`. A fast character slowed drops to the same speed as a slow
        // one slowed, which is what the original does and what the content is balanced against.
        let mut fast = Stats::default();
        fast.boost(Stat::Speed, 75);
        let slow = Stats::default();

        let slowed = Rules {
            slowed: true,
            ..Rules::NONE
        };
        assert_eq!(fast.movement_speed(&slowed), BASE_SPEED);
        assert_eq!(slow.movement_speed(&slowed), BASE_SPEED);
        assert!(fast.movement_speed(&Rules::NONE) > BASE_SPEED);
    }

    #[test]
    fn a_rooted_character_has_no_speed_at_all() {
        let mut stats = Stats::default();
        stats.boost(Stat::Speed, 75);

        let rooted = Rules {
            rooted: true,
            ..Rules::NONE
        };
        assert_eq!(stats.movement_speed(&rooted), 0.0);
    }

    #[test]
    fn being_sick_zeroes_vitality_without_stopping_regeneration_entirely() {
        // `vit = 0` leaves the base of six. A sick character recovers slowly rather than not at all.
        let mut stats = Stats::default();
        stats.boost(Stat::HpRegen, 60);

        let sick = Rules {
            sick: true,
            ..Rules::NONE
        };
        assert_eq!(stats.health_regen(&sick), BASE_HP_REGEN);
        assert!(stats.health_regen(&Rules::NONE) > BASE_HP_REGEN);
    }

    #[test]
    fn quiet_stops_magic_returning_entirely() {
        let mut stats = Stats::default();
        stats.boost(Stat::MpRegen, 60);

        let quiet = Rules {
            no_magic_regen: true,
            ..Rules::NONE
        };
        assert_eq!(stats.magic_regen(&quiet), 0.0);
        assert!(stats.magic_regen(&Rules::NONE) > 0.0);
    }

    #[test]
    fn a_character_with_nothing_in_a_stat_is_not_useless() {
        // Zero attack must not mean zero damage, or a fresh character could not kill anything.
        let none = Rules::NONE;
        let empty = Stats::default();

        assert!(empty.damage_multiplier(&none) > 0.0);
        assert!(empty.shot_cooldown_ms(&none, 1.0) > 0);
        assert!(empty.movement_speed(&none) > 0.0);
    }

    #[test]
    fn something_that_cannot_move_stays_where_it_is() {
        assert_eq!(Stats::still().movement_speed(&Rules::NONE), 0.0);
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
