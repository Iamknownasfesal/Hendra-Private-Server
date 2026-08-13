//! Populating a realm, and closing it.
//!
//! # What decides where something spawns
//!
//! The content, and nothing else. Every object carries a terrain it belongs to, a probability per
//! square, and a ceiling on how many of it a realm may hold. All three are read from the catalog
//! rather than written here, so adding an enemy to the game is a content change.
//!
//! # Why a realm closes
//!
//! A realm is finite. It fills once, empties as it is cleared, and then gives way to the castle at
//! the end of it. Without a closing condition it would refill forever and the fight at the end
//! would never happen.

use hendra_content::{Catalog, ObjectType, Terrain as TerrainKind};

/// How full a realm is, and what that means for what happens next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Filling, or full. Enemies respawn.
    Open,

    /// Enough has been cleared that the way to the castle is open.
    Closing,

    /// The castle is up and the realm no longer refills.
    Closed,
}

/// The share of a realm's population that must be cleared before it closes.
pub const CLOSE_AT: f32 = 0.9;

/// How many enemies a realm holds when full.
///
/// Not a content value: the original scales it with the map, and a realm that held every enemy the
/// content allowed would be unplayable rather than full.
pub const POPULATION: usize = 1500;

/// One kind of enemy a realm may hold, and how many of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spawn {
    pub kind: ObjectType,

    /// Where it belongs. `None` means anywhere.
    pub terrain: Option<TerrainKind>,

    /// Relative weight, from the content's own probability.
    pub weight: f32,

    /// The most of this one realm may hold, from `per_realm_max`.
    pub ceiling: Option<usize>,
}

/// Everything a realm can be populated with, read once at load.
///
/// Built from the catalog rather than a list, so an enemy given a terrain appears in realms
/// without anything else being changed.
///
/// The terrain is what marks an enemy as belonging outdoors. `SpawnProbability` is read where the
/// content gives one and defaults to an equal share where it does not, which is every enemy in the
/// shipped files: the original carries its weights in a hardcoded table rather than in the XML,
/// and reproducing that table here would put content back in the code.
pub fn spawnable(catalog: &Catalog) -> Vec<Spawn> {
    catalog
        .objects()
        .filter(|desc| desc.enemy)
        .filter_map(|desc| {
            let terrain = desc.terrain.as_deref().and_then(TerrainKind::from_name)?;
            if terrain == TerrainKind::None {
                return None;
            }

            Some(Spawn {
                kind: desc.object_type,
                terrain: Some(terrain),
                weight: if desc.spawn_probability > 0.0 {
                    desc.spawn_probability
                } else {
                    1.0
                },
                ceiling: desc.per_realm_max.map(|most| most.max(0) as usize),
            })
        })
        .collect()
}

/// How a realm fills and empties.
#[derive(Debug, Clone)]
pub struct Realm {
    phase: Phase,

    /// How many were here when it was full, which is what "cleared" is measured against.
    peak: usize,
}

impl Default for Realm {
    fn default() -> Realm {
        Realm::new()
    }
}

impl Realm {
    pub fn new() -> Realm {
        Realm {
            phase: Phase::Open,
            peak: 0,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// How many have been cleared, as a share of the fullest the realm has been.
    ///
    /// Measured against the peak rather than a constant, so a realm that never filled completely
    /// still closes when it is emptied.
    pub fn cleared(&self, alive: usize) -> f32 {
        if self.peak == 0 {
            return 0.0;
        }
        1.0 - (alive as f32 / self.peak as f32).clamp(0.0, 1.0)
    }

    /// Tells the realm how many enemies are alive, and returns how many more it wants.
    ///
    /// A closing or closed realm wants none: refilling is what would stop the fight at the end
    /// from ever arriving.
    pub fn wants(&mut self, alive: usize) -> usize {
        self.peak = self.peak.max(alive);

        if self.phase == Phase::Open && self.cleared(alive) >= CLOSE_AT {
            self.phase = Phase::Closing;
        }

        match self.phase {
            Phase::Open => POPULATION.saturating_sub(alive),
            Phase::Closing | Phase::Closed => 0,
        }
    }

    /// Marks the castle as up.
    pub fn close(&mut self) {
        self.phase = Phase::Closed;
    }
}

/// Chooses what to spawn on a square of a given terrain.
///
/// `roll` is a value in `0.0..1.0`, so the caller owns the randomness and a test can make the
/// choice deterministic.
pub fn choose(
    spawnable: &[Spawn],
    terrain: TerrainKind,
    held: &dyn Fn(ObjectType) -> usize,
    roll: f32,
) -> Option<ObjectType> {
    // Only what belongs here, and only what the realm has room for. Filtering before weighting is
    // what keeps a full ceiling from taking a share of the roll and spawning nothing.
    let eligible: Vec<&Spawn> = spawnable
        .iter()
        .filter(|spawn| spawn.terrain.is_none_or(|wanted| wanted == terrain))
        .filter(|spawn| spawn.ceiling.is_none_or(|most| held(spawn.kind) < most))
        .collect();

    let total: f32 = eligible.iter().map(|spawn| spawn.weight).sum();
    if total <= 0.0 {
        return None;
    }

    let mut wanted = roll.clamp(0.0, 0.999) * total;
    for spawn in eligible {
        wanted -= spawn.weight;
        if wanted <= 0.0 {
            return Some(spawn.kind);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawns() -> Vec<Spawn> {
        vec![
            Spawn {
                kind: ObjectType(1),
                terrain: Some(TerrainKind::LowForest),
                weight: 1.0,
                ceiling: None,
            },
            Spawn {
                kind: ObjectType(2),
                terrain: Some(TerrainKind::LowPlains),
                weight: 1.0,
                ceiling: None,
            },
            Spawn {
                kind: ObjectType(3),
                terrain: None,
                weight: 1.0,
                ceiling: Some(1),
            },
        ]
    }

    #[test]
    fn the_shipped_content_can_actually_populate_a_realm() {
        // The first version filtered on `SpawnProbability`, which no object in the shipped files
        // carries, so it found nothing and said so only as a silent zero. A rule that matches
        // nothing is worse than no rule, because it looks like it works.
        let Ok((catalog, _)) =
            Catalog::load_dir(std::path::Path::new("../../../godot-client/assets/xml"))
        else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        let spawnable = spawnable(&catalog);
        assert!(
            spawnable.len() > 50,
            "only {} enemies can populate a realm",
            spawnable.len()
        );

        // And every terrain a realm has must have something that belongs on it, or that part of
        // the map would be empty however long anyone waited.
        for terrain in [
            TerrainKind::LowForest,
            TerrainKind::MidPlains,
            TerrainKind::HighSand,
            TerrainKind::Mountains,
        ] {
            let eligible = spawnable
                .iter()
                .filter(|spawn| spawn.terrain.is_none_or(|wanted| wanted == terrain))
                .count();
            assert!(eligible > 0, "nothing belongs on {terrain:?}");
        }
    }

    #[test]
    fn a_new_realm_wants_filling() {
        let mut realm = Realm::new();
        assert_eq!(realm.wants(0), POPULATION);
        assert_eq!(realm.phase(), Phase::Open);
    }

    #[test]
    fn a_realm_tops_itself_up_as_it_is_cleared() {
        let mut realm = Realm::new();
        realm.wants(POPULATION);

        assert_eq!(realm.wants(POPULATION - 10), 10);
    }

    #[test]
    fn a_realm_closes_once_it_is_nearly_cleared_and_stops_refilling() {
        // Without this it would refill forever and the fight at the end would never happen.
        let mut realm = Realm::new();
        realm.wants(POPULATION);

        let nearly = (POPULATION as f32 * (1.0 - CLOSE_AT)) as usize;
        assert!(realm.wants(nearly + 100) > 0, "still filling");

        assert_eq!(realm.wants(nearly), 0, "and now it is done");
        assert_eq!(realm.phase(), Phase::Closing);
    }

    #[test]
    fn clearing_is_measured_against_the_fullest_it_has_been() {
        // A realm that never filled completely should still close when it is emptied.
        let mut realm = Realm::new();
        realm.wants(100);

        assert!((realm.cleared(50) - 0.5).abs() < 0.01);
        assert!((realm.cleared(0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn a_closed_realm_never_wants_anything_again() {
        let mut realm = Realm::new();
        realm.wants(POPULATION);
        realm.close();

        assert_eq!(realm.wants(0), 0);
        assert_eq!(realm.phase(), Phase::Closed);
    }

    #[test]
    fn only_what_belongs_on_a_terrain_is_chosen_for_it() {
        let spawns = spawns();
        let none = |_: ObjectType| 0;

        for roll in [0.0f32, 0.3, 0.6, 0.9] {
            let chosen = choose(&spawns, TerrainKind::LowForest, &none, roll);
            assert!(
                matches!(chosen, Some(ObjectType(1)) | Some(ObjectType(3))),
                "roll {roll} chose {chosen:?} for forest"
            );
        }
    }

    #[test]
    fn something_at_its_ceiling_is_not_chosen_at_all() {
        // Filtered before weighting, so a full ceiling does not take a share of the roll and spawn
        // nothing.
        let spawns = spawns();
        let full = |kind: ObjectType| if kind == ObjectType(3) { 1 } else { 0 };

        for roll in [0.0f32, 0.25, 0.5, 0.75, 0.999] {
            assert_ne!(
                choose(&spawns, TerrainKind::LowForest, &full, roll),
                Some(ObjectType(3)),
                "roll {roll}"
            );
        }
    }

    #[test]
    fn a_terrain_nothing_belongs_to_gets_only_what_belongs_anywhere() {
        let spawns = spawns();
        let none = |_: ObjectType| 0;

        assert_eq!(
            choose(&spawns, TerrainKind::MidForest, &none, 0.5),
            Some(ObjectType(3))
        );
    }

    #[test]
    fn nothing_eligible_chooses_nothing_rather_than_the_first_thing() {
        let spawns = spawns();
        let full = |_: ObjectType| 999;

        assert_eq!(choose(&spawns, TerrainKind::MidForest, &full, 0.5), None);
    }

    #[test]
    fn weight_decides_how_often_something_is_chosen() {
        let heavy = vec![
            Spawn {
                kind: ObjectType(1),
                terrain: None,
                weight: 9.0,
                ceiling: None,
            },
            Spawn {
                kind: ObjectType(2),
                terrain: None,
                weight: 1.0,
                ceiling: None,
            },
        ];
        let none = |_: ObjectType| 0;

        let common = (0..100)
            .filter(|n| {
                choose(&heavy, TerrainKind::MidForest, &none, *n as f32 / 100.0)
                    == Some(ObjectType(1))
            })
            .count();

        assert!((85..=95).contains(&common), "{common} of a hundred");
    }
}
