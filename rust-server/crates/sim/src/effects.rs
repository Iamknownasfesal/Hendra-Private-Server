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
    ///
    /// The blow is still worked out in full and still reported in full: `Enemy.HitByProjectile`
    /// (`Enemy.cs:106-119`) computes `dmg`, skips only the `HP -=` when Invulnerable, and puts the
    /// whole of `dmg` in the `Damage` packet. Reporting a zero instead is a boss that shrugs off a
    /// hit and shows nothing for it, which reads to a player as a shot that missed.
    pub no_damage: bool,

    /// Held exactly where it stands, whatever it or its client asks for.
    ///
    /// `Entity.ResolveNewLocation` (`Entity.cs:324`) returns the current position unchanged under
    /// either Paralyzed or Petrify, and `StatsManager.GetSpeed` (`:138`) reads zero under
    /// Paralyzed. Nothing else roots: a paused or frozen entity keeps whatever position its client
    /// or its behaviour last asked for.
    pub rooted: bool,

    /// Cannot fire the behaviour tree's `Shoot`.
    ///
    /// `Shoot.cs:132`, which is the only place Stunned stops a shot. A player's own weapon is
    /// never gated on it: neither `PlayerShootHandler` nor `Player.ValidatePlayerShoot`
    /// (`Player.AntiCheat.cs:88`) mentions any condition effect, so a stunned player who fires
    /// anyway is fired for.
    pub silenced: bool,

    /// The world has stopped for this one.
    ///
    /// Paused alone. `Player.Tick` (`Player.cs:565`) skips regeneration, effects, the ocean
    /// trench, activate effects, the fame counter, deferred hits and ground damage; `DamageCounter`
    /// (`:80`) hands out no experience; a world quake sends them to the Nexus.
    pub paused: bool,

    /// The behaviour tree does not run at all.
    ///
    /// Stasis alone. `Entity.Tick` (`Entity.cs:225`) skips `TickState` under Stasis and under
    /// nothing else, so the tree is frozen mid-state rather than merely refused its movement. A
    /// paused enemy, by contrast, keeps thinking, moving and shooting.
    pub frozen: bool,

    /// Hidden from other players.
    pub invisible: bool,

    /// Not something an enemy will chase or shoot at.
    ///
    /// `Player.IsVisibleToEnemy` (`Player.Effects.cs:102-113`): Paused, Invisible, Hidden. Wider
    /// than `invisible`, which is only about other players. Stasis is deliberately not in it —
    /// an entity in stasis is still hunted, it simply takes nothing from what reaches it.
    pub unseen_by_enemies: bool,

    /// Health regeneration is suppressed.
    pub no_health_regen: bool,

    /// Magic regeneration is suppressed.
    pub no_magic_regen: bool,

    /// Multiplies defence before it is subtracted.
    pub defence_multiplier: f32,

    /// Defence counts as nothing.
    pub ignore_defence: bool,

    /// Sharpens a hit by a fifth, against anything.
    pub cursed: bool,

    /// Softens a hit by a tenth, but only against a player.
    ///
    /// Kept apart from [`Rules::cursed`] because the original keeps two copies of the defence
    /// formula and only one of them reads it. `StatsManager.cs:85`, which every enemy and every
    /// breakable object goes through, has no `Petrify` clause at all; `StatsManager.cs:107`, which
    /// only a player goes through, has one. A petrified enemy therefore takes full damage.
    pub petrified: bool,

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
        paused: false,
        frozen: false,
        invisible: false,
        unseen_by_enemies: false,
        no_health_regen: false,
        no_magic_regen: false,
        defence_multiplier: 1.0,
        ignore_defence: false,
        cursed: false,
        petrified: false,
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

    /// What a ninja's speed costs, in magic a second.
    ///
    /// From `HandleEffects`. It is what stops the effect being free movement: run out and it ends.
    ///
    /// Twelve rather than the ten the original writes, because the original does not spend it a
    /// second at a time. `MP = Math.Max(0, (int)(MP - 10 * time.ElaspedMsDelta / 1000f))`
    /// (`Player.Effects.cs:53`) truncates to a whole point of magic on every tick, and the tick it
    /// runs on is the 332 ms world tick — `Player.Tick` is reached from `World.Tick`, which
    /// `TickWorlds1` only runs once two logic ticks have accumulated (`FLLogicTicker.cs:149-156`).
    /// Ten a second is 3.32 a tick, truncation takes the whole four, and four every 332 ms is
    /// 12.05 a second. Spending the written ten would give a ninja a fifth more running than the
    /// original ever gave one.
    pub const MAGIC_PER_SECOND: f32 = 4.0 / 0.332;

    /// The share of a hit that always lands, however much defence is in the way.
    pub const DAMAGE_FLOOR: f32 = 0.25;

    /// Reads a set of effects.
    pub fn of(conditions: ConditionSet) -> Rules {
        use ConditionEffect::*;

        let mut rules = Rules::NONE;

        // The three states the original keeps apart. Paused stops a player's own upkeep, Stasis
        // stops a behaviour tree, and neither is the other: `Entity.Tick` freezes the tree under
        // Stasis alone, while `Player.Tick` skips its upkeep under Paused alone.
        rules.paused = conditions.contains(Paused);
        rules.frozen = conditions.contains(Stasis);

        // `Enemy.HitByProjectile` returns before anything else when Invincible, and its damage
        // block is skipped entirely under Paused or Stasis. `Player.IsInvulnerable` is the same
        // three plus Invulnerable, and the player's hit path returns on all four.
        rules.untouchable = conditions.contains(Invincible) || rules.paused || rules.frozen;

        // Invulnerable is the softer one: the hit registers and its effects apply, but the health
        // subtraction is skipped and the defence calculation returns zero.
        rules.no_damage = rules.untouchable || conditions.contains(Invulnerable);

        rules.rooted = conditions.contains(Paralyzed) || conditions.contains(Petrify);
        rules.silenced = conditions.contains(Stunned);
        rules.invisible = conditions.contains(Invisible) || conditions.contains(Hidden);
        rules.unseen_by_enemies = rules.invisible || rules.paused;

        // Sick and Bleeding stop health returning; Quiet and NinjaSpeedy stop magic.
        rules.no_health_regen = conditions.contains(Sick) || conditions.contains(Bleeding);
        rules.no_magic_regen = conditions.contains(Quiet) || conditions.contains(NinjaSpeedy);

        if conditions.contains(Armored) {
            rules.defence_multiplier = 2.0;
        }
        rules.ignore_defence = conditions.contains(ArmorBroken);

        rules.petrified = conditions.contains(Petrify);
        rules.cursed = conditions.contains(Curse);

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
    ///
    /// `StatsManager.GetDefenseDamage`, both copies of it. `is_player` picks which: the static one
    /// at `StatsManager.cs:85` for anything else, the instance one at `:107` for a player. They
    /// differ only in the `Petrify` clause, and only one of them has it.
    ///
    /// Every step that multiplies truncates to a whole number before the next one reads it, and the
    /// caller truncates again — `(int)StatsManager.GetDefenseDamage(...)` at `Enemy.cs:70` and
    /// `:106`, `Player.cs:786` and `:813`. Rounding instead disagrees with the original by a point
    /// wherever the floor lands on a half, which is every third value of a hit that defence has
    /// swallowed.
    ///
    /// What the target's health actually loses is a separate question, and the caller asks it:
    /// [`Rules::no_damage`] skips the subtraction and changes nothing about the number, exactly as
    /// the original's `if (!HasConditionEffect(Invulnerable)) HP -= dmg;` sits beside a `Damage`
    /// packet carrying the whole of `dmg`.
    pub fn damage_after_defence(
        &self,
        raw: i32,
        defence: i32,
        armor_piercing: bool,
        is_player: bool,
    ) -> i32 {
        // Armour doubles defence before armour-piercing or broken armour throws it away, so a
        // pierced shot is unaffected by whether the target was armoured.
        let defence = if armor_piercing || self.ignore_defence {
            0
        } else {
            (defence as f32 * self.defence_multiplier) as i32
        };

        // The floor is a share of the raw hit, not of what defence left, so enough defence still
        // lets a quarter through. Not clamped at zero: the original does not clamp either, and a
        // negative defence is meant to make a hit land harder.
        let floor = raw as f32 * Rules::DAMAGE_FLOOR;
        let mut taken = ((raw - defence) as f32).max(floor);

        if is_player && self.petrified {
            taken = (taken * 0.9) as i32 as f32;
        }
        if self.cursed {
            taken = (taken * 1.20) as i32 as f32;
        }

        taken as i32
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

    // Only the immunity. `ApplyCondition` (`Entity.cs:738-772`) is eight paired tests and nothing
    // else — being untouchable is not among them. What keeps an effect off an invincible entity in
    // the original is the caller: a bullet never reaches it (`Enemy.cs:102`, `Player.cs:770`), a
    // blast guards its own `ApplyConditionEffect` (`Grenade.cs:97`), and so does an activated
    // ability (`Player.UseItem.cs:1197`). Folding it in here instead refuses effects the original
    // allows — an administrator, who is invincible while hidden, could not be given one at all.
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
        assert_eq!(rules.damage_after_defence(100, 0, false, false), 100);
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
            assert!(!rules.rooted, "{effect:?} does not hold anybody still");
        }
    }

    #[test]
    fn pausing_and_freezing_are_different_states() {
        // `Player.Tick` skips its upkeep under Paused and says nothing about Stasis;
        // `Entity.Tick` freezes the behaviour tree under Stasis and says nothing about Paused. A
        // single flag standing for both makes a paused enemy stop thinking, which it does not, and
        // an entity in stasis invisible to the enemies still hunting it.
        let paused = Rules::of(with(&[ConditionEffect::Paused]));
        let frozen = Rules::of(with(&[ConditionEffect::Stasis]));

        assert!(paused.paused && !paused.frozen);
        assert!(frozen.frozen && !frozen.paused);

        assert!(paused.unseen_by_enemies, "IsVisibleToEnemy checks Paused");
        assert!(
            !frozen.unseen_by_enemies,
            "and says nothing about Stasis, so an enemy still hunts them"
        );
    }

    #[test]
    fn petrify_holds_an_entity_where_it_stands() {
        // `Entity.ResolveNewLocation` returns the current position under Paralyzed *or* Petrify,
        // and it is the entity method, so an enemy is held by it exactly as a player is.
        let petrified = Rules::of(with(&[ConditionEffect::Petrify]));

        assert!(petrified.rooted);
        assert!(!petrified.silenced, "a petrified enemy still shoots");
    }

    #[test]
    fn a_stunned_player_is_only_stopped_by_their_own_client() {
        // Stunned appears nowhere in `Player`: not in `PlayerShootHandler`, not in
        // `ValidatePlayerShoot`, not in `GetAttackFrequency`. The live `StatsManager` has no
        // Stunned clause at all -- the one that did is commented out at `StatsManager.cs:167-179`.
        // It gates the behaviour tree's `Shoot` and the six tossing behaviours, and nothing else.
        let stunned = Rules::of(with(&[ConditionEffect::Stunned]));

        assert!(stunned.silenced);
        assert!(!stunned.rooted, "and does not hold anybody still either");
    }

    #[test]
    fn armour_doubles_defence_and_broken_armour_removes_it() {
        // Doubled, not increased by a fixed amount: `def *= 2` in StatsManager.
        let plain = Rules::NONE;
        let armoured = Rules::of(with(&[ConditionEffect::Armored]));
        let broken = Rules::of(with(&[ConditionEffect::ArmorBroken]));

        assert_eq!(plain.damage_after_defence(100, 20, false, false), 80);
        assert_eq!(armoured.damage_after_defence(100, 20, false, false), 60);
        assert_eq!(broken.damage_after_defence(100, 20, false, false), 100);
    }

    #[test]
    fn a_quarter_of_every_hit_lands_however_much_defence_is_in_the_way() {
        // `float limit = dmg * 0.25f`. Without a floor, enough defence makes a character immortal.
        assert_eq!(
            Rules::NONE.damage_after_defence(100, 1_000, false, false),
            25
        );
        assert_eq!(
            Rules::NONE.damage_after_defence(40, 1_000, false, false),
            10
        );
    }

    #[test]
    fn armour_piercing_ignores_defence() {
        assert_eq!(Rules::NONE.damage_after_defence(100, 80, true, false), 100);
    }

    #[test]
    fn petrify_softens_a_hit_and_curse_sharpens_it() {
        let petrified = Rules::of(with(&[ConditionEffect::Petrify]));
        let cursed = Rules::of(with(&[ConditionEffect::Curse]));

        assert_eq!(petrified.damage_after_defence(100, 0, false, true), 90);
        assert_eq!(cursed.damage_after_defence(100, 0, false, true), 120);
    }

    #[test]
    fn petrify_protects_a_player_and_does_nothing_for_an_enemy() {
        // The original keeps two copies of the formula and only the player's has a Petrify clause:
        // `StatsManager.cs:121` against `StatsManager.cs:85`, which has none. Folding the tenth into
        // one shared multiplier makes a petrified boss take ten per cent less than it should.
        let petrified = Rules::of(with(&[ConditionEffect::Petrify]));

        assert_eq!(petrified.damage_after_defence(100, 0, false, true), 90);
        assert_eq!(petrified.damage_after_defence(100, 0, false, false), 100);
    }

    #[test]
    fn every_multiplication_is_truncated_the_way_a_cast_to_int_is() {
        // `(int)(ret * 1.20)` and the caller's own `(int)`, both of which throw the fraction away
        // rather than rounding it. Rounding disagrees with the original by a whole point of health
        // wherever the arithmetic lands past a half.
        //
        // 43 raw against enough defence to reach the floor: 43 * 0.25 = 10.75, which the original
        // reports as 10.
        assert_eq!(
            Rules::NONE.damage_after_defence(43, 1_000, false, false),
            10
        );

        // 7 raw, cursed: 7 * 1.20 = 8.4, reported as 8.
        let cursed = Rules::of(with(&[ConditionEffect::Curse]));
        assert_eq!(cursed.damage_after_defence(7, 0, false, false), 8);

        // Truncated once per multiplication rather than once at the end: 100 petrified is 90, and
        // 90 cursed is 108. A single combined 1.08 would agree here and not everywhere.
        let both = Rules::of(with(&[ConditionEffect::Petrify, ConditionEffect::Curse]));
        assert_eq!(both.damage_after_defence(100, 0, false, true), 108);
    }

    #[test]
    fn negative_defence_makes_a_hit_land_harder() {
        // `dmg - def` with no clamp on `def`, so a negative defence adds. Clamping it at zero is a
        // reasonable-looking guard that silently disagrees with every debuff that lowers defence
        // below nothing.
        assert_eq!(
            Rules::NONE.damage_after_defence(100, -20, false, false),
            120
        );
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

        // There is no separate "cannot cast" flag, because the original has no such gate. Quiet
        // empties the magic bar every tick (`Player.Effects.cs:36`) and `UseItem` refuses only on
        // `MP < item.MpCost`, so an ability that costs nothing still works while quiet.
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
    fn being_untouchable_is_not_an_immunity() {
        // `ApplyCondition` is eight paired tests and nothing else. What keeps a bullet's effect off
        // an invincible entity is that the bullet never lands; what keeps a blast's off is the
        // blast's own guard. Refusing here as well would mean an administrator, invincible for as
        // long as they are hidden, could not be given a condition effect by any means at all.
        let invincible = with(&[ConditionEffect::Invincible]);

        assert!(accepts(invincible, ConditionEffect::Slowed));
        assert!(accepts(invincible, ConditionEffect::Paralyzed));
        assert!(accepts(invincible, ConditionEffect::Healing));

        // What does refuse is the matching immunity, whatever else is held.
        let armoured = with(&[ConditionEffect::Invincible, ConditionEffect::SlowedImmune]);
        assert!(!accepts(armoured, ConditionEffect::Slowed));
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
