//! The compiled form of a behaviour file, and what running it produces.
//!
//! # Why the runtime does not touch the world
//!
//! A behaviour asks for things, such as moving one way or firing a spread, and reads a small set of
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

use std::sync::Arc;

/// What a behaviour can perceive.
///
/// Deliberately small. Every field here is something the simulation must compute for every enemy
/// every tick, so each one has to earn its place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Senses<'a> {
    pub x: f32,
    pub y: f32,

    pub hp: i32,
    pub max_hp: i32,

    /// Where the entity started, for behaviours that keep it near home.
    pub spawn_x: f32,
    pub spawn_y: f32,

    /// The closest player, and how far away they are.
    pub nearest_player: Option<Nearby>,

    /// Everything else in sight.
    ///
    /// Needed because a great many behaviours are about other entities rather than about players:
    /// waiting for the guardians to die, ordering minions into a state, healing whatever is
    /// standing nearby. Borrowed rather than owned so a tick allocates nothing.
    pub nearby: &'a [Neighbour],

    /// Damage taken since the last tick, for transitions that react to being hit.
    pub damage_taken: i32,
}

/// Another entity, as a behaviour sees it.
///
/// `kind` is opaque here: this crate has no catalog and no opinion about what an object type
/// means. The host resolves names to types once at load and compares numbers thereafter, so
/// nothing in the tick loop is ever a string comparison.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Neighbour {
    pub kind: u16,

    /// Opaque to this crate, so an action can name this entity back to the host.
    pub id: u32,

    pub x: f32,
    pub y: f32,
    pub distance: f32,

    pub hp: i32,
    pub max_hp: i32,

    pub player: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Nearby {
    pub x: f32,
    pub y: f32,
    pub distance: f32,
}

impl Senses<'_> {
    pub fn health_fraction(&self) -> f32 {
        if self.max_hp <= 0 {
            return 1.0;
        }
        (self.hp as f32 / self.max_hp as f32).clamp(0.0, 1.0)
    }

    /// Whether anything of this kind is within a radius.
    pub fn any_within(&self, kind: u16, radius: f32) -> bool {
        self.nearby
            .iter()
            .any(|other| other.kind == kind && other.distance <= radius)
    }

    /// The closest entity of a kind within a radius.
    pub fn nearest_of(&self, kind: u16, radius: f32) -> Option<&Neighbour> {
        self.nearby
            .iter()
            .filter(|other| other.kind == kind && other.distance <= radius)
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
    }
}

/// What a behaviour wants to happen.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Move at this speed, in tiles per second, in this direction.
    Move {
        angle: f32,
        speed: f32,
    },

    /// Fire a spread of projectiles centred on `angle`.
    Shoot {
        angle: f32,
        count: u32,
        /// Degrees between adjacent shots.
        spread: f32,
        projectile: u8,
    },

    Heal {
        amount: i32,
    },

    /// Ask the world to create children.
    Spawn {
        child: NameRef,
        count: u32,

        /// Where, relative to the entity. Zero for "on top of me".
        offset_x: f32,
        offset_y: f32,

        /// The state the children start in, when they should not start at their own beginning.
        state: Option<Arc<str>>,
    },

    /// Remove this entity without it counting as a kill.
    Vanish,

    /// Apply a condition effect. `target` decides to whom.
    Effect {
        /// The host's effect number. Opaque here, as entity kinds are.
        effect: u8,
        duration_ms: u32,
        radius: f32,
        target: EffectTarget,
    },

    /// Change which sprite is drawn, for bosses that visibly change phase.
    Texture {
        index: u8,
    },

    /// Grow or shrink toward a size, in hundredths.
    Resize {
        /// Change per second. Negative shrinks.
        rate: f32,
        target: u16,
    },

    /// Blink a colour.
    Flash {
        colour: u32,
        period_ms: u32,
        repeats: u32,
    },

    /// Take an effect away.
    RemoveEffect {
        effect: u8,
    },

    /// Raise maximum health by how many players are nearby.
    ScaleHealth {
        per_player: i32,
        maximum_extra: i32,
        radius: f32,
    },

    /// Say something. Bosses announce their phases, and it is how a fight is legible.
    Say {
        text: Arc<str>,

        /// Heard across the world rather than only nearby.
        broadcast: bool,
    },

    /// Become a different entity, keeping position but not health.
    Transform {
        into: NameRef,
    },

    /// Tell nearby entities of a kind to enter a state.
    ///
    /// This is how a boss drives its minions, and it is the second most used behaviour in the
    /// game's content. `kind` of `None` means every entity in range.
    Order {
        radius: f32,
        kind: Option<NameRef>,
        state: Arc<str>,
    },

    /// Heal others rather than self.
    HealOthers {
        radius: f32,
        amount: i32,

        /// `None` heals anything, which is what the group forms of this do.
        kind: Option<NameRef>,

        /// Whether players are healed rather than other enemies.
        players: bool,
    },

    /// Damage everything in a circle, some distance away.
    Grenade {
        offset_x: f32,
        offset_y: f32,
        radius: f32,
        damage: i32,
        effect: Option<u8>,
        effect_ms: u32,
    },

    /// Put a portal down. Used on death, so a dungeon has a way in.
    Portal {
        name: NameRef,
        duration_ms: u32,
    },

    /// Replace the ground in a circle.
    Ground {
        tile: NameRef,
        radius: f32,
    },

    /// Stop this entity awarding experience, for summons that would otherwise farm it.
    NoExperience,

    /// Remove other entities of a kind nearby, for bosses that clean up their own summons.
    RemoveNearby {
        radius: f32,
        kind: Option<NameRef>,
    },
}

/// Who an effect lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTarget {
    /// The entity running the behaviour.
    Myself,

    /// Players within the radius.
    Players,

    /// Other entities within the radius.
    Others,
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
        child: NameRef,
        max_children: u32,
        cooldown_ms: u32,
    },

    /// Spawn children only while few enough of them are already nearby.
    ///
    /// Different from [`Primitive::Spawn`] in what bounds it: this counts what is actually in the
    /// world rather than what this entity remembers making, so killing the children lets it make
    /// more and its own death does not leak a count.
    Reproduce {
        child: NameRef,
        density_radius: f32,
        density_max: u32,
        cooldown_ms: u32,
    },

    /// Throw something to a spot, rather than dropping it underfoot.
    TossObject {
        child: NameRef,
        radius: f32,
        /// `None` throws toward whoever is nearest.
        fixed_angle: Option<f32>,
        cooldown_ms: u32,
        /// How long the ground is marked before the object lands.
        warning_ms: u32,
    },

    /// An explosion at a distance, which is how most telegraphed attacks are written.
    Grenade {
        radius: f32,
        damage: i32,
        range: f32,
        cooldown_ms: u32,
        effect: Option<u8>,
        effect_ms: u32,
    },

    /// Remove the entity. Used by summons that expire.
    Suicide,

    /// Remove the entity after a time, without it counting as a kill.
    Decay {
        after_ms: u32,
    },

    /// Hold a condition effect for as long as the state lasts.
    ///
    /// The most used behaviour in the game's content by a wide margin. It is how anything is made
    /// invulnerable, paralysed, or invisible for a phase.
    ConditionalEffect {
        effect: u8,
        duration_ms: u32,
        target: EffectTarget,
        radius: f32,
    },

    /// Change the sprite drawn, so a phase change is visible.
    SetAltTexture {
        index: u8,
    },

    /// Blink a colour, which is how the game telegraphs a phase change.
    Flash {
        colour: u32,
        period_ms: u32,
        repeats: u32,
    },

    /// Take an effect away, the counterpart to [`Primitive::ConditionalEffect`].
    RemoveEffect {
        effect: u8,
    },

    /// Raise maximum health with the number of players nearby.
    ///
    /// What stops a boss built for a crowd being trivial when two people find it, and the reverse.
    ScaleHealth {
        per_player: i32,
        maximum_extra: i32,
        radius: f32,
    },

    ChangeSize {
        rate: f32,
        target: u16,
    },

    /// Say something, occasionally.
    Taunt {
        lines: Vec<Arc<str>>,
        probability: f32,
        cooldown_ms: u32,
        broadcast: bool,
    },

    /// Drive other entities into a state. The second most used behaviour there is.
    Order {
        radius: f32,
        /// `None` orders everything in range.
        kind: Option<NameRef>,
        state: Arc<str>,
        /// Given once on entering the state rather than repeated while in it.
        once: bool,
    },

    /// Become something else.
    Transform {
        into: NameRef,
    },

    /// Stay near a named entity, and interpose.
    Protect {
        speed: f32,
        protectee: NameRef,
        acquire_range: f32,
        protect_range: f32,
        reprotect_range: f32,
    },

    /// Heal others rather than self.
    HealOthers {
        radius: f32,
        amount: i32,
        /// `None` heals anything nearby.
        kind: Option<NameRef>,
        players: bool,
        cooldown_ms: u32,
    },

    /// Run to a fixed point, in world coordinates.
    MoveTo {
        x: f32,
        y: f32,
        speed: f32,
    },

    /// Head in a fixed direction, for a distance.
    MoveLine {
        speed: f32,
        angle: f32,
    },

    /// Pace between two points either side of the spawn.
    BackAndForth {
        speed: f32,
        distance: f32,
    },

    /// Rush the nearest player, then rest.
    Charge {
        speed: f32,
        range: f32,
        cooldown_ms: u32,
    },

    /// Circle outward and back, which is what the game's spiral attacks are made of.
    Swirl {
        speed: f32,
        radius: f32,
        /// Whether the circle is centred on a player rather than on the spawn.
        targeted: bool,
    },

    /// Walk back to where it started.
    ReturnToSpawn {
        speed: f32,
        /// How close is close enough to stop.
        tolerance: f32,
    },

    /// Keep at least this far from the nearest player, without fleeing further.
    StayAbove {
        speed: f32,
        altitude: f32,
    },

    /// Stop awarding experience.
    NoExperience,

    /// Remove nearby entities of a kind.
    RemoveNearby {
        radius: f32,
        kind: Option<NameRef>,
    },

    /// Replace the ground in a circle.
    GroundTransform {
        tile: NameRef,
        radius: f32,
        cooldown_ms: u32,
    },

    /// Something that happens when the entity dies rather than while it lives.
    ///
    /// Held as one primitive with a payload rather than as a dozen, because they share everything
    /// except what they do: none of them run during a tick, all of them run exactly once, and the
    /// world reaches for them at the same moment.
    OnDeath(Box<DeathEffect>),

    /// Run children together on a period.
    Every {
        period_ms: u32,
        children: Vec<Primitive>,
    },

    /// Run children only while a condition holds.
    When {
        condition: Box<Condition>,
        children: Vec<Primitive>,
    },

    /// Run one child per turn, advancing each time the current one acts.
    ///
    /// What makes a boss's attack pattern a pattern rather than a scramble.
    Sequence {
        children: Vec<Primitive>,
    },

    /// Run the first child that wants to act, and no others.
    ///
    /// This is what makes an enemy look decided rather than twitchy: it chases if it can,
    /// otherwise it keeps its distance, otherwise it wanders.
    Prioritize(Vec<Primitive>),

    /// A primitive the runtime does not implement yet.
    ///
    /// Kept rather than rejected so that one unimplemented behaviour costs that behaviour and not
    /// the whole enemy. The compiler reports it once, by name, at load.
    Unsupported {
        name: String,
    },
}

impl Primitive {
    /// How many cooldown slots this primitive needs, counting its children.
    pub fn slots(&self) -> usize {
        match self {
            Primitive::Prioritize(children)
            | Primitive::Every { children, .. }
            | Primitive::When { children, .. }
            | Primitive::Sequence { children } => {
                1 + children.iter().map(Primitive::slots).sum::<usize>()
            }
            _ => 1,
        }
    }

    /// Whatever this does when the entity dies, if anything.
    pub fn death_effect(&self) -> Option<&DeathEffect> {
        match self {
            Primitive::OnDeath(effect) => Some(effect),
            _ => None,
        }
    }
}

/// What an entity does as it dies.
#[derive(Debug, Clone, PartialEq)]
pub enum DeathEffect {
    /// Leave something behind in its place.
    Spawn { child: NameRef, count: u32 },

    /// Become something else rather than dying.
    TransformInto { child: NameRef },

    /// Drop a way into somewhere.
    Portal {
        name: NameRef,
        probability: f32,
        duration_ms: u32,
    },

    /// Change the ground where it stood.
    ChangeGround { tile: NameRef, radius: f32 },

    /// Remove other entities of a kind, so a boss takes its summons with it.
    RemoveObjects { radius: f32, kind: Option<NameRef> },

    /// Drive whatever survives into a state.
    Order {
        radius: f32,
        kind: Option<NameRef>,
        state: Arc<str>,
    },

    /// Pass this entity's remaining health onto others as damage.
    TransferDamage { radius: f32, kind: Option<NameRef> },
}

/// When a state gives way to another.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// After this long in the current state.
    Timed {
        after_ms: u32,
    },

    PlayerWithin {
        radius: f32,
    },
    NoPlayerWithin {
        radius: f32,
    },

    /// Health at or below this fraction of the maximum.
    HpBelow {
        fraction: f32,
    },

    /// Something of this kind is within the radius.
    EntityWithin {
        kind: NameRef,
        radius: f32,
    },

    /// Nothing of any of these kinds is within the radius.
    ///
    /// Plural because that is how the content uses it: "when every guardian is dead". Held as a
    /// list rather than as several transitions because all of them must be absent at once.
    NoneWithin {
        kinds: Vec<NameRef>,
        radius: f32,
    },

    /// After a time drawn once from a range, so a group entering together does not leave together.
    TimedRandom {
        min_ms: u32,
        max_ms: u32,
    },

    /// This much damage has been taken since entering the state.
    DamageTaken {
        amount: i32,
    },

    /// Has not moved for this long.
    NotMoving {
        after_ms: u32,
    },

    /// A condition the runtime does not implement. Never fires, and is reported at load.
    Unsupported {
        name: String,
    },
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

/// A reference to an entity name, resolved once at load.
///
/// An index rather than a string, because these are compared every tick against every neighbour
/// and a string comparison there would be the most expensive thing in the loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NameRef(pub u32);

impl NameRef {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One enemy's compiled behaviour.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub name: String,
    pub states: Vec<CompiledState>,
    pub root: usize,
    pub slots: usize,
    pub loot: Vec<LootEntry>,

    /// Every entity name the behaviours mention, interned.
    ///
    /// Kept as text because this crate cannot resolve them: it has no catalog and stays testable
    /// without one. The host calls [`Program::resolve`] once at load.
    pub names: Vec<String>,

    /// What the host resolved each name to. `None` for a name it does not have, which is a content
    /// problem worth reporting rather than a reason to refuse the enemy.
    pub kinds: Vec<Option<u16>>,
}

/// One entry in a loot table.
#[derive(Debug, Clone, PartialEq)]
pub enum LootEntry {
    /// A named item at a probability.
    Item { name: String, chance: f32 },

    /// Anything of a tier and kind.
    Tier { tier: u8, kind: String, chance: f32 },
}

impl Program {
    pub fn state(&self, index: usize) -> Option<&CompiledState> {
        self.states.get(index)
    }

    /// Turns every name the behaviours mention into the host's own type.
    ///
    /// Called once at load. Returns the names the host did not recognise, so a content directory
    /// missing an enemy is a line in a log rather than a boss that silently never wakes up.
    ///
    /// The unknown names are owned rather than borrowed, so the caller can still read the program
    /// it just resolved, since reporting which enemy has the problem needs its name.
    pub fn resolve(&mut self, mut lookup: impl FnMut(&str) -> Option<u16>) -> Vec<String> {
        self.kinds = self.names.iter().map(|name| lookup(name)).collect();

        self.names
            .iter()
            .zip(&self.kinds)
            .filter(|(_, kind)| kind.is_none())
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// What a name resolved to, or `None` if it was never resolved or is not known.
    pub fn kind_of(&self, name: NameRef) -> Option<u16> {
        self.kinds.get(name.index()).copied().flatten()
    }

    /// The text behind a name reference, for diagnostics.
    pub fn text_of(&self, name: NameRef) -> &str {
        self.names
            .get(name.index())
            .map(String::as_str)
            .unwrap_or("")
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
