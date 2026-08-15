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
//! On a clock, not on a body count, and on the server's clock rather than the realm's own:
//! `Realm.Tick` (`Realm.cs:61`) reads `time.TotalElapsedMs`, which counts from when the server
//! started. Every realm therefore closes on the same half-hour, and one opened a few seconds before
//! that half-hour closes almost as soon as it exists.
//!
//! # The realm is never deleted
//!
//! It is recycled. When a closed realm empties, `Realm.Tick` (`Realm.cs:65-69`) calls `Init()`
//! again -- a fresh map, fresh set pieces and a fresh Oryx -- and clears `Closed`. The world object
//! itself survives, which is also why `Realm.jw` carries `persist: true` and the self-delete in
//! `World.Tick` (`World.cs:614`) never reaches it.

use hendra_content::{Catalog, ObjectType, TERRAIN_COUNT, Terrain as TerrainKind};

/// How full a realm is, and what happens next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Open. Enemies are kept topped up and players may arrive.
    Open,

    /// Announced but not yet closed. Players may still arrive during the warning.
    Closing,

    /// Closed. Nobody new arrives, though the population is still tended.
    Closed,
}

/// How often the realm closes, measured on the server's clock rather than the realm's.
pub const REALM_LIFETIME_MS: u64 = 1_800_000;

/// How wide the window is in which the half-hour is noticed.
///
/// `secondsElapsed % 1800 < 10` (`Realm.cs:62`) is a ten-second eligibility window rather than an
/// instant, because the check runs on a background task that may miss the exact second.
pub const CLOSE_WINDOW_MS: u64 = 10_000;

/// How much warning the realm gives before it closes.
pub const CLOSING_WARNING_MS: u64 = 60_000;

/// How long after closing before everybody is sent to the castle.
pub const CASTLE_AFTER_MS: u64 = 22_000;

/// How long a quake shakes before it moves anybody.
///
/// `World.QuakeToWorld` (`World.cs:552`) broadcasts the earthquake and only reconnects players
/// eight seconds later, which is what makes the shake mean something.
pub const QUAKE_AFTER_MS: u64 = 8_000;

/// How long Oryx sleeps between doing anything.
///
/// `Oryx.Tick` (`Oryx.cs:632`) returns immediately until ten seconds have passed, and everything
/// else he does is counted in these.
pub const ORYX_TICK_MS: u64 = 10_000;

/// Oryx taunts on every other ten-second tick, so every twenty seconds.
pub const TAUNT_EVERY_TICKS: u32 = 2;

/// Oryx checks the population on every sixth ten-second tick, so every minute.
pub const ENSURE_EVERY_TICKS: u32 = 6;

/// How many times a new realm is filled to its target.
///
/// Twice, and not by design. `Realm.Init` (`Realm.cs:40-41`) constructs an `Oryx`, whose
/// constructor (`Oryx.cs:496`) calls `Init()`, and then calls `Init()` on it again. The second pass
/// zeroes its own counters without looking at what the first pass placed, so a realm opens holding
/// roughly twice what its map asks for and is thinned back on Oryx's first population check ten
/// seconds later.
pub const INITIAL_FILLS: usize = 2;

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

/// A piece of the closing sequence waiting on its timer.
///
/// The original queues these as `WorldTimer`s on the world (`Oryx.cs:873`, `:882`,
/// `World.cs:552`), so each is armed by the step before it and runs on the world's own tick rather
/// than on Oryx's ten-second one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Shut the realm and say so.
    Close,

    /// Raise the castle and quake everybody into it.
    Castle,

    /// Move everybody the quake was for.
    Quake,
}

/// The life of one realm: where it is in its closing sequence, and when Oryx last stirred.
#[derive(Debug, Clone)]
pub struct Realm {
    /// `Oryx.Closing`: set when the sequence starts and cleared only by a reset.
    closing: bool,

    /// `World.Closed`: set a minute after the warning, and what refuses arrivals.
    closed: bool,

    /// This realm's own clock, which the queued steps are timed against.
    age_ms: u64,

    /// The closing sequence's outstanding timers, as `(when, what)`.
    steps: Vec<(u64, Step)>,

    /// The server clock at Oryx's last ten-second tick.
    ///
    /// Zero to begin with and compared against the server's uptime, as `Oryx._prevTick` is: a realm
    /// opened after the server has been up ten seconds therefore gets its first taunt and its first
    /// population check on its very first tick.
    prev_oryx_ms: u64,

    /// How many ten-second ticks Oryx has taken, which is what his two cadences count.
    oryx_ticks: u32,

    /// How many of each terrain there should be, from the map.
    targets: [usize; TERRAIN_COUNT],

    /// Which of [`EVENTS`] this overseer may still raise, by index.
    ///
    /// Struck off as they are used, for the ones the content says may only happen once in a realm.
    /// A new overseer -- which is what a reset builds -- starts with the whole list again.
    events: Vec<usize>,
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

    /// Oryx is due to say something about what still guards him.
    Taunt,

    /// A minute's warning, said to the whole server.
    Warned,

    /// Closed: no more arrivals.
    Closed,

    /// The castle is due, and everybody still here is quaked into it.
    Castle,

    /// The quake is over and the players it was for are moved.
    Quake,

    /// A closed realm has emptied and is built again from nothing.
    Reset,
}

impl Realm {
    pub fn new() -> Realm {
        Realm {
            closing: false,
            closed: false,
            age_ms: 0,
            steps: Vec::new(),
            prev_oryx_ms: 0,
            oryx_ticks: 0,
            targets: [0; TERRAIN_COUNT],
            events: (0..EVENTS.len()).collect(),
        }
    }

    /// Which event Oryx raises next, and what it is called.
    ///
    /// `OnEnemyKilled` (`Oryx.cs:838`) draws one at random from whatever is left. It does this
    /// every time a quest enemy dies: the quarter chance that used to gate it is commented out
    /// (`Oryx.cs:833`), so every kill raises something.
    pub fn choose_event(&self, roll: f32) -> Option<(usize, &'static str, crate::setpiece::Kind)> {
        let index = *pick(&self.events, roll)?;
        let (name, kind) = EVENTS.get(index)?;
        Some((index, name, *kind))
    }

    /// Strikes an event off, for one the content says may happen only once in a realm.
    pub fn spend_event(&mut self, index: usize) {
        self.events.retain(|held| *held != index);
    }

    /// Tells the realm how big each terrain is, which is what its population is drawn from.
    pub fn measure(&mut self, squares: &[u32; TERRAIN_COUNT]) {
        self.targets = targets(squares);
    }

    pub fn phase(&self) -> Phase {
        match (self.closing, self.closed) {
            (_, true) => Phase::Closed,
            (true, false) => Phase::Closing,
            (false, false) => Phase::Open,
        }
    }

    /// Whether the closing sequence has started, which is what refuses a second `/closerealm`.
    pub fn is_closing(&self) -> bool {
        self.closing
    }

    /// How many enemies this realm holds when it is at its target.
    pub fn population(&self) -> usize {
        self.targets.iter().sum()
    }

    /// Whether a player may arrive.
    ///
    /// `Realm.AllowedAccess` (`Realm.cs:30`) refuses a closed realm to everybody, administrators
    /// included: the map is about to be reset underneath whoever is standing on it.
    pub fn admits(&self) -> bool {
        !self.closed
    }

    /// Advances the realm's clock and returns everything that has become due.
    ///
    /// More than one thing can come due at once, and does: Oryx's first ten-second tick is both his
    /// first taunt and his first population check, because zero is divisible by two and by six.
    ///
    /// `uptime_ms` is the server's clock rather than this realm's, because that is what the
    /// original's `RealmTime.TotalElapsedMs` carries and what both the half-hour close and Oryx's
    /// own cadence are measured against.
    pub fn advance(&mut self, elapsed_ms: u64, uptime_ms: u64, players: usize) -> Vec<Event> {
        self.age_ms += elapsed_ms;
        let mut events = Vec::new();

        // The world's timers run at the top of its tick, before the realm looks at itself.
        let mut due: Vec<Step> = Vec::new();
        self.steps.retain(|(when, step)| {
            if *when <= self.age_ms {
                due.push(*step);
                false
            } else {
                true
            }
        });

        for step in due {
            match step {
                Step::Close => {
                    self.closed = true;
                    events.push(Event::Closed);
                    self.steps
                        .push((self.age_ms + CASTLE_AFTER_MS, Step::Castle));
                }

                Step::Castle => {
                    // The three taunts are said whether or not anybody is left to hear them:
                    // `SendToCastle` (`Oryx.cs:887-891`) broadcasts before it counts heads.
                    events.push(Event::Castle);

                    if players > 0 {
                        // `QuakeToWorld` closes the realm again -- already closed here -- and moves
                        // nobody for eight seconds.
                        self.closed = true;
                        self.steps.push((self.age_ms + QUAKE_AFTER_MS, Step::Quake));
                    }
                }

                Step::Quake => events.push(Event::Quake),
            }
        }

        // Half past every hour of server uptime, within a ten-second window.
        let seconds = uptime_ms / 1000;
        if seconds > 10 && seconds % (REALM_LIFETIME_MS / 1000) < CLOSE_WINDOW_MS / 1000 {
            if self.close_now() {
                events.push(Event::Warned);
            }
        }

        // A closed realm with nobody in it is rebuilt where it stands rather than deleted. Any step
        // already queued stays queued, so a realm reset before its castle was due still says the
        // castle's lines when their timer comes round.
        if self.closed && players == 0 {
            self.closed = false;
            self.closing = false;

            // A brand new overseer, whose own clock starts at nothing: the first tick of a fresh
            // `Oryx` compares against a zero `_prevTick` and so runs at once. His list of events
            // starts whole again too, so a realm that spent its one Skull Shrine may have another.
            self.oryx_ticks = 0;
            self.prev_oryx_ms = 0;
            self.events = (0..EVENTS.len()).collect();

            events.push(Event::Reset);
        }

        if uptime_ms.saturating_sub(self.prev_oryx_ms) > ORYX_TICK_MS {
            if self.oryx_ticks % TAUNT_EVERY_TICKS == 0 {
                events.push(Event::Taunt);
            }
            if self.oryx_ticks % ENSURE_EVERY_TICKS == 0 {
                events.push(Event::Ensure);
            }
            self.oryx_ticks += 1;
            self.prev_oryx_ms = uptime_ms;
        }

        events
    }

    /// Starts the closing sequence, and says whether it started.
    ///
    /// `InitCloseRealm` (`Oryx.cs:869`) is entered from the half-hour and from `/closerealm` alike,
    /// and the command refuses when the sequence is already running.
    pub fn close_now(&mut self) -> bool {
        if self.closing {
            return false;
        }

        self.closing = true;
        self.steps
            .push((self.age_ms + CLOSING_WARNING_MS, Step::Close));
        true
    }

    /// What to do about the population, given what is alive on each terrain.
    ///
    /// Held in a band rather than at a number: below three quarters of its target a terrain is
    /// topped up to the target, above one and a half it is thinned back to it, and in between it is
    /// left alone.
    ///
    /// Not gated on the realm being open. `Oryx.Tick` (`Oryx.cs:638`) gates only his taunts on
    /// `Closed`, so a realm in the last minute of its life is still being restocked.
    pub fn adjustments(&self, alive: &[usize; TERRAIN_COUNT]) -> Vec<Adjustment> {
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

/// What Oryx says about one kind of enemy.
///
/// From the `CriticalEnemies` table (`Oryx.cs:50`). An empty list is the original's `null`, which
/// is the difference between a taunt he does not have and one he chooses not to say.
pub struct Taunts {
    /// The object this is about, by the name the content gives it.
    pub name: &'static str,

    /// Said when one of these is placed in the realm.
    pub spawn: &'static [&'static str],

    /// Said when several are alive. `{COUNT}` is replaced with how many.
    pub number: &'static [&'static str],

    /// Said when one is alive, or when there is no line for several.
    pub last: &'static [&'static str],

    /// Said when one is killed. `{PLAYER}` is replaced with whoever killed it.
    pub killed: &'static [&'static str],
}

/// The seventeen enemies Oryx has anything to say about.
///
/// Three of them -- Cube God, Dragon Head and shtrs Defense System -- are still here although the
/// events that place them are commented out of `_events` (`Oryx.cs:36`, `:42`, `:44`). The table is
/// consulted by name whenever any quest enemy dies, so leaving them costs nothing and removing them
/// would silence Oryx about a Cube God placed by a map rather than by him.
pub const CRITICAL: &[Taunts] = &[
    Taunts {
        name: "Lich",
        spawn: &[],
        number: &[
            "I am invincible while my {COUNT} Liches still stand!",
            "My {COUNT} Liches will feast on your essence!",
        ],
        last: &[
            "My final Lich shall consume your souls!",
            "My final Lich will protect me forever!",
        ],
        killed: &[],
    },
    Taunts {
        name: "Ent Ancient",
        spawn: &[],
        number: &[
            "Mortal scum! My {COUNT} Ent Ancients will defend me forever!",
            "My forest of {COUNT} Ent Ancients is all the protection I need!",
        ],
        last: &[
            "My final Ent Ancient will destroy you all!",
            "My final Ent Ancient shall crush you!",
        ],
        killed: &[],
    },
    Taunts {
        name: "Oasis Giant",
        spawn: &[],
        number: &[
            "My {COUNT} Oasis Giants will feast on your flesh!",
            "You have no hope against my {COUNT} Oasis Giants!",
        ],
        last: &[
            "A powerful Oasis Giant still fights for me!",
            "You will never defeat me while an Oasis Giant remains!",
        ],
        killed: &[],
    },
    Taunts {
        name: "Phoenix Lord",
        spawn: &[],
        number: &[
            "Maggots! My {COUNT} Phoenix Lord will burn you to ash!",
            "My {COUNT} Phoenix Lords will serve me forever!",
        ],
        last: &[
            "My final Phoenix Lord will never fall!",
            "My last Phoenix Lord will blacken your bones!",
        ],
        killed: &[],
    },
    Taunts {
        name: "Ghost King",
        spawn: &[],
        number: &[
            "My {COUNT} Ghost Kings give me more than enough protection!",
            "Pathetic humans! My {COUNT} Ghost Kings shall destroy you utterly!",
        ],
        last: &[
            "A mighty Ghost King remains to guard me!",
            "My final Ghost King is untouchable!",
        ],
        killed: &[],
    },
    Taunts {
        name: "Cyclops God",
        spawn: &[],
        number: &[
            "Cretins! I have {COUNT} Cyclops Gods to guard me!",
            "My {COUNT} powerful Cyclops Gods will smash you!",
        ],
        last: &[
            "My last Cyclops God will smash you to pieces!",
            "My final Cyclops God shall crush your puny skulls!",
        ],
        killed: &[],
    },
    Taunts {
        name: "Red Demon",
        spawn: &[],
        number: &[
            "Fools! There is no escape from my {COUNT} Red Demons!",
            "My legion of {COUNT} Red Demons live only to serve me!",
        ],
        last: &[
            "My final Red Demon is unassailable!",
            "A Red Demon still guards me!",
        ],
        killed: &[],
    },
    Taunts {
        name: "Skull Shrine",
        spawn: &["Your futile efforts are no match for a Skull Shrine!"],
        number: &[
            "Insects!  {COUNT} Skull Shrines still protect me",
            "You hairless apes will never overcome my {COUNT} Skull Shrines!",
            "You frail humans will never defeat my {COUNT} Skull Shrines!",
            "Miserable worms like you cannot stand against my {COUNT} Skull Shrines!",
            "Imbeciles! My {COUNT} Skull Shrines make me invincible!",
        ],
        last: &[
            "Pathetic fools!  A Skull Shrine guards me!",
            "Miserable scum!  My Skull Shrine is invincible!",
        ],
        killed: &[
            "You defaced a Skull Shrine!  Minions, to arms!",
            "{PLAYER} razed one of my Skull Shrines -- I WILL HAVE MY REVENGE!",
            "{PLAYER}, you will rue the day you dared to defile my Skull Shrine!",
            "{PLAYER}, you contemptible pig! Ruining my Skull Shrine will be the last mistake you ever make!",
            "{PLAYER}, you insignificant cur! The penalty for destroying a Skull Shrine is death!",
        ],
    },
    Taunts {
        name: "Cube God",
        spawn: &["Your meager abilities cannot possibly challenge a Cube God!"],
        number: &[
            "Filthy vermin! My {COUNT} Cube Gods will exterminate you!",
            "Loathsome slugs! My {COUNT} Cube Gods will defeat you!",
            "You piteous cretins! {COUNT} Cube Gods still guard me!",
            "Your pathetic rabble will never survive against my {COUNT} Cube Gods!",
            "You feeble creatures have no hope against my {COUNT} Cube Gods!",
        ],
        last: &[
            "Worthless mortals! A mighty Cube God defends me!",
            "Wretched mongrels!  An unconquerable Cube God is my bulwark!",
        ],
        killed: &[
            "You have dispatched my Cube God, but you will never escape my Realm!",
            "{PLAYER}, you pathetic swine! How dare you assault my Cube God?",
            "{PLAYER}, you wretched dog! You killed my Cube God!",
            "{PLAYER}, you may have destroyed my Cube God but you will never defeat me!",
            "I have many more Cube Gods, {PLAYER}!",
        ],
    },
    Taunts {
        name: "The Magicial lord of sky",
        spawn: &["WHO COME TO THIS REALM?"],
        number: &[],
        last: &[],
        killed: &["HOW CAN YOU KILL MY SKY LORD????????????!!"],
    },
    Taunts {
        name: "LH Sentry",
        spawn: &["I'm very spooky boiii sexy"],
        number: &[],
        last: &[],
        killed: &[],
    },
    Taunts {
        name: "Pentaract",
        spawn: &["Behold my Pentaract, and despair!"],
        number: &[
            "Wretched creatures! {COUNT} Pentaracts remain!",
            "You detestable humans will never defeat my {COUNT} Pentaracts!",
            "My {COUNT} Pentaracts will protect me forever!",
            "Your weak efforts will never overcome my {COUNT} Pentaracts!",
            "Defiance is useless! My {COUNT} Pentaracts will crush you!",
        ],
        last: &[
            "I am invincible while my Pentaract stands!",
            "Ignorant fools! A Pentaract guards me still!",
        ],
        killed: &[
            "That was but one of many Pentaracts!",
            "You have razed my Pentaract, but you will die here in my Realm!",
            "{PLAYER}, you lowly scum!  You'll regret that you ever touched my Pentaract!",
            "{PLAYER}, you flea-ridden animal! You destoryed my Pentaract!",
            "{PLAYER}, by destroying my Pentaract you have sealed your own doom!",
        ],
    },
    Taunts {
        name: "Grand Sphinx",
        spawn: &["At last, a Grand Sphinx will teach you to respect!"],
        number: &[
            "You dull-spirited apes! You shall pose no challenge for {COUNT} Grand Sphinxes!",
            "Regret your choices, blasphemers! My {COUNT} Grand Sphinxes will teach you respect!",
            "My {COUNT} Grand Sphinxes protect my Chamber with their lives!",
            "My Grand Sphinxes will bewitch you with their beauty!",
        ],
        last: &[
            "A Grand Sphinx is more than a match for this rabble.",
            "You festering rat-catchers! A Grand Sphinx will make you doubt your purpose!",
            "Gaze upon the beauty of the Grand Sphinx and feel your last hopes drain away.",
        ],
        killed: &[
            "The death of my Grand Sphinx shall be avenged!",
            "My Grand Sphinx, she was so beautiful. I will kill you myself, {PLAYER}!",
            "My Grand Sphinx had lived for thousands of years! You, {PLAYER}, will not survive the day!",
            "{PLAYER}, you up-jumped goat herder! You shall pay for defeating my Grand Sphinx!",
            "{PLAYER}, you pestiferous lout! I will not forget what you did to my Grand Sphinx!",
            "{PLAYER}, you foul ruffian! Do not think I forget your defiling of my Grand Sphinx!",
        ],
    },
    Taunts {
        name: "Lord of the Lost Lands",
        spawn: &[
            "Cower in fear of my Lord of the Lost Lands!",
            "My Lord of the Lost Lands will make short work of you!",
        ],
        number: &[
            "Cower before your destroyer! You stand no chance against {COUNT} Lords of the Lost Lands!",
            "Your pathetic band of fighters will be crushed under the might feet of my {COUNT} Lords of the Lost Lands!",
            "Feel the awesome might of my {COUNT} Lords of the Lost Lands!",
            "Together, my {COUNT} Lords of the Lost Lands will squash you like a bug!",
            "Do not run! My {COUNT} Lords of the Lost Lands only wish to greet you!",
        ],
        last: &[
            "Give up now! You stand no chance against a Lord of the Lost Lands!",
            "Pathetic fools! My Lord of the Lost Lands will crush you all!",
            "You are nothing but disgusting slime to be scraped off the foot of my Lord of the Lost Lands!",
        ],
        killed: &[
            "How dare you foul-mouthed hooligans treat my Lord of the Lost Lands with such indignity!",
            "What trickery is this?! My Lord of the Lost Lands was invincible!",
            "You win this time, {PLAYER}, but mark my words:  You will fall before the day is done.",
            "{PLAYER}, I will never forget you exploited my Lord of the Lost Lands' weakness!",
            "{PLAYER}, you have done me a service! That Lord of the Lost Lands was not worthy of serving me.",
            "You got lucky this time {PLAYER}, but you stand no chance against me!",
        ],
    },
    Taunts {
        name: "Hermit God",
        spawn: &["My Hermit God's thousand tentacles shall drag you to a watery grave!"],
        number: &[
            "You will make a tasty snack for my Hermit Gods!",
            "I will enjoy watching my {COUNT} Hermit Gods fight over your corpse!",
        ],
        last: &[
            "You will be pulled to the bottom of the sea by my mighty Hermit God.",
            "Flee from my Hermit God, unless you desire a watery grave!",
            "My Hermit God awaits more sacrifices for the majestic Thessal.",
            "My Hermit God will pull you beneath the waves!",
            "You will make a tasty snack for my Hermit God!",
        ],
        killed: &[
            "This is preposterous!  There is no way you could have defeated my Hermit God!",
            "You were lucky this time, {PLAYER}!  You will rue this day that you killed my Hermit God!",
            "You naive imbecile, {PLAYER}! Without my Hermit God, Dreadstump is free to roam the seas without fear!",
            "My Hermit God was more than you'll ever be, {PLAYER}. I will kill you myself!",
        ],
    },
    Taunts {
        name: "Ghost Ship",
        spawn: &[
            "My Ghost Ship will terrorize you pathetic peasants!",
            "A Ghost Ship has entered the Realm.",
        ],
        number: &[],
        last: &[
            "My Ghost Ship will send you to a watery grave.",
            "You filthy mongrels stand no chance against my Ghost Ship!",
            "My Ghost Ship's cannonballs will crush your pathetic Knights!",
        ],
        killed: &[
            "My Ghost Ship will return!",
            "Alas, my beautiful Ghost Ship has sunk!",
            "{PLAYER}, you foul creature.  I shall see to your death personally!",
            "{PLAYER}, has crossed me for the last time! My Ghost Ship shall be avenged.",
            "{PLAYER} is such a jerk!",
            "How could a creature like {PLAYER} defeat my dreaded Ghost Ship?!",
            "The spirits of the sea will seek revenge on your worthless soul, {PLAYER}!",
        ],
    },
    Taunts {
        name: "Dragon Head",
        spawn: &[
            "The Rock Dragon has been summoned.",
            "Beware my Rock Dragon. All who face him shall perish.",
        ],
        number: &[],
        last: &[
            "My Rock Dragon will end your pathetic existence!",
            "Fools, no one can withstand the power of my Rock Dragon!",
            "The Rock Dragon will guard his post until the bitter end.",
            "The Rock Dragon will never let you enter the Lair of Draconis.",
        ],
        killed: &[
            "My Rock Dragon will return!",
            "The Rock Dragon has failed me!",
            "{PLAYER} knows not what he has done.  That Lair was guarded for the Realm's own protection!",
            "{PLAYER}, you have angered me for the last time!",
            "{PLAYER} will never survive the trials that lie ahead.",
            "A filthy weakling like {PLAYER} could never have defeated my Rock Dragon!!!",
            "You shall not live to see the next sunrise, {PLAYER}!",
        ],
    },
    Taunts {
        name: "shtrs Defense System",
        spawn: &[
            "The Shatters has been discovered!?!",
            "The Forgotten King has raised his Avatar!",
        ],
        number: &[],
        last: &[
            "Attacking the Avatar of the Forgotten King would be...unwise.",
            "Kill the Avatar, and you risk setting free an abomination.",
            "Before you enter the Shatters you must defeat the Avatar of the Forgotten King!",
        ],
        killed: &[
            "The Avatar has been defeated!",
            "How could simpletons kill The Avatar of the Forgotten King!?",
            "{PLAYER} has unleashed an evil upon this Realm.",
            "{PLAYER}, you have awoken the Forgotten King. Enjoy a slow death!",
            "{PLAYER} will never survive what lies in the depths of the Shatters.",
            "Enjoy your little victory while it lasts, {PLAYER}!",
        ],
    },
];

/// What Oryx has to say about a kind of enemy, if anything.
pub fn taunts_for(name: &str) -> Option<&'static Taunts> {
    CRITICAL.iter().find(|taunt| taunt.name == name)
}

/// The events Oryx raises, one for every quest enemy killed in his realm.
///
/// The eight live entries of `_events` (`Oryx.cs:33`). Three more are in the source and commented
/// out -- Cube God, Dragon Head and shtrs Defense System -- so a realm never sees them however long
/// it stays open, and they are not here.
///
/// The name is what the object is called in the content, which is how the ceiling on it is looked
/// up: an event whose object declares `PerRealmMax` of one is struck off the list the first time it
/// is placed, and cannot happen again in that realm.
pub const EVENTS: &[(&str, crate::setpiece::Kind)] = &[
    ("Skull Shrine", crate::setpiece::Kind::SkullShrine),
    ("Pentaract", crate::setpiece::Kind::Pentaract),
    ("Grand Sphinx", crate::setpiece::Kind::Sphinx),
    (
        "Lord of the Lost Lands",
        crate::setpiece::Kind::LordOfTheLostLands,
    ),
    ("Hermit God", crate::setpiece::Kind::Hermit),
    ("Ghost Ship", crate::setpiece::Kind::GhostShip),
    ("The Magicial lord of sky", crate::setpiece::Kind::LordOfSky),
    ("LH Sentry", crate::setpiece::Kind::Spooky),
];

/// The line Oryx says about how many of something still guard him.
///
/// `HandleAnnouncements` (`Oryx.cs:736`) picks one of the seventeen at random whether or not any
/// are alive, and says nothing when none are. The last-one line is used when exactly one is left,
/// and also when there is no line for several at all -- which is how a Ghost Ship, whose table has
/// no `{COUNT}` lines, is always spoken of in the singular.
pub fn announcement(taunt: &Taunts, count: usize, roll: f32) -> Option<String> {
    if count == 0 {
        return None;
    }

    if (count == 1 && !taunt.last.is_empty()) || (!taunt.last.is_empty() && taunt.number.is_empty())
    {
        return Some(pick(taunt.last, roll)?.to_string());
    }

    Some(pick(taunt.number, roll)?.replace("{COUNT}", &count.to_string()))
}

/// The line Oryx says when one of these is killed, with the killer named.
///
/// A kill nobody is credited with drops the lines that would have named somebody, which can leave
/// nothing to say at all.
pub fn eulogy(taunt: &Taunts, killer: Option<&str>, roll: f32) -> Option<String> {
    let lines: Vec<&&str> = taunt
        .killed
        .iter()
        .filter(|line| killer.is_some() || !line.contains("{PLAYER}"))
        .collect();

    let index = ((roll.clamp(0.0, 1.0) * lines.len() as f32) as usize).min(lines.len().max(1) - 1);
    let line = lines.get(index)?;

    Some(line.replace("{PLAYER}", killer.unwrap_or("")))
}

/// One of a list, by a roll in `0.0..1.0`.
pub fn pick<T>(from: &[T], roll: f32) -> Option<&T> {
    if from.is_empty() {
        return None;
    }

    let index = ((roll.clamp(0.0, 1.0) * from.len() as f32) as usize).min(from.len() - 1);
    from.get(index)
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

    /// Runs a realm forward in twenty-per-second ticks, holding the same crowd throughout, and
    /// hands back everything that came due with the second it came due at.
    fn run(realm: &mut Realm, seconds: u64, players: usize) -> Vec<(u64, Event)> {
        let mut seen = Vec::new();
        let start = realm.age_ms;

        for step in 0..(seconds * 20) {
            let uptime = start + (step + 1) * 50;
            for event in realm.advance(50, uptime, players) {
                seen.push(((uptime - start) / 1000, event));
            }
        }

        seen
    }

    /// Only the events the closing sequence is made of, which is what the timings are about.
    fn sequence(seen: &[(u64, Event)]) -> Vec<(u64, Event)> {
        seen.iter()
            .copied()
            .filter(|(_, event)| !matches!(event, Event::Taunt | Event::Ensure))
            .collect()
    }

    #[test]
    fn closing_a_realm_takes_a_minute_then_another_twenty_two_seconds_then_eight_more() {
        // `Oryx.InitCloseRealm` queues sixty seconds, `CloseRealm` twenty-two, and the quake in
        // `World.QuakeToWorld` another eight before it moves anybody.
        let mut realm = measured(14_000);
        assert!(realm.close_now());
        assert_eq!(realm.phase(), Phase::Closing);
        assert!(
            realm.admits(),
            "the warning is a minute to get out, not a door"
        );

        let seen = sequence(&run(&mut realm, 120, 1));

        assert_eq!(
            seen,
            vec![(60, Event::Closed), (82, Event::Castle), (90, Event::Quake),]
        );
    }

    #[test]
    fn a_realm_that_is_already_closing_will_not_start_again() {
        let mut realm = measured(14_000);
        assert!(realm.close_now());
        assert!(!realm.close_now(), "the command refuses the second time");
    }

    #[test]
    fn a_closed_realm_takes_nobody_in() {
        let mut realm = measured(14_000);
        realm.close_now();

        run(&mut realm, 59, 1);
        assert!(realm.admits(), "still inside the minute's warning");

        run(&mut realm, 2, 1);
        assert!(!realm.admits(), "nobody arrives in a closed realm");
    }

    #[test]
    fn an_empty_realm_is_never_quaked_into_a_castle() {
        // `SendToCastle` says its three lines and then returns, leaving the realm to reset.
        let mut realm = measured(14_000);
        realm.close_now();

        let seen = sequence(&run(&mut realm, 120, 0));
        assert!(
            !seen.iter().any(|(_, event)| *event == Event::Quake),
            "{seen:?}"
        );
    }

    #[test]
    fn a_closed_realm_that_empties_is_built_again_where_it_stands() {
        // The realm object is recycled rather than deleted: a fresh map, fresh set pieces and a
        // fresh Oryx, with `Closed` cleared.
        let mut realm = measured(14_000);
        realm.close_now();

        let seen = sequence(&run(&mut realm, 61, 0));
        assert!(seen.contains(&(60, Event::Closed)));
        assert!(seen.contains(&(60, Event::Reset)), "{seen:?}");

        assert!(realm.admits(), "a reset realm takes players again");
        assert_eq!(realm.phase(), Phase::Open);
    }

    #[test]
    fn a_realm_still_fills_itself_after_it_has_closed() {
        // An oddity worth keeping: `Oryx.Tick` gates only his taunts on `Closed`, so a realm in the
        // last twenty seconds of its life is still being restocked.
        let mut realm = measured(140_000);
        realm.close_now();
        run(&mut realm, 61, 1);

        assert_eq!(realm.phase(), Phase::Closed);
        let alive = [0usize; TERRAIN_COUNT];
        assert!(!realm.adjustments(&alive).is_empty());
    }

    #[test]
    fn oryx_taunts_every_twenty_seconds_and_counts_heads_every_minute() {
        let mut realm = measured(14_000);
        let seen = run(&mut realm, 121, 1);

        let taunts: Vec<u64> = seen
            .iter()
            .filter(|(_, event)| *event == Event::Taunt)
            .map(|(at, _)| *at)
            .collect();
        let counts: Vec<u64> = seen
            .iter()
            .filter(|(_, event)| *event == Event::Ensure)
            .map(|(at, _)| *at)
            .collect();

        // The first ten-second tick is both, because zero divides by two and by six alike.
        assert_eq!(taunts, vec![10, 30, 50, 70, 90, 110]);
        assert_eq!(counts, vec![10, 70]);
    }

    #[test]
    fn the_half_hour_closes_a_realm_on_the_servers_clock_rather_than_its_own() {
        // A realm opened at twenty-nine minutes past closes a minute later, not half an hour later:
        // `Realm.Tick` reads the server's total elapsed time.
        let mut realm = measured(14_000);

        let just_before = REALM_LIFETIME_MS - 1_000;
        assert!(
            realm.advance(50, just_before, 1).contains(&Event::Warned) == false,
            "not yet"
        );

        let events = realm.advance(50, REALM_LIFETIME_MS, 1);
        assert!(events.contains(&Event::Warned), "{events:?}");
        assert!(realm.is_closing());
    }

    #[test]
    fn the_half_hour_is_a_ten_second_window_rather_than_an_instant() {
        for uptime in [REALM_LIFETIME_MS, REALM_LIFETIME_MS + 9_000] {
            let mut realm = measured(14_000);
            assert!(
                realm.advance(50, uptime, 1).contains(&Event::Warned),
                "{uptime} is inside the window"
            );
        }

        let mut realm = measured(14_000);
        assert!(
            !realm
                .advance(50, REALM_LIFETIME_MS + 10_000, 1)
                .contains(&Event::Warned),
            "and ten seconds past it is not"
        );
    }

    #[test]
    fn oryx_says_nothing_about_something_that_is_not_there() {
        let lich = taunts_for("Lich").expect("Oryx knows about liches");
        assert_eq!(announcement(lich, 0, 0.5), None);
    }

    #[test]
    fn oryx_counts_what_is_left_and_speaks_of_one_in_the_singular() {
        let lich = taunts_for("Lich").expect("Oryx knows about liches");

        let many = announcement(lich, 7, 0.0).expect("a line about seven");
        assert!(many.contains('7'), "{many}");
        assert!(!many.contains("{COUNT}"), "{many}");

        let last = announcement(lich, 1, 0.0).expect("a line about the last one");
        assert!(last.contains("final"), "{last}");
    }

    #[test]
    fn something_with_no_line_for_several_is_always_spoken_of_in_the_singular() {
        // A Ghost Ship's table has no `{COUNT}` lines at all, so however many are afloat Oryx talks
        // about the one.
        let ship = taunts_for("Ghost Ship").expect("Oryx knows about ghost ships");
        assert!(ship.number.is_empty());

        let line = announcement(ship, 4, 0.0).expect("a line about a ghost ship");
        assert!(ship.last.contains(&line.as_str()), "{line}");
    }

    #[test]
    fn a_kill_nobody_is_credited_with_drops_the_lines_that_would_have_named_somebody() {
        let shrine = taunts_for("Skull Shrine").expect("Oryx knows about skull shrines");

        for step in 0..20 {
            let roll = step as f32 / 20.0;

            let named = eulogy(shrine, Some("Fesal"), roll).expect("a line");
            assert!(!named.contains("{PLAYER}"), "{named}");

            let anonymous = eulogy(shrine, None, roll).expect("a line");
            assert!(
                !anonymous.contains("{PLAYER}") && !anonymous.contains("Fesal"),
                "{anonymous}"
            );
        }
    }

    #[test]
    fn oryx_has_eight_events_and_they_are_all_drawn() {
        // The three commented out of `_events` must not be here: a realm never sees a Cube God, a
        // Rock Dragon or the Shatters avatar however long it stays open.
        assert_eq!(EVENTS.len(), 8);
        for banned in ["Cube God", "Dragon Head", "shtrs Defense System"] {
            assert!(
                !EVENTS.iter().any(|(name, _)| *name == banned),
                "{banned} is commented out of the original"
            );
        }

        let realm = Realm::new();
        for step in 0..40 {
            assert!(realm.choose_event(step as f32 / 40.0).is_some());
        }
    }

    #[test]
    fn an_event_that_may_happen_once_does_not_happen_twice() {
        let mut realm = Realm::new();

        let (index, name, _) = realm.choose_event(0.0).expect("an event");
        realm.spend_event(index);

        for step in 0..40 {
            let (_, again, _) = realm.choose_event(step as f32 / 40.0).expect("an event");
            assert_ne!(again, name, "the spent event came up again");
        }
    }

    #[test]
    fn a_reset_realm_may_raise_the_event_it_had_spent() {
        // The list belongs to the overseer, and a reset builds a new one.
        let mut realm = measured(14_000);
        for index in 0..EVENTS.len() {
            realm.spend_event(index);
        }
        assert!(realm.choose_event(0.5).is_none());

        realm.close_now();
        run(&mut realm, 61, 0);

        assert!(realm.choose_event(0.5).is_some());
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
