//! What a condition effect does.
//!
//! Effects were stored, expired and sent to clients, and nothing in the simulation read them. That
//! made `conditional_effect(invulnerable)` — the most used behaviour in the game's content — mark a
//! boss invulnerable while leaving it perfectly killable, and paralysis a word rather than a state.
//!
//! # Why a struct rather than bit tests
//!
//! The hot paths ask these questions constantly: every projectile against every target, every
//! movement of every entity, every tick. Testing eight bits at each of those points is eight
//! branches on a value that has not changed since the tick began.
//!
//! [`Rules`] is derived once per entity per use and answers in arithmetic. The multipliers are
//! already combined, so the damage path multiplies by one number rather than asking which of Weak,
//! Damaging and Berserk are set.
//!
//! # What is not here
//!
//! Confused, Drunk, Hallucinating, Blind and Darkness change what a player *sees*, not what is
//! true. They are carried in the snapshot and the client draws them; the server would be wrong to
//! act on them, because a server that lies to itself cannot arbitrate.

use hendra_content::{ConditionEffect, ConditionSet};

/// What a set of effects means, precomputed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rules {
    /// Refuses all damage.
    pub invulnerable: bool,

    /// Cannot move at all.
    pub rooted: bool,

    /// Cannot shoot.
    pub silenced: bool,

    /// Cannot use abilities.
    pub quiet: bool,

    /// Cannot be healed.
    pub sick: bool,

    /// Neither moves nor thinks. Different from rooted: a paused entity does not act either.
    pub paused: bool,

    /// Hidden from other players.
    pub invisible: bool,

    /// Multiplies movement.
    pub speed: f32,

    /// Multiplies damage dealt.
    pub damage_dealt: f32,

    /// Multiplies damage taken.
    pub damage_taken: f32,

    /// Multiplies weapon cooldown. Above one is slower.
    pub cooldown: f32,

    /// Health per second, positive or negative.
    pub health_per_second: f32,

    /// Added to defence, which may take it below zero.
    pub defence: i32,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules::NONE
    }
}

impl Rules {
    /// An entity with nothing on it.
    pub const NONE: Rules = Rules {
        invulnerable: false,
        rooted: false,
        silenced: false,
        quiet: false,
        sick: false,
        paused: false,
        invisible: false,
        speed: 1.0,
        damage_dealt: 1.0,
        damage_taken: 1.0,
        cooldown: 1.0,
        health_per_second: 0.0,
        defence: 0,
    };

    /// How much health `Healing` and `Bleeding` move per second.
    pub const HEAL_PER_SECOND: f32 = 20.0;
    pub const BLEED_PER_SECOND: f32 = 20.0;

    /// What `Armored` and `ArmorBroken` are worth.
    ///
    /// Armour is added rather than multiplied because defence is already subtracted from damage;
    /// multiplying it would make armour worthless against small hits and absolute against large
    /// ones, which is the opposite of what it is for.
    pub const ARMOUR: i32 = 20;

    /// Reads a set of effects.
    pub fn of(conditions: ConditionSet) -> Rules {
        use ConditionEffect::*;

        let mut rules = Rules::NONE;

        // Invincible and Invulnerable differ in the original by whether they stop status effects
        // as well as damage. Both stop damage, which is what this path is deciding.
        rules.invulnerable = conditions.contains(Invulnerable) || conditions.contains(Invincible);

        // Stasis is not just immobility — a thing in stasis is out of the fight entirely.
        rules.paused = conditions.contains(Paused) || conditions.contains(Stasis);
        rules.rooted =
            rules.paused || conditions.contains(Paralyzed) || conditions.contains(Petrify);

        rules.silenced = conditions.contains(Stunned) || rules.paused;
        rules.quiet = conditions.contains(Quiet) || rules.paused;
        rules.sick = conditions.contains(Sick);
        rules.invisible = conditions.contains(Invisible);

        // Speed. Slowed and Speedy can be held at once — the content does apply both — and the
        // result is that they cancel, which is what multiplying gives without a special case.
        if conditions.contains(Slowed) {
            rules.speed *= 0.5;
        }
        if conditions.contains(Speedy) {
            rules.speed *= 1.5;
        }
        if conditions.contains(NinjaSpeedy) {
            rules.speed *= 1.6;
        }

        if conditions.contains(Weak) {
            rules.damage_dealt *= 0.5;
        }
        if conditions.contains(Damaging) {
            rules.damage_dealt *= 1.5;
        }
        if conditions.contains(Berserk) {
            rules.damage_dealt *= 1.25;
            rules.cooldown *= 0.75;
        }

        if conditions.contains(Curse) {
            rules.damage_taken *= 1.2;
        }
        if conditions.contains(Hexed) {
            rules.damage_taken *= 1.25;
        }

        if conditions.contains(Dazed) {
            rules.cooldown *= 1.5;
        }

        if conditions.contains(Healing) {
            rules.health_per_second += Rules::HEAL_PER_SECOND;
        }
        if conditions.contains(Bleeding) {
            rules.health_per_second -= Rules::BLEED_PER_SECOND;
        }

        if conditions.contains(Armored) {
            rules.defence += Rules::ARMOUR;
        }
        if conditions.contains(ArmorBroken) {
            rules.defence -= Rules::ARMOUR;
        }

        rules
    }
}

/// Whether an effect may be given, or is refused by an immunity already held.
///
/// The immunities are the reason this is a function rather than a lookup: `SlowedImmune` refuses
/// `Slowed` and nothing else, and getting the pairing wrong makes a ring that reads as protection
/// do nothing at all.
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

    // Invincible refuses everything harmful. Whether an effect is harmful is decided by whether it
    // has an immunity of its own, which is exactly the set the game treats as attacks.
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
        assert_eq!(rules.speed, 1.0);
        assert_eq!(rules.damage_dealt, 1.0);
        assert!(!rules.invulnerable);
    }

    #[test]
    fn invulnerability_is_read_from_either_spelling() {
        // The content uses both, and a boss that used the other one would have been killable.
        assert!(Rules::of(with(&[ConditionEffect::Invulnerable])).invulnerable);
        assert!(Rules::of(with(&[ConditionEffect::Invincible])).invulnerable);
    }

    #[test]
    fn opposing_speed_effects_cancel_rather_than_fighting() {
        let both = Rules::of(with(&[ConditionEffect::Slowed, ConditionEffect::Speedy]));

        assert!(
            (both.speed - 0.75).abs() < 0.001,
            "half then half again as fast: {}",
            both.speed
        );
        assert!(Rules::of(with(&[ConditionEffect::Slowed])).speed < 1.0);
        assert!(Rules::of(with(&[ConditionEffect::Speedy])).speed > 1.0);
    }

    #[test]
    fn stasis_takes_an_entity_out_of_the_fight_entirely() {
        let rules = Rules::of(with(&[ConditionEffect::Stasis]));

        assert!(rules.paused);
        assert!(rules.rooted, "and cannot move");
        assert!(rules.silenced, "and cannot shoot");
    }

    #[test]
    fn paralysis_stops_movement_but_not_shooting() {
        // The difference between the two matters: a paralysed enemy is still dangerous, and
        // conflating them would make every paralysis a stun.
        let rules = Rules::of(with(&[ConditionEffect::Paralyzed]));

        assert!(rules.rooted);
        assert!(!rules.silenced);
        assert!(!rules.paused);
    }

    #[test]
    fn armour_is_added_rather_than_multiplied() {
        // Multiplying defence makes armour worthless against small hits and absolute against large
        // ones, which is the opposite of what it is for.
        assert_eq!(
            Rules::of(with(&[ConditionEffect::Armored])).defence,
            Rules::ARMOUR
        );
        assert_eq!(
            Rules::of(with(&[ConditionEffect::ArmorBroken])).defence,
            -Rules::ARMOUR
        );
        assert_eq!(
            Rules::of(with(&[
                ConditionEffect::Armored,
                ConditionEffect::ArmorBroken
            ]))
            .defence,
            0,
            "held at once they cancel"
        );
    }

    #[test]
    fn bleeding_and_healing_move_health_in_opposite_directions() {
        assert!(Rules::of(with(&[ConditionEffect::Healing])).health_per_second > 0.0);
        assert!(Rules::of(with(&[ConditionEffect::Bleeding])).health_per_second < 0.0);
        assert_eq!(
            Rules::of(with(&[ConditionEffect::Healing, ConditionEffect::Bleeding]))
                .health_per_second,
            0.0
        );
    }

    #[test]
    fn an_immunity_refuses_exactly_its_own_effect() {
        // A ring of protection that refused the wrong effect would read as protection and do
        // nothing, which is the worst of both.
        let slowed_immune = with(&[ConditionEffect::SlowedImmune]);

        assert!(!accepts(slowed_immune, ConditionEffect::Slowed));
        assert!(accepts(slowed_immune, ConditionEffect::Paralyzed));
        assert!(accepts(slowed_immune, ConditionEffect::Dazed));
    }

    #[test]
    fn every_immunity_is_paired_with_something() {
        // Each of these exists in the content to stop one specific effect, so a pairing that was
        // never written is an immunity that silently does nothing.
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
            let held = with(&[immunity]);
            assert!(
                !accepts(held, effect),
                "{immunity:?} should refuse {effect:?}"
            );
        }
    }

    #[test]
    fn invincibility_refuses_everything_that_could_be_resisted() {
        let invincible = with(&[ConditionEffect::Invincible]);

        assert!(!accepts(invincible, ConditionEffect::Slowed));
        assert!(!accepts(invincible, ConditionEffect::Paralyzed));
        assert!(!accepts(invincible, ConditionEffect::Curse));

        // But not the beneficial ones, which have no immunity because nobody resists them.
        assert!(accepts(invincible, ConditionEffect::Healing));
        assert!(accepts(invincible, ConditionEffect::Speedy));
    }

    #[test]
    fn seeing_things_is_the_client_s_business_and_changes_nothing_here() {
        // A server that acted on these would be arbitrating from a picture it knows is false.
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
