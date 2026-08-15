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

    /// The closest player an enemy can see, and how far away they are.
    ///
    /// Invisible, paused and newly arrived players are left out, which is what `IsVisibleToEnemy`
    /// decides. Almost everything reads this one.
    pub nearest_player: Option<Nearby>,

    /// The closest player of any kind, hiding or not.
    ///
    /// Read only by the handful of behaviours the game marks `seeInvis`, which are written that way
    /// on purpose: an enemy that runs from you is meant to run whether or not you are hiding.
    pub nearest_player_hiding: Option<Nearby>,

    /// What players nearby have said this tick, with how far away each speaker was.
    ///
    /// Borrowed and usually empty: speech is rare, and a tick that allocated for it would allocate
    /// for every enemy in the world to hear nothing.
    pub said: &'a [(f32, &'a str)],

    /// Everything else in sight.
    ///
    /// Needed because a great many behaviours are about other entities rather than about players:
    /// waiting for the guardians to die, ordering minions into a state, healing whatever is
    /// standing nearby. Borrowed rather than owned so a tick allocates nothing.
    pub nearby: &'a [Neighbour],

    /// Damage taken since the last tick, for transitions that react to being hit.
    pub damage_taken: i32,

    /// Whether the host is stunned.
    ///
    /// Three bare flags rather than the whole condition set, because this crate has no catalog and
    /// no opinion about what an object type means; these are the only effects a behaviour itself
    /// reads. Everything else a condition does — being held still, taking no damage, being
    /// unworth attacking — the host decides when it carries an action out.
    ///
    /// `Shoot.cs:132` holds fire while stunned, and does so *before* the cooldown is rolled, so a
    /// stunned enemy fires the instant it recovers rather than waiting out a cycle it never spent.
    pub stunned: bool,

    /// Whether the host is dazed, which halves a volley.
    ///
    /// `Shoot.cs:136`: `count = (int)Math.Ceiling(_count / 2.0)`, rounded up, so a single shot
    /// stays a single shot and a five-shot fan becomes three.
    pub dazed: bool,

    /// Whether the host is paralysed.
    ///
    /// Not what stops it moving — that is the host's own job, since `Entity.ResolveNewLocation`
    /// refuses the move outright (`realm/Entity.cs:322-330`). What a behaviour does with this is
    /// zero its own speed, permanently and for every enemy of its type: see
    /// [`zero_speed_on_paralysis`].
    pub paralyzed: bool,
}

/// State the original keeps on the behaviour object rather than on the enemy running it.
///
/// `BehaviorDb` builds one `State` tree per object id at startup (`logic/BehaviorDb.cs:68-88`) and
/// `Entity.SwitchTo` points every instance's `CurrentState` at that same tree
/// (`realm/Entity.cs:236-245`). Only the `ref object state` threaded through
/// `Behavior.Tick` into `Entity.StateStorage` is per entity (`logic/Behavior.cs:11-24`); a plain
/// field of a behaviour is one variable shared by every enemy of that type, in every world, for the
/// lifetime of the process.
///
/// Three such fields matter, and they are what this module holds:
///
/// * the `speed` every movement behaviour zeroes under paralysis and never restores,
/// * the `_rotateCount` a `Shoot` advances with each volley (`logic/behaviors/Shoot.cs:147,178`),
/// * `MoveTo2`'s `once` and `returned` (`logic/behaviors/MoveTo2.cs:14-15`), which nothing in the
///   shipped content reaches but which are the same shape.
///
/// Keyed by program name and slot, because that pair names one behaviour object: `Init` is handed a
/// freshly built `new State(...)` for every one of the 752 ids the behaviour database registers, so
/// no two ids share a tree, and a slot is one behaviour's position within its own tree. Static
/// rather than held on a [`Program`], because each world holds its own clone of the compiled
/// programs where the original holds one tree they all borrow.
///
/// The mechanic above rests only on files byte-identical to the 2020 import. That last count does
/// not: `wServer/logic/db/` is absent from the baseline, because the import gitignored it
/// (`Server-Side/.gitignore:240`) while the csproj still compiled it, so the scripts here arrived
/// separately — see `docs/audit/11-the-reference-itself.md`. It is the tree we transpile from and
/// the one the server runs, but "752" describes that tree rather than the pristine reference, and
/// searching the baseline for a behaviour finds nothing at all.
mod shared {
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{OnceLock, PoisonError, RwLock};

    /// Whether any speed has been zeroed yet, so the common case never touches the map.
    ///
    /// Every movement behaviour asks this every tick, and until somebody throws a paralysing trap
    /// the answer is no for the whole process.
    static ANY_ZEROED: AtomicBool = AtomicBool::new(false);

    fn zeroed() -> &'static RwLock<HashMap<String, HashSet<usize>>> {
        static ZEROED: OnceLock<RwLock<HashMap<String, HashSet<usize>>>> = OnceLock::new();
        ZEROED.get_or_init(Default::default)
    }

    /// How many volleys each rotating `Shoot` has fired.
    ///
    /// Only the shots that actually turn are counted. The original advances `_rotateCount` on every
    /// volley whether or not `_rotateAngle` is set, but the count is read solely as
    /// `_rotateAngle * _rotateCount`, so for a shot that does not turn the number is unobservable —
    /// and counting those would put every enemy shot in the game through this lock.
    fn volleys() -> &'static RwLock<HashMap<String, HashMap<usize, u32>>> {
        static VOLLEYS: OnceLock<RwLock<HashMap<String, HashMap<usize, u32>>>> = OnceLock::new();
        VOLLEYS.get_or_init(Default::default)
    }

    pub(super) fn is_zeroed(name: &str, slot: usize) -> bool {
        if !ANY_ZEROED.load(Ordering::Relaxed) {
            return false;
        }
        zeroed()
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .is_some_and(|slots| slots.contains(&slot))
    }

    pub(super) fn zero(name: &str, slot: usize) {
        zeroed()
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(name.to_owned())
            .or_default()
            .insert(slot);
        ANY_ZEROED.store(true, Ordering::Relaxed);
    }

    /// Reads the volley count and advances it, as `a += _rotateAngle * _rotateCount; _rotateCount++`
    /// does.
    pub(super) fn next_volley(name: &str, slot: usize) -> u32 {
        let mut table = volleys().write().unwrap_or_else(PoisonError::into_inner);
        let counter = table
            .entry(name.to_owned())
            .or_default()
            .entry(slot)
            .or_insert(0);
        let fired = *counter;
        *counter = counter.wrapping_add(1);
        fired
    }
}

/// Zeroes a movement behaviour's speed if its host is paralysed, and says whether it was already
/// zero when this tick began.
///
/// Every movement behaviour in the original writes `speed = 0` at the top of `TickCore` while its
/// host is paralysed — `Wander.cs:33`, `Follow.cs:51`, `Buzz.cs:44`, `StayBack.cs:36`,
/// `StayCloseToSpawn.cs:35`, `Swirl.cs:50`, `Protect.cs:45`, `StayAbove.cs:29`,
/// `BackAndForth.cs:31` and `Orbit.cs:66` — and nothing ever writes it back: outside the
/// constructor, `speed = 0` is the only assignment to that field in the behaviour library. Since
/// the field is shared (see [`shared`]), paralysing one enemy for one tick zeroes the speed of
/// every enemy of its type for the rest of the process.
///
/// Zero is not still, either. `Utils.GetSpeed` is `5.55f * spd + 0.74f` with no paralysis term
/// (`realm/Utils.cs:297-300`), so a crippled behaviour walks at 0.74 tiles a second rather than
/// standing: a `follow(0.75)` chaser drops from 4.90 to 0.74 and never recovers.
///
/// The two answers differ for exactly one behaviour. `Orbit` reads its speed out of the per-entity
/// state storage that `OnStateEntry` seeded (`Orbit.cs:41-55, :77`), so the write in its `TickCore`
/// does nothing to the tick that makes it and bites only from the next state entry onwards;
/// everything else reads the field it just wrote and slows within the same tick.
///
/// Call it for every movement behaviour that is reached, before any early return, because that is
/// where the original writes: ahead of the target search, so a wanderer with nothing to chase and a
/// follower that has lost its player are zeroed just the same.
pub fn zero_speed_on_paralysis(program: &Program, slot: usize, paralyzed: bool) -> bool {
    let was = shared::is_zeroed(&program.name, slot);
    if paralyzed && !was {
        shared::zero(&program.name, slot);
    }
    was
}

/// How many volleys a turning `Shoot` has fired before this one, counting every enemy of its type.
///
/// `_rotateCount` is a plain field of the `Shoot` object (`logic/behaviors/Shoot.cs:147`), advanced
/// once per volley that actually fires (`:178`, inside the branch that found a target) and reset
/// nowhere: `Shoot.OnStateEntry` writes only the cooldown into the per-entity state, and the class
/// has no `OnStateExit`. So the sweep does not belong to an enemy or to a state — it belongs to the
/// type, and it keeps turning across a boss's phases, across its death, and across every copy of it
/// the server ever spawns.
///
/// Where a type has one instance at a time this is invisible. Where it has several, they share one
/// sweep: two towers firing on the same tick take consecutive numbers, so the pattern advances twice
/// per round and the pair fire at angles a step apart rather than together.
pub fn next_rotation(program: &Program, slot: usize) -> u32 {
    shared::next_volley(&program.name, slot)
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

    /// Where this target was one position sample ago, which is what a leading shot aims with.
    ///
    /// `Shoot.Predict` asks the target for `TryGetHistory(1)` (`logic/behaviors/Shoot.cs:94`), and
    /// the original writes one entry per *world* tick rather than per logic tick. The origin is
    /// what a target that has not been sampled twice yet reports, because the original's ring
    /// starts zeroed and is never primed — an enemy shooting at somebody who has just arrived
    /// leads them toward the corner of the map. See [`PREDICT_SAMPLE_MS`].
    pub past_x: f32,
    pub past_y: f32,
}

impl Nearby {
    /// A target with no movement history, which is what the original reports for anything that has
    /// been in the world for less than two position samples.
    pub fn at(x: f32, y: f32, distance: f32) -> Nearby {
        Nearby {
            x,
            y,
            distance,
            past_x: 0.0,
            past_y: 0.0,
        }
    }
}

/// How far ahead of itself a target is led, as a multiple of one sample's travel.
///
/// `PREDICT_NUM_TICKS` (`logic/behaviors/Shoot.cs:93`), described there as "magic determined by
/// experiment".
pub const PREDICT_STEPS: f32 = 4.0;

/// How far apart the position samples a leading shot is built from are, in milliseconds.
///
/// Not the tick: `Entity.Tick` writes one entry per *world* tick (`realm/Entity.cs:230`), and
/// `FLLogicTicker.TickWorlds1` only runs the world tick once the accumulated delta reaches 200 ms
/// (`realm/FLLogicTicker.cs:149-156`). At the shipped six ticks a second (`bin/wServer.json:23`,
/// 166 ms a tick) the first delta to clear 200 ms is two ticks, so a sample lands every 332 ms.
///
/// This is the number that decides how hard an enemy leads: the shot is aimed four samples of
/// travel ahead of where the target is, so it leads by between 1.3 and 2.7 seconds of the target's
/// movement depending where in the sampling period the shot falls. Reading `TryGetHistory(1)` as
/// "one tick" and porting it to a fifty-millisecond tick would lead by a fifth of a second, and
/// every leading enemy in the game would shoot behind its target instead of in front of it.
pub const PREDICT_SAMPLE_MS: u32 = 332;

/// How long one of the original's logic ticks is, in milliseconds.
///
/// `FLLogicTicker.MsPT` is `1000 / TPS` (`realm/FLLogicTicker.cs:30`) and the shipped server runs
/// six ticks a second (`bin/wServer.json:23`), so integer division makes a tick 166 ms. Every
/// behaviour and every transition is ticked on that clock and on no other: `TickWorlds1` hands each
/// world a `RealmTime` whose `ElaspedMsDelta` is exactly one `MsPT`, and `World.TickLogic` walks the
/// enemies with it (`realm/worlds/World.cs:670-695`).
pub const LOGIC_TICK_MS: u32 = 166;

/// How long a duration written in the content actually lasts on the original's clock.
///
/// Every countdown in the original is spelt the same way — `if (cool <= 0) { act; cool = period; }
/// else { cool -= time.ElaspedMsDelta; }` (`logic/behaviors/Shoot.cs:206-215`,
/// `logic/transitions/TimedTransition.cs:26-34`, `logic/behaviors/Decay.cs:27-33`, and so on). Two
/// things follow from that shape, and both of them make the real period longer than the number
/// written in the behaviour:
///
/// * the countdown only ever moves in whole 166 ms ticks, so a period is rounded *up* to a tick, and
/// * the tick that acts does not decrement, and the tick that takes the counter to zero or below
///   does not act, so one whole extra tick separates each action from the next.
///
/// A shoot written `cooldown: 500` therefore fires every 830 ms, not every 500 ms, and one written
/// `cooldown: 1000` fires every 1328 ms. Taking the written number at face value on a 50 ms tick
/// makes every timed enemy in the game act between a third and two thirds faster than it does on the
/// original — which is a change to enemy damage output, not to smoothness.
pub const fn on_the_original_clock(ms: u32) -> u32 {
    (ms.div_ceil(LOGIC_TICK_MS) + 1) * LOGIC_TICK_MS
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

/// How fast a behaviour's speed argument actually carries an enemy, in tiles per second.
///
/// `Utils.GetSpeed` (`realm/Utils.cs:297`), which every movement behaviour in the original passes
/// its speed through — `Wander.cs:50`, `Follow.cs:93`, `Orbit.cs:77`, and twelve more. It is
/// affine rather than a plain multiple, and the constant term matters at both ends: a wander
/// written at a tenth still creeps forward at 1.3 tiles a second, and a follow written at one
/// reaches 6.3 rather than ten.
///
/// The halving for a slowed host is applied to the whole result, so it takes the constant term with
/// it. It is the only condition this function knows about: an enemy under `Speedy` moves at its
/// ordinary speed in the original, and `Paralyzed` is handled by refusing the move rather than by
/// slowing it (`realm/Entity.cs:324`).
///
/// Which is why the constant term is what a paralysed enemy's type is left with once the effect
/// ends: the behaviour's own speed argument has been set to nought and never restored, and
/// `5.55 * 0 + 0.74` is 0.74 rather than zero. See [`zero_speed_on_paralysis`].
pub fn tiles_per_second(speed: f32, slowed: bool) -> f32 {
    let tiles = 5.55 * speed + 0.74;
    if slowed { tiles / 2.0 } else { tiles }
}

/// What a behaviour wants to happen.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Move in this direction, at the speed the content asked for.
    ///
    /// The speed is the behaviour's own argument rather than a distance: turning it into tiles per
    /// second is [`tiles_per_second`], and only the host knows whether the entity is slowed.
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

        /// How long before it arrives. Zero appears at once.
        delay_ms: u32,

        /// Whether the children are worth experience when they die.
        gives_no_xp: bool,
    },

    /// Remove this entity without it counting as a kill.
    ///
    /// `dies` separates the two behaviours that produce one. `Suicide` calls `Enemy.Death`
    /// (`Suicide.cs:22`), which runs the entity's own death behaviours and is how most of the
    /// game's dungeon portals are dropped; `Decay` calls `LeaveWorld` (`Decay.cs:31`), which runs
    /// none of them. Neither pays experience or loot.
    Vanish {
        dies: bool,
    },

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

    /// Replace the ground in a circle, or a single square when the radius is nothing.
    Ground {
        tile: NameRef,
        radius: f32,

        /// Where the change is centred, relative to the entity.
        offset_x: f32,
        offset_y: f32,
    },

    /// Draw a setpiece where the entity is standing.
    Setpiece {
        name: String,
    },

    /// Stop this entity awarding experience, for summons that would otherwise farm it.
    NoExperience,

    /// Remove other entities of a kind nearby, for bosses that clean up their own summons.
    RemoveNearby {
        radius: f32,
        kind: Option<NameRef>,

        /// Whether each one dies rather than simply going.
        ///
        /// `RemoveEntity` marks every entity it finds `Spawned` and then kills it
        /// (`RemoveEntity.cs:27-28`), so their own death behaviours run while their experience and
        /// loot do not. `RemoveTileObject` clears map squares instead and kills nothing.
        dies: bool,
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

        /// How long after entering the state the first shot waits.
        ///
        /// `coolDownOffset`, and the single most used argument in the content after the cooldown
        /// itself. It is how a boss builds a rotating pattern: a dozen shoots with the same
        /// enormous cooldown and offsets a fifth of a second apart, each firing once in sequence.
        /// Dropped, they all fire on the same frame and then nothing does anything again.
        cooldown_offset_ms: u32,

        projectile: u8,

        /// How far away a target may be to be shot at, in tiles.
        ///
        /// `Shoot`'s first positional argument, passed by every use in the content. Without it an
        /// enemy fires at anything inside the twenty-tile sense radius rather than its own range,
        /// which is often four to eight, and nothing has a distance at which it is safe.
        acquire_range: f32,

        /// Where to aim with nothing in range, if the content says.
        ///
        /// With neither this nor a fixed angle an enemy holds fire, which is what the original does.
        default_angle: Option<f32>,

        /// Turns the whole spread, in degrees.
        angle_offset: f32,

        /// How often the shot leads a moving target rather than aiming where it stands.
        ///
        /// A probability, rolled per shot: `_predictive != 0 && _predictive > Random.NextDouble()`
        /// (`logic/behaviors/Shoot.cs:159`). Two hundred and eighty-one shots in the content ask
        /// for it, a hundred and twenty of them at one, which is every shot. Values above one occur
        /// and mean the same as one.
        predictive: f32,

        /// How far the whole spread turns with each volley, in degrees.
        ///
        /// `rotateAngle`, multiplied by a count that the original keeps on the *behaviour* rather
        /// than on the enemy and never resets (`logic/behaviors/Shoot.cs:178`). Forty shots in the
        /// content use it, and it is what makes a fixed spread sweep the room instead of standing
        /// still.
        rotate_angle: f32,
    },

    Wander {
        speed: f32,
    },

    /// Dart a short way in one of the eight compass directions, then pause and pick another.
    ///
    /// `Buzz`, and different from wandering in the way that matters to a player: a wanderer drifts
    /// and can be led, and this darts and cannot. Insects and wisps move this way.
    Buzz {
        speed: f32,

        /// How far one dart carries, in tiles.
        distance: f32,

        /// How long it waits between darts.
        cooldown_ms: u32,
    },

    Follow {
        speed: f32,
        acquire_range: f32,
        /// How close it tries to get before stopping.
        range: f32,

        /// How long a spell of following lasts. Zero follows for as long as the state does.
        duration_ms: u32,

        /// How long after a spell ends before another may start.
        cooldown_ms: u32,
    },

    Orbit {
        speed: f32,
        radius: f32,
        acquire_range: f32,

        /// What to circle. `None` circles the nearest player.
        ///
        /// 137 of the content's 167 orbits name something, and they are the ones that make an
        /// encounter readable: crystals circling a tracker, guardians circling their king. Without
        /// it every one of them circles whoever is closest, and a ring the player is meant to move
        /// around becomes a ring that follows them.
        target: Option<NameRef>,

        /// How much faster or slower than `speed` this particular entity circles.
        ///
        /// Drawn once per state entry, uniformly across plus and minus this. It is not optional in
        /// the original: left out, it defaults to a tenth of the speed (`Orbit.cs:38`), so every
        /// orbit in the content varies whether or not the script says so.
        speed_variance: f32,

        /// The same for the distance it keeps, defaulting to a tenth of the *speed* as the
        /// original's does — which is what makes a wide slow ring almost exactly circular.
        radius_variance: f32,

        /// Which way round. `None` means either way, drawn per entity.
        ///
        /// The original's default is anticlockwise rather than random (`Orbit.cs:32`, `false`).
        clockwise: Option<bool>,
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

        /// The share of `max_children` that appear on entering the state.
        ///
        /// A fraction, defaulting to a half; the count is the truncated product.
        initial_spawn: f32,

        cooldown_ms: u32,

        /// Whether the children are worth experience.
        ///
        /// `givesNoXp`, whose default in the original is **true**: unless a script says otherwise,
        /// a spawner's children are worth nothing at all. Awarding for them turns every spawner in
        /// the game into a place to stand and level.
        gives_no_xp: bool,
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

        /// How long after entering the state the first throw waits.
        ///
        /// The same idiom as a shot's, and used the same way: thirty-seven tosses in the content
        /// are written as a group with one long cooldown and offsets a fraction of a second apart,
        /// which is a boss laying a pattern of eggs rather than dropping all of them at once.
        cooldown_offset_ms: u32,

        /// How far out it lands, drawn per throw when the content gives both bounds.
        ///
        /// Both or neither: the original only rolls when it has a pair (`TossObject.cs:125`), and
        /// otherwise throws exactly its range.
        min_range: Option<f32>,
        max_range: Option<f32>,

        /// How long the ground is marked before the object lands.
        warning_ms: u32,
    },

    /// An explosion at a distance, which is how most telegraphed attacks are written.
    Grenade {
        radius: f32,
        damage: i32,
        range: f32,

        /// Thrown at a set angle rather than at whoever is nearest.
        fixed_angle: Option<f32>,

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
    ///
    /// The animating form of this steps the index from `index` up to `last` and holds there, or
    /// starts over when it loops. Five uses in the content animate; the other two hundred and fifty
    /// set one sprite and stop.
    SetAltTexture {
        index: u8,

        /// The last sprite of the run. `None` is the original's `-1`, which does not animate.
        last: Option<u8>,

        /// How long each sprite is held.
        step_ms: u32,

        /// Whether it starts over from the first sprite rather than stopping at the last.
        looping: bool,
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

    /// Run to a point.
    MoveTo {
        x: f32,
        y: f32,
        speed: f32,

        /// Whether the point is an offset from where the entity stood when it entered this state.
        ///
        /// `MoveTo2`'s `isMapPosition`, inverted: twenty-five of the twenty-nine uses in the
        /// content are offsets of a few tiles — `(-4, 0)`, `(0, 8)`, `(6, -6)` — and reading those
        /// as map coordinates sends the enemy to the top corner of the world instead of four tiles
        /// to its left. `MoveTo`, the other spelling, is always absolute.
        relative: bool,
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

    /// Remove nearby entities of a kind, killing them where the original kills them.
    RemoveNearby {
        radius: f32,
        kind: Option<NameRef>,
        dies: bool,
    },

    /// Draw a setpiece where the entity is standing.
    ///
    /// The name is one of the structures a realm is built with. Held as a name rather than resolved
    /// here, because the drawing lives in the simulation and this crate does not know about it.
    ApplySetpiece {
        name: String,
    },

    /// Replace the ground in a circle, or one named square of it.
    GroundTransform {
        tile: NameRef,
        radius: f32,
        cooldown_ms: u32,

        /// One square, this far from the entity, instead of the circle.
        ///
        /// `relativeX` and `relativeY`, which the original reads only as a pair (`GroundTransform`
        /// `.cs:50`) and which every use in the content gives: the ghost ship lays its beach one
        /// square at a time, and a circle centred on the enemy paints the wrong ones.
        offset: Option<(f32, f32)>,
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

    /// What this behaviour's cooldown starts at when its state is entered.
    ///
    /// Zero for everything but a shot and a throw, which start at their offset so that a group of
    /// them written together acts in sequence rather than at once.
    pub fn entry_cooldown_ms(&self) -> u32 {
        match self {
            Primitive::Shoot {
                cooldown_offset_ms, ..
            }
            | Primitive::TossObject {
                cooldown_offset_ms, ..
            } => *cooldown_offset_ms,

            // A run of sprites waits a whole step before its first change, which is what
            // `SetAltTexture.OnStateEntry` arms its timer to.
            Primitive::SetAltTexture { step_ms, .. } => *step_ms,

            _ => 0,
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
    TransformInto {
        child: NameRef,

        /// How many appear, drawn between the two inclusive.
        ///
        /// Both one in most of the content, but seven uses ask for more — three at once, or between
        /// five and seven — and one of them is what turns a killed segment into a cluster.
        min: u32,
        max: u32,

        /// How often it happens at all. Four uses are written well below one.
        probability: f32,
    },

    /// Drop a way into somewhere.
    Portal {
        name: NameRef,
        probability: f32,
        duration_ms: u32,
    },

    /// Change the ground where it stood.
    ///
    /// `ChangeGroundOnDeath(GroundToChange, ChangeTo, dist)` (`ChangeGroundOnDeath.cs:26-59`) is a
    /// `dist` by `dist` square whose corner is half a `dist` up and left of the square the enemy
    /// died on, and it rewrites only squares that are already one of the named kinds. An empty
    /// `sources` is the original's `groundToChange == null`, which changes every square in reach.
    ChangeGround {
        sources: Vec<NameRef>,
        targets: Vec<NameRef>,
        dist: u32,
    },

    /// Remove other entities of a kind, so a boss takes its summons with it.
    RemoveObjects { radius: f32, kind: Option<NameRef> },

    /// Drive whatever survives into a state.
    Order {
        radius: f32,
        kind: Option<NameRef>,
        state: Arc<str>,
    },

    /// Hand who hurt this entity to the nearest one of a kind, so that whoever fought it is
    /// eligible for what that one drops.
    TransferDamage { radius: f32, kind: Option<NameRef> },

    /// `CopyDamageOnDeath`, which does nothing.
    ///
    /// It looks like [`DeathEffect::TransferDamage`] and reads like it, but the method it ends in
    /// is empty (`Enemy.cs:56-58`), so the nearest entity of the named kind is found and then
    /// dropped. Kept as its own effect rather than folded into the transfer because the content
    /// asks for it fourteen times and the answer to all fourteen is nothing.
    CopyDamage { radius: f32, kind: Option<NameRef> },
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

        /// Whether a player an enemy cannot normally see counts.
        ///
        /// Ten enemies in the game are written this way, and they are written that way on purpose:
        /// a Candyland enemy that runs from you is meant to run whether or not you are hiding, and a
        /// sprite that teleports away is meant to escape an invisible pursuer.
        see_invis: bool,
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

    /// Somebody nearby said something matching this.
    ///
    /// The one condition a player can trigger on purpose. A dungeon whose door opens when you say
    /// the right word is built out of this, and without it the door has no handle.
    PlayerSaid {
        /// What to listen for. A whole word rather than a pattern: the original compiles a regular
        /// expression, and every use in the content is a plain word, so this matches the word and
        /// says so rather than pulling in an expression engine for eight uses.
        word: String,

        /// How near they have to be, or `None` for anywhere in the world.
        within: Option<f32>,

        /// Whether capitals matter.
        exact_case: bool,
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

    /// Where this transition's own countdown lives, unique within the program.
    ///
    /// The original stores a transition's countdown against the transition object
    /// (`logic/Transition.cs:24-35`), not against the state it sits in, and nothing clears it when
    /// the state changes. A transition therefore keeps whatever was left of its countdown while its
    /// state is not current, and picks it up from there when the state is entered again.
    pub slot: usize,
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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Program {
    pub name: String,
    pub states: Vec<CompiledState>,
    pub root: usize,
    pub slots: usize,

    /// How many countdowns the program's transitions between them need.
    pub transition_slots: usize,

    pub loot: Vec<LootEntry>,

    /// Every entity name the behaviours mention, interned.
    ///
    /// Kept as text because this crate cannot resolve them: it has no catalog and stays testable
    /// without one. The host calls [`Program::resolve`] once at load.
    pub names: Vec<String>,

    /// What the host resolved each name to. Empty for a name it does not have, which is a content
    /// problem worth reporting rather than a reason to refuse the enemy.
    ///
    /// A list rather than one type, because a name in the content is either one object or a whole
    /// group of them: `heal_group("Crystals")` means every crystal, and `spawn_group("Dwarves")`
    /// means one dwarf chosen from the several the group holds. Reading a group name as an object
    /// name finds nothing, which is a boss healing an empty set and looking as though it works.
    pub kinds: Vec<Vec<u16>>,
}

/// One entry in a loot table.
#[derive(Debug, Clone, PartialEq)]
pub enum LootEntry {
    /// A named item at a probability.
    Item {
        name: String,
        chance: f32,

        /// How many must drop whatever the roll says.
        ///
        /// `numRequired`. The original rolls as usual and then forces out however many did not
        /// appear, so an entry with one required and a three-in-ten chance always drops.
        required: u32,

        /// How much damage a player must have done before this is theirs, or zero for a drop
        /// anybody may take.
        ///
        /// `ItemLoot`'s own fourth argument (`logic/loot/MobDrops.cs:47`). An entry can carry one
        /// without being written inside a `Threshold` block; where it is written inside one, the
        /// block's share overrides this, since `MobDrops.Populate` applies the enclosing override
        /// whenever it is not negative (`MobDrops.cs:36`).
        threshold: f32,
    },

    /// Anything of a tier and kind.
    Tier {
        tier: u8,
        kind: String,
        chance: f32,
        required: u32,

        /// As [`LootEntry::Item::threshold`], from `TierLoot`'s fifth argument
        /// (`logic/loot/MobDrops.cs:72`).
        threshold: f32,
    },

    /// Loot that belongs to whoever earned it, rather than to whoever reaches the bag first.
    ///
    /// `share` is how much of the enemy's health somebody has to have taken off before they are
    /// eligible. Everything inside is then rolled once per eligible player into a bag only they can
    /// open, which is what makes a boss's drops worth fighting for rather than worth standing near.
    Threshold {
        share: f32,
        children: Vec<LootEntry>,
    },
}

impl LootEntry {
    /// How many of this must drop whatever the roll says.
    pub fn required(&self) -> u32 {
        match self {
            LootEntry::Item { required, .. } | LootEntry::Tier { required, .. } => *required,
            LootEntry::Threshold { .. } => 0,
        }
    }

    /// What share of an enemy somebody must have damaged to be eligible for this.
    ///
    /// Zero for anything that drops into the bag everybody can reach.
    pub fn share(&self) -> f32 {
        match self {
            LootEntry::Threshold { share, .. } => *share,
            _ => 0.0,
        }
    }
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
    pub fn resolve(&mut self, mut lookup: impl FnMut(&str) -> Vec<u16>) -> Vec<String> {
        self.kinds = self.names.iter().map(|name| lookup(name)).collect();

        self.names
            .iter()
            .zip(&self.kinds)
            .filter(|(_, kinds)| kinds.is_empty())
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// What a name resolved to, or `None` if it was never resolved or is not known.
    ///
    /// The first, for everything that means one kind of thing. A name that turned out to be a group
    /// has several, and the callers that mean "any of these" ask [`Program::kinds_of`] instead.
    pub fn kind_of(&self, name: NameRef) -> Option<u16> {
        self.kinds.get(name.index())?.first().copied()
    }

    /// Everything a name resolved to, which is more than one when the name is a group.
    pub fn kinds_of(&self, name: NameRef) -> &[u16] {
        self.kinds
            .get(name.index())
            .map(Vec::as_slice)
            .unwrap_or(&[])
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
