//! The compiled form of a behaviour file, and what running it produces.
//!
//! # Why the runtime does not touch the world
//!
//! A behaviour asks for things — move this way, fire that spread, heal — and reads a small set of
//! facts about its surroundings. It never reaches into the simulation. That keeps the two
//! independent in the direction that matters: an enemy's behaviour can be tested by handing it a
//! position and a nearby player and reading back what it wanted to do, with no world, no tick loop
//! and no content catalog involved.
//!
//! ```text
//!   Senses  ──▶  Program::tick  ──▶  [Action]
//!   (what it     (the state          (what the world
//!    can see)     machine)            should do)
//! ```

/// What a behaviour can perceive.
///
/// Deliberately small. Every field here is something the simulation must compute for every enemy
/// every tick, so each one has to earn its place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Senses {
    pub x: f32,
    pub y: f32,

    pub hp: i32,
    pub max_hp: i32,

    /// Where the entity started, for behaviours that keep it near home.
    pub spawn_x: f32,
    pub spawn_y: f32,

    /// The closest player, and how far away they are.
    pub nearest_player: Option<Nearby>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Nearby {
    pub x: f32,
    pub y: f32,
    pub distance: f32,
}

impl Senses {
    pub fn health_fraction(&self) -> f32 {
        if self.max_hp <= 0 {
            return 1.0;
        }
        (self.hp as f32 / self.max_hp as f32).clamp(0.0, 1.0)
    }
}

/// What a behaviour wants to happen.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Move at this speed, in tiles per second, in this direction.
    Move { angle: f32, speed: f32 },

    /// Fire a spread of projectiles centred on `angle`.
    Shoot {
        angle: f32,
        count: u32,
        /// Degrees between adjacent shots.
        spread: f32,
        projectile: u8,
    },

    Heal { amount: i32 },

    /// Ask the world to create children.
    Spawn { child: String, count: u32 },

    /// Remove this entity without it counting as a kill.
    Vanish,
}

/// One thing an enemy does.
#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    Shoot {
        count: u32,
        spread: f32,
        /// `None` means aim at the nearest player.
        fixed_angle: Option<f32>,
        cooldown_ms: u32,
        projectile: u8,
    },

    Wander {
        speed: f32,
    },

    Follow {
        speed: f32,
        acquire_range: f32,
        /// How close it tries to get before stopping.
        range: f32,
    },

    Orbit {
        speed: f32,
        radius: f32,
        acquire_range: f32,
    },

    StayBack {
        speed: f32,
        distance: f32,
    },

    StayCloseToSpawn {
        speed: f32,
        range: f32,
    },

    HealSelf {
        amount: i32,
        cooldown_ms: u32,
    },

    Spawn {
        child: String,
        max_children: u32,
        cooldown_ms: u32,
    },

    /// Remove the entity. Used by summons that expire.
    Suicide,

    /// Run the first child that wants to act, and no others.
    ///
    /// This is what makes an enemy look deliberate rather than twitchy: it chases if it can,
    /// otherwise it keeps its distance, otherwise it wanders.
    Prioritize(Vec<Primitive>),

    /// A primitive the runtime does not implement yet.
    ///
    /// Kept rather than rejected so that one unimplemented behaviour costs that behaviour and not
    /// the whole enemy. The compiler reports it once, by name, at load.
    Unsupported { name: String },
}

impl Primitive {
    /// How many cooldown slots this primitive needs, counting its children.
    pub fn slots(&self) -> usize {
        match self {
            Primitive::Prioritize(children) => {
                1 + children.iter().map(Primitive::slots).sum::<usize>()
            }
            _ => 1,
        }
    }
}

/// When a state gives way to another.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// After this long in the current state.
    Timed { after_ms: u32 },

    PlayerWithin { radius: f32 },
    NoPlayerWithin { radius: f32 },

    /// Health at or below this fraction of the maximum.
    HpBelow { fraction: f32 },

    /// A condition the runtime does not implement. Never fires, and is reported at load.
    Unsupported { name: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledTransition {
    pub condition: Condition,
    /// Index into [`Program::states`].
    pub target: usize,
}

/// One state, with its position in the tree resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledState {
    pub name: String,

    /// The enclosing state, if any. Behaviours and transitions are inherited from ancestors, which
    /// is what lets a boss have a movement pattern that persists across its attack phases.
    pub parent: Option<usize>,

    /// Where a fresh entry to this state actually lands. Entering a state with children means
    /// entering its first child, all the way down.
    pub entry: usize,

    pub behaviours: Vec<Primitive>,
    pub transitions: Vec<CompiledTransition>,

    /// Where this state's cooldown slots begin.
    pub slot_base: usize,
}

/// One enemy's compiled behaviour.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub name: String,
    pub states: Vec<CompiledState>,
    pub root: usize,
    pub slots: usize,
    pub loot: Vec<LootEntry>,
}

/// One entry in a loot table.
#[derive(Debug, Clone, PartialEq)]
pub enum LootEntry {
    /// A named item at a probability.
    Item { name: String, chance: f32 },

    /// Anything of a tier and kind.
    Tier {
        tier: u8,
        kind: String,
        chance: f32,
    },
}

impl Program {
    pub fn state(&self, index: usize) -> Option<&CompiledState> {
        self.states.get(index)
    }

    pub fn state_named(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|state| state.name == name)
    }

    /// Every state from `index` up to the root, nearest first.
    ///
    /// Behaviours and transitions run from the innermost state outwards, so an inner state's
    /// transition takes precedence over an outer one competing for the same moment.
    pub fn ancestry(&self, index: usize, into: &mut Vec<usize>) {
        into.clear();
        let mut current = Some(index);
        while let Some(at) = current {
            into.push(at);
            current = self.states.get(at).and_then(|state| state.parent);
        }
    }
}

/// A whole file, compiled.
#[derive(Debug, Clone, Default)]
pub struct Programs {
    pub programs: Vec<Program>,
}

impl Programs {
    pub fn get(&self, name: &str) -> Option<&Program> {
        self.programs.iter().find(|program| program.name == name)
    }

    pub fn len(&self) -> usize {
        self.programs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.programs.is_empty()
    }
}
