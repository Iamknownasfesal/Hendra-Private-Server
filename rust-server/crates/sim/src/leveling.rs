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

/// Fame earned from a lifetime's experience.
///
/// One per thousand, halving in rate past two hundred thousand so that late experience is worth
/// less than early experience.
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
/// `max_hp / 10 * multiplier`, capped at a share of the player's own next level. An enemy that
/// awards nothing returns zero however large it is.
pub fn experience_for_kill(
    enemy_max_hp: i32,
    experience_multiplier: f32,
    awards_experience: bool,
    player_level: i16,
    was_quest: bool,
) -> i32 {
    if !awards_experience || player_level >= MAX_LEVEL {
        return 0;
    }

    let raw = (enemy_max_hp as f32 / HEALTH_PER_EXPERIENCE) * experience_multiplier.max(0.0);
    let share = if was_quest { QUEST_SHARE } else { LEVEL_SHARE };
    let cap = experience_goal(player_level) as f32 * share;

    raw.min(cap).max(0.0) as i32
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

    /// Adds experience, levelling as many times as it is worth.
    ///
    /// `roll` returns a value in `0.0..1.0` and is called once per stat per level, so the caller
    /// owns the randomness and a test can make levelling deterministic.
    pub fn gain(
        &mut self,
        class: &PlayerDesc,
        stats: &mut Stats,
        experience: i32,
        mut roll: impl FnMut() -> f32,
    ) -> Advance {
        let mut advance = Advance::default();
        if experience <= 0 {
            return advance;
        }

        self.experience = self.experience.saturating_add(experience);

        // A loop rather than a single step, because one large kill can be worth more than one
        // level to a low character and stopping at one would silently discard the rest.
        while self.level < MAX_LEVEL
            && self.experience - experience_at(self.level) >= experience_goal(self.level)
        {
            self.level += 1;
            advance.levels_gained += 1;

            for stat in STATS {
                let growth = class.stat(stat);
                let span = (growth.max_increase - growth.min_increase).max(0);
                let gained = growth.min_increase + (roll() * (span + 1) as f32) as i32;
                stats.raise(class, stat, gained.min(growth.max_increase));
            }
        }

        advance.reached_maximum = advance.levels_gained > 0 && self.level >= MAX_LEVEL;

        let fame = fame_from_experience(self.experience);
        if fame > self.fame {
            advance.fame_gained = fame - self.fame;
            self.fame = fame;
        }

        advance
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
    fn one_enormous_gain_levels_more_than_once() {
        // Stopping at one level would silently discard the rest of a large gain.
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();

        let advance = progress.gain(&class, &mut stats, 10_000, || 0.5);

        assert!(progress.level > 5, "reached {}", progress.level);
        assert_eq!(advance.levels_gained, progress.level - 1);
    }

    #[test]
    fn levelling_stops_at_twenty() {
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let mut progress = Progress::new();

        progress.gain(&class, &mut stats, 10_000_000, || 0.5);

        assert_eq!(progress.level, MAX_LEVEL);
        assert!(
            progress
                .gain(&class, &mut stats, 10_000, || 0.5)
                .levels_gained
                == 0
        );
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

        progress.gain(&class, &mut stats, 1_000_000, || 0.999);

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

        progress.gain(&class, &mut stats, 1_000_000, || 0.999);

        assert_eq!(stats.base(Stat::Defense), before);
    }

    #[test]
    fn a_kill_is_worth_a_tenth_of_the_enemys_health() {
        // At level ten the cap is ninety-five, so neither of these reaches it.
        assert_eq!(experience_for_kill(300, 1.0, true, 10, false), 30);
        assert_eq!(experience_for_kill(300, 2.0, true, 10, false), 60);
    }

    #[test]
    fn the_cap_binds_sooner_for_a_low_character_than_a_high_one() {
        // The same enemy is worth less to someone who has barely started, which is what stops one
        // large kill carrying a new character several levels.
        let low = experience_for_kill(3_000, 1.0, true, 1, false);
        let high = experience_for_kill(3_000, 1.0, true, 15, false);

        assert!(low < high, "{low} against {high}");
        assert_eq!(low, (experience_goal(1) as f32 * LEVEL_SHARE) as i32);
    }

    #[test]
    fn no_kill_is_worth_more_than_a_tenth_of_a_level() {
        // Without the cap, one enormous enemy takes a character from one to twenty.
        let huge = experience_for_kill(1_000_000, 1.0, true, 1, false);
        assert_eq!(huge, (experience_goal(1) as f32 * LEVEL_SHARE) as i32);
        assert!(huge < experience_goal(1), "still needs several kills");
    }

    #[test]
    fn an_enemy_that_awards_nothing_is_worth_nothing_however_large() {
        assert_eq!(experience_for_kill(1_000_000, 10.0, false, 1, false), 0);
    }

    #[test]
    fn a_character_at_the_maximum_earns_no_more_experience() {
        assert_eq!(experience_for_kill(10_000, 1.0, true, MAX_LEVEL, false), 0);
    }

    #[test]
    fn the_enemy_you_were_sent_to_is_worth_five_times_the_cap() {
        // The whole reason to follow the arrow. The same enemy killed off your own bat is worth a
        // tenth of a level; the one you were sent to is worth half of one.
        let huge = 10_000_000;

        let ordinary = experience_for_kill(huge, 1.0, true, 10, false);
        let sent = experience_for_kill(huge, 1.0, true, 10, true);

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
            experience_for_kill(small, 1.0, true, 10, true),
            experience_for_kill(small, 1.0, true, 10, false)
        );
    }

    #[test]
    fn fame_is_one_per_thousand_and_slows_after_two_hundred_thousand() {
        assert_eq!(fame_from_experience(0), 0);
        assert_eq!(fame_from_experience(999), 0);
        assert_eq!(fame_from_experience(5_000), 5);
        assert_eq!(fame_from_experience(200_000), 200);
        assert_eq!(fame_from_experience(300_000), 300);
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

        let first = progress.gain(&class, &mut stats, 5_000, || 0.5);
        assert_eq!(first.fame_gained, 5);
        assert_eq!(progress.fame, 5);

        // Experience that crosses no thousand adds no fame.
        let second = progress.gain(&class, &mut stats, 10, || 0.5);
        assert_eq!(second.fame_gained, 0);
        assert_eq!(progress.fame, 5);
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
