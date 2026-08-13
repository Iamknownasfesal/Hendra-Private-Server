//! Populating a realm, and closing it.
//!
//! Follows `wServer/realm/Oryx.cs`, which is where the original keeps all of this.
//!
//! # How many enemies a realm holds
//!
//! Per terrain, from the map itself: count the squares of a terrain and divide by how many squares
//! that terrain gives each enemy. A realm map is mostly low ground, so the low terrains end up with
//! hundreds and the mountains, at one enemy per two hundred squares, end up dense.
//!
//! The population is then held in a band rather than at a number. Below three quarters it is topped
//! up, above one and a half it is thinned, and in between it is left alone. A realm that is exactly
//! at its target after every kill would feel like a treadmill.
//!
//! # What lives where
//!
//! A table, as in the original. It is tempting to read this from the content instead, but the
//! content does not carry it: `SpawnProbability` appears on no shipped object, and the `Terrain`
//! field on a description is not what the original consults. Oryx keeps his own list of who belongs
//! on which ground and how often, and this is that list.
//!
//! # Why a realm closes
//!
//! On a timer, not on a body count. Half an hour after it opens the realm is called closed, and the
//! players in it are given a minute's warning and then sent to the castle. Closing on clearance
//! would mean a busy realm never closed and an empty one closed immediately.

use hendra_content::{Catalog, ObjectType, TERRAIN_COUNT, Terrain as TerrainKind};

/// How full a realm is, and what happens next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Open. Enemies are kept topped up and players may arrive.
    Open,

    /// Announced but not yet closed. Players may still arrive during the warning.
    Closing,

    /// Closed. Nobody new arrives and nothing new spawns.
    Closed,

    /// The castle is up and everybody still here is going to it.
    Emptying,
}

/// How long a realm stays open.
pub const REALM_LIFETIME_MS: u64 = 1_800_000;

/// How much warning the realm gives before it closes.
pub const CLOSING_WARNING_MS: u64 = 60_000;

/// How long after closing before everybody is sent to the castle.
pub const CASTLE_AFTER_MS: u64 = 22_000;

/// How often the population is checked.
///
/// Oryx ticks every ten seconds and ensures population on every sixth of those.
pub const ENSURE_EVERY_MS: u64 = 60_000;

/// Below this share of its target, a terrain is topped up.
pub const TOP_UP_BELOW: f32 = 0.75;

/// Above this share of its target, a terrain is thinned.
pub const THIN_ABOVE: f32 = 1.5;

/// How far from a player something may appear, and how far from one it must be to be thinned.
pub const PLAYER_CLEARANCE: f32 = 10.0;

/// How far from the chosen point the members of a group land.
pub const GROUP_SPREAD: f32 = 5.0;

/// What lives on one terrain, and how thinly.
struct Region {
    terrain: TerrainKind,

    /// Squares of this terrain per enemy. Two hundred in the mountains, fifteen hundred on the
    /// shore, which is what makes the top of the map dangerous and the beach a walk.
    per_enemy: u32,

    /// Names and their share of the roll. They sum to one.
    mobs: &'static [(&'static str, f32)],
}

/// Who belongs where, from `Oryx.cs`.
const REGIONS: &[Region] = &[
    Region {
        terrain: TerrainKind::ShoreSand,
        per_enemy: 1500,
        mobs: &[
            ("Pirate", 0.3),
            ("Piratess", 0.1),
            ("Snake", 0.2),
            ("Scorpion Queen", 0.4),
        ],
    },
    Region {
        terrain: TerrainKind::ShorePlains,
        per_enemy: 1550,
        mobs: &[
            ("Bandit Leader", 0.4),
            ("Red Gelatinous Cube", 0.2),
            ("Purple Gelatinous Cube", 0.2),
            ("Green Gelatinous Cube", 0.2),
        ],
    },
    Region {
        terrain: TerrainKind::LowPlains,
        per_enemy: 1400,
        mobs: &[
            ("Hobbit Mage", 0.5),
            ("Undead Hobbit Mage", 0.4),
            ("Sumo Master", 0.1),
        ],
    },
    Region {
        terrain: TerrainKind::LowForest,
        per_enemy: 1400,
        mobs: &[
            ("Elf Wizard", 0.2),
            ("Goblin Mage", 0.2),
            ("Easily Enraged Bunny", 0.3),
            ("Forest Nymph", 0.3),
        ],
    },
    Region {
        terrain: TerrainKind::LowSand,
        per_enemy: 1400,
        mobs: &[
            ("Sandsman King", 0.4),
            ("Giant Crab", 0.2),
            ("Sand Devil", 0.4),
        ],
    },
    Region {
        terrain: TerrainKind::MidPlains,
        per_enemy: 1550,
        mobs: &[
            ("Fire Sprite", 0.1),
            ("Ice Sprite", 0.1),
            ("Magic Sprite", 0.1),
            ("Pink Blob", 0.07),
            ("Gray Blob", 0.07),
            ("Earth Golem", 0.04),
            ("Paper Golem", 0.04),
            ("Big Green Slime", 0.08),
            ("Swarm", 0.05),
            ("Wasp Queen", 0.2),
            ("Shambling Sludge", 0.03),
            ("Orc King", 0.06),
            ("Candy Gnome", 0.02),
        ],
    },
    Region {
        terrain: TerrainKind::MidForest,
        per_enemy: 1550,
        mobs: &[
            ("Dwarf King", 0.3),
            ("Metal Golem", 0.05),
            ("Clockwork Golem", 0.05),
            ("Werelion", 0.1),
            ("Horned Drake", 0.3),
            ("Red Spider", 0.1),
            ("Black Bat", 0.1),
        ],
    },
    Region {
        terrain: TerrainKind::MidSand,
        per_enemy: 1400,
        mobs: &[
            ("Desert Werewolf", 0.25),
            ("Fire Golem", 0.1),
            ("Darkness Golem", 0.1),
            ("Sand Phantom", 0.2),
            ("Nomadic Shaman", 0.25),
            ("Great Lizard", 0.1),
        ],
    },
    Region {
        terrain: TerrainKind::HighPlains,
        per_enemy: 900,
        mobs: &[
            ("Shield Orc Key", 0.2),
            ("Urgle", 0.2),
            ("Undead Dwarf God", 0.6),
        ],
    },
    Region {
        terrain: TerrainKind::HighForest,
        per_enemy: 750,
        mobs: &[
            ("Ogre King", 0.4),
            ("Dragon Egg", 0.1),
            ("Lizard God", 0.5),
            ("Beer God", 0.1),
        ],
    },
    Region {
        terrain: TerrainKind::HighSand,
        per_enemy: 1000,
        mobs: &[("Minotaur", 0.4), ("Flayer God", 0.4), ("Flamer King", 0.2)],
    },
    Region {
        terrain: TerrainKind::Mountains,
        per_enemy: 200,
        mobs: &[
            ("White Demon", 0.09),
            ("Sprite God", 0.08),
            ("Medusa", 0.08),
            ("Ent God", 0.1),
            ("Beholder", 0.09),
            ("Flying Brain", 0.09),
            ("Slime God", 0.08),
            ("Ghost God", 0.07),
            ("Rock Bot", 0.05),
            ("Djinn", 0.09),
            ("Leviathan", 0.07),
            ("Arena Headless Horseman", 0.01),
        ],
    },
];

/// One kind of enemy a realm may place, resolved against the catalog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spawn {
    pub kind: ObjectType,

    /// Where it belongs.
    pub terrain: TerrainKind,

    /// Its share of the roll for that terrain.
    pub weight: f32,

    /// How many appear at once, where the description asks for a group.
    pub group: Option<hendra_content::SpawnCount>,

    /// How thinly this terrain is populated, in squares per enemy.
    pub per_enemy: u32,
}

/// Everything a realm can be populated with, resolved once at load.
///
/// A name the catalog does not have is reported rather than skipped quietly: a typo here is a whole
/// terrain that stays empty, and the only symptom is a stretch of map nobody ever fights in.
pub fn spawnable(catalog: &Catalog) -> Vec<Spawn> {
    let (found, missing) = spawnable_reporting(catalog);
    for name in &missing {
        tracing::warn!(enemy = %name, "a realm spawn is not in the content");
    }
    found
}

/// The same, with the names that could not be resolved handed back rather than logged.
pub fn spawnable_reporting(catalog: &Catalog) -> (Vec<Spawn>, Vec<&'static str>) {
    let mut found = Vec::new();
    let mut missing = Vec::new();

    for region in REGIONS {
        for (name, weight) in region.mobs {
            let Some(kind) = catalog.type_of(name) else {
                missing.push(*name);
                continue;
            };

            found.push(Spawn {
                kind,
                terrain: region.terrain,
                weight: *weight,
                group: catalog.object(kind).and_then(|desc| desc.spawn_count),
                per_enemy: region.per_enemy,
            });
        }
    }

    (found, missing)
}

/// How many enemies each terrain should hold, from the squares it covers.
///
/// Terrains nothing is listed for get nothing, however much of the map they cover: an unlisted
/// terrain is not an empty list but a place enemies are not meant to be.
pub fn targets(squares: &[u32; TERRAIN_COUNT]) -> [usize; TERRAIN_COUNT] {
    let mut targets = [0usize; TERRAIN_COUNT];

    for region in REGIONS {
        let index = region.terrain as usize;
        targets[index] = (squares[index] / region.per_enemy.max(1)) as usize;
    }

    targets
}

/// What a realm wants done to its population.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Adjustment {
    pub terrain: TerrainKind,

    /// How many to add, or zero.
    pub add: usize,

    /// How many to remove, or zero.
    pub remove: usize,
}

/// The life of one realm: how long it has been open, and how close it is to done.
#[derive(Debug, Clone)]
pub struct Realm {
    phase: Phase,
    age_ms: u64,

    /// When the phase last changed, so each step of the closing sequence is timed from the last.
    changed_at_ms: u64,

    /// When the population was last checked.
    ensured_at_ms: u64,

    /// How many of each terrain there should be, from the map.
    targets: [usize; TERRAIN_COUNT],
}

impl Default for Realm {
    fn default() -> Realm {
        Realm::new()
    }
}

/// Something that has become true and needs saying or doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The population is due a check.
    Ensure,

    /// A minute's warning.
    Warned,

    /// Closed: no more arrivals, no more spawns.
    Closed,

    /// Everybody still here goes to the castle.
    Castle,
}

impl Realm {
    pub fn new() -> Realm {
        Realm {
            phase: Phase::Open,
            age_ms: 0,
            changed_at_ms: 0,
            ensured_at_ms: 0,
            targets: [0; TERRAIN_COUNT],
        }
    }

    /// Tells the realm how big each terrain is, which is what its population is drawn from.
    pub fn measure(&mut self, squares: &[u32; TERRAIN_COUNT]) {
        self.targets = targets(squares);
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// How many enemies this realm holds when it is at its target.
    pub fn population(&self) -> usize {
        self.targets.iter().sum()
    }

    /// Whether a player may arrive.
    ///
    /// A closed realm is about to be emptied, and letting somebody in during the last twenty
    /// seconds means putting them straight into the quake out of it.
    pub fn admits(&self) -> bool {
        matches!(self.phase, Phase::Open | Phase::Closing)
    }

    /// Whether the realm still fills itself.
    pub fn spawns(&self) -> bool {
        matches!(self.phase, Phase::Open | Phase::Closing)
    }

    /// Advances the realm's clock and returns what has become due.
    ///
    /// One step per call: each stage of the closing sequence is timed from the one before it, so
    /// they cannot all fire in the same tick after a stall.
    pub fn advance(&mut self, elapsed_ms: u64) -> Option<Event> {
        self.age_ms += elapsed_ms;

        let since_change = self.age_ms.saturating_sub(self.changed_at_ms);

        match self.phase {
            Phase::Open if self.age_ms >= REALM_LIFETIME_MS => {
                self.enter(Phase::Closing);
                return Some(Event::Warned);
            }
            Phase::Closing if since_change >= CLOSING_WARNING_MS => {
                self.enter(Phase::Closed);
                return Some(Event::Closed);
            }
            Phase::Closed if since_change >= CASTLE_AFTER_MS => {
                self.enter(Phase::Emptying);
                return Some(Event::Castle);
            }
            _ => {}
        }

        if self.spawns() && self.age_ms.saturating_sub(self.ensured_at_ms) >= ENSURE_EVERY_MS {
            self.ensured_at_ms = self.age_ms;
            return Some(Event::Ensure);
        }

        None
    }

    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.changed_at_ms = self.age_ms;
    }

    /// What to do about the population, given what is alive on each terrain.
    ///
    /// Held in a band rather than at a number: below three quarters of its target a terrain is
    /// topped up to the target, above one and a half it is thinned back to it, and in between it is
    /// left alone.
    pub fn adjustments(&self, alive: &[usize; TERRAIN_COUNT]) -> Vec<Adjustment> {
        if !self.spawns() {
            return Vec::new();
        }

        let mut wanted = Vec::new();

        for (index, here) in alive.iter().copied().enumerate() {
            let target = self.targets[index];
            if target == 0 {
                continue;
            }

            let Some(terrain) = TerrainKind::from_index(index as u8) else {
                continue;
            };

            if (here as f32) < target as f32 * TOP_UP_BELOW {
                wanted.push(Adjustment {
                    terrain,
                    add: target - here,
                    remove: 0,
                });
            } else if (here as f32) > target as f32 * THIN_ABOVE {
                wanted.push(Adjustment {
                    terrain,
                    add: 0,
                    remove: here - target,
                });
            }
        }

        wanted
    }

    /// The first fill, which puts every terrain straight at its target.
    pub fn opening(&self) -> Vec<Adjustment> {
        (0..TERRAIN_COUNT)
            .filter(|index| self.targets[*index] > 0)
            .filter_map(|index| {
                Some(Adjustment {
                    terrain: TerrainKind::from_index(index as u8)?,
                    add: self.targets[index],
                    remove: 0,
                })
            })
            .collect()
    }
}

/// Chooses what to place on a terrain.
///
/// `roll` is a value in `0.0..1.0`, so the caller owns the randomness and a test can make the
/// choice deterministic. The weights for a terrain sum to one, and a roll past the end of them
/// chooses nothing, exactly as the original's `GetRandomObjType` returns zero.
pub fn choose(spawnable: &[Spawn], terrain: TerrainKind, roll: f32) -> Option<Spawn> {
    let mut passed = 0.0;

    for spawn in spawnable.iter().filter(|spawn| spawn.terrain == terrain) {
        passed += spawn.weight;
        if passed > roll {
            return Some(*spawn);
        }
    }

    None
}

/// A standard normal sample, from two uniform ones.
///
/// Box-Muller, as the original uses, so a group's size varies the same way rather than uniformly.
pub fn normal(first: f32, second: f32) -> f32 {
    let first = first.clamp(f32::MIN_POSITIVE, 1.0);
    (-2.0 * first.ln()).sqrt() * (std::f32::consts::TAU * second).sin()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawns() -> Vec<Spawn> {
        vec![
            Spawn {
                kind: ObjectType(1),
                terrain: TerrainKind::LowForest,
                weight: 0.75,
                group: None,
                per_enemy: 1400,
            },
            Spawn {
                kind: ObjectType(2),
                terrain: TerrainKind::LowForest,
                weight: 0.25,
                group: None,
                per_enemy: 1400,
            },
            Spawn {
                kind: ObjectType(3),
                terrain: TerrainKind::Mountains,
                weight: 1.0,
                group: None,
                per_enemy: 200,
            },
        ]
    }

    fn measured(squares: u32) -> Realm {
        let mut counts = [0u32; TERRAIN_COUNT];
        counts[TerrainKind::LowForest as usize] = squares;

        let mut realm = Realm::new();
        realm.measure(&counts);
        realm
    }

    #[test]
    fn every_enemy_oryx_names_is_in_the_content() {
        // A name the catalog does not have is a whole stretch of map that stays empty, and the only
        // symptom is that nobody ever fights there.
        let Ok((catalog, _)) =
            Catalog::load_dir(std::path::Path::new("../../../godot-client/assets/xml"))
        else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        let (found, missing) = spawnable_reporting(&catalog);
        assert!(missing.is_empty(), "not in the content: {missing:?}");
        assert_eq!(
            found.len(),
            REGIONS.iter().map(|r| r.mobs.len()).sum::<usize>()
        );

        // And every terrain the table lists must have something on it.
        for region in REGIONS {
            assert!(
                found.iter().any(|spawn| spawn.terrain == region.terrain),
                "nothing belongs on {:?}",
                region.terrain
            );
        }
    }

    #[test]
    fn a_terrains_population_comes_from_how_much_of_it_there_is() {
        let mut squares = [0u32; TERRAIN_COUNT];
        squares[TerrainKind::LowForest as usize] = 14_000;
        squares[TerrainKind::Mountains as usize] = 14_000;

        let targets = targets(&squares);

        // The same area of mountain holds seven times what the same area of forest does, because
        // the mountains give each enemy two hundred squares and the forest fourteen hundred.
        assert_eq!(targets[TerrainKind::LowForest as usize], 10);
        assert_eq!(targets[TerrainKind::Mountains as usize], 70);
    }

    #[test]
    fn a_terrain_nothing_is_listed_for_gets_nothing_however_big_it_is() {
        let mut squares = [0u32; TERRAIN_COUNT];
        squares[TerrainKind::BeachTowels as usize] = 1_000_000;

        assert_eq!(targets(&squares)[TerrainKind::BeachTowels as usize], 0);
    }

    #[test]
    fn an_opening_realm_fills_every_terrain_to_its_target() {
        let realm = measured(14_000);
        let opening = realm.opening();

        assert_eq!(opening.len(), 1);
        assert_eq!(opening[0].terrain, TerrainKind::LowForest);
        assert_eq!(opening[0].add, 10);
    }

    #[test]
    fn a_population_inside_the_band_is_left_alone() {
        // Holding it exactly at target after every kill would feel like a treadmill.
        let realm = measured(140_000);
        let target = realm.population();

        let mut alive = [0usize; TERRAIN_COUNT];
        for held in [
            (target as f32 * 0.8) as usize,
            target,
            (target as f32 * 1.4) as usize,
        ] {
            alive[TerrainKind::LowForest as usize] = held;
            assert!(
                realm.adjustments(&alive).is_empty(),
                "{held} of {target} was adjusted"
            );
        }
    }

    #[test]
    fn a_terrain_that_has_been_cleared_is_topped_up_to_its_target() {
        let realm = measured(140_000);
        let target = realm.population();

        let mut alive = [0usize; TERRAIN_COUNT];
        alive[TerrainKind::LowForest as usize] = target / 2;

        let wanted = realm.adjustments(&alive);
        assert_eq!(wanted.len(), 1);
        assert_eq!(wanted[0].add, target - target / 2);
        assert_eq!(wanted[0].remove, 0);
    }

    #[test]
    fn a_terrain_that_has_overfilled_is_thinned_back_to_its_target() {
        // Enemies that breed can push a terrain far past what the map asks for.
        let realm = measured(140_000);
        let target = realm.population();

        let mut alive = [0usize; TERRAIN_COUNT];
        alive[TerrainKind::LowForest as usize] = target * 2;

        let wanted = realm.adjustments(&alive);
        assert_eq!(wanted.len(), 1);
        assert_eq!(wanted[0].add, 0);
        assert_eq!(wanted[0].remove, target);
    }

    #[test]
    fn a_realm_closes_on_its_clock_rather_than_on_a_body_count() {
        // A busy realm would never close on clearance and an empty one would close immediately.
        let mut realm = measured(14_000);

        assert_eq!(realm.advance(REALM_LIFETIME_MS - 1), Some(Event::Ensure));
        assert_eq!(realm.phase(), Phase::Open);

        assert_eq!(realm.advance(1), Some(Event::Warned));
        assert_eq!(realm.phase(), Phase::Closing);
        assert!(
            realm.admits(),
            "the warning is a minute to get out, not a door"
        );

        assert_eq!(realm.advance(CLOSING_WARNING_MS), Some(Event::Closed));
        assert_eq!(realm.phase(), Phase::Closed);
        assert!(!realm.admits(), "nobody arrives in a closed realm");
        assert!(!realm.spawns());

        assert_eq!(realm.advance(CASTLE_AFTER_MS), Some(Event::Castle));
        assert_eq!(realm.phase(), Phase::Emptying);
    }

    #[test]
    fn the_closing_sequence_takes_its_steps_one_at_a_time() {
        // Each step is timed from the one before, so a stalled tick does not fire all of them at
        // once and quake everybody out with no warning at all.
        let mut realm = measured(14_000);

        assert_eq!(realm.advance(REALM_LIFETIME_MS), Some(Event::Warned));

        assert_ne!(
            realm.advance(1),
            Some(Event::Closed),
            "a minute has not passed"
        );
        assert_eq!(realm.phase(), Phase::Closing);
    }

    #[test]
    fn a_closing_realm_still_fills_itself_and_a_closed_one_does_not() {
        let mut realm = measured(140_000);
        let mut alive = [0usize; TERRAIN_COUNT];

        realm.advance(REALM_LIFETIME_MS);
        assert!(!realm.adjustments(&alive).is_empty(), "still filling");

        realm.advance(CLOSING_WARNING_MS);
        assert!(realm.adjustments(&alive).is_empty(), "and now it is done");

        alive[TerrainKind::LowForest as usize] = 1;
        assert!(realm.adjustments(&alive).is_empty());
    }

    #[test]
    fn the_population_is_checked_every_minute_and_not_more_often() {
        let mut realm = measured(14_000);

        assert_eq!(realm.advance(ENSURE_EVERY_MS), Some(Event::Ensure));
        assert_eq!(realm.advance(ENSURE_EVERY_MS - 1), None);
        assert_eq!(realm.advance(1), Some(Event::Ensure));
    }

    #[test]
    fn only_what_belongs_on_a_terrain_is_chosen_for_it() {
        let spawns = spawns();

        for roll in [0.0f32, 0.3, 0.6, 0.9] {
            let chosen = choose(&spawns, TerrainKind::LowForest, roll).map(|spawn| spawn.kind);
            assert!(
                matches!(chosen, Some(ObjectType(1)) | Some(ObjectType(2))),
                "roll {roll} chose {chosen:?} for forest"
            );
        }
    }

    #[test]
    fn a_terrain_nothing_belongs_to_gets_nothing_rather_than_the_first_thing() {
        assert_eq!(choose(&spawns(), TerrainKind::MidForest, 0.5), None);
    }

    #[test]
    fn weight_decides_how_often_something_is_chosen() {
        let spawns = spawns();

        let common = (0..100)
            .filter(|n| {
                choose(&spawns, TerrainKind::LowForest, *n as f32 / 100.0).map(|spawn| spawn.kind)
                    == Some(ObjectType(1))
            })
            .count();

        assert!((70..=80).contains(&common), "{common} of a hundred");
    }

    #[test]
    fn a_group_varies_in_size_but_stays_inside_its_bounds() {
        let count = hendra_content::SpawnCount {
            mean: 5,
            std_dev: 2,
            min: 3,
            max: 10,
        };

        let mut seen = std::collections::HashSet::new();
        for step in 0..100 {
            let sample = normal(
                (step as f32 + 0.5) / 100.0,
                (step as f32 * 0.37).fract().max(0.01),
            );
            let size = count.size(sample);
            assert!((3..=10).contains(&size), "{size} from {sample}");
            seen.insert(size);
        }

        assert!(seen.len() > 1, "every group was the same size");
    }
}
