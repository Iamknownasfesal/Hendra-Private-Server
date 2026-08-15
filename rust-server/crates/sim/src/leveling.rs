//! Experience, levels and fame.
//!
//! # Where the numbers come from
//!
//! Every formula here is read from the original server's `Player.Leveling` and `DamageCounter`. A
//! server that used different curves would disagree with the experience bar the client draws and
//! with every expectation a returning player has.
//!
//! # How a kill is shared
//!
//! Not by damage contribution. Every player within [`SHARE_RADIUS`] of the kill receives the same
//! amount, capped at a share of their own next level, and a paused player receives none. That is
//! deliberate in the original: it means helping someone else's fight is never worse for you than
//! standing elsewhere, and it is why the game's dungeons fill up rather than emptying into races.

use hendra_content::{PlayerDesc, STATS};

use crate::stats::Stats;

/// The highest level a character reaches.
pub const MAX_LEVEL: i16 = 20;

/// How far from a kill experience carries.
pub const SHARE_RADIUS: f32 = 25.0;

/// The share of a level one kill may be worth.
///
/// Without a cap, one enormous enemy would take a character from one to twenty. With it, levelling
/// is always at least ten kills whatever is being fought.
const LEVEL_SHARE: f32 = 0.1;

/// The share of a level the enemy your quest arrow points at may be worth.
///
/// Five times the usual, from `DamageCounter`. It is the whole reason to follow the arrow: the same
/// enemy killed off your own bat is worth a tenth of a level, and killing the one you were sent to
/// is worth half of one.
const QUEST_SHARE: f32 = 0.5;

/// What one enemy is worth before the cap: a tenth of its health, scaled by its own multiplier.
const HEALTH_PER_EXPERIENCE: f32 = 10.0;

/// What a kill is worth in a world whose name carries "Theatre".
///
/// `DamageCounter.cs:95-96` matches the world's display name and takes a third of whatever the cap
/// allowed. The Puppet Master's Theatre is the only world in the content it matches, and its waves
/// are dense enough that full rate would make it the fastest levelling in the game.
pub const THEATRE_SHARE: f32 = 0.33;

/// What an experience boost multiplies a kill by.
///
/// Flat, from `DamageCounter.cs:99`: the boost item's own multiplier is never consulted, only
/// whether there is time left on it.
pub const BOOST_MULTIPLIER: f32 = 2.0;

/// Experience needed to move from `level` to the next.
pub fn experience_goal(level: i16) -> i32 {
    50 + (level.max(1) as i32 - 1) * 100
}

/// Total experience a character has accumulated by the time it reaches `level`.
pub fn experience_at(level: i16) -> i32 {
    let level = level.max(1) as i32;
    if level == 1 {
        return 0;
    }
    50 * (level - 1) + (level - 2) * (level - 1) * 50
}

/// Fame earned from a lifetime's experience: one per thousand.
///
/// The branch is the original's, from `Player.Leveling.cs:234`, and the two arms compute the same
/// thing — `200 + (e - 200_000) / 1000` is `e / 1000`. It reads as though late experience was meant
/// to be worth less and the halving was never written. Kept branch and all, because straightening
/// it would invite someone to make the arms differ and quietly change what a lifetime is worth.
pub fn fame_from_experience(experience: i32) -> i32 {
    const KNEE: i32 = 200 * 1000;
    if experience < KNEE {
        experience / 1000
    } else {
        200 + (experience - KNEE) / 1000
    }
}

/// The next fame milestone above `fame`, or zero once the last is passed.
pub fn fame_goal(fame: i32) -> i32 {
    match fame {
        f if f >= 2000 => 0,
        f if f >= 800 => 2000,
        f if f >= 400 => 800,
        f if f >= 150 => 400,
        f if f >= 20 => 150,
        _ => 20,
    }
}

/// How many stars a fame total is worth.
pub fn stars(best_fame: i32) -> i32 {
    match best_fame {
        f if f >= 2000 => 5,
        f if f >= 800 => 4,
        f if f >= 400 => 3,
        f if f >= 150 => 2,
        f if f >= 20 => 1,
        _ => 0,
    }
}

/// What one kill is worth to one player.
///
/// `max_hp / 10 * multiplier`, capped at a share of the player's own next level, then scaled by
/// where the kill happened and whether the player is boosted. An enemy that awards nothing returns
/// zero however large it is.
///
/// The two scalings come after the cap, which is `DamageCounter.cs:85-99` in order: the cap is
/// chosen and applied, and only then does the theatre take its third and the boost its doubling. A
/// boosted kill is therefore worth twice the cap rather than being held at it.
///
/// Rounding happens once, at the very end. `DamageCounter` carries a `float` through every step and
/// casts once at `(int)playerXp` (`DamageCounter.cs:104`), so truncating earlier would lose
/// fractions the later multiplications would have kept.
///
/// The top level is not a gate on this. `DamageCounter.Death` (`DamageCounter.cs:82-99`) never
/// looks at the level except to decide whether an experience boost doubles the figure, and
/// `EnemyKilled` adds whatever it is handed (`Player.Leveling.cs:317-320`). Only the level-up
/// itself stops at twenty, which is what leaves a finished character still earning the experience
/// its fame is counted from.
pub fn experience_for_kill(
    enemy_max_hp: i32,
    experience_multiplier: f32,
    awards_experience: bool,
    player_level: i16,
    was_quest: bool,
    world_multiplier: f32,
    boosted: bool,
) -> i32 {
    if !awards_experience {
        return 0;
    }

    let raw = (enemy_max_hp as f32 / HEALTH_PER_EXPERIENCE) * experience_multiplier.max(0.0);
    let share = if was_quest { QUEST_SHARE } else { LEVEL_SHARE };
    let cap = experience_goal(player_level) as f32 * share;

    let mut earned = raw.min(cap).max(0.0) * world_multiplier.max(0.0);

    // A finished character gets no doubling: `TickActivateEffects` clears the boost the moment the
    // level reaches twenty (`Player.cs:595-597`) and the award checks the level again besides.
    if boosted && player_level < MAX_LEVEL {
        earned *= BOOST_MULTIPLIER;
    }

    earned as i32
}

/// A character's progress, and what changes when it advances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub level: i16,
    pub experience: i32,
    pub fame: i32,
}

/// What advancing produced, for whoever has to tell the player about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Advance {
    pub levels_gained: i16,
    pub fame_gained: i32,
    pub reached_maximum: bool,

    /// Whether this gain carried the character past a fame milestone.
    ///
    /// `CalculateFame` compares the goal before against the goal after and announces a completed
    /// class quest when the second is higher (`Player.Leveling.cs:242-253`). The comparison is
    /// `>` rather than `!=`, and [`fame_goal`] returns zero once the last milestone is behind, so
    /// passing two thousand fame — the end of the ladder — announces nothing. That is the
    /// original's arithmetic rather than a decision, and it is what players saw.
    ///
    /// The original measures against the best fame this class has ever reached rather than against
    /// this character's, so a second character of the same class re-crossing a milestone its
    /// predecessor already passed announces nothing. That record lives on the account rather than
    /// in the world, so this measures against the character alone: a milestone crossed is a
    /// milestone announced.
    pub crossed_fame_goal: bool,
}

impl Advance {
    pub fn is_nothing(&self) -> bool {
        self.levels_gained == 0 && self.fame_gained == 0
    }
}

impl Progress {
    /// A character at the beginning.
    pub fn new() -> Progress {
        Progress {
            level: 1,
            experience: 0,
            fame: 0,
        }
    }

    /// How much more experience this level needs.
    pub fn to_next_level(&self) -> i32 {
        (experience_at(self.level) + experience_goal(self.level) - self.experience).max(0)
    }

    /// Adds experience and advances at most one level.
    ///
    /// `roll` returns a value in `0.0..1.0` and is called once per stat, so the caller owns the
    /// randomness and a test can make levelling deterministic.
    ///
    /// One level per call, never two, because `Player.Leveling.CheckLevelUp` is a single `if`
    /// (`Player.Leveling.cs:270`) rather than a loop. A kill worth more than a level leaves the
    /// surplus on the total, where the next kill collects it: the level is late by one kill rather
    /// than the experience being lost.
    ///
    /// The fame is recalculated only when no level was gained, which is where `CheckLevelUp` puts
    /// its call to `CalculateFame` (`Player.Leveling.cs:304`). A level-up therefore holds the fame
    /// back until the next kill that does not produce one.
    ///
    /// A kill worth nothing still runs both checks. `EnemyKilled` guards only the addition — `if
    /// (exp != 0) Experience += exp` — and then calls `CheckLevelUp` unconditionally
    /// (`Player.Leveling.cs:317-322`). So an enemy that awards no experience at all is what banks
    /// the fame a level-up held back, and can hand over a level the surplus had already paid for.
    pub fn gain(
        &mut self,
        class: &PlayerDesc,
        stats: &mut Stats,
        experience: i32,
        mut roll: impl FnMut() -> f32,
    ) -> Advance {
        let mut advance = Advance::default();
        if experience < 0 {
            return advance;
        }

        self.experience = self.experience.saturating_add(experience);

        if self.level < MAX_LEVEL
            && self.experience - experience_at(self.level) >= experience_goal(self.level)
        {
            self.level += 1;
            advance.levels_gained = 1;
            advance.reached_maximum = self.level >= MAX_LEVEL;

            for stat in STATS {
                let growth = class.stat(stat);
                let span = (growth.max_increase - growth.min_increase).max(0);
                let gained = growth.min_increase + (roll() * (span + 1) as f32) as i32;
                stats.raise(class, stat, gained);
            }

            return advance;
        }

        let fame = fame_from_experience(self.experience);
        if fame > self.fame {
            advance.fame_gained = fame - self.fame;
            advance.crossed_fame_goal = fame_goal(fame) > fame_goal(self.fame);
            self.fame = fame;
        }

        advance
    }

    /// Jumps straight to the top level, granting the stats the skipped levels would have.
    ///
    /// Each stat gains the average of its per-level range for every level not yet reached, rather
    /// than a fresh roll per level: a shortcut should not be able to roll badly. Experience is left
    /// where it is, so this raises the level without also crediting kills nobody made.
    ///
    /// Returns whether anything changed, which is false for a character already at the top.
    pub fn jump_to_max_level(&mut self, class: &PlayerDesc, stats: &mut Stats) -> bool {
        if self.level >= MAX_LEVEL {
            return false;
        }

        let levels = (MAX_LEVEL + 1 - self.level) as i32;
        for stat in STATS {
            let growth = class.stat(stat);
            stats.raise(
                class,
                stat,
                (growth.max_increase + growth.min_increase) * levels / 2,
            );
        }

        self.level = MAX_LEVEL;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hendra_content::{Node, ObjectType, Stat};

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
      <LevelIncrease min="2" max="8">MaxMagicPoints</LevelIncrease>
      <LevelIncrease min="0" max="2">Attack</LevelIncrease>
      <LevelIncrease min="1" max="2">Speed</LevelIncrease>
   </Object>
</Objects>"#;

    fn wizard() -> PlayerDesc {
        let document = Node::parse(WIZARD).unwrap();
        let node = document.children_named("Object").next().unwrap();
        PlayerDesc::parse(node, ObjectType(0x030e)).unwrap()
    }

    #[test]
    fn jumping_to_twenty_grants_the_average_gain_for_every_level_skipped() {
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::default();
        progress.level = 1;

        assert!(progress.jump_to_max_level(&class, &mut stats));
        assert_eq!(progress.level, MAX_LEVEL);

        // `(max + min) * (21 - level) / 2` from Level20Command.cs, which is the average per-level
        // gain over the twenty levels: health rises by (30 + 20) * 20 / 2 = 500 from its starting
        // hundred, and attack by (2 + 0) * 20 / 2 = 20 from its starting twelve.
        assert_eq!(stats.base(Stat::MaxHitPoints), 600);
        assert_eq!(stats.base(Stat::Attack), 32);

        // A stat the class never raises per level is left exactly where it started.
        assert_eq!(stats.base(Stat::Defense), 0);
    }

    #[test]
    fn jumping_to_twenty_stops_at_the_class_maximum() {
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::default();
        progress.level = 1;

        progress.jump_to_max_level(&class, &mut stats);

        // Magic rises by (8 + 2) * 20 / 2 = 100 to two hundred, well under its ceiling, while speed
        // rises by (2 + 1) * 20 / 2 = 30 to forty-two, also under. The clamp is what stops a class
        // with a wide range from overshooting, and `raise` applies it.
        assert!(stats.base(Stat::MaxMagicPoints) <= class.stat(Stat::MaxMagicPoints).maximum);
        assert!(stats.base(Stat::Speed) <= class.stat(Stat::Speed).maximum);
    }

    #[test]
    fn a_character_already_at_twenty_is_left_alone() {
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::default();
        progress.level = MAX_LEVEL;
        let before = stats.base(Stat::MaxHitPoints);

        assert!(!progress.jump_to_max_level(&class, &mut stats));
        assert_eq!(
            stats.base(Stat::MaxHitPoints),
            before,
            "nothing was granted twice"
        );
    }

    #[test]
    fn the_first_level_costs_fifty_and_each_one_costs_a_hundred_more() {
        assert_eq!(experience_goal(1), 50);
        assert_eq!(experience_goal(2), 150);
        assert_eq!(experience_goal(3), 250);
        assert_eq!(experience_goal(20), 1950);
    }

    #[test]
    fn the_running_total_agrees_with_the_per_level_goals() {
        // `experience_at` and `experience_goal` are separate formulas in the original, and they
        // must agree or the bar the client draws never fills.
        let mut running = 0;
        for level in 1..MAX_LEVEL {
            assert_eq!(
                experience_at(level),
                running,
                "the total at level {level} is wrong"
            );
            running += experience_goal(level);
        }
    }

    #[test]
    fn a_character_levels_when_it_has_earned_the_goal_and_not_before() {
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();

        progress.gain(&class, &mut stats, 49, || 0.5);
        assert_eq!(progress.level, 1, "one short");

        progress.gain(&class, &mut stats, 1, || 0.5);
        assert_eq!(progress.level, 2);
    }

    #[test]
    fn one_enormous_gain_is_still_only_one_level() {
        // `CheckLevelUp` is an `if`, not a loop. The surplus stays on the total and the next kill
        // collects it, so a character cannot be carried several levels by one enemy.
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();

        let advance = progress.gain(&class, &mut stats, 10_000, || 0.5);

        assert_eq!(advance.levels_gained, 1);
        assert_eq!(progress.level, 2);
        assert_eq!(progress.experience, 10_000, "the surplus is kept");

        // And the level after it is one kill away rather than lost.
        assert_eq!(
            progress.gain(&class, &mut stats, 1, || 0.5).levels_gained,
            1
        );
        assert_eq!(progress.level, 3);
    }

    #[test]
    fn levelling_stops_at_twenty() {
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();

        for _ in 0..40 {
            progress.gain(&class, &mut stats, 10_000_000, || 0.5);
        }

        assert_eq!(progress.level, MAX_LEVEL);
        assert!(
            progress
                .gain(&class, &mut stats, 10_000, || 0.5)
                .levels_gained
                == 0
        );
    }

    #[test]
    fn experience_still_accrues_at_the_top_level() {
        // Only the level-up is gated on `Level < 20`; `EnemyKilled` adds the experience whatever
        // the level, which is where a finished character's fame comes from.
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();
        progress.level = MAX_LEVEL;

        assert!(experience_for_kill(10_000, 1.0, true, MAX_LEVEL, false, 1.0, false) > 0);

        let advance = progress.gain(&class, &mut stats, 3_000, || 0.5);

        assert_eq!(progress.level, MAX_LEVEL);
        assert_eq!(progress.experience, 3_000);
        assert_eq!(advance.fame_gained, 3);
    }

    #[test]
    fn every_level_up_stays_inside_the_class_s_declared_range() {
        let class = wizard();

        // Both extremes of the roll, since a range is only respected if both ends are.
        for roll in [0.0f32, 0.999] {
            let mut stats = Stats::starting(&class);
            let mut progress = Progress::new();
            let before = stats.base(Stat::MaxHitPoints);

            progress.gain(&class, &mut stats, 50, || roll);

            let growth = class.stat(Stat::MaxHitPoints);
            let gained = stats.base(Stat::MaxHitPoints) - before;
            assert!(
                (growth.min_increase..=growth.max_increase).contains(&gained),
                "gained {gained}, expected {}..={}",
                growth.min_increase,
                growth.max_increase
            );
        }
    }

    #[test]
    fn no_stat_is_levelled_past_its_class_ceiling() {
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();

        for _ in 0..MAX_LEVEL {
            progress.gain(&class, &mut stats, 1_000_000, || 0.999);
        }

        for stat in STATS {
            assert!(
                stats.base(stat) <= class.stat(stat).maximum,
                "{stat:?} reached {} against a maximum of {}",
                stats.base(stat),
                class.stat(stat).maximum
            );
        }
    }

    #[test]
    fn a_stat_the_class_does_not_grow_stays_where_it_started() {
        // Defense has no LevelIncrease for a wizard, so twenty levels must leave it alone.
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();
        let before = stats.base(Stat::Defense);

        for _ in 0..MAX_LEVEL {
            progress.gain(&class, &mut stats, 1_000_000, || 0.999);
        }

        assert_eq!(stats.base(Stat::Defense), before);
    }

    #[test]
    fn a_kill_is_worth_a_tenth_of_the_enemys_health() {
        // At level ten the cap is ninety-five, so neither of these reaches it.
        assert_eq!(
            experience_for_kill(300, 1.0, true, 10, false, 1.0, false),
            30
        );
        assert_eq!(
            experience_for_kill(300, 2.0, true, 10, false, 1.0, false),
            60
        );
    }

    #[test]
    fn the_cap_binds_sooner_for_a_low_character_than_a_high_one() {
        // The same enemy is worth less to someone who has barely started, which is what stops one
        // large kill carrying a new character several levels.
        let low = experience_for_kill(3_000, 1.0, true, 1, false, 1.0, false);
        let high = experience_for_kill(3_000, 1.0, true, 15, false, 1.0, false);

        assert!(low < high, "{low} against {high}");
        assert_eq!(low, (experience_goal(1) as f32 * LEVEL_SHARE) as i32);
    }

    #[test]
    fn no_kill_is_worth_more_than_a_tenth_of_a_level() {
        // Without the cap, one enormous enemy takes a character from one to twenty.
        let huge = experience_for_kill(1_000_000, 1.0, true, 1, false, 1.0, false);
        assert_eq!(huge, (experience_goal(1) as f32 * LEVEL_SHARE) as i32);
        assert!(huge < experience_goal(1), "still needs several kills");
    }

    #[test]
    fn an_enemy_that_awards_nothing_is_worth_nothing_however_large() {
        assert_eq!(
            experience_for_kill(1_000_000, 10.0, false, 1, false, 1.0, false),
            0
        );
    }

    #[test]
    fn a_character_at_the_maximum_is_capped_by_the_goal_it_will_never_reach() {
        // `ExperienceGoal` at twenty is 1950, and a tenth of it is the ceiling on any one kill.
        assert_eq!(
            experience_for_kill(10_000, 1.0, true, MAX_LEVEL, false, 1.0, false),
            (experience_goal(MAX_LEVEL) as f32 * LEVEL_SHARE) as i32
        );
    }

    #[test]
    fn the_enemy_you_were_sent_to_is_worth_five_times_the_cap() {
        // The whole reason to follow the arrow. The same enemy killed off your own bat is worth a
        // tenth of a level; the one you were sent to is worth half of one.
        let huge = 10_000_000;

        let ordinary = experience_for_kill(huge, 1.0, true, 10, false, 1.0, false);
        let sent = experience_for_kill(huge, 1.0, true, 10, true, 1.0, false);

        assert_eq!(ordinary, (experience_goal(10) as f32 * 0.1) as i32);
        assert_eq!(sent, (experience_goal(10) as f32 * 0.5) as i32);
        assert_eq!(sent, ordinary * 5);
    }

    #[test]
    fn a_quest_raises_the_ceiling_rather_than_the_reward() {
        // A small enemy is worth what it is worth. The share is a cap, so raising it does nothing
        // for something that was never near it.
        let small = 300;

        assert_eq!(
            experience_for_kill(small, 1.0, true, 10, true, 1.0, false),
            experience_for_kill(small, 1.0, true, 10, false, 1.0, false)
        );
    }

    #[test]
    fn the_theatre_takes_a_third_of_what_the_cap_allowed() {
        // `DamageCounter.cs:95-96`. At level ten the cap is ninety-five, and the theatre pays a
        // third of that rather than a third of the enemy.
        let huge = 10_000_000;

        assert_eq!(
            experience_for_kill(huge, 1.0, true, 10, false, 1.0, false),
            95
        );
        assert_eq!(
            experience_for_kill(huge, 1.0, true, 10, false, THEATRE_SHARE, false),
            31,
            "ninety-five thirds, truncated"
        );
    }

    #[test]
    fn a_boost_doubles_past_the_cap_rather_than_being_held_at_it() {
        // The doubling is applied after the cap has already bound (`DamageCounter.cs:85-99`), so a
        // boosted kill is worth twice what an unboosted one is capped at.
        let huge = 10_000_000;

        let plain = experience_for_kill(huge, 1.0, true, 10, false, 1.0, false);
        let boosted = experience_for_kill(huge, 1.0, true, 10, false, 1.0, true);

        assert_eq!(plain, 95);
        assert_eq!(boosted, 190);
    }

    #[test]
    fn a_finished_character_is_not_doubled() {
        // `XPBoostTime != 0 && Level < 20` (`DamageCounter.cs:98`), and the boost is cleared
        // outright the moment the level reaches twenty.
        assert_eq!(
            experience_for_kill(10_000, 1.0, true, MAX_LEVEL, false, 1.0, true),
            experience_for_kill(10_000, 1.0, true, MAX_LEVEL, false, 1.0, false)
        );
    }

    #[test]
    fn the_reward_is_rounded_once_at_the_very_end() {
        // `DamageCounter` casts once, at `(int)playerXp`. Ninety-nine health is 9.9 experience,
        // and a third of that is 3.267 -- but a third of a truncated nine is 2.97, which is two.
        assert_eq!(
            experience_for_kill(99, 1.0, true, 2, false, THEATRE_SHARE, false),
            3
        );

        // The same the other way: seven and a half doubled is fifteen, where doubling a truncated
        // seven is fourteen.
        assert_eq!(experience_for_kill(75, 1.0, true, 2, false, 1.0, true), 15);
    }

    #[test]
    fn a_kill_worth_nothing_still_banks_the_fame_a_level_up_held_back() {
        // `EnemyKilled` guards only the addition and runs `CheckLevelUp` regardless
        // (`Player.Leveling.cs:317-322`), so the kill after a level-up banks the fame whether or
        // not it was worth anything itself. A `GivesNoXp` summon dying nearby is enough.
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();

        progress.level = MAX_LEVEL - 1;
        progress.experience = experience_at(MAX_LEVEL - 1);

        let levelling = progress.gain(&class, &mut stats, experience_goal(MAX_LEVEL - 1), || 0.5);
        assert_eq!(levelling.levels_gained, 1);
        assert_eq!(progress.fame, 0, "held back by the level-up");

        let worthless = progress.gain(&class, &mut stats, 0, || 0.5);
        assert_eq!(worthless.fame_gained, 18);
        assert_eq!(progress.fame, 18);
    }

    #[test]
    fn a_kill_worth_nothing_hands_over_the_level_the_surplus_already_paid_for() {
        // One level per call, so a huge kill leaves a surplus. The next kill collects the level it
        // bought even when that kill was worth nothing at all.
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();

        progress.gain(&class, &mut stats, 10_000, || 0.5);
        assert_eq!(progress.level, 2);

        let worthless = progress.gain(&class, &mut stats, 0, || 0.5);
        assert_eq!(worthless.levels_gained, 1);
        assert_eq!(progress.level, 3);
        assert_eq!(progress.experience, 10_000, "and nothing was added");
    }

    #[test]
    fn fame_is_one_per_thousand_and_slows_after_two_hundred_thousand() {
        assert_eq!(fame_from_experience(0), 0);
        assert_eq!(fame_from_experience(999), 0);
        assert_eq!(fame_from_experience(5_000), 5);
        assert_eq!(fame_from_experience(200_000), 200);
        assert_eq!(fame_from_experience(300_000), 300);
    }

    /// Crossing a milestone is what a completed class quest is, and the last one is not announced.
    ///
    /// `CalculateFame` compares `newGoal > FameGoal` (`Player.Leveling.cs:244`). Passing twenty
    /// fame lifts the goal from twenty to a hundred and fifty and announces; passing two thousand
    /// drops it to zero, which is not greater, so the end of the ladder says nothing. That is the
    /// original's arithmetic and the reason a five-star character never sees the message.
    #[test]
    fn a_completed_class_quest_is_a_milestone_crossed_and_the_last_one_is_silent() {
        let class = wizard();
        let mut stats = Stats::starting(&class);

        let mut progress = Progress {
            level: MAX_LEVEL,
            experience: 19_000,
            fame: 19,
        };
        let crossed = progress.gain(&class, &mut stats, 2_000, || 0.5);
        assert_eq!(progress.fame, 21);
        assert_eq!(crossed.fame_gained, 2);
        assert!(crossed.crossed_fame_goal, "twenty is a milestone");

        let ordinary = progress.gain(&class, &mut stats, 1_000, || 0.5);
        assert_eq!(ordinary.fame_gained, 1);
        assert!(
            !ordinary.crossed_fame_goal,
            "an ordinary fame gain is not a class quest"
        );

        let mut topped = Progress {
            level: MAX_LEVEL,
            experience: 1_999_000,
            fame: 1_999,
        };
        let last = topped.gain(&class, &mut stats, 1_000, || 0.5);
        assert_eq!(topped.fame, 2_000);
        assert!(
            !last.crossed_fame_goal,
            "the goal drops to zero rather than rising, so nothing is announced"
        );
    }

    #[test]
    fn fame_goals_climb_through_the_milestones_and_then_stop() {
        assert_eq!(fame_goal(0), 20);
        assert_eq!(fame_goal(20), 150);
        assert_eq!(fame_goal(150), 400);
        assert_eq!(fame_goal(400), 800);
        assert_eq!(fame_goal(800), 2000);
        assert_eq!(fame_goal(2000), 0, "nothing left to reach");
    }

    #[test]
    fn stars_follow_the_same_milestones() {
        assert_eq!(stars(0), 0);
        assert_eq!(stars(19), 0);
        assert_eq!(stars(20), 1);
        assert_eq!(stars(2000), 5);
        assert_eq!(stars(100_000), 5, "five is the most there is");
    }

    #[test]
    fn fame_follows_experience_without_being_awarded_twice() {
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();
        progress.level = MAX_LEVEL;

        let first = progress.gain(&class, &mut stats, 5_000, || 0.5);
        assert_eq!(first.fame_gained, 5);
        assert_eq!(progress.fame, 5);

        // Experience that crosses no thousand adds no fame.
        let second = progress.gain(&class, &mut stats, 10, || 0.5);
        assert_eq!(second.fame_gained, 0);
        assert_eq!(progress.fame, 5);
    }

    #[test]
    fn a_level_up_holds_the_fame_back_until_the_next_kill() {
        // `CheckLevelUp` calls `CalculateFame` only on the branch where nothing levelled, so the
        // fame a levelling kill earned is banked by the kill after it.
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();

        // One short of the last level, so the first kill levels and the second cannot.
        progress.level = MAX_LEVEL - 1;
        progress.experience = experience_at(MAX_LEVEL - 1);

        let levelling = progress.gain(&class, &mut stats, experience_goal(MAX_LEVEL - 1), || 0.5);
        assert_eq!(levelling.levels_gained, 1);
        assert_eq!(levelling.fame_gained, 0);
        assert_eq!(
            progress.fame, 0,
            "eighteen thousand experience and no fame yet"
        );

        let ordinary = progress.gain(&class, &mut stats, 1, || 0.5);
        assert_eq!(ordinary.fame_gained, 18);
        assert_eq!(progress.fame, 18);
    }

    #[test]
    fn nothing_gained_changes_nothing() {
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();
        let before = (progress, stats);

        assert!(progress.gain(&class, &mut stats, 0, || 0.5).is_nothing());
        assert!(progress.gain(&class, &mut stats, -50, || 0.5).is_nothing());
        assert_eq!((progress, stats), before);
    }
}
