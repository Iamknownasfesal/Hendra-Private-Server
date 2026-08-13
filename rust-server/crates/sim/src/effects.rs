//! What a condition effect does.
//!
//! An entity carries a set of condition effects. This turns that set into the arithmetic the
//! simulation acts on: what refuses damage, what stops movement, what scales a multiplier.
//!
//! # Where the numbers come from
//!
//! Every constant and every rule here is read from the original server's `StatsManager`,
//! `Enemy.Damage`, `Player.HitByProjectile` and `Player.Effects`. A server that disagreed with
//! those would disagree with every number a player can see and every fight the content was
//! balanced against.
//!
//! # Why a struct rather than bit tests
//!
//! The hot paths ask these questions constantly: every projectile against every target, every
//! movement of every entity, every tick. Testing eight bits at each of those points is eight
//! branches on a value that has not changed since the tick began.
//!
//! # What is not here
//!
//! Confused, Drunk, Hallucinating, Blind and Darkness change what a player sees, not what is true.
//! They travel in the snapshot and the client draws them; a server that acted on them would be
//! arbitrating from a picture it knows is false.

use hendra_content::{ConditionEffect, ConditionSet};

/// What a set of effects means, precomputed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rules {
    /// Cannot be hit at all. The projectile passes without registering, and no effect it carries
    /// lands. Distinct from [`Rules::no_damage`], which registers the hit and takes its effects.
    pub untouchable: bool,

    /// Registers hits and takes their effects, but loses no health.
    pub no_damage: bool,

    /// Cannot move.
    pub rooted: bool,

    /// Cannot shoot.
    pub silenced: bool,

    /// Cannot use abilities, and holds no magic.
    pub quiet: bool,

    /// Neither moves nor thinks.
    pub paused: bool,

    /// Hidden from other players.
    pub invisible: bool,

    /// Health regeneration is suppressed.
    pub no_health_regen: bool,

    /// Magic regeneration is suppressed.
    pub no_magic_regen: bool,

    /// Multiplies defence before it is subtracted.
    pub defence_multiplier: f32,

    /// Defence counts as nothing.
    pub ignore_defence: bool,

    /// Multiplies damage after defence.
    pub damage_taken: f32,

    /// Health per second, positive or negative.
    pub health_per_second: f32,

    /// Attack counts as nothing, holding damage at the minimum multiplier.
    pub weak: bool,

    /// Damage dealt is multiplied by one and a half.
    pub damaging: bool,

    /// Dexterity counts as nothing, holding rate of fire at the minimum.
    pub dazed: bool,

    /// Rate of fire is multiplied by one and a half.
    pub berserk: bool,

    /// Speed is multiplied by one and a half.
    pub speedy: bool,

    /// Speed is held at the base, whatever the stat says.
    pub slowed: bool,

    /// Vitality counts as nothing for regeneration.
    pub sick: bool,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules::NONE
    }
}

impl Rules {
    /// An entity with nothing on it.
    pub const NONE: Rules = Rules {
        untouchable: false,
        no_damage: false,
        rooted: false,
        silenced: false,
        quiet: false,
        paused: false,
        invisible: false,
        no_health_regen: false,
        no_magic_regen: false,
        defence_multiplier: 1.0,
        ignore_defence: false,
        damage_taken: 1.0,
        health_per_second: 0.0,
        weak: false,
        damaging: false,
        dazed: false,
        berserk: false,
        speedy: false,
        slowed: false,
        sick: false,
    };

    /// How much health `Healing` and `Bleeding` move per second.
    pub const HEALTH_PER_SECOND: f32 = 28.0;

    /// The share of a hit that always lands, however much defence is in the way.
    pub const DAMAGE_FLOOR: f32 = 0.25;

    /// Reads a set of effects.
    pub fn of(conditions: ConditionSet) -> Rules {
        use ConditionEffect::*;

        let mut rules = Rules::NONE;

        // `Enemy.HitByProjectile` returns before anything else when Invincible, and its damage
        // block is skipped entirely under Paused or Stasis.
        rules.paused = conditions.contains(Paused) || conditions.contains(Stasis);
        rules.untouchable = conditions.contains(Invincible) || rules.paused;

        // Invulnerable is the softer one: the hit registers and its effects apply, but the health
        // subtraction is skipped and the defence calculation returns zero.
        rules.no_damage = rules.untouchable || conditions.contains(Invulnerable);

        rules.rooted = rules.paused || conditions.contains(Paralyzed);
        rules.silenced = conditions.contains(Stunned) || rules.paused;
        rules.quiet = conditions.contains(Quiet) || rules.paused;
        rules.invisible = conditions.contains(Invisible);

        // Sick and Bleeding stop health returning; Quiet and NinjaSpeedy stop magic.
        rules.no_health_regen = conditions.contains(Sick) || conditions.contains(Bleeding);
        rules.no_magic_regen = conditions.contains(Quiet) || conditions.contains(NinjaSpeedy);

        if conditions.contains(Armored) {
            rules.defence_multiplier = 2.0;
        }
        rules.ignore_defence = conditions.contains(ArmorBroken);

        if conditions.contains(Petrify) {
            rules.damage_taken *= 0.9;
        }
        if conditions.contains(Curse) {
            rules.damage_taken *= 1.20;
        }

        // Healing is refused while Sick, and bleeding never takes the last point.
        if conditions.contains(Healing) && !conditions.contains(Sick) {
            rules.health_per_second += Rules::HEALTH_PER_SECOND;
        }
        if conditions.contains(Bleeding) {
            rules.health_per_second -= Rules::HEALTH_PER_SECOND;
        }

        // These are read by the stat formulas rather than folded into a multiplier here, because
        // the original applies them inside the same expression that reads the stat: Weak does not
        // halve damage, it holds attack at its minimum.
        rules.weak = conditions.contains(Weak);
        rules.damaging = conditions.contains(Damaging);
        rules.dazed = conditions.contains(Dazed);
        rules.berserk = conditions.contains(Berserk);
        rules.speedy = conditions.contains(Speedy);
        rules.slowed = conditions.contains(Slowed);
        rules.sick = conditions.contains(Sick);

        rules
    }

    /// What a raw hit costs after defence and the effects on the entity taking it.
    pub fn damage_after_defence(&self, raw: i32, defence: i32, armor_piercing: bool) -> i32 {
        if self.no_damage {
            return 0;
        }

        let defence = if armor_piercing || self.ignore_defence {
            0
        } else {
            (defence as f32 * self.defence_multiplier) as i32
        };

        let floor = raw as f32 * Rules::DAMAGE_FLOOR;
        let reduced = (raw - defence.max(0)) as f32;

        (reduced.max(floor) * self.damage_taken).round().max(0.0) as i32
    }
}

/// Whether an effect may be given, or is refused by an immunity already held.
///
/// `SlowedImmune` refuses `Slowed` and nothing else, so getting the pairing wrong makes a ring that
/// reads as protection do nothing at all.
pub fn accepts(conditions: ConditionSet, effect: ConditionEffect) -> bool {
    use ConditionEffect::*;

    let immunity = match effect {
        Slowed => Some(SlowedImmune),
        Dazed => Some(DazedImmune),
        Paralyzed => Some(ParalyzeImmune),
        Stunned => Some(StunImmune),
        Stasis => Some(StasisImmune),
        Petrify => Some(PetrifyImmune),
        Curse => Some(CurseImmune),
        ArmorBroken => Some(ArmorBreakImmune),
        _ => None,
    };

    // An untouchable entity takes no effect from a hit at all. Whether an effect is one an enemy
    // inflicts is decided by whether it has an immunity, which is exactly that set.
    if immunity.is_some() && conditions.contains(Invincible) {
        return false;
    }

    immunity.is_none_or(|immune| !conditions.contains(immune))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(effects: &[ConditionEffect]) -> ConditionSet {
        let mut set = ConditionSet::EMPTY;
        for effect in effects {
            set.insert(*effect);
        }
        set
    }

    #[test]
    fn nothing_held_means_nothing_changed() {
        let rules = Rules::of(ConditionSet::EMPTY);

        assert_eq!(rules, Rules::NONE);
        assert_eq!(rules.damage_after_defence(100, 0, false), 100);
    }

    #[test]
    fn invincible_is_not_hit_at_all_while_invulnerable_only_loses_no_health() {
        // The original distinguishes these. Invincible returns from HitByProjectile before anything
        // happens, so no condition effect the shot carried lands. Invulnerable takes the hit and
        // its effects and skips only the health subtraction.
        let invincible = Rules::of(with(&[ConditionEffect::Invincible]));
        let invulnerable = Rules::of(with(&[ConditionEffect::Invulnerable]));

        assert!(invincible.untouchable);
        assert!(invincible.no_damage);

        assert!(!invulnerable.untouchable, "the hit still registers");
        assert!(invulnerable.no_damage, "but costs nothing");
    }

    #[test]
    fn a_paused_or_frozen_entity_cannot_be_damaged() {
        // The damage block in Enemy.cs is skipped entirely under either.
        for effect in [ConditionEffect::Paused, ConditionEffect::Stasis] {
            let rules = Rules::of(with(&[effect]));
            assert!(rules.untouchable, "{effect:?} should refuse the hit");
            assert!(rules.paused);
            assert!(rules.rooted);
        }
    }

    #[test]
    fn armour_doubles_defence_and_broken_armour_removes_it() {
        // Doubled, not increased by a fixed amount: `def *= 2` in StatsManager.
        let plain = Rules::NONE;
        let armoured = Rules::of(with(&[ConditionEffect::Armored]));
        let broken = Rules::of(with(&[ConditionEffect::ArmorBroken]));

        assert_eq!(plain.damage_after_defence(100, 20, false), 80);
        assert_eq!(armoured.damage_after_defence(100, 20, false), 60);
        assert_eq!(broken.damage_after_defence(100, 20, false), 100);
    }

    #[test]
    fn a_quarter_of_every_hit_lands_however_much_defence_is_in_the_way() {
        // `float limit = dmg * 0.25f`. Without a floor, enough defence makes a character immortal.
        assert_eq!(Rules::NONE.damage_after_defence(100, 1_000, false), 25);
        assert_eq!(Rules::NONE.damage_after_defence(40, 1_000, false), 10);
    }

    #[test]
    fn armour_piercing_ignores_defence() {
        assert_eq!(Rules::NONE.damage_after_defence(100, 80, true), 100);
    }

    #[test]
    fn petrify_softens_a_hit_and_curse_sharpens_it() {
        let petrified = Rules::of(with(&[ConditionEffect::Petrify]));
        let cursed = Rules::of(with(&[ConditionEffect::Curse]));

        assert_eq!(petrified.damage_after_defence(100, 0, false), 90);
        assert_eq!(cursed.damage_after_defence(100, 0, false), 120);
    }

    #[test]
    fn paralysis_stops_movement_but_not_shooting() {
        let rules = Rules::of(with(&[ConditionEffect::Paralyzed]));

        assert!(rules.rooted);
        assert!(!rules.silenced, "a paralysed enemy is still dangerous");
        assert!(!rules.paused);
    }

    #[test]
    fn bleeding_and_healing_move_health_in_opposite_directions() {
        assert_eq!(
            Rules::of(with(&[ConditionEffect::Healing])).health_per_second,
            Rules::HEALTH_PER_SECOND
        );
        assert_eq!(
            Rules::of(with(&[ConditionEffect::Bleeding])).health_per_second,
            -Rules::HEALTH_PER_SECOND
        );
    }

    #[test]
    fn being_sick_refuses_healing_rather_than_racing_it() {
        // `HasConditionEffect(Healing) && !HasConditionEffect(Sick)`. Applying both and letting
        // them cancel would look the same until one of the two numbers changed.
        let both = Rules::of(with(&[ConditionEffect::Healing, ConditionEffect::Sick]));

        assert_eq!(both.health_per_second, 0.0);
        assert!(both.no_health_regen);
    }

    #[test]
    fn bleeding_stops_health_returning_as_well_as_taking_it() {
        let rules = Rules::of(with(&[ConditionEffect::Bleeding]));

        assert!(rules.no_health_regen, "CanHpRegen refuses while bleeding");
        assert!(rules.health_per_second < 0.0);
    }

    #[test]
    fn quiet_and_ninja_speed_stop_magic_returning() {
        assert!(Rules::of(with(&[ConditionEffect::Quiet])).no_magic_regen);
        assert!(Rules::of(with(&[ConditionEffect::NinjaSpeedy])).no_magic_regen);
        assert!(Rules::of(with(&[ConditionEffect::Quiet])).quiet);
    }

    #[test]
    fn an_immunity_refuses_exactly_its_own_effect() {
        let slowed_immune = with(&[ConditionEffect::SlowedImmune]);

        assert!(!accepts(slowed_immune, ConditionEffect::Slowed));
        assert!(accepts(slowed_immune, ConditionEffect::Paralyzed));
        assert!(accepts(slowed_immune, ConditionEffect::Dazed));
    }

    #[test]
    fn every_immunity_is_paired_with_something() {
        let pairs = [
            (ConditionEffect::SlowedImmune, ConditionEffect::Slowed),
            (ConditionEffect::DazedImmune, ConditionEffect::Dazed),
            (ConditionEffect::ParalyzeImmune, ConditionEffect::Paralyzed),
            (ConditionEffect::StunImmune, ConditionEffect::Stunned),
            (ConditionEffect::StasisImmune, ConditionEffect::Stasis),
            (ConditionEffect::PetrifyImmune, ConditionEffect::Petrify),
            (ConditionEffect::CurseImmune, ConditionEffect::Curse),
            (
                ConditionEffect::ArmorBreakImmune,
                ConditionEffect::ArmorBroken,
            ),
        ];

        for (immunity, effect) in pairs {
            assert!(
                !accepts(with(&[immunity]), effect),
                "{immunity:?} should refuse {effect:?}"
            );
        }
    }

    #[test]
    fn an_untouchable_entity_takes_no_effect_from_a_hit() {
        let invincible = with(&[ConditionEffect::Invincible]);

        assert!(!accepts(invincible, ConditionEffect::Slowed));
        assert!(!accepts(invincible, ConditionEffect::Paralyzed));

        // The beneficial ones have no immunity because nobody resists them.
        assert!(accepts(invincible, ConditionEffect::Healing));
        assert!(accepts(invincible, ConditionEffect::Speedy));
    }

    #[test]
    fn seeing_things_is_the_client_s_business_and_changes_nothing_here() {
        for effect in [
            ConditionEffect::Confused,
            ConditionEffect::Drunk,
            ConditionEffect::Hallucinating,
            ConditionEffect::Blind,
            ConditionEffect::Darkness,
        ] {
            assert_eq!(
                Rules::of(with(&[effect])),
                Rules::NONE,
                "{effect:?} should not change the simulation"
            );
        }
    }
}
