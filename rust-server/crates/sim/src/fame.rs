//! What a character did, and what its death is worth because of it.
//!
//! Follows `common/FameStats.cs`. A character earns fame by living; the bonuses are what it earns
//! for *how* it lived, and they are the reason anybody plays a class a particular way. Never firing
//! a shot that hits is worth a quarter again; killing gods rather than monsters is worth a tenth;
//! finishing every kind of dungeon is worth a tenth.
//!
//! # Why they compound
//!
//! Each bonus is a share of the fame *including the bonuses before it*, not of the base. The
//! original does this by threading a running total through the loop, and it matters: twenty bonuses
//! of a tenth each come to two and a half times the base rather than three times it. The order the
//! bonuses are declared in is therefore part of the arithmetic and not a matter of taste.
//!
//! # What is counted and what is not
//!
//! Everything here is counted by the server, from things the server already decides: a shot is
//! counted where it is fired, a hit where it lands, a kill where the enemy dies. Nothing is taken
//! from a client, because a client that reports its own accuracy will report whatever earns the most.

/// What one character has done, from the moment it was made.
///
/// Held per character rather than per account: the bonuses ask what *this* character did, and a
/// counter shared across characters would pay every one of them for the first one's dungeon runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub shots: i32,

    /// Shots that landed on something. The accuracy bonuses are the ratio of these to `shots`.
    pub shots_that_hit: i32,

    pub abilities_used: i32,
    pub tiles_seen: i32,
    pub teleports: i32,
    pub potions_drunk: i32,

    pub monster_kills: i32,
    pub god_kills: i32,
    pub cube_kills: i32,
    pub oryx_kills: i32,

    pub quests_completed: i32,
    pub level_up_assists: i32,

    /// Which kinds of dungeon have been finished, one bit each. Held as a set rather than a count
    /// per kind, because the only question asked is whether every kind has been done at least once.
    pub dungeons_completed: u32,
}

/// The kinds of dungeon the tunnel-rat bonus asks about.
///
/// From `FameStats`, which counts ten. Named rather than numbered so the content deciding which
/// world is which does not have to agree with a bit position.
pub const DUNGEON_KINDS: &[&str] = &[
    "Pirate Cave",
    "Undead Lair",
    "Abyss of Demons",
    "Snake Pit",
    "Spider Den",
    "Sprite World",
    "Tomb of the Ancients",
    "Ocean Trench",
    "Forbidden Jungle",
    "Manor of the Immortals",
];

impl Tally {
    /// Notes that a kind of dungeon has been finished.
    pub fn completed(&mut self, dungeon: &str) {
        if let Some(index) = DUNGEON_KINDS.iter().position(|kind| *kind == dungeon) {
            self.dungeons_completed |= 1 << index;
        }
    }

    /// Whether every kind has been finished at least once.
    pub fn every_dungeon(&self) -> bool {
        let all = (1u32 << DUNGEON_KINDS.len()) - 1;
        self.dungeons_completed & all == all
    }

    /// How many shots landed, as a share of those fired.
    ///
    /// Zero when nothing has been fired, so a character that never shot is not perfectly accurate.
    pub fn accuracy(&self) -> f64 {
        if self.shots <= 0 {
            return 0.0;
        }
        self.shots_that_hit as f64 / self.shots as f64
    }

    /// How many kills were gods, as a share of all kills.
    pub fn godliness(&self) -> f64 {
        let all = self.god_kills + self.monster_kills;
        if all <= 0 {
            return 0.0;
        }
        self.god_kills as f64 / all as f64
    }
}

/// One bonus: what it is called, why it was earned, and what it paid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bonus {
    pub name: &'static str,
    pub why: &'static str,
    pub fame: i32,
}

/// What a character is, as the bonuses need to ask about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Finished {
    pub level: i16,
    pub fame: i32,

    /// Whether this is the first character on the account ever to die.
    pub first_death: bool,

    /// What the four worn items add, as a percentage. `FameBonus` in the content.
    pub equipment_bonus: i32,

    /// Whether this beat every other character the account has had.
    pub best_yet: bool,
}

/// The highest level a character reaches, which several bonuses ask for.
const MAX_LEVEL: i16 = 20;

/// Every bonus, in the order the original declares them.
///
/// The order is part of the arithmetic: each is a share of the fame including the bonuses before it,
/// so moving one changes what the others pay.
type Rule = (
    &'static str,
    &'static str,
    fn(&Tally, Finished) -> bool,
    f64,
);

const BONUSES: &[Rule] = &[
    (
        "Ancestor",
        "first death of any of your characters",
        |_, who| who.first_death,
        0.1,
    ),
    (
        "Pacifist",
        "never shot a bullet which hit an enemy",
        |tally, _| tally.shots_that_hit == 0,
        0.25,
    ),
    (
        "Thirsty",
        "never drank a potion",
        |tally, _| tally.potions_drunk == 0,
        0.25,
    ),
    (
        "Mundane",
        "never used a special ability",
        |tally, who| who.level == MAX_LEVEL && tally.abilities_used == 0,
        0.25,
    ),
    (
        "Boots on the Ground",
        "never teleported",
        |tally, _| tally.teleports == 0,
        0.25,
    ),
    (
        "Tunnel Rat",
        "completed every dungeon type",
        |tally, _| tally.every_dungeon(),
        0.1,
    ),
    (
        "Enemy of the Gods",
        "more than a tenth of kills are gods",
        |tally, who| who.level == MAX_LEVEL && tally.godliness() > 0.1,
        0.1,
    ),
    (
        "Slayer of the Gods",
        "more than half of kills are gods",
        |tally, who| who.level == MAX_LEVEL && tally.godliness() > 0.5,
        0.1,
    ),
    (
        "Oryx Slayer",
        "dealt the killing blow to Oryx",
        |tally, _| tally.oryx_kills > 0,
        0.1,
    ),
    (
        "Accurate",
        "accuracy better than a quarter",
        |tally, who| who.level == MAX_LEVEL && tally.accuracy() > 0.25,
        0.1,
    ),
    (
        "Sharpshooter",
        "accuracy better than half",
        |tally, who| who.level == MAX_LEVEL && tally.accuracy() > 0.5,
        0.1,
    ),
    (
        "Sniper",
        "accuracy better than three quarters",
        |tally, who| who.level == MAX_LEVEL && tally.accuracy() > 0.75,
        0.1,
    ),
    (
        "Explorer",
        "more than a million tiles uncovered",
        |tally, _| tally.tiles_seen > 1_000_000,
        0.05,
    ),
    (
        "Cartographer",
        "more than four million tiles uncovered",
        |tally, _| tally.tiles_seen > 4_000_000,
        0.05,
    ),
    (
        "Team Player",
        "more than a hundred party level ups",
        |tally, _| tally.level_up_assists > 100,
        0.1,
    ),
    (
        "Leader of Men",
        "more than a thousand party level ups",
        |tally, _| tally.level_up_assists > 1000,
        0.1,
    ),
    (
        "Doer of Deeds",
        "more than a thousand quests completed",
        |tally, _| tally.quests_completed > 1000,
        0.1,
    ),
    (
        "Friend of the Cubes",
        "never killed a cube",
        |tally, who| who.level == MAX_LEVEL && tally.cube_kills == 0,
        0.1,
    ),
];

/// What a character's death is worth, and which bonuses it earned.
///
/// Each bonus is a share of the fame *including the bonuses before it*, which is what the original
/// does by threading a running total through its loop. Twenty tenths therefore come to two and a
/// half times the base rather than three times it.
pub fn bonuses(tally: &Tally, who: Finished) -> (i32, Vec<Bonus>) {
    let mut earned = 0i32;
    let mut awarded = Vec::new();

    for (name, why, holds, share) in BONUSES {
        if !holds(tally, who) {
            continue;
        }

        let fame = ((who.fame + earned) as f64 * share) as i32;
        earned += fame;
        awarded.push(Bonus { name, why, fame });
    }

    // What the four worn items add. Last of the ordinary bonuses, as in the original, so it is a
    // share of everything above it.
    if who.equipment_bonus > 0 {
        let fame = ((who.fame + earned) as f64 * (who.equipment_bonus as f64 / 100.0)) as i32;
        earned += fame;
        awarded.push(Bonus {
            name: "Well Equipped",
            why: "wearing something that pays",
            fame,
        });
    }

    // And first born last of all: beating every character the account has had is worth a tenth of
    // the whole, bonuses included.
    if who.best_yet {
        let fame = ((who.fame + earned) as f64 * 0.1) as i32;
        earned += fame;
        awarded.push(Bonus {
            name: "First Born",
            why: "your best character yet",
            fame,
        });
    }

    (who.fame + earned, awarded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nobody() -> Finished {
        Finished {
            level: 1,
            fame: 100,
            first_death: false,
            equipment_bonus: 0,
            best_yet: false,
        }
    }

    /// A character that fought, drank and teleported, so no "never did it" bonus applies.
    fn ordinary() -> Tally {
        Tally {
            shots: 100,
            shots_that_hit: 10,
            abilities_used: 5,
            teleports: 3,
            potions_drunk: 4,
            monster_kills: 50,
            cube_kills: 1,
            ..Tally::default()
        }
    }

    #[test]
    fn a_character_that_did_nothing_special_gets_what_it_earned() {
        let (total, awarded) = bonuses(&ordinary(), nobody());

        assert_eq!(total, 100, "{awarded:?}");
        assert!(awarded.is_empty());
    }

    #[test]
    fn every_bonus_the_original_has_is_here() {
        // Twenty in `FameStats.cs`: eighteen in the table, plus well-equipped and first-born, which
        // it works out after the loop because both are shares of everything above them.
        let named: Vec<&str> = BONUSES.iter().map(|(name, _, _, _)| *name).collect();

        for expected in [
            "Ancestor",
            "Pacifist",
            "Thirsty",
            "Mundane",
            "Boots on the Ground",
            "Tunnel Rat",
            "Enemy of the Gods",
            "Slayer of the Gods",
            "Oryx Slayer",
            "Accurate",
            "Sharpshooter",
            "Sniper",
            "Explorer",
            "Cartographer",
            "Team Player",
            "Leader of Men",
            "Doer of Deeds",
            "Friend of the Cubes",
        ] {
            assert!(named.contains(&expected), "{expected} is missing");
        }
        assert_eq!(named.len(), 18);
    }

    #[test]
    fn bonuses_compound_rather_than_all_taking_a_share_of_the_base() {
        // The order they are declared in is part of the arithmetic. Two tenths of a hundred is
        // twenty-one when they compound and twenty when they do not.
        let tally = Tally {
            oryx_kills: 1,
            ..ordinary()
        };
        let who = Finished {
            first_death: true,
            ..nobody()
        };

        let (total, awarded) = bonuses(&tally, who);

        assert_eq!(awarded.len(), 2, "{awarded:?}");
        assert_eq!(awarded[0].fame, 10, "a tenth of a hundred");
        assert_eq!(awarded[1].fame, 11, "a tenth of a hundred and ten");
        assert_eq!(total, 121);
    }

    #[test]
    fn the_never_did_it_bonuses_need_the_character_to_never_have_done_it() {
        let quiet = Tally {
            shots: 0,
            shots_that_hit: 0,
            abilities_used: 0,
            teleports: 0,
            potions_drunk: 0,
            ..Tally::default()
        };

        let (_, awarded) = bonuses(&quiet, nobody());
        let named: Vec<&str> = awarded.iter().map(|bonus| bonus.name).collect();

        assert!(named.contains(&"Pacifist"));
        assert!(named.contains(&"Thirsty"));
        assert!(named.contains(&"Boots on the Ground"));

        // Not these two: both ask for level twenty, and this character is level one.
        assert!(!named.contains(&"Mundane"));
        assert!(!named.contains(&"Friend of the Cubes"));
    }

    #[test]
    fn the_accuracy_bonuses_need_level_twenty() {
        // A level-one character with three lucky shots is not a sniper.
        let sharp = Tally {
            shots: 4,
            shots_that_hit: 4,
            ..ordinary()
        };

        let (_, awarded) = bonuses(&sharp, nobody());
        assert!(awarded.is_empty(), "{awarded:?}");

        let grown = Finished {
            level: 20,
            ..nobody()
        };
        let (_, awarded) = bonuses(&sharp, grown);
        let named: Vec<&str> = awarded.iter().map(|bonus| bonus.name).collect();

        assert!(named.contains(&"Accurate"));
        assert!(named.contains(&"Sharpshooter"));
        assert!(named.contains(&"Sniper"));
    }

    #[test]
    fn a_character_that_never_fired_is_not_perfectly_accurate() {
        // Zero of zero is not one. Without this every pacifist would also be a sniper.
        let quiet = Tally::default();
        assert_eq!(quiet.accuracy(), 0.0);

        let grown = Finished {
            level: 20,
            ..nobody()
        };
        let (_, awarded) = bonuses(&quiet, grown);
        let named: Vec<&str> = awarded.iter().map(|bonus| bonus.name).collect();

        assert!(!named.contains(&"Sniper"), "{named:?}");
    }

    #[test]
    fn the_tunnel_rat_needs_every_kind_and_not_ten_of_one() {
        let mut tally = ordinary();

        for _ in 0..50 {
            tally.completed("Snake Pit");
        }
        assert!(!tally.every_dungeon(), "ten of one is not every kind");

        for kind in DUNGEON_KINDS {
            tally.completed(kind);
        }
        assert!(tally.every_dungeon());

        let (_, awarded) = bonuses(&tally, nobody());
        assert!(awarded.iter().any(|bonus| bonus.name == "Tunnel Rat"));
    }

    #[test]
    fn a_dungeon_nothing_names_is_not_counted() {
        let mut tally = Tally::default();
        tally.completed("Somewhere Else");
        assert_eq!(tally.dungeons_completed, 0);
    }

    #[test]
    fn what_is_worn_pays_a_share_of_everything_above_it() {
        let who = Finished {
            equipment_bonus: 10,
            ..nobody()
        };

        let (total, awarded) = bonuses(&ordinary(), who);

        assert_eq!(awarded.len(), 1);
        assert_eq!(awarded[0].name, "Well Equipped");
        assert_eq!(awarded[0].fame, 10);
        assert_eq!(total, 110);
    }

    #[test]
    fn the_best_character_yet_is_paid_last_and_on_the_whole() {
        let who = Finished {
            best_yet: true,
            equipment_bonus: 10,
            ..nobody()
        };

        let (total, awarded) = bonuses(&ordinary(), who);

        assert_eq!(awarded.len(), 2);
        assert_eq!(awarded[1].name, "First Born");
        assert_eq!(awarded[1].fame, 11, "a tenth of a hundred and ten");
        assert_eq!(total, 121);
    }

    #[test]
    fn a_character_with_no_fame_earns_no_bonus_however_well_it_played() {
        // Every bonus is a share, and a share of nothing is nothing. Worth saying because it means
        // the counters cannot mint fame on their own.
        let who = Finished {
            fame: 0,
            level: 20,
            first_death: true,
            best_yet: true,
            equipment_bonus: 50,
        };

        let (total, awarded) = bonuses(&Tally::default(), who);

        assert_eq!(total, 0);
        assert!(awarded.iter().all(|bonus| bonus.fame == 0));
    }
}
