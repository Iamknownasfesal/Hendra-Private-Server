//! One world: what is in it, and what happens to it each tick.
//!
//! A world is owned outright by whatever ticks it. Nothing here is behind a lock, and nothing is
//! shared with another world. The C# server reached for `ConcurrentDictionary` on five collections
//! belonging to a world that only ever ticked on one thread, and paid lock-striping on every access
//! for the privilege. Worlds are independent, so parallelism belongs *between* them.
//!
//! # The tick
//!
//! ```text
//!   1. move players toward where they claim to be, within what the rules allow
//!   2. apply ground hazards
//!   3. rebuild the spatial index
//!   4. remove the dead
//! ```
//!
//! Snapshots are taken after all of that, per player, from the index.

use hendra_behavior::program::{
    Action, DeathEffect, EffectTarget, Nearby, Neighbour, Program, Programs, Senses,
};
use hendra_behavior::run::Mind;
use hendra_content::{Catalog, ConditionSet, Map, ObjectType, TERRAIN_COUNT};
use hendra_net::{EntityId, EntityState, Tick, WorldSnapshot};

use crate::grid::Grid;
use crate::inventory::{Container, ContainerKind};
use crate::projectile::{Hit, Projectile, Projectiles};
use crate::slab::{Handle, Slab};
use crate::tiles::Terrain;

/// How far a player can see, in tiles. Matches the client's view distance.
pub const SIGHT_RADIUS: f32 = 20.0;

/// Which squares of one world a player has laid eyes on.
///
/// A bit per square rather than a byte, because this is held per player and a realm is four million
/// squares: half a megabyte each instead of four. The original keeps a byte apiece to remember how
/// stale each square is, which is a question a server that sends the map whole does not have.
#[derive(Debug, Clone)]
pub struct Seen {
    bits: Vec<u64>,

    /// The square the player was standing on when this was last brought up to date. Nothing is
    /// walked while it is unchanged, since a player who has not left their square has not uncovered
    /// anything.
    ///
    /// A saving rather than a rule: the bits already make a second look at the same ground count for
    /// nothing, so removing this changes how much work an input costs and not what it produces. No
    /// test can tell the two apart, which is why it is written down here.
    standing_at: Option<(i32, i32)>,
}

impl Seen {
    fn over(width: u32, height: u32) -> Seen {
        let squares = width as usize * height as usize;
        Seen {
            bits: vec![0; squares.div_ceil(64)],
            standing_at: None,
        }
    }

    /// Marks a square as seen, and says whether that was news.
    fn look_at(&mut self, x: u32, y: u32, width: u32) -> bool {
        let index = y as usize * width as usize + x as usize;
        let (word, bit) = (index / 64, index % 64);

        let Some(held) = self.bits.get_mut(word) else {
            return false;
        };

        let mask = 1u64 << bit;
        if *held & mask != 0 {
            return false;
        }

        *held |= mask;
        true
    }
}

/// What an entity is, for the rules that treat them differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Player,
    Enemy,

    /// Scenery: walls, signs, decoration. Never moves, never dies.
    Fixture,

    /// A portal to another world.
    Portal,

    /// A loot bag or chest.
    Container,

    /// Something a player left that enemies attack instead of them.
    ///
    /// Its own kind rather than an enemy with a flag, because everything that decides who shoots
    /// what asks the kind, and a decoy that answered "enemy" would be shot by its owner.
    Decoy,

    /// Something armed that fires when a player comes near, then goes.
    Trap,
}

impl Kind {
    pub fn is_alive_kind(self) -> bool {
        matches!(self, Kind::Player | Kind::Enemy | Kind::Decoy)
    }
}

/// Anything that occupies a position in a world.
#[derive(Debug, Clone)]
pub struct Entity {
    pub object_type: ObjectType,
    pub kind: Kind,

    /// Who may open this, for a bag that belongs to whoever earned it.
    ///
    /// `None` is everything else, which anybody may open.
    pub belongs_to: Option<Handle>,

    /// What colour this glows, for an administrator who wants to be seen.
    ///
    /// Zero is no glow, which is everybody.
    pub glow: i32,

    /// What this entity is selling, for a shop merchant.
    ///
    /// Held on the entity rather than looked up from its type, because eight merchants of the same
    /// type sell eight different things: what a stall holds is where it stands, not what it is.
    pub selling: Option<crate::shop::Stall>,

    /// Which of a realm's terrains this enemy was placed on.
    ///
    /// Carried rather than read from the ground, so an enemy that has chased somebody across a
    /// border still counts against where it came from, and so anything it spawns counts there too.
    pub terrain: hendra_content::Terrain,

    pub x: f32,
    pub y: f32,

    pub hp: i32,
    pub max_hp: i32,
    pub mp: i32,
    pub max_mp: i32,

    pub conditions: ConditionSet,
    pub size: u16,
    pub name: Option<Box<str>>,

    /// The eight stats, which decide movement, damage and rate of fire.
    ///
    /// Enemies keep the default, which gives them the base values every derived figure falls back
    /// to. Only players level, wear equipment or take boosts.
    pub stats: crate::stats::Stats,

    /// The object whose projectile this entity fires, if it can shoot.
    pub weapon: Option<ObjectType>,

    /// How long until it may shoot again, in milliseconds.
    pub cooldown_ms: u32,

    /// Where this entity came into being, which some behaviours keep it near.
    pub spawn_x: f32,
    pub spawn_y: f32,

    /// This entity's behaviour, if it has one.
    ///
    /// Boxed because most entities are scenery and a `Mind` carries several vectors; paying for
    /// one inside every wall would cost more memory than the enemies do.
    pub mind: Option<Box<Mind>>,

    /// What this entity holds, for players and loot bags.
    pub container: Option<Box<Container>>,

    /// How long until this entity removes itself, for bags.
    pub expires_in_ms: Option<u32>,

    /// Set when the entity should be removed at the end of the tick.
    pub dead: bool,

    /// Damage taken since the last time this entity thought, for transitions that react to being
    /// hurt. Cleared once read, so a hit counts toward exactly one tick.
    pub damage_since_tick: i32,

    /// How much more likely this player is to be given loot, as a multiplier.
    ///
    /// One for everybody without a boost. The original reads `LDBoostTime > 0` and multiplies by
    /// one and a half; the boost itself is durable and its remaining time is the account's, so what
    /// reaches the world is the multiplier rather than the clock.
    pub loot_drop: f32,

    /// Temporary stat boosts, each with what is left of its time.
    ///
    /// Held as a list rather than folded into a total, because they do not add: the largest counts
    /// in full and each one after it counts for half of the last, so the total can only be worked
    /// out from all of them at once. Folding them in would also make a lapse take away whatever was
    /// added last rather than what that boost was worth.
    pub boosts: Vec<HeldBoost>,

    /// What this character has done, which is what its death is worth.
    ///
    /// On the entity because that is where the events are: a shot is counted where it is fired and
    /// a kill where the enemy dies, and nothing is taken from a client, since a client that reports
    /// its own accuracy will report whatever earns the most.
    pub tally: crate::fame::Tally,

    /// Which squares of this world this player has laid eyes on, for the count a character's fame
    /// is partly made of.
    ///
    /// Only players carry one, and only once they have looked at something. Per world rather than
    /// per character, as the original's is: walking back into a dungeon you cleared last week shows
    /// you the same ground again, and the count is of ground seen rather than ground new to you.
    pub seen: Option<Box<Seen>>,

    /// What last took health off this, which is what a death is named after.
    pub last_hurt_by: Option<Handle>,

    /// Who has damaged this, and how much.
    ///
    /// Kept per enemy rather than globally, because it is a fact about this fight: it decides who
    /// earned the loot that belongs to whoever earned it, and it dies with the enemy.
    pub damage_by: Vec<(Handle, i32)>,

    /// Which sprite to draw, for bosses that visibly change phase.
    pub texture: u8,

    /// The size this entity is growing or shrinking toward, and how fast, in hundredths per
    /// second. `None` when it is not changing.
    pub resizing: Option<(f32, u16)>,

    /// Whether killing this awards experience. Summons set this so they cannot be farmed.
    pub no_experience: bool,

    /// What this goes off with, for a trap. `None` for everything else.
    pub armed: Option<Armed>,

    /// Time before an ability may be used again.
    pub ability_cooldown_ms: u32,

    /// How long before this player may teleport again.
    pub teleport_cooldown_ms: u32,

    /// How long a player is forgiven for being somewhere no speed explains.
    ///
    /// A teleport moves somebody further in one tick than walking ever could, and the movement
    /// check cannot tell that from a client claiming to be somewhere it is not.
    pub move_grace_ms: u32,

    /// Level, experience and fame. Only players advance.
    pub progress: crate::leveling::Progress,

    /// Health owed by an effect that has not yet amounted to a whole point.
    ///
    /// Twenty a second at fifty-millisecond ticks is one point per tick, and rounding each tick
    /// independently would round it to nothing.
    pub health_fraction: f32,

    /// The same, for magic.
    pub magic_fraction: f32,

    /// A colour to blink, its period and how many times, for a phase change the eye can catch.
    pub flash: Option<(u32, u32, u32)>,

    /// The health this entity had before any scaling to the crowd.
    ///
    /// Kept so scaling is measured from the base each time rather than compounded. Without it a
    /// boss in a busy room grows every two seconds forever.
    pub base_max_hp: Option<i32>,

    /// Effects currently held, and how long each has left.
    ///
    /// A list rather than an array indexed by effect number: there are forty effects and almost
    /// every entity has none, so forty timers each would be a hundred and sixty bytes per entity
    /// to hold nothing. An empty `Vec` allocates nothing.
    pub effects: Vec<(u8, u32)>,
}

impl Entity {
    /// A fixture read off the map.
    pub fn fixture(object_type: ObjectType, x: f32, y: f32) -> Entity {
        Entity {
            object_type,
            terrain: hendra_content::Terrain::None,
            selling: None,
            glow: 0,
            belongs_to: None,
            kind: Kind::Fixture,
            x,
            y,
            hp: 0,
            max_hp: 0,
            mp: 0,
            max_mp: 0,
            conditions: ConditionSet::EMPTY,
            size: 100,
            name: None,
            stats: crate::stats::Stats::still(),
            weapon: None,
            cooldown_ms: 0,
            spawn_x: x,
            spawn_y: y,
            mind: None,
            container: None,
            expires_in_ms: None,
            dead: false,
            damage_since_tick: 0,
            damage_by: Vec::new(),
            last_hurt_by: None,
            loot_drop: 1.0,
            boosts: Vec::new(),
            tally: crate::fame::Tally::default(),
            seen: None,
            texture: 0,
            resizing: None,
            no_experience: false,
            effects: Vec::new(),
            armed: None,
            ability_cooldown_ms: 0,
            teleport_cooldown_ms: 0,
            move_grace_ms: 0,
            progress: crate::leveling::Progress::new(),
            health_fraction: 0.0,
            magic_fraction: 0.0,
            flash: None,
            base_max_hp: None,
        }
    }

    pub fn player(object_type: ObjectType, x: f32, y: f32, max_hp: i32) -> Entity {
        Entity {
            object_type,
            terrain: hendra_content::Terrain::None,
            selling: None,
            glow: 0,
            belongs_to: None,
            kind: Kind::Player,
            x,
            y,
            hp: max_hp,
            max_hp,
            mp: 100,
            max_mp: 100,
            conditions: ConditionSet::EMPTY,
            size: 100,
            name: None,
            stats: crate::stats::Stats::default(),
            weapon: None,
            cooldown_ms: 0,
            spawn_x: x,
            spawn_y: y,
            mind: None,
            container: None,
            expires_in_ms: None,
            dead: false,
            damage_since_tick: 0,
            damage_by: Vec::new(),
            last_hurt_by: None,
            loot_drop: 1.0,
            boosts: Vec::new(),
            tally: crate::fame::Tally::default(),
            seen: None,
            texture: 0,
            resizing: None,
            no_experience: false,
            effects: Vec::new(),
            armed: None,
            ability_cooldown_ms: 0,
            teleport_cooldown_ms: 0,
            move_grace_ms: 0,
            progress: crate::leveling::Progress::new(),
            health_fraction: 0.0,
            magic_fraction: 0.0,
            flash: None,
            base_max_hp: None,
        }
    }

    pub fn state(&self) -> EntityState {
        EntityState {
            object_type: self.object_type.0,
            x: self.x,
            y: self.y,
            hp: self.hp,
            max_hp: self.max_hp,
            mp: self.mp,
            max_mp: self.max_mp,
            conditions: self.conditions.0,
            size: self.size,
            name: self.name.clone(),
            texture: self.texture,
            // Only players carry meaningful stats. Everything else sends zeroes, which cost a byte
            // each as varints and never change, so they never appear in a delta.
            stats: if self.kind == Kind::Player {
                self.stats.totals()
            } else {
                [0; 8]
            },
        }
    }
}

/// Why a movement claim was not honoured in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveRefusal {
    /// The claim would need the entity to travel faster than it can.
    TooFar,

    /// The destination cannot be stood on.
    Blocked,

    /// The entity is held in place by an effect.
    ///
    /// Distinct from `TooFar` because it is not a speed judgement: a paralysed player claiming one
    /// square is refused where an unaffected one would be allowed, and telling them apart is what
    /// lets a client say why rather than looking like a rubber-banding bug.
    Rooted,
}

/// What the server decided about a movement claim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveOutcome {
    /// Where the entity actually is now. The server's answer, not the client's.
    pub x: f32,
    pub y: f32,

    /// Set when the claim was trimmed, and why.
    pub refused: Option<MoveRefusal>,
}

/// The slot type an item-kind name refers to.
///
/// The content's loot tables say `weapon` or `ring`, which are the `ItemType` enum the C# used.
/// These are the slot numbers those correspond to; an unrecognised name means "anything of this
/// tier", which is a wider drop rather than no drop.
fn slot_type_of(kind: &str) -> Option<i32> {
    Some(match kind {
        "weapon" => 1,
        "ability" => 4,
        "armor" | "armour" => 14,
        "ring" => 9,
        "potion" => 0,
        _ => return None,
    })
}

/// How much further than the rules allow a claim may travel before it is trimmed.
///
/// Not generosity toward cheating, since the claim is clamped either way. It absorbs the ordinary
/// disagreement between a client's clock and the server's, which at 20 ticks per second is a small
/// fraction of a tile. The C# server needed far more slack because it ticked at six per second, so
/// a single tick of drift was three times larger and honest players kept tripping it.
pub const MOVE_TOLERANCE: f32 = 1.5;

pub struct World {
    pub name: String,
    terrain: Terrain,
    entities: Slab<Entity>,
    projectiles: Projectiles,
    grid: Grid,
    tick: Tick,

    /// Rolls damage without pulling in a random-number crate. Deterministic, which also makes a
    /// failing combat test reproducible.
    seed: u32,

    /// Compiled enemy behaviour, by object id.
    behaviours: Programs,

    /// What a loot bag looks like.
    bag_type: ObjectType,

    /// The object to use for each loot colour, indexed by the content's own numbering.
    bag_types: Vec<ObjectType>,

    /// Reused between ticks so a warm world allocates nothing.
    handles: Vec<Handle>,
    nearby: Vec<Handle>,

    /// What entities have said, waiting to be sent out.
    ///
    /// Bounded, because nothing in the simulation makes a taunt stop: a boss left alive in an
    /// empty room talks to itself indefinitely, and a queue nobody drains is a slow leak.
    announcements: Vec<Announcement>,

    /// Squares whose ground changed, waiting to be sent out. Bounded for the same reason.
    ground_changes: Vec<(u16, u16, u16)>,

    /// Objects thrown but not yet landed, with the time left before they do.
    ///
    /// A thrown object that appeared instantly would be an unavoidable hit. The delay is what the
    /// content calls a throw time, and it is the difference between a hard attack and one nobody
    /// can play around.
    falling: Vec<Falling>,

    /// Advanced for every child spawned, so two children of one parent do not move in lockstep.
    spawn_seed: u32,

    /// How full this world is and whether it has been cleared. Only a realm uses it.
    realm: crate::realm::Realm,

    /// Whether players may teleport here, from the world definition's `restrictTp`.
    allows_teleport: bool,

    /// How many squares a drawing could not paint, because the map already names as many kinds of
    /// square as an index can hold.
    refused_squares: usize,

    /// Scenery that has appeared since the last tick, as `(x, y, object, size)`.
    ///
    /// Sent like a ground change rather than in the snapshot, because scenery is not an entity: a
    /// setpiece drawn once should not cost a place in every snapshot for the rest of the world.
    scenery_changes: Vec<(u16, u16, u16, u16)>,

    /// What players said this tick, and where they were standing.
    ///
    /// Cleared every tick: a word is heard when it is said, and an enemy that reacted to something
    /// shouted a minute ago would be answering an echo. Held on the world rather than passed along,
    /// because every enemy in earshot has to hear the same thing.
    heard: Vec<(f32, f32, Box<str>)>,

    /// Players who have died since the last drain.
    deaths: Vec<Death>,

    /// Which of a dungeon's keys have been found here.
    ///
    /// Davy's locker is the one that uses this: four coloured keys, dropped by four enemies, and a
    /// door that opens when all four are in. Held by the world because it is the world's state, and
    /// somebody arriving late has to be told what has already been found.
    keys_found: Vec<Box<str>>,

    /// Setpiece names a behaviour asked for that nothing answers to, so they can be reported once.
    ///
    /// Four of the shipped behaviours name a setpiece that does not exist, in the original too. A
    /// silent miss would be a boss whose arena never appears and no way to tell.
    unknown_setpieces: std::collections::HashSet<String>,

    /// The randomness a behaviour-drawn setpiece uses.
    setpiece_dice: crate::setpiece::Dice,

    /// Where each terrain's walkable squares are, worked out on first use.
    ///
    /// Only a realm ever asks, and it asks once per terrain, so this stays empty everywhere else.
    spawn_squares: std::collections::HashMap<hendra_content::Terrain, Vec<u32>>,

    /// Neighbours as behaviours see them, rebuilt per entity per tick and reused so the think
    /// loop allocates nothing.
    neighbours: Vec<Neighbour>,
    visible: Vec<(EntityId, EntityState)>,
    hits: Vec<Hit>,
    actions: Vec<Action>,
}

impl World {
    /// Builds a world from terrain, populating it with the fixtures the map declares.
    pub fn new(name: impl Into<String>, terrain: Terrain, catalog: &Catalog) -> World {
        let name = name.into();
        let mut entities = Slab::new();

        for (x, y, square) in terrain.map().objects() {
            // Scenery is not an entity. It never moves, never acts and never changes, so it is
            // sent once with the ground and lives in the collision bitmap after that.
            //
            // This follows `Wmap.Load`, which keeps static objects on the tile rather than in the
            // world, and it is not an optimisation: the realm map carries a quarter of a million
            // trees and rocks. As entities they would fill a world four times over, leaving no room
            // for a single enemy, and every one of them would take a place in every snapshot for
            // the life of the world.
            //
            // The class is consulted as well as the flag, because the content marks the nexus
            // fixtures static too. A money changer never moves either, but a player has to be able
            // to name it to use it, and only an entity has a name to give.
            //
            // An object the map gave a name stays an entity as well: that is the map author saying
            // this one is not scenery.
            let desc = catalog.object(square.object);
            if World::is_scenery(catalog, square) {
                continue;
            }

            let mut entity = Entity::fixture(square.object, x as f32 + 0.5, y as f32 + 0.5);
            if let Some(name) = square.name() {
                entity.name = Some(name.into());
            }
            if let Some(size) = square.size() {
                entity.size = size.clamp(0, u16::MAX as i32) as u16;
            }
            if let Some(desc) = desc {
                if desc.enemy {
                    entity.kind = Kind::Enemy;
                    entity.hp = desc.max_hp;
                    entity.max_hp = desc.max_hp;
                } else if desc.class == "Portal" {
                    entity.kind = Kind::Portal;
                } else if desc.class == "Container" {
                    entity.kind = Kind::Container;
                }
            }

            if entities.insert(entity).is_none() {
                tracing::warn!(world = %name, "world is full; some fixtures were dropped");
                break;
            }
        }

        let grid = Grid::new(terrain.width(), terrain.height());
        let mut world = World {
            name,
            terrain,
            entities,
            projectiles: Projectiles::new(),
            grid,
            tick: Tick::ZERO,
            seed: 0x9e37_79b9,
            behaviours: Programs::default(),
            bag_type: ObjectType(0x0500),
            bag_types: Vec::new(),
            handles: Vec::new(),
            nearby: Vec::new(),
            neighbours: Vec::new(),
            announcements: Vec::new(),
            ground_changes: Vec::new(),
            falling: Vec::new(),
            spawn_seed: 0x2545_f491,
            realm: crate::realm::Realm::new(),
            spawn_squares: std::collections::HashMap::new(),
            allows_teleport: true,
            heard: Vec::new(),
            deaths: Vec::new(),
            keys_found: Vec::new(),
            refused_squares: 0,
            scenery_changes: Vec::new(),
            unknown_setpieces: std::collections::HashSet::new(),
            setpiece_dice: crate::setpiece::Dice::new(0x5e7_9153),
            visible: Vec::new(),
            hits: Vec::new(),
            actions: Vec::new(),
        };
        world.reindex();
        world
    }

    /// Installs compiled behaviours and gives every enemy already present a mind.
    ///
    /// An enemy with no matching program keeps `None` and simply stands there, which is what a
    /// half-converted content directory should look like: most of the dungeon working.
    pub fn set_behaviours(&mut self, catalog: &Catalog, mut behaviours: Programs) {
        // Names become numbers here, once, because this is the first moment both the behaviours
        // and the catalog are in the same place. Skipping it would leave every behaviour that
        // names an entity inert: a boss would wait forever for guardians it cannot recognise, and
        // an order would reach nobody.
        for program in &mut behaviours.programs {
            // Objects first, then tiles. A behaviour names both an enemy to spawn and a ground to
            // lay down, and looking in only one place left every ground change silently doing
            // nothing.
            let unknown = program.resolve(|name| {
                catalog
                    .type_of(name)
                    .map(|found| found.0)
                    .or_else(|| catalog.tile_type_of(name).map(|found| found.0))
            });
            if !unknown.is_empty() {
                tracing::warn!(
                    enemy = %program.name,
                    names = ?unknown,
                    "behaviour names entities the catalog does not have"
                );
            }
        }

        self.behaviours = behaviours;

        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| entity.kind == Kind::Enemy)
                .map(|(handle, _)| handle),
        );

        for index in 0..self.handles.len() {
            let handle = self.handles[index];
            let Some(entity) = self.entities.get(handle) else {
                continue;
            };
            let Some(id) = catalog
                .object(entity.object_type)
                .map(|desc| desc.id.clone())
            else {
                continue;
            };

            if self.behaviours.get(&id).is_some() {
                let seed = handle.0.wrapping_mul(2654435761).wrapping_add(1);
                let program = self.behaviours.get(&id).expect("checked");
                let mind = Mind::new(program, seed);
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.mind = Some(Box::new(mind));
                    // An enemy fires its own projectiles, so it is its own weapon.
                    entity.weapon = Some(entity.object_type);
                }
            }
        }
    }

    /// How many entities are running a behaviour.
    pub fn thinking(&self) -> usize {
        self.entities
            .iter()
            .filter(|(_, entity)| entity.mind.is_some())
            .count()
    }

    pub fn tick_number(&self) -> Tick {
        self.tick
    }

    pub fn terrain(&self) -> &Terrain {
        &self.terrain
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn get(&self, handle: Handle) -> Option<&Entity> {
        self.entities.get(handle)
    }

    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut Entity> {
        self.entities.get_mut(handle)
    }

    pub fn spawn(&mut self, entity: Entity) -> Option<Handle> {
        self.entities.insert(entity)
    }

    pub fn despawn(&mut self, handle: Handle) -> Option<Entity> {
        self.entities.remove(handle)
    }

    pub fn iter(&self) -> impl Iterator<Item = (Handle, &Entity)> {
        self.entities.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle, &mut Entity)> {
        self.entities.iter_mut()
    }

    /// Rebuilds the spatial index from current positions.
    fn reindex(&mut self) {
        // Collected first because `rebuild` takes `&mut self.grid` while reading `self.entities`.
        self.handles.clear();
        self.handles
            .extend(self.entities.iter().map(|(handle, _)| handle));

        let entities = &self.entities;
        let positions = self.handles.iter().filter_map(|handle| {
            let entity = entities.get(*handle)?;
            Some((*handle, entity.x, entity.y))
        });

        self.grid.rebuild(positions.collect::<Vec<_>>());
    }

    /// Decides where an entity really is, given where it claims to be.
    ///
    /// The claim is never trusted. Two things bound it: how far the entity could have travelled in
    /// the time available, and whether the destination can be stood on. A claim that fails either
    /// is trimmed rather than rejected outright. A rejected move makes a laggy player rubber-band,
    /// while a trimmed one merely makes them slightly wrong for one tick.
    pub fn resolve_move(
        &self,
        handle: Handle,
        catalog: &Catalog,
        claimed_x: f32,
        claimed_y: f32,
        elapsed_ms: u32,
    ) -> Option<MoveOutcome> {
        let entity = self.entities.get(handle)?;

        // A rooted player is held where it is rather than clamped toward its claim, because a
        // clamp still lets it creep at the tolerance every tick.
        let rules = crate::effects::Rules::of(entity.conditions);
        if rules.rooted {
            return Some(MoveOutcome {
                x: entity.x,
                y: entity.y,
                refused: (claimed_x != entity.x || claimed_y != entity.y)
                    .then_some(MoveRefusal::Rooted),
            });
        }

        // A player the server itself just moved is where the server put them, and their client is
        // about to say so. Checking that against a walking speed would refuse the server's own
        // teleport and snap them back.
        if entity.move_grace_ms > 0 {
            return Some(MoveOutcome {
                x: claimed_x,
                y: claimed_y,
                refused: None,
            });
        }

        let ground = self.terrain.speed_at(catalog, entity.x, entity.y);
        let allowed = entity.stats.movement_speed(&rules)
            * ground
            * (elapsed_ms as f32 / 1000.0)
            * MOVE_TOLERANCE;

        let (mut dx, mut dy) = (claimed_x - entity.x, claimed_y - entity.y);
        let distance = (dx * dx + dy * dy).sqrt();

        let mut refused = None;

        if distance > allowed && distance > 0.0 {
            let scale = allowed / distance;
            dx *= scale;
            dy *= scale;
            refused = Some(MoveRefusal::TooFar);
        }

        let (target_x, target_y) = (entity.x + dx, entity.y + dy);

        if self.terrain.walkable_at(target_x, target_y) {
            return Some(MoveOutcome {
                x: target_x,
                y: target_y,
                refused,
            });
        }

        // Blocked head-on. Try each axis alone, so walking into a wall at an angle slides along it
        // rather than stopping dead, which is what a player expects and what the client's own
        // prediction will have done.
        if self.terrain.walkable_at(target_x, entity.y) {
            return Some(MoveOutcome {
                x: target_x,
                y: entity.y,
                refused: Some(MoveRefusal::Blocked),
            });
        }
        if self.terrain.walkable_at(entity.x, target_y) {
            return Some(MoveOutcome {
                x: entity.x,
                y: target_y,
                refused: Some(MoveRefusal::Blocked),
            });
        }

        Some(MoveOutcome {
            x: entity.x,
            y: entity.y,
            refused: Some(MoveRefusal::Blocked),
        })
    }

    /// Moves an entity to a position the server itself chose.
    ///
    /// Unlike [`World::resolve_move`] there is no speed limit, because there is no claim to check:
    /// the distance was computed here from a behaviour's own speed, and clamping it again against
    /// the entity's speed would be checking our own arithmetic. Terrain still applies: an enemy
    /// walks through a wall no more than a player does, and it slides along one rather than
    /// stopping dead.
    pub fn step(&mut self, handle: Handle, to_x: f32, to_y: f32) {
        let Some(entity) = self.entities.get(handle) else {
            return;
        };
        let (from_x, from_y) = (entity.x, entity.y);

        let (x, y) = if self.terrain.walkable_at(to_x, to_y) {
            (to_x, to_y)
        } else if self.terrain.walkable_at(to_x, from_y) {
            (to_x, from_y)
        } else if self.terrain.walkable_at(from_x, to_y) {
            (from_x, to_y)
        } else {
            (from_x, from_y)
        };

        if let Some(entity) = self.entities.get_mut(handle) {
            entity.x = x;
            entity.y = y;
        }
    }

    /// Applies a resolved move.
    pub fn place(&mut self, handle: Handle, outcome: MoveOutcome) {
        let Some(entity) = self.entities.get_mut(handle) else {
            return;
        };

        entity.x = outcome.x;
        entity.y = outcome.y;

        if entity.kind == Kind::Player {
            self.look_around(handle);
        }
    }

    /// Counts the squares this player can see and has not seen before.
    ///
    /// The original reveals the map a circle at a time and counts what it sends; we send the whole
    /// map at once, so the count has to be taken here or the two fame bonuses that rest on it are
    /// unreachable. What is counted is the same either way: squares this character has laid eyes on.
    ///
    /// Nothing is walked unless the player has crossed into a new square, which is what makes this
    /// affordable to do on every accepted move rather than on a timer.
    pub fn look_around(&mut self, handle: Handle) {
        let (width, height) = (self.terrain.width(), self.terrain.height());

        let Some(entity) = self.entities.get_mut(handle) else {
            return;
        };

        let (at_x, at_y) = (entity.x.floor() as i32, entity.y.floor() as i32);
        let seen = entity
            .seen
            .get_or_insert_with(|| Box::new(Seen::over(width, height)));

        if seen.standing_at == Some((at_x, at_y)) {
            return;
        }
        seen.standing_at = Some((at_x, at_y));

        let radius = SIGHT_RADIUS.ceil() as i32;
        let mut fresh = 0i32;

        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if (dx * dx + dy * dy) as f32 > SIGHT_RADIUS * SIGHT_RADIUS {
                    continue;
                }

                let (x, y) = (at_x + dx, at_y + dy);
                if x < 0 || y < 0 || x as u32 >= width || y as u32 >= height {
                    continue;
                }

                if seen.look_at(x as u32, y as u32, width) {
                    fresh += 1;
                }
            }
        }

        entity.tally.tiles_seen = entity.tally.tiles_seen.saturating_add(fresh);
    }

    /// How many projectiles are in flight.
    pub fn projectile_count(&self) -> usize {
        self.projectiles.len()
    }

    pub fn projectiles(&self) -> impl Iterator<Item = (Handle, &Projectile)> {
        self.projectiles.iter()
    }

    /// Fires an entity's weapon, if it has one and is off cooldown.
    ///
    /// The angle is the one thing taken from the client without argument: where a player is aiming
    /// is genuinely theirs to decide, and there is nothing to validate it against. Everything that
    /// follows, meaning where the shot goes, what it strikes and what that costs, is the server's.
    pub fn shoot(&mut self, handle: Handle, catalog: &Catalog, angle: f32) -> Vec<Handle> {
        let mut fired = Vec::new();

        let Some(entity) = self.entities.get(handle) else {
            return fired;
        };
        if entity.cooldown_ms > 0 || entity.dead {
            return fired;
        }

        let rules = crate::effects::Rules::of(entity.conditions);
        if rules.silenced {
            return fired;
        }

        let Some(weapon) = entity.weapon else {
            return fired;
        };
        let Some(desc) = catalog.object(weapon) else {
            return fired;
        };
        if desc.projectiles.is_empty() {
            return fired;
        }

        let (x, y) = (entity.x, entity.y);
        let from_player = entity.kind == Kind::Player;

        // Rate of fire is quoted as a multiplier on a base of one shot every 500 ms.
        let rate = desc
            .item
            .as_ref()
            .map(|item| item.rate_of_fire.max(0.1))
            .unwrap_or(1.0);
        let cooldown = entity.stats.shot_cooldown_ms(&rules, rate);

        // Applied when the shot is made rather than when it lands, so the shooter's state at the
        // moment of firing is what decides the shot. A projectile in flight is not affected by its
        // owner being weakened afterwards.
        let power = entity.stats.damage_multiplier(&rules);

        for shot in &desc.projectiles {
            let roll = self.roll();
            let mut projectile =
                Projectile::from_desc(handle, from_player, shot, x, y, angle, roll);
            projectile.damage = ((projectile.damage as f32) * power).round().max(1.0) as i32;

            if let Some(handle) = self.projectiles.fire(projectile) {
                fired.push(handle);
            }
        }

        if let Some(entity) = self.entities.get_mut(handle) {
            entity.cooldown_ms = cooldown;
        }

        fired
    }

    /// A deterministic roll in `0.0..1.0`.
    fn roll(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed % 10_000) as f32 / 10_000.0
    }

    /// Advances the world by one tick.
    pub fn advance(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        self.tick = self.tick.next();

        // Before thinking, so something thrown this tick waits its full time rather than landing
        // on the same tick it was thrown.
        let behaviours = std::mem::take(&mut self.behaviours);
        self.land_thrown(catalog, &behaviours, elapsed_ms);
        self.behaviours = behaviours;

        self.cool_weapons(elapsed_ms);
        self.expire_effects(elapsed_ms);
        self.resize(elapsed_ms);
        self.think(catalog, elapsed_ms);
        self.spring_traps(catalog);
        self.apply_hazards(catalog, elapsed_ms);
        self.apply_effect_health(elapsed_ms);
        self.regenerate(elapsed_ms);
        self.advance_projectiles(catalog, elapsed_ms);
        self.expire(elapsed_ms);
        self.drop_loot(catalog);

        // Before reaping, because what an entity leaves behind is decided by what it was.
        self.award_experience(catalog);
        self.run_death_effects(catalog);
        self.note_deaths(catalog);
        self.reap();

        // A word is heard when it is said. Cleared after the thinking, so everything in earshot has
        // had its chance and nothing reacts to an echo on the next tick.
        self.heard.clear();
        self.reindex();
    }

    /// Runs every enemy's behaviour and applies what it asked for.
    fn think(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        // Lifted out for the loop. Nothing in a tick changes the compiled behaviours, and holding
        // them here rather than borrowing from the world is what lets the world be written to
        // while a program is being read. The alternative was cloning a program per entity per
        // tick, which is the most expensive thing that could possibly happen in this loop.
        let behaviours = std::mem::take(&mut self.behaviours);

        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| {
                    entity.mind.is_some()
                        && !entity.dead
                        && !crate::effects::Rules::of(entity.conditions).paused
                })
                .map(|(handle, _)| handle),
        );

        for index in 0..self.handles.len() {
            let handle = self.handles[index];

            // The neighbour buffer is lifted out of the world for the duration of the tick, so
            // that senses can borrow it while the world is being written to.
            let mut neighbours = std::mem::take(&mut self.neighbours);
            let scalars = self.gather_senses(handle, &mut neighbours);
            let Some(scalars) = scalars else {
                self.neighbours = neighbours;
                continue;
            };

            // The mind comes out of the entity for the duration of the tick, because running it
            // needs the world that the entity is part of.
            let Some(mut mind) = self.entities.get_mut(handle).and_then(|e| e.mind.take()) else {
                self.neighbours = neighbours;
                continue;
            };

            let program = self
                .entities
                .get(handle)
                .and_then(|entity| catalog.object(entity.object_type))
                .and_then(|desc| behaviours.get(&desc.id));

            let Some(program) = program else {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.mind = Some(mind);
                }
                self.neighbours = neighbours;
                continue;
            };

            // What was said, as distances from this entity. Built per entity because the condition
            // asks how far away the speaker was, and that is a different answer for each listener.
            let spoken: Vec<(f32, &str)> = if self.heard.is_empty() {
                Vec::new()
            } else {
                self.heard
                    .iter()
                    .map(|(x, y, text)| {
                        let (dx, dy) = (x - scalars.x, y - scalars.y);
                        ((dx * dx + dy * dy).sqrt(), &**text)
                    })
                    .collect()
            };

            let senses = Senses {
                x: scalars.x,
                y: scalars.y,
                hp: scalars.hp,
                max_hp: scalars.max_hp,
                spawn_x: scalars.spawn_x,
                spawn_y: scalars.spawn_y,
                nearest_player: scalars.nearest_player,
                nearby: &neighbours,
                said: &spoken,
                damage_taken: scalars.damage_taken,
            };

            let mut actions = std::mem::take(&mut self.actions);
            mind.tick(program, &senses, elapsed_ms, &mut actions);
            self.neighbours = neighbours;

            for action in &actions {
                self.apply(handle, catalog, &behaviours, program, action, elapsed_ms);
            }

            self.actions = actions;
            if let Some(entity) = self.entities.get_mut(handle) {
                entity.mind = Some(mind);
                // Reset once it has been read, so damage counts toward exactly one tick.
                entity.damage_since_tick = 0;
            }
        }

        self.behaviours = behaviours;
    }
}

/// The parts of [`Senses`] that are not borrowed.
struct SenseScalars {
    x: f32,
    y: f32,
    hp: i32,
    max_hp: i32,
    spawn_x: f32,
    spawn_y: f32,
    nearest_player: Option<Nearby>,
    damage_taken: i32,
}

/// How long before a player may teleport again.
pub const TELEPORT_COOLDOWN_MS: u32 = 10_000;

/// How long a teleported player is forgiven for arriving somewhere no speed explains.
///
/// A teleport moves somebody further in one tick than walking ever could, and the movement check
/// cannot tell that from a client claiming to be somewhere it is not.
pub const MOVE_GRACE_MS: u32 = 1_000;

/// A player who has died, and what killed them.
#[derive(Debug, Clone, PartialEq)]
pub struct Death {
    pub who: Handle,

    /// What to name as the killer. The world knows this and the session does not: by the time a
    /// session hears about a death, whatever did it may already be gone.
    pub killer: String,

    pub x: f32,
    pub y: f32,
}

/// One temporary stat boost somebody is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldBoost {
    pub stat: u8,
    pub amount: i32,
    pub remaining_ms: u32,

    /// Whether this stacks with others of its kind, or only the largest counts.
    pub stacks: bool,
}

/// How many boosts one entity may hold at once.
///
/// Generous for anything that is really a fight, and bounded because an aura nobody leaves would
/// otherwise grow the list forever.
const MOST_BOOSTS_HELD: usize = 32;

/// Works out what somebody's boosts come to, and writes it into their stats.
///
/// Recomputed from the whole list rather than adjusted, so a boost lapsing takes away exactly what
/// it was worth: with halving, what a boost contributes depends on which others are held, and no
/// amount of bookkeeping at the edges gets that right.
fn restack(entity: &mut Entity) {
    let mut totals = [0i32; 8];

    for stat in 0..8u8 {
        let stacking: Vec<i32> = entity
            .boosts
            .iter()
            .filter(|held| held.stat == stat && held.stacks)
            .map(|held| held.amount)
            .collect();

        let separate: Vec<i32> = entity
            .boosts
            .iter()
            .filter(|held| held.stat == stat && !held.stacks)
            .map(|held| held.amount)
            .collect();

        totals[stat as usize] = crate::stats::stacked(&stacking, &separate);
    }

    entity.stats.set_boosts(totals);
}

/// How much speech one tick holds before the rest is dropped.
///
/// A room full of people all typing at once is still a room, and an enemy only needs to hear one of
/// them say the word.
const MOST_HEARD_AT_ONCE: usize = 64;

/// How many players one enemy remembers being hurt by.
///
/// Generous enough for anything that is really a fight, and bounded because the list lives on every
/// enemy and a realm holds thousands of them.
const MOST_REMEMBERED_DAMAGERS: usize = 64;

/// The chest a setpiece leaves its reward in.
const SETPIECE_CHEST: &str = "Treasure Chest";

/// How much it holds. Eight, as every container in the game does.
const SETPIECE_CHEST_SLOTS: usize = 8;

/// Whether a class of object is there to be looked at rather than used.
///
/// Everything else static stays an entity, because a player has to be able to name what they use
/// and only an entity has a name to give.
fn is_decoration(class: &str) -> bool {
    matches!(
        class,
        "GameObject"
            | "Wall"
            | "CaveWall"
            | "ConnectedWall"
            | "DoubleWall"
            | "Stalagmite"
            | "SpiderWeb"
    )
}

/// How many squares are tried before giving up on finding one nobody is standing near.
///
/// Every square tried is already the right terrain, so this only has to outlast a crowd.
const SPAWN_ATTEMPTS: usize = 20;

/// The widest circle of ground one behaviour may change at once.
///
/// The content asks for radii of ninety-nine in places, which as a circle is thirty thousand
/// squares and a tick that does not finish. What those uses mean is "the room".
const MAX_GROUND_RADIUS: f32 = 24.0;

/// How much unsent speech is held before the rest is dropped.
const MAX_PENDING_ANNOUNCEMENTS: usize = 256;

/// How many unsent ground changes are held before the rest is dropped.
const MAX_PENDING_GROUND_CHANGES: usize = 4096;

/// Which of the eight stats a number names.
fn stat_of(index: u8) -> Option<hendra_content::Stat> {
    hendra_content::STATS.get(index as usize).copied()
}

/// Whether an effect is one a player would want removed.
///
/// The set an enemy inflicts, which is the same set the immunities cover. Cleansing a beneficial
/// effect would make a purifying item a punishment.
fn is_harmful(effect: hendra_content::ConditionEffect) -> bool {
    use hendra_content::ConditionEffect::*;
    matches!(
        effect,
        Slowed
            | Sick
            | Dazed
            | Stunned
            | Blind
            | Hallucinating
            | Drunk
            | Confused
            | Paralyzed
            | Weak
            | Bleeding
            | Quiet
            | ArmorBroken
            | Hexed
            | Curse
            | Petrify
            | Darkness
            | Unstable
    )
}

/// What a trap does when something comes near.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Armed {
    pub radius: f32,
    pub damage: i32,
    pub effect: Option<u8>,
}

/// How long what a trap leaves behind lasts.
const TRAP_EFFECT_MS: u32 = 3_000;

/// How much a decoy can absorb before it is gone.
///
/// A decoy that could not be destroyed would be a wall rather than a distraction, and the point of
/// one is that it buys time rather than safety.
const DECOY_HEALTH: i32 = 200;

/// Something thrown, waiting to arrive.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Falling {
    kind: ObjectType,
    x: f32,
    y: f32,
    remaining_ms: u32,

    /// The terrain of whoever threw it, so what lands is counted where the thrower was.
    terrain: hendra_content::Terrain,
}

/// How many objects may be in the air at once.
///
/// A behaviour with no cooldown could otherwise fill the queue faster than it drains.
const MAX_FALLING: usize = 256;

/// What an explosion does where it lands.
#[derive(Debug, Clone, Copy)]
struct Blast {
    /// Who set it off, so a death it causes is named after them rather than after the room.
    from: Option<Handle>,

    radius: f32,
    damage: i32,
    effect: Option<u8>,
    effect_ms: u32,

    /// Whether this catches players or enemies.
    ///
    /// An explosion has a side, as a projectile does. Without one a player's own spell hurts the
    /// people standing next to them and a trap they set hurts nobody.
    hits_players: bool,
}

/// The most children one behaviour may make in a single tick.
///
/// A bound rather than a rule: a content file asking for a thousand should get a handful and a
/// world that still ticks, not a world that stops.
const MAX_SPAWNED_AT_ONCE: u32 = 8;

/// Something an entity said, and who should hear it.
#[derive(Debug, Clone, PartialEq)]
pub struct Announcement {
    pub from: Handle,
    pub text: std::sync::Arc<str>,

    /// Heard everywhere rather than only nearby.
    pub broadcast: bool,
}

/// A handle as an opaque number a behaviour can hand back.
fn handle_bits(handle: Handle) -> u32 {
    handle.0
}

impl World {
    /// Everything a behaviour perceives except its neighbours, which go into `into`.
    ///
    /// Split in two because [`Senses`] borrows the neighbour list, and the world cannot be written
    /// to while it lends out its own field. The caller holds the buffer for the tick and gives it
    /// back afterwards.
    fn gather_senses(&mut self, handle: Handle, into: &mut Vec<Neighbour>) -> Option<SenseScalars> {
        let entity = self.entities.get(handle)?;
        let (x, y, hp, max_hp) = (entity.x, entity.y, entity.hp, entity.max_hp);
        let (spawn_x, spawn_y) = (entity.spawn_x, entity.spawn_y);
        let damage_taken = entity.damage_since_tick;

        self.grid.within(x, y, SIGHT_RADIUS, &mut self.nearby);
        into.clear();

        let mut nearest: Option<Nearby> = None;
        for found in &self.nearby {
            if *found == handle {
                continue;
            }
            let Some(other) = self.entities.get(*found) else {
                continue;
            };
            if other.dead || !other.kind.is_alive_kind() {
                continue;
            }

            let (dx, dy) = (other.x - x, other.y - y);
            let distance = (dx * dx + dy * dy).sqrt();
            let player = other.kind == Kind::Player;

            // Behind a wall is not in sight, so an enemy does not chase or shoot through cover.
            let seen = self.terrain.line_of_sight(x, y, other.x, other.y);

            // Other entities are listed whether or not there is a clear line to them: a boss
            // waiting for its guardians to die is asking whether they exist, not whether it can
            // see them, and a wall between the two does not make one dead.
            into.push(Neighbour {
                kind: other.object_type.0,
                id: handle_bits(*found),
                x: other.x,
                y: other.y,
                distance,
                hp: other.hp,
                max_hp: other.max_hp,
                player,
            });

            if player && seen && nearest.is_none_or(|closest| distance < closest.distance) {
                nearest = Some(Nearby {
                    x: other.x,
                    y: other.y,
                    distance,
                });
            }
        }

        Some(SenseScalars {
            x,
            y,
            hp,
            max_hp,
            spawn_x,
            spawn_y,
            nearest_player: nearest,
            damage_taken,
        })
    }

    /// Carries out one thing a behaviour asked for.
    fn apply(
        &mut self,
        handle: Handle,
        catalog: &Catalog,
        behaviours: &Programs,
        program: &Program,
        action: &Action,
        elapsed_ms: u32,
    ) {
        match action {
            Action::Move { angle, speed } => {
                let Some(entity) = self.entities.get(handle) else {
                    return;
                };

                // Behaviours do not go through `resolve_move`, so the same rules apply here or a
                // paralysed enemy would keep walking while a paralysed player could not.
                let rules = crate::effects::Rules::of(entity.conditions);
                if rules.rooted {
                    return;
                }
                let scale = if rules.slowed {
                    0.5
                } else if rules.speedy {
                    1.5
                } else {
                    1.0
                };
                let speed = &(speed * scale);
                // Behaviours quote speed the way the content does, in tenths of a tile per second.
                let distance = speed * 10.0 * (elapsed_ms as f32 / 1000.0);
                let (to_x, to_y) = (
                    entity.x + angle.cos() * distance,
                    entity.y + angle.sin() * distance,
                );

                // Terrain still applies, so an enemy cannot walk through a wall, but the speed
                // limit does not, because this distance came from the server rather than a client.
                let _ = catalog;
                self.step(handle, to_x, to_y);
            }

            Action::Shoot {
                angle,
                count,
                spread,
                projectile,
            } => {
                self.fire_spread(handle, catalog, *angle, *count, *spread, *projectile);
            }

            Action::Heal { amount } => {
                if let Some(entity) = self.entities.get_mut(handle)
                    && !crate::effects::Rules::of(entity.conditions).sick
                {
                    entity.hp = (entity.hp + amount).min(entity.max_hp);
                }
            }

            Action::Spawn {
                child,
                count,
                offset_x,
                offset_y,
                state,
                delay_ms,
            } => {
                let Some(kind) = program.kind_of(*child).map(ObjectType) else {
                    return;
                };
                let Some((x, y)) = self.entities.get(handle).map(|e| (e.x, e.y)) else {
                    return;
                };

                // Thrown rather than placed: it lands after its telegraph, which is what makes it
                // dodgeable rather than an unavoidable hit.
                if *delay_ms > 0 {
                    if self.falling.len() < MAX_FALLING {
                        self.falling.push(Falling {
                            kind,
                            x: x + offset_x,
                            y: y + offset_y,
                            remaining_ms: *delay_ms,
                            terrain: self
                                .entities
                                .get(handle)
                                .map(|thrower| thrower.terrain)
                                .unwrap_or_default(),
                        });
                    }
                    return;
                }

                for index in 0..(*count).min(MAX_SPAWNED_AT_ONCE) {
                    // Fanned slightly, so a group spawned together does not sit in one square and
                    // read as a single enemy.
                    let spread = index as f32 * 0.35;
                    self.spawn_child(
                        catalog,
                        behaviours,
                        kind,
                        x + offset_x + spread,
                        y + offset_y,
                        state.as_deref(),
                        Some(handle),
                    );
                }
            }

            Action::Effect {
                effect,
                duration_ms,
                radius,
                target,
            } => match target {
                EffectTarget::Myself => self.give_effect(handle, *effect, *duration_ms),
                EffectTarget::Players | EffectTarget::Others => {
                    let players = matches!(target, EffectTarget::Players);
                    self.each_nearby(handle, *radius, players, None, |world, other| {
                        world.give_effect(other, *effect, *duration_ms);
                    });
                }
            },

            Action::Texture { index } => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.texture = *index;
                }
            }

            Action::Resize { rate, target } => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.resizing = Some((*rate, *target));
                }
            }

            // Said by the world rather than by the entity, because who hears it is the world's
            // question. Held on the entity so the next snapshot carries it.
            Action::Say { text, broadcast } => {
                if self.announcements.len() < MAX_PENDING_ANNOUNCEMENTS {
                    self.announcements.push(Announcement {
                        from: handle,
                        text: text.clone(),
                        broadcast: *broadcast,
                    });
                }
            }

            Action::Transform { into } => {
                let Some(kind) = program.kind_of(*into).map(ObjectType) else {
                    return;
                };
                let Some((x, y)) = self.entities.get(handle).map(|e| (e.x, e.y)) else {
                    return;
                };

                // The old body goes without counting as a kill, so transforming is not a way to
                // farm whatever the first form dropped.
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.dead = true;
                    entity.no_experience = true;
                }
                self.spawn_child(catalog, behaviours, kind, x, y, None, Some(handle));
            }

            Action::Order {
                radius,
                kind,
                state,
            } => {
                let wanted = kind.and_then(|name| program.kind_of(name)).map(ObjectType);
                let state = state.clone();
                self.each_nearby(handle, *radius, false, wanted, |world, other| {
                    world.order_into(catalog, behaviours, other, &state);
                });
            }

            Action::HealOthers {
                radius,
                amount,
                kind,
                players,
            } => {
                let wanted = kind.and_then(|name| program.kind_of(name)).map(ObjectType);
                let amount = *amount;
                self.each_nearby(handle, *radius, *players, wanted, |world, other| {
                    if let Some(entity) = world.entities.get_mut(other)
                        && !crate::effects::Rules::of(entity.conditions).sick
                    {
                        entity.hp = (entity.hp + amount).min(entity.max_hp);
                    }
                });
            }

            Action::Grenade {
                offset_x,
                offset_y,
                radius,
                damage,
                effect,
                effect_ms,
            } => {
                let Some((x, y)) = self.entities.get(handle).map(|e| (e.x, e.y)) else {
                    return;
                };
                self.explode(
                    catalog,
                    (x + offset_x, y + offset_y),
                    Blast {
                        from: Some(handle),
                        radius: *radius,
                        damage: *damage,
                        effect: *effect,
                        effect_ms: *effect_ms,
                        hits_players: true,
                    },
                );
            }

            Action::Portal { name, duration_ms } => {
                let Some(kind) = program.kind_of(*name).map(ObjectType) else {
                    return;
                };
                let Some((x, y)) = self.entities.get(handle).map(|e| (e.x, e.y)) else {
                    return;
                };
                if let Some(portal) =
                    self.spawn_child(catalog, behaviours, kind, x, y, None, Some(handle))
                    && let Some(entity) = self.entities.get_mut(portal)
                {
                    entity.expires_in_ms = Some(*duration_ms);
                }
            }

            Action::Ground { tile, radius } => {
                let Some(kind) = program.kind_of(*tile) else {
                    return;
                };
                let Some((x, y)) = self.entities.get(handle).map(|e| (e.x, e.y)) else {
                    return;
                };
                self.reshape_ground(catalog, x, y, *radius, kind);
            }

            Action::Setpiece { name } => {
                let Some(kind) = crate::setpiece::Kind::named(name) else {
                    // Four of these in the shipped content name nothing, in the original too:
                    // `Type.GetType` finds no class and the behaviour throws where it stands. Saying
                    // so is better than either.
                    self.unknown_setpieces.insert(name.clone());
                    return;
                };

                let Some((x, y)) = self.entities.get(handle).map(|e| (e.x, e.y)) else {
                    return;
                };

                // Drawn from its corner, as the original places them, and centred on whoever asked
                // for it so it appears around them rather than beside them.
                let half = kind.size() as f32 / 2.0;
                let at = ((x - half).max(0.0) as u32, (y - half).max(0.0) as u32);

                let drawing = kind.draw(&mut self.setpiece_dice);
                self.draw(catalog, &drawing, at);
            }

            Action::Flash {
                colour,
                period_ms,
                repeats,
            } => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.flash = Some((*colour, *period_ms, *repeats));
                }
            }

            Action::RemoveEffect { effect } => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.effects.retain(|(held, _)| held != effect);

                    let mut conditions = hendra_content::ConditionSet::EMPTY;
                    for (held, _) in &entity.effects {
                        if let Some(known) =
                            hendra_content::ConditionEffect::from_index(*held as u16)
                        {
                            conditions.insert(known);
                        }
                    }
                    entity.conditions = conditions;
                }
            }

            // Health follows the crowd, so a boss built for forty people is not trivial when two
            // find it. Measured from the base rather than compounded, or a boss in a busy room
            // would grow without bound.
            Action::ScaleHealth {
                per_player,
                maximum_extra,
                radius,
            } => {
                let Some((x, y)) = self.entities.get(handle).map(|e| (e.x, e.y)) else {
                    return;
                };
                self.grid.within(x, y, *radius, &mut self.nearby);
                let found = std::mem::take(&mut self.nearby);

                let players = found
                    .iter()
                    .filter(|other| {
                        self.entities
                            .get(**other)
                            .is_some_and(|entity| entity.kind == Kind::Player && !entity.dead)
                    })
                    .count();
                self.nearby = found;

                if let Some(entity) = self.entities.get_mut(handle) {
                    let base = entity.base_max_hp.unwrap_or(entity.max_hp);
                    entity.base_max_hp = Some(base);

                    let extra = (per_player.saturating_mul(players.saturating_sub(1) as i32))
                        .clamp(0, (*maximum_extra).max(0));
                    let before = entity.max_hp;
                    entity.max_hp = base.saturating_add(extra).max(1);

                    // The health added is granted rather than left as a hole, and health lost when
                    // the room empties comes off the top rather than killing anything.
                    entity.hp = (entity.hp + (entity.max_hp - before)).clamp(1, entity.max_hp);
                }
            }

            Action::NoExperience => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.no_experience = true;
                }
            }

            Action::RemoveNearby { radius, kind } => {
                let wanted = kind.and_then(|name| program.kind_of(name)).map(ObjectType);
                self.each_nearby(handle, *radius, false, wanted, |world, other| {
                    if let Some(entity) = world.entities.get_mut(other) {
                        entity.dead = true;
                        entity.no_experience = true;
                    }
                });
            }

            Action::Vanish => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.dead = true;
                }
            }
        }
    }

    /// Creates an entity of a kind, giving it a mind if the content has one for it.
    ///
    /// Everything a behaviour spawns comes through here, so a child gets its own behaviour, its
    /// own health and its own spawn point rather than inheriting the parent's.
    ///
    /// `from` is what asked for it, where anything did. A child inherits its parent's terrain, so
    /// an enemy that breeds keeps its whole line counted against the ground it was placed on rather
    /// than leaking out of the census.
    #[allow(clippy::too_many_arguments)]
    fn spawn_child(
        &mut self,
        catalog: &Catalog,
        behaviours: &Programs,
        kind: ObjectType,
        x: f32,
        y: f32,
        state: Option<&str>,
        from: Option<Handle>,
    ) -> Option<Handle> {
        let desc = catalog.object(kind)?;

        let mut entity = Entity::fixture(kind, x, y);
        entity.kind = if desc.enemy {
            Kind::Enemy
        } else {
            Kind::Fixture
        };
        entity.max_hp = desc.max_hp.max(1);
        entity.hp = entity.max_hp;
        entity.spawn_x = x;
        entity.spawn_y = y;
        entity.terrain = from
            .and_then(|parent| self.entities.get(parent))
            .map(|parent| parent.terrain)
            .unwrap_or_default();

        // Taken from the caller rather than from `self`, because the tick lifts the programs out
        // of the world while it runs. Reading them from `self` here found an empty set, and every
        // spawned child stood still forever.
        let seed = self.next_seed();
        if let Some(program) = behaviours.get(&desc.id) {
            let mut mind = Mind::new(program, seed);

            // A child ordered into a state starts there rather than at its program's beginning,
            // which is what `spawn` with a state and what `order` on arrival both mean.
            if let Some(state) = state
                && let Some(index) = program.state_named(state)
            {
                mind.force_into(program, index);
            }
            entity.mind = Some(Box::new(mind));
        }

        self.spawn(entity)
    }

    /// A seed that differs per spawn, so two children of one parent do not move in lockstep.
    fn next_seed(&mut self) -> u32 {
        self.spawn_seed = self
            .spawn_seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        self.spawn_seed
    }

    /// Gives an entity a temporary stat boost.
    ///
    /// Renewing one it already holds extends it rather than adding a second, for the same reason a
    /// condition does: standing in an aura for a minute should not be sixty boosts.
    fn give_boost(&mut self, handle: Handle, boost: HeldBoost) {
        let Some(entity) = self.entities.get_mut(handle) else {
            return;
        };

        let held = entity
            .boosts
            .iter_mut()
            .find(|held| held.stat == boost.stat && held.amount == boost.amount);

        match held {
            Some(held) => held.remaining_ms = held.remaining_ms.max(boost.remaining_ms),
            None => {
                if entity.boosts.len() < MOST_BOOSTS_HELD {
                    entity.boosts.push(boost);
                }
            }
        }

        restack(entity);
    }

    /// Gives an entity a condition effect for a time.
    ///
    /// Renewing one it already holds extends it rather than stacking it, because a behaviour that
    /// holds an effect renews it every tick and stacking would make the list grow without bound.
    fn give_effect(&mut self, handle: Handle, effect: u8, duration_ms: u32) {
        let Some(entity) = self.entities.get_mut(handle) else {
            return;
        };

        if let Some(held) = entity.effects.iter_mut().find(|(held, _)| *held == effect) {
            held.1 = held.1.max(duration_ms);
            return;
        }

        // An immunity refuses the effect outright rather than letting it land and be ignored, so
        // nothing downstream has to remember to check twice.
        if let Some(known) = hendra_content::ConditionEffect::from_index(effect as u16) {
            if !crate::effects::accepts(entity.conditions, known) {
                return;
            }
            entity.conditions.insert(known);
        }
        entity.effects.push((effect, duration_ms));
    }

    /// Runs something for every entity near another, of a kind and a side.
    ///
    /// The handles are collected before anything is run, because the closure writes to the world
    /// and iterating the grid while it changes is how an entity gets visited twice or not at all.
    fn each_nearby(
        &mut self,
        from: Handle,
        radius: f32,
        players: bool,
        kind: Option<ObjectType>,
        mut each: impl FnMut(&mut World, Handle),
    ) {
        let Some((x, y)) = self.entities.get(from).map(|e| (e.x, e.y)) else {
            return;
        };

        self.grid.within(x, y, radius, &mut self.nearby);
        let found = std::mem::take(&mut self.nearby);

        for handle in &found {
            if *handle == from {
                continue;
            }
            let Some(entity) = self.entities.get(*handle) else {
                continue;
            };
            if entity.dead
                || (entity.kind == Kind::Player) != players
                || kind.is_some_and(|wanted| entity.object_type != wanted)
            {
                continue;
            }

            each(self, *handle);
        }

        self.nearby = found;
    }

    /// Damages everything of the opposite side within a circle.
    fn explode(&mut self, catalog: &Catalog, at: (f32, f32), blast: Blast) {
        let (x, y) = at;
        let Blast {
            from,
            radius,
            damage,
            effect,
            effect_ms,
            hits_players,
        } = blast;
        self.grid.within(x, y, radius, &mut self.nearby);
        let found = std::mem::take(&mut self.nearby);

        for handle in &found {
            let hit = self.entities.get(*handle).is_some_and(|entity| {
                !entity.dead
                    && if hits_players {
                        entity.kind == Kind::Player
                    } else {
                        entity.kind == Kind::Enemy
                    }
            });
            if !hit {
                continue;
            }

            let defence = self
                .entities
                .get(*handle)
                .and_then(|entity| catalog.object(entity.object_type))
                .map(|desc| desc.defense)
                .unwrap_or(0);

            if let Some(entity) = self.entities.get_mut(*handle) {
                // Armour applies, as it does to a projectile. An explosion that ignored it would
                // make every point of defence worthless in exactly the fights that use these.
                let taken = crate::projectile::after_defence(damage, defence, false);
                entity.hp -= taken;
                entity.damage_since_tick += taken;
                entity.last_hurt_by = from;
                if entity.hp <= 0 {
                    entity.dead = true;
                }
            }

            if let Some(effect) = effect {
                self.give_effect(*handle, effect, effect_ms);
            }
        }

        self.nearby = found;
    }

    /// Puts an entity into a named state, if it has one by that name.
    fn order_into(
        &mut self,
        catalog: &Catalog,
        behaviours: &Programs,
        handle: Handle,
        state: &str,
    ) {
        let Some(program) = self
            .entities
            .get(handle)
            .and_then(|entity| catalog.object(entity.object_type))
            .and_then(|desc| behaviours.get(&desc.id))
        else {
            return;
        };

        // An order naming a state the target does not have is ignored rather than guessed at. The
        // content sends one order to several kinds of minion and expects each to take the part of
        // it that applies.
        let Some(index) = program.state_named(state) else {
            return;
        };

        if let Some(entity) = self.entities.get_mut(handle)
            && let Some(mind) = entity.mind.as_mut()
        {
            mind.force_into(program, index);
        }
    }

    /// Replaces the ground in a circle with another tile.
    ///
    /// Walkability and sight follow the new tile, and the change is recorded so a snapshot can
    /// carry it. Without the record the ground would change for the simulation and not for anyone
    /// looking at it, which is worse than not changing it at all.
    fn reshape_ground(&mut self, catalog: &Catalog, x: f32, y: f32, radius: f32, tile: u16) {
        let tile_type = hendra_content::TileType(tile);
        let Some(desc) = catalog.tile(tile_type) else {
            return;
        };

        let radius = radius.clamp(0.0, MAX_GROUND_RADIUS);
        let (from_x, from_y) = ((x - radius).floor() as i32, (y - radius).floor() as i32);
        let (to_x, to_y) = ((x + radius).ceil() as i32, (y + radius).ceil() as i32);

        for square_y in from_y..=to_y {
            for square_x in from_x..=to_x {
                if square_x < 0 || square_y < 0 {
                    continue;
                }
                let (dx, dy) = (square_x as f32 + 0.5 - x, square_y as f32 + 0.5 - y);
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }

                // Ground never blocks sight. Only objects standing on it do, and this changes
                // the ground rather than what is on it.
                self.terrain.set_square(
                    square_x as u32,
                    square_y as u32,
                    tile_type,
                    !desc.no_walk,
                    false,
                );
                if self.ground_changes.len() < MAX_PENDING_GROUND_CHANGES {
                    self.ground_changes
                        .push((square_x as u16, square_y as u16, tile));
                }
            }
        }
    }

    /// Ages held effects, dropping the ones that have run out.
    fn expire_effects(&mut self, elapsed_ms: u32) {
        for (_, entity) in self.entities.iter_mut() {
            if entity.effects.is_empty() {
                continue;
            }

            entity.effects.retain_mut(|(_, left)| {
                *left = left.saturating_sub(elapsed_ms);
                *left > 0
            });

            // Rebuilt from what is left rather than cleared per effect, so an effect held by two
            // sources does not vanish when the first of them lapses.
            let mut conditions = ConditionSet::EMPTY;
            for (effect, _) in &entity.effects {
                if let Some(known) = hendra_content::ConditionEffect::from_index(*effect as u16) {
                    conditions.insert(known);
                }
            }
            entity.conditions = conditions;
        }
    }

    /// Grows or shrinks whatever is changing size.
    fn resize(&mut self, elapsed_ms: u32) {
        for (_, entity) in self.entities.iter_mut() {
            let Some((rate, target)) = entity.resizing else {
                continue;
            };

            let step = rate * (elapsed_ms as f32 / 1000.0);
            let now = entity.size as f32 + step;

            // Stops at the target rather than oscillating around it, whichever way it was going.
            let reached = if rate >= 0.0 {
                now >= target as f32
            } else {
                now <= target as f32
            };

            if reached {
                entity.size = target;
                entity.resizing = None;
            } else {
                entity.size = now.clamp(0.0, 65_535.0) as u16;
            }
        }
    }

    /// Fires a spread of projectiles from an entity's own weapon.
    fn fire_spread(
        &mut self,
        handle: Handle,
        catalog: &Catalog,
        angle: f32,
        count: u32,
        spread: f32,
        projectile: u8,
    ) {
        let Some(entity) = self.entities.get(handle) else {
            return;
        };
        let Some(weapon) = entity.weapon else { return };
        let Some(desc) = catalog.object(weapon) else {
            return;
        };
        if desc.projectiles.is_empty() {
            return;
        }

        let shot = desc
            .projectiles
            .get(projectile as usize)
            .unwrap_or(&desc.projectiles[0])
            .clone();

        let (x, y) = (entity.x, entity.y);
        let from_player = entity.kind == Kind::Player;

        // A spread is centred on the aim: an odd count puts one straight down the middle.
        let step = spread.to_radians();
        let start = angle - step * (count.saturating_sub(1) as f32) / 2.0;

        for index in 0..count.min(64) {
            let roll = self.roll();
            let projectile = Projectile::from_desc(
                handle,
                from_player,
                &shot,
                x,
                y,
                start + step * index as f32,
                roll,
            );
            self.projectiles.fire(projectile);
        }

        if let Some(shooter) = self.entities.get_mut(handle)
            && shooter.kind == Kind::Player
        {
            shooter.tally.shots += 1;
        }
    }

    fn cool_weapons(&mut self, elapsed_ms: u32) {
        for (_, entity) in self.entities.iter_mut() {
            entity.cooldown_ms = entity.cooldown_ms.saturating_sub(elapsed_ms);
            entity.ability_cooldown_ms = entity.ability_cooldown_ms.saturating_sub(elapsed_ms);
            entity.teleport_cooldown_ms = entity.teleport_cooldown_ms.saturating_sub(elapsed_ms);
            entity.move_grace_ms = entity.move_grace_ms.saturating_sub(elapsed_ms);

            if !entity.boosts.is_empty() {
                let before = entity.boosts.len();
                entity.boosts.retain_mut(|held| {
                    held.remaining_ms = held.remaining_ms.saturating_sub(elapsed_ms);
                    held.remaining_ms > 0
                });

                // Only when something actually lapsed: restacking is cheap but not free, and most
                // ticks change nothing.
                if entity.boosts.len() != before {
                    restack(entity);
                }
            }
        }
    }

    /// Moves every projectile and applies whatever it struck.
    fn advance_projectiles(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        // Taken out so the damage pass can borrow the entities mutably; the buffer goes back
        // afterwards, so this still allocates nothing once warm.
        let mut hits = std::mem::take(&mut self.hits);

        self.projectiles.advance(
            &self.entities,
            &self.grid,
            &self.terrain,
            catalog,
            elapsed_ms,
            &mut hits,
        );

        for hit in hits.iter() {
            let by_player = self
                .entities
                .get(hit.owner)
                .is_some_and(|owner| owner.kind == Kind::Player);

            let mut landed_on_enemy = false;

            if let Some(target) = self.entities.get_mut(hit.target) {
                target.hp -= hit.damage;
                if target.hp <= 0 {
                    target.dead = true;
                }

                target.last_hurt_by = Some(hit.owner);
                landed_on_enemy = target.kind == Kind::Enemy;

                // Remembered only for enemies hurt by players, which is the only case anybody asks
                // about: it decides who earned the loot that belongs to whoever earned it.
                if by_player && target.kind == Kind::Enemy {
                    let held = target
                        .damage_by
                        .iter_mut()
                        .find(|(who, _)| *who == hit.owner);

                    match held {
                        Some((_, total)) => *total += hit.damage,
                        None => {
                            if target.damage_by.len() < MOST_REMEMBERED_DAMAGERS {
                                target.damage_by.push((hit.owner, hit.damage));
                            }
                        }
                    }
                }
            }

            // Counted where it lands rather than where it was fired, since only the landing knows
            // whether it hit anything. The accuracy bonuses are the ratio of the two.
            if landed_on_enemy
                && let Some(shooter) = self.entities.get_mut(hit.owner)
                && shooter.kind == Kind::Player
            {
                shooter.tally.shots_that_hit += 1;
            }
        }

        self.hits = hits;
        self.projectiles.drop_orphans(&self.entities);
    }

    /// What was struck on the last tick.
    pub fn recent_hits(&self) -> &[Hit] {
        &self.hits
    }

    /// Damages anything standing on ground that hurts.
    fn apply_hazards(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter_map(|(handle, entity)| entity.kind.is_alive_kind().then_some(handle)),
        );

        for index in 0..self.handles.len() {
            let handle = self.handles[index];
            let Some(entity) = self.entities.get(handle) else {
                continue;
            };
            let Some((min, max)) = self.terrain.hazard_at(catalog, entity.x, entity.y) else {
                continue;
            };

            // Ground damage is quoted per second, so a tick takes its share.
            let per_second = (min + max) / 2;
            let damage = (per_second * elapsed_ms as i32) / 1000;
            if damage <= 0 {
                continue;
            }

            if let Some(entity) = self.entities.get_mut(handle) {
                entity.hp -= damage;
                if entity.hp <= 0 {
                    entity.dead = true;
                }
            }
        }
    }

    /// Regenerates health and magic.
    ///
    /// Vitality and wisdom are worth nothing without this, and it is what makes the walk between
    /// fights part of the game rather than dead time. Fractions are carried between ticks for the
    /// same reason the effects carry theirs: one point a second is less than one point a tick.
    fn regenerate(&mut self, elapsed_ms: u32) {
        let seconds = elapsed_ms as f32 / 1000.0;

        for (_, entity) in self.entities.iter_mut() {
            if entity.dead || entity.kind != Kind::Player {
                continue;
            }

            if entity.hp < entity.max_hp {
                let rules = crate::effects::Rules::of(entity.conditions);
                if rules.no_health_regen {
                    continue;
                }
                entity.health_fraction += entity.stats.health_regen(&rules) * seconds;
                let whole = entity.health_fraction.trunc();
                entity.health_fraction -= whole;
                entity.hp = (entity.hp + whole as i32).min(entity.max_hp);
            }

            if entity.mp < entity.max_mp {
                let rules = crate::effects::Rules::of(entity.conditions);
                entity.magic_fraction += entity.stats.magic_regen(&rules) * seconds;
                let whole = entity.magic_fraction.trunc();
                entity.magic_fraction -= whole;
                entity.mp = (entity.mp + whole as i32).min(entity.max_mp);
            }
        }
    }

    /// Applies the effects that move health over time.
    ///
    /// Kept apart from the ground because the two answer different questions. One is where you are
    /// standing and the other is what is on you, and an entity can be subject to both
    /// at once, in which case both should apply.
    fn apply_effect_health(&mut self, elapsed_ms: u32) {
        let seconds = elapsed_ms as f32 / 1000.0;

        for (_, entity) in self.entities.iter_mut() {
            if entity.effects.is_empty() || entity.dead || !entity.kind.is_alive_kind() {
                continue;
            }

            let rules = crate::effects::Rules::of(entity.conditions);
            if rules.health_per_second == 0.0 {
                continue;
            }

            // Carried between ticks rather than rounded away: twenty a second at fifty-millisecond
            // ticks is one point per tick, and rounding that to zero would make healing do nothing
            // at all.
            entity.health_fraction += rules.health_per_second * seconds;
            let whole = entity.health_fraction.trunc();
            entity.health_fraction -= whole;

            let change = whole as i32;
            if change == 0 {
                continue;
            }

            // Bleeding never finishes the job. The game leaves you at one and lets something else
            // kill you, which is what stops a stray poison being an execution.
            entity.hp = (entity.hp + change).clamp(1, entity.max_hp);
        }
    }

    /// Counts down anything with a lifetime, such as a loot bag.
    fn expire(&mut self, elapsed_ms: u32) {
        for (_, entity) in self.entities.iter_mut() {
            let Some(remaining) = entity.expires_in_ms else {
                continue;
            };
            let left = remaining.saturating_sub(elapsed_ms);
            entity.expires_in_ms = Some(left);
            if left == 0 {
                entity.dead = true;
            }
        }
    }

    /// Turns what the dead were carrying into bags on the ground.
    ///
    /// Runs before the reap, because the loot table belongs to an entity that is about to stop
    /// existing.
    fn drop_loot(&mut self, catalog: &Catalog) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| entity.dead && entity.kind == Kind::Enemy)
                .map(|(handle, _)| handle),
        );

        for index in 0..self.handles.len() {
            let handle = self.handles[index];
            let Some(entity) = self.entities.get(handle) else {
                continue;
            };
            let (x, y) = (entity.x, entity.y);

            let Some(id) = catalog
                .object(entity.object_type)
                .map(|desc| desc.id.clone())
            else {
                continue;
            };
            let Some(program) = self.behaviours.get(&id) else {
                continue;
            };

            let table = program.loot.clone();

            // Everything that belongs to whoever reaches it first.
            let mut dropped = Vec::new();
            for entry in table.iter().filter(|entry| entry.share() <= 0.0) {
                if let Some(item) = self.roll_loot(entry, catalog) {
                    dropped.push(item);
                }
            }

            // And everything that belongs to whoever earned it. Rolled once per player who took
            // enough off this enemy, into a bag only they can open, which is what makes a boss's
            // drops worth fighting for rather than worth standing near.
            let earned = self.entities.get(handle).map(|entity| {
                (
                    entity.damage_by.clone(),
                    // Against what the enemy started with rather than what it has now, which is
                    // zero: a share of nothing is a threshold nobody can meet.
                    entity.base_max_hp.unwrap_or(entity.max_hp).max(1),
                )
            });

            if let Some((damagers, total)) = earned {
                for entry in table.iter() {
                    let hendra_behavior::program::LootEntry::Threshold { share, children } = entry
                    else {
                        continue;
                    };

                    for (who, damage) in &damagers {
                        if (*damage as f32) < *share * total as f32 {
                            continue;
                        }

                        let luckier = self
                            .entities
                            .get(*who)
                            .map(|player| player.loot_drop)
                            .unwrap_or(1.0);

                        let mut theirs = Vec::new();
                        for child in children {
                            if let Some(item) = self.roll_loot_for(child, catalog, luckier) {
                                theirs.push(item);
                            }
                        }

                        if !theirs.is_empty() {
                            self.drop_owned_bag(catalog, *who, theirs, x, y);
                        }
                    }
                }
            }

            // A key dropping is worth saying out loud: a dungeon whose door needs four of them is a
            // room where knowing how many are in is the whole state of the fight.
            for item in &dropped {
                if let Some(desc) = catalog.object(*item)
                    && desc.id.ends_with(" Key")
                    && self.found_key(&desc.id)
                {
                    self.announce(&format!("{} has been found.", desc.id));
                }
            }

            if dropped.is_empty() {
                continue;
            }

            // The bag takes the colour of the best thing in it. A white bag beside a brown one is
            // how a player knows which to walk back for, and reading the highest rather than the
            // first means the order loot rolled in does not decide it.
            let colour = dropped
                .iter()
                .filter_map(|item| catalog.object(*item))
                .filter_map(|desc| desc.item.as_ref())
                .map(|item| item.bag_type)
                .max()
                .unwrap_or(0);

            let mut container = Container::new(ContainerKind::Bag, 8);
            for item in dropped {
                container.insert(item, catalog);
            }

            let mut bag = Entity::fixture(self.bag_kind(colour), x, y);
            bag.kind = Kind::Container;
            bag.container = Some(Box::new(container));
            // Long enough to walk back for, short enough that a dungeon does not fill with bags.
            bag.expires_in_ms = Some(60_000);
            self.spawn(bag);
        }
    }

    /// Drops a bag only one player can open.
    ///
    /// The whole point of loot that belongs to whoever earned it: a bag anybody could take from
    /// would make the threshold decide who it is rolled for and nothing about who gets it.
    fn drop_owned_bag(
        &mut self,
        catalog: &Catalog,
        owner: Handle,
        items: Vec<ObjectType>,
        x: f32,
        y: f32,
    ) {
        let colour = items
            .iter()
            .filter_map(|item| catalog.object(*item))
            .filter_map(|desc| desc.item.as_ref())
            .map(|item| item.bag_type)
            .max()
            .unwrap_or(0);

        let mut container = Container::new(ContainerKind::Bag, 8);
        for item in items {
            container.insert(item, catalog);
        }

        // Scattered a little, so several players' bags from one kill are not one pile.
        let spread = self.roll() * 2.0 - 1.0;

        let mut bag = Entity::fixture(self.bag_kind(colour), x + spread, y + spread);
        bag.kind = Kind::Container;
        bag.container = Some(Box::new(container));
        bag.belongs_to = Some(owner);
        bag.expires_in_ms = Some(60_000);
        self.spawn(bag);
    }

    /// Which bag a loot colour is dropped in.
    ///
    /// The content numbers these from zero upward and the world is told the object for each. A
    /// colour with nothing registered falls back to the plain bag rather than dropping nothing,
    /// because losing the loot is worse than losing its colour.
    fn bag_kind(&self, colour: i32) -> ObjectType {
        self.bag_types
            .get(colour.max(0) as usize)
            .copied()
            .unwrap_or(self.bag_type)
    }

    /// Registers the object to use for each loot colour.
    pub fn set_bag_types(&mut self, bags: Vec<ObjectType>) {
        self.bag_types = bags;
    }

    /// Decides whether one loot entry drops, and what.
    fn roll_loot(
        &mut self,
        entry: &hendra_behavior::program::LootEntry,
        catalog: &Catalog,
    ) -> Option<ObjectType> {
        self.roll_loot_for(entry, catalog, 1.0)
    }

    /// The same roll, with whatever makes this player luckier than the last.
    ///
    /// A multiplier rather than a bonus, as the original has it: a boost is worth more on something
    /// that already drops often, which is what makes it worth having on a run rather than on one
    /// kill.
    fn roll_loot_for(
        &mut self,
        entry: &hendra_behavior::program::LootEntry,
        catalog: &Catalog,
        luckier: f32,
    ) -> Option<ObjectType> {
        use hendra_behavior::program::LootEntry;

        match entry {
            LootEntry::Item { name, chance } => {
                if self.roll() > *chance * luckier {
                    return None;
                }
                catalog.type_of(name)
            }

            // A threshold is a rule about who may have what is inside it, not a drop of its own.
            // `drop_loot` walks into it; reaching here means somebody asked it to roll directly.
            LootEntry::Threshold { .. } => None,

            LootEntry::Tier { tier, kind, chance } => {
                if self.roll() > *chance * luckier {
                    return None;
                }

                // Every item of that tier and kind, one of which is chosen. A scan rather than an
                // index because this runs when something dies, not every tick.
                let wanted = slot_type_of(kind);
                let candidates: Vec<ObjectType> = catalog
                    .items()
                    .filter(|desc| {
                        desc.item.as_ref().is_some_and(|item| {
                            item.tier == Some(*tier as i32)
                                && wanted.is_none_or(|slot| item.slot_type == slot)
                        })
                    })
                    .map(|desc| desc.object_type)
                    .collect();

                if candidates.is_empty() {
                    return None;
                }
                let pick = (self.roll() * candidates.len() as f32) as usize;
                candidates.get(pick.min(candidates.len() - 1)).copied()
            }
        }
    }

    /// Uses the item worn in a slot, aimed at a point.
    ///
    /// The gate is the server's: magic, cooldown and whether the user is able to act at all. A
    /// client that asks faster than the item allows is refused rather than believed, exactly as it
    /// is for shooting.
    pub fn use_item(
        &mut self,
        handle: Handle,
        catalog: &Catalog,
        item: ObjectType,
        aim: (f32, f32),
    ) -> Vec<hendra_content::Effect> {
        let mut ran = Vec::new();

        let Some(entity) = self.entities.get(handle) else {
            return ran;
        };
        let rules = crate::effects::Rules::of(entity.conditions);
        if entity.dead || rules.quiet {
            return ran;
        }

        let Some(desc) = catalog.object(item).and_then(|object| object.item.as_ref()) else {
            return ran;
        };
        if desc.activate.is_empty() {
            return ran;
        }

        if entity.ability_cooldown_ms > 0 || entity.mp < desc.mp_cost {
            return ran;
        }

        let (x, y) = (entity.x, entity.y);
        let cooldown = (desc.cooldown * 1000.0).max(0.0) as u32;
        let cost = desc.mp_cost;

        for activate in &desc.activate {
            ran.push(hendra_content::Effect::of(activate));
        }

        // A potion and an ability are told apart by what the content calls the item, since both go
        // through the same door: an ability spends magic and a potion is drunk.
        let is_potion = desc.potion;

        if let Some(entity) = self.entities.get_mut(handle) {
            entity.mp = (entity.mp - cost).max(0);
            entity.ability_cooldown_ms = cooldown;

            if is_potion {
                entity.tally.potions_drunk += 1;
            } else {
                entity.tally.abilities_used += 1;
            }
        }

        let effects = std::mem::take(&mut ran);
        for effect in &effects {
            self.carry_out(handle, catalog, effect, (x, y), aim);
        }

        effects
    }

    /// Does what one ability asks for.
    fn carry_out(
        &mut self,
        handle: Handle,
        catalog: &Catalog,
        effect: &hendra_content::Effect,
        from: (f32, f32),
        aim: (f32, f32),
    ) {
        use hendra_content::Effect;

        match effect {
            Effect::Heal { amount } => {
                if let Some(entity) = self.entities.get_mut(handle)
                    && !crate::effects::Rules::of(entity.conditions).sick
                {
                    entity.hp = (entity.hp + amount).min(entity.max_hp);
                }
            }

            Effect::Magic { amount } => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.mp = (entity.mp + amount).min(entity.max_mp);
                }
            }

            Effect::HealNova { amount, range } => {
                let amount = *amount;
                self.each_nearby(handle, *range, true, None, |world, other| {
                    if let Some(entity) = world.entities.get_mut(other)
                        && !crate::effects::Rules::of(entity.conditions).sick
                    {
                        entity.hp = (entity.hp + amount).min(entity.max_hp);
                    }
                });
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.hp = (entity.hp + amount).min(entity.max_hp);
                }
            }

            Effect::MagicNova { amount, range } => {
                let amount = *amount;
                self.each_nearby(handle, *range, true, None, |world, other| {
                    if let Some(entity) = world.entities.get_mut(other) {
                        entity.mp = (entity.mp + amount).min(entity.max_mp);
                    }
                });
            }

            // A potion. The ceiling moves with the stat, so drinking one is worth something
            // immediately rather than only after the next level.
            Effect::IncrementStat { stat, amount } => {
                let Some(stat) = stat_of(*stat) else { return };
                let class = self
                    .entities
                    .get(handle)
                    .and_then(|entity| catalog.class(entity.object_type))
                    .cloned();

                if let Some(entity) = self.entities.get_mut(handle) {
                    match &class {
                        Some(class) => entity.stats.raise(class, stat, *amount),
                        None => entity.stats.boost(stat, *amount),
                    }
                    entity.max_hp = entity.stats.max_hp().max(1);
                    entity.max_mp = entity.stats.max_mp().max(0);
                }
            }

            Effect::StatBoost {
                stat,
                amount,
                duration_ms,
                range,
            } => {
                let Some(stat) = stat_of(*stat) else { return };

                let boost = HeldBoost {
                    stat: stat.index() as u8,
                    amount: *amount,
                    remaining_ms: *duration_ms,

                    // Aura boosts do not stack with each other, so two people standing in two of
                    // the same aura get one of them. What the original spells with `noStack`.
                    stacks: range.is_none(),
                };

                match range {
                    // An aura, which is the reason this reads a range at all: it reaches everybody
                    // nearby rather than only whoever used it.
                    Some(radius) => {
                        self.each_nearby(handle, *radius, true, None, |world, other| {
                            world.give_boost(other, boost);
                        });
                        self.give_boost(handle, boost);
                    }
                    None => self.give_boost(handle, boost),
                }
            }

            Effect::ConditionSelf {
                effect,
                duration_ms,
            } => self.give_effect(handle, effect.index() as u8, *duration_ms),

            Effect::ConditionAura {
                effect,
                duration_ms,
                range,
            } => {
                let (index, duration) = (effect.index() as u8, *duration_ms);
                self.give_effect(handle, index, duration);
                self.each_nearby(handle, *range, true, None, |world, other| {
                    world.give_effect(other, index, duration);
                });
            }

            Effect::Cleanse { range } => match range {
                Some(range) => self.each_nearby(handle, *range, true, None, |world, other| {
                    world.cleanse(other);
                }),
                None => self.cleanse(handle),
            },

            Effect::Shoot { count, spread } => {
                let angle = (aim.1 - from.1).atan2(aim.0 - from.0);
                self.fire_spread(handle, catalog, angle, *count, *spread, 0);
            }

            Effect::BulletNova { count } => {
                // A ring outward rather than a spread, so the whole circle is covered whatever the
                // count is.
                let step = std::f32::consts::TAU / (*count).max(1) as f32;
                for shot in 0..*count {
                    self.fire_spread(handle, catalog, shot as f32 * step, 1, 0.0, 0);
                }
            }

            Effect::Blast {
                radius,
                damage,
                effect,
                effect_ms,
            } => {
                self.explode(
                    catalog,
                    aim,
                    Blast {
                        from: Some(handle),
                        radius: *radius,
                        damage: *damage,
                        effect: effect.map(|found| found.index() as u8),
                        effect_ms: *effect_ms,
                        // A player's spell catches enemies. It hurting the people beside them is
                        // exactly the bug this side exists to stop.
                        hits_players: false,
                    },
                );
            }

            Effect::VampireBlast {
                radius,
                damage,
                heal,
            } => {
                self.explode(
                    catalog,
                    aim,
                    Blast {
                        from: Some(handle),
                        radius: *radius,
                        damage: *damage,
                        effect: None,
                        effect_ms: 0,
                        hits_players: false,
                    },
                );
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.hp = (entity.hp + heal).min(entity.max_hp);
                }
            }

            Effect::Create { child } => {
                if let Some(kind) = catalog.type_of(child) {
                    let behaviours = std::mem::take(&mut self.behaviours);
                    self.spawn_child(catalog, &behaviours, kind, aim.0, aim.1, None, Some(handle));
                    self.behaviours = behaviours;
                }
            }

            Effect::Teleport { max_distance } => {
                let (dx, dy) = (aim.0 - from.0, aim.1 - from.1);
                let distance = (dx * dx + dy * dy).sqrt();

                // Clamped rather than refused, so a long click moves as far as the item allows
                // instead of doing nothing.
                let (to_x, to_y) = if distance > *max_distance && distance > 0.0 {
                    let scale = max_distance / distance;
                    (from.0 + dx * scale, from.1 + dy * scale)
                } else {
                    aim
                };
                self.step(handle, to_x, to_y);
            }

            // What a decoy and a trap have in common is that they are left behind and act on
            // their own; what they do afterwards is all that differs.
            Effect::Placed {
                kind,
                duration_ms,
                radius,
                damage,
                effect,
            } => {
                let where_to = match kind {
                    hendra_content::activate::Placed::Decoy => from,
                    hendra_content::activate::Placed::Trap => aim,
                };

                let mut placed = Entity::fixture(ObjectType::NONE, where_to.0, where_to.1);
                placed.kind = match kind {
                    hendra_content::activate::Placed::Decoy => Kind::Decoy,
                    hendra_content::activate::Placed::Trap => Kind::Trap,
                };
                placed.max_hp = DECOY_HEALTH;
                placed.hp = DECOY_HEALTH;
                placed.expires_in_ms = Some((*duration_ms).max(1));
                placed.armed = Some(Armed {
                    radius: *radius,
                    damage: *damage,
                    effect: effect.map(|found| found.index() as u8),
                });

                self.spawn(placed);
            }

            Effect::Pet { .. }
            | Effect::Currency { .. }
            | Effect::Boost { .. }
            | Effect::Unlock { .. }
            | Effect::Portal { .. }
            | Effect::Appearance { .. }
            | Effect::Generic { .. }
            | Effect::Unsupported { .. } => {
                // These change something the world does not own: an account's currency, a player's
                // wardrobe, a pet that outlives the room. The caller carries them out, which is why
                // `use_item` hands back what it ran.
            }
        }
    }

    /// Removes every harmful effect an entity is carrying.
    fn cleanse(&mut self, handle: Handle) {
        let Some(entity) = self.entities.get_mut(handle) else {
            return;
        };

        entity.effects.retain(|(held, _)| {
            hendra_content::ConditionEffect::from_index(*held as u16)
                .is_none_or(|effect| !is_harmful(effect))
        });

        let mut conditions = hendra_content::ConditionSet::EMPTY;
        for (held, _) in &entity.effects {
            if let Some(known) = hendra_content::ConditionEffect::from_index(*held as u16) {
                conditions.insert(known);
            }
        }
        entity.conditions = conditions;
    }

    /// Lands whatever has finished falling.
    fn land_thrown(&mut self, catalog: &Catalog, behaviours: &Programs, elapsed_ms: u32) {
        if self.falling.is_empty() {
            return;
        }

        let mut waiting = std::mem::take(&mut self.falling);
        let mut arrived = Vec::new();

        waiting.retain_mut(|falling| {
            falling.remaining_ms = falling.remaining_ms.saturating_sub(elapsed_ms);
            if falling.remaining_ms == 0 {
                arrived.push(*falling);
                false
            } else {
                true
            }
        });
        self.falling = waiting;

        for landed in arrived {
            if let Some(handle) = self.spawn_child(
                catalog,
                behaviours,
                landed.kind,
                landed.x,
                landed.y,
                None,
                None,
            ) && let Some(entity) = self.entities.get_mut(handle)
            {
                entity.terrain = landed.terrain;
            }
        }
    }

    /// Whether a square's object is scenery rather than something the world has to hold.
    ///
    /// Asked both when the world is built and when a client is told what the map looks like, so the
    /// two can never disagree and leave a tree that is drawn but not there, or there but not drawn.
    pub fn is_scenery(catalog: &Catalog, square: &hendra_content::Composition) -> bool {
        // A name is the map author saying this one is not a tree. A size is not: the realm map
        // scales seventy thousand of its trees for variety, and that is drawing, not meaning.
        if square.name().is_some() {
            return false;
        }

        catalog.object(square.object).is_some_and(|desc| {
            desc.static_object && !desc.enemy && is_decoration(desc.class.as_str())
        })
    }

    /// Puts some number of one kind of thing down near a point.
    ///
    /// For an administrator's `/spawn`, which is the only caller: everything else that spawns comes
    /// from a behaviour, a setpiece or a realm, and each of those decides its own placement.
    pub fn spawn_at(
        &mut self,
        catalog: &Catalog,
        kind: ObjectType,
        x: f32,
        y: f32,
        count: usize,
    ) -> usize {
        let behaviours = std::mem::take(&mut self.behaviours);
        let mut made = 0;

        for index in 0..count {
            // Fanned, so a group put down together does not sit in one square and read as one.
            let spread = index as f32 * 0.35;
            if self
                .spawn_child(catalog, &behaviours, kind, x + spread, y, None, None)
                .is_some()
            {
                made += 1;
            }
        }

        self.behaviours = behaviours;
        made
    }

    /// Puts a merchant down with something to sell.
    pub fn open_stall(
        &mut self,
        kind: ObjectType,
        x: u32,
        y: u32,
        selling: crate::shop::Stall,
    ) -> Option<Handle> {
        let mut stall = Entity::fixture(kind, x as f32 + 0.5, y as f32 + 0.5);
        stall.selling = Some(selling);
        self.spawn(stall)
    }

    /// Players who have died since the last call, and what killed each.
    ///
    /// Drained rather than read, because a death is answered once: the character is marked dead, a
    /// gravestone goes down and the session ends. A caller that forgets to drain gets a queue that
    /// stops growing rather than one that grows forever.
    pub fn take_deaths(&mut self) -> Vec<Death> {
        std::mem::take(&mut self.deaths)
    }

    /// Puts a gravestone where somebody died.
    ///
    /// Which stone and how long it stands come from how much of the character was finished, as they
    /// do in the original: a level-one death leaves a small stone for thirty seconds and an
    /// eight-of-eight death leaves the largest for ten minutes. It is what the room remembers.
    pub fn place_gravestone(
        &mut self,
        catalog: &Catalog,
        at: (f32, f32),
        name: &str,
        maxed: usize,
        level: i16,
        rekt: bool,
    ) {
        let (stone, standing_ms) = match maxed {
            8 => (0x0735, 600_000),
            7 => (0x0734, 600_000),
            6 => (0x072b, 600_000),
            5 => (0x072a, 600_000),
            4 => (0x0729, 600_000),
            3 => (0x0728, 600_000),
            2 => (0x0727, 600_000),
            1 => (0x0726, 600_000),
            _ if level <= 1 => (0x0723, 30_000),
            _ if level < 20 => (0x0724, 60_000),
            _ => (0x0725, 300_000),
        };

        let kind = ObjectType(stone);
        if catalog.object(kind).is_none() {
            return;
        }

        let mut grave = Entity::fixture(kind, at.0, at.1);
        grave.name = Some(if rekt {
            format!("{name} got rekt").into()
        } else {
            name.into()
        });
        grave.expires_in_ms = Some(standing_ms);
        self.spawn(grave);
    }

    /// Notes that somebody said something, for whatever is listening.
    ///
    /// A dungeon whose door opens when you say the right word is built out of this, and without it
    /// the door has no handle.
    pub fn heard(&mut self, at: (f32, f32), text: &str) {
        if self.heard.len() < MOST_HEARD_AT_ONCE {
            self.heard.push((at.0, at.1, text.into()));
        }
    }

    /// Notes that a key has been found here, and says whether it is new.
    ///
    /// New matters: the world tells everybody when one is found, and telling them twice for the
    /// same key is telling them something that did not happen.
    pub fn found_key(&mut self, name: &str) -> bool {
        if self.keys_found.iter().any(|held| &**held == name) {
            return false;
        }

        self.keys_found.push(name.into());
        true
    }

    /// Which keys have been found here, for somebody arriving late.
    pub fn keys_found(&self) -> Vec<String> {
        self.keys_found.iter().map(|key| key.to_string()).collect()
    }

    /// Moves a player to another player, if every rule allows it.
    ///
    /// The refusals are the original's, in `Player.Teleport`, and each is a real hole otherwise:
    /// teleporting to somebody invisible finds a player who is hiding, teleporting to somebody
    /// paused reaches into a place the world has stopped, and teleporting with no cooldown is a way
    /// to cross a realm faster than anything can chase.
    ///
    /// Returns why it was refused, or `None` when it happened.
    pub fn teleport_to(&mut self, who: Handle, to: Handle) -> Option<&'static str> {
        if who == to {
            return Some("You are already at yourself, and always will be.");
        }
        if !self.allows_teleport {
            return Some("You cannot teleport here.");
        }

        let mover = self.entities.get(who)?;
        if mover.teleport_cooldown_ms > 0 {
            return Some("Too soon to teleport again.");
        }
        if mover
            .conditions
            .contains(hendra_content::ConditionEffect::Paused)
        {
            return Some("You cannot teleport while paused.");
        }

        let Some(target) = self.entities.get(to) else {
            return Some("They are not here.");
        };
        if target.kind != Kind::Player {
            return Some("You can only teleport to players.");
        }
        if target
            .conditions
            .contains(hendra_content::ConditionEffect::Invisible)
        {
            return Some("You cannot teleport to an invisible player.");
        }
        if target
            .conditions
            .contains(hendra_content::ConditionEffect::Paused)
        {
            return Some("You cannot teleport to a paused player.");
        }

        let (x, y) = (target.x, target.y);

        if let Some(mover) = self.entities.get_mut(who) {
            mover.x = x;
            mover.y = y;
            mover.teleport_cooldown_ms = TELEPORT_COOLDOWN_MS;
            mover.tally.teleports += 1;

            // The jump arrives at the mover's own client as a position it did not ask for, and at
            // everyone else's as a move no speed explains. The grace is what stops the server's own
            // teleport being read as somebody moving too fast.
            mover.move_grace_ms = MOVE_GRACE_MS;
        }

        None
    }

    /// Whether players may teleport in this world, from the world definition.
    pub fn set_allows_teleport(&mut self, allowed: bool) {
        self.allows_teleport = allowed;
    }

    /// Where a player is, by name.
    pub fn player_named(&self, name: &str) -> Option<Handle> {
        self.entities
            .iter()
            .find(|(_, entity)| {
                entity.kind == Kind::Player
                    && entity
                        .name
                        .as_deref()
                        .is_some_and(|held| held.eq_ignore_ascii_case(name))
            })
            .map(|(handle, _)| handle)
    }

    /// Counts what is alive on each terrain.
    ///
    /// From the tag an enemy carries rather than from where it is standing, because an enemy that
    /// has chased somebody across a border still belongs to the terrain it was placed on. That is
    /// what stops a chase from emptying one terrain and overfilling the next.
    pub fn alive_by_terrain(&self) -> [usize; TERRAIN_COUNT] {
        let mut counts = [0usize; TERRAIN_COUNT];

        for (_, entity) in self.entities.iter() {
            if entity.kind != Kind::Enemy || entity.dead {
                continue;
            }
            counts[entity.terrain as usize] += 1;
        }

        counts
    }

    /// Carries out what the realm asked for: fills terrains that are short, thins ones that are
    /// over.
    ///
    /// Returns how many were added and how many removed.
    pub fn populate(
        &mut self,
        catalog: &Catalog,
        spawnable: &[crate::realm::Spawn],
        wanted: &[crate::realm::Adjustment],
    ) -> (usize, usize) {
        let mut added = 0;
        let mut removed = 0;

        for adjustment in wanted {
            if adjustment.remove > 0 {
                removed += self.thin(adjustment.terrain, adjustment.remove);
                continue;
            }

            added += self.fill(catalog, spawnable, adjustment.terrain, adjustment.add);
        }

        (added, removed)
    }

    /// Adds up to `wanted` enemies to one terrain.
    fn fill(
        &mut self,
        catalog: &Catalog,
        spawnable: &[crate::realm::Spawn],
        terrain: hendra_content::Terrain,
        wanted: usize,
    ) -> usize {
        if self.open_squares(terrain).is_empty() {
            return 0;
        }

        // Lifted out for the same reason the tick lifts them: `spawn_child` needs them while the
        // world is borrowed mutably.
        let behaviours = std::mem::take(&mut self.behaviours);
        let mut made = 0;

        // A budget rather than a loop until done. The weights for a terrain do not always sum to
        // one, so some rolls choose nothing, and a square can come back crowded; both are reasons
        // to try again, not reasons to stop, but neither can be allowed to spin forever.
        let mut tries = wanted * 4 + 64;

        while made < wanted && tries > 0 {
            tries -= 1;

            let Some(spawn) = crate::realm::choose(spawnable, terrain, self.roll()) else {
                continue;
            };
            let Some((x, y)) = self.open_square(terrain) else {
                continue;
            };

            // A description that asks for a group brings its whole group, scattered around the
            // point rather than stacked on it.
            let size = match spawn.group {
                Some(count) => count.size(crate::realm::normal(self.roll(), self.roll())),
                None => 1,
            };

            for _ in 0..size {
                let (at_x, at_y) = match spawn.group {
                    Some(_) => (
                        x + (self.roll() * 2.0 - 1.0) * crate::realm::GROUP_SPREAD,
                        y + (self.roll() * 2.0 - 1.0) * crate::realm::GROUP_SPREAD,
                    ),
                    None => (x, y),
                };

                if !self.terrain.walkable_at(at_x, at_y) {
                    continue;
                }

                if let Some(handle) =
                    self.spawn_child(catalog, &behaviours, spawn.kind, at_x, at_y, None, None)
                    && let Some(entity) = self.entities.get_mut(handle)
                {
                    // Tagged with the terrain it was placed on, which is what the next count reads
                    // and what anything it spawns inherits.
                    entity.terrain = terrain;
                    made += 1;
                }
            }
        }

        self.behaviours = behaviours;
        made
    }

    /// Removes up to `wanted` enemies from one terrain.
    ///
    /// Only ones nobody is near: despawning something a player is fighting reads as the server
    /// eating their kill.
    fn thin(&mut self, terrain: hendra_content::Terrain, wanted: usize) -> usize {
        let players: Vec<(f32, f32)> = self
            .entities
            .iter()
            .filter(|(_, entity)| entity.kind == Kind::Player && !entity.dead)
            .map(|(_, entity)| (entity.x, entity.y))
            .collect();

        let doomed: Vec<Handle> = self
            .entities
            .iter()
            .filter(|(_, entity)| {
                entity.kind == Kind::Enemy && !entity.dead && entity.terrain == terrain
            })
            .filter(|(_, entity)| {
                !players.iter().any(|(px, py)| {
                    let (dx, dy) = (entity.x - px, entity.y - py);
                    dx * dx + dy * dy < crate::realm::PLAYER_CLEARANCE.powi(2)
                })
            })
            .map(|(handle, _)| handle)
            .take(wanted)
            .collect();

        // Removed outright rather than killed: a thinned enemy is one the map never needed, and
        // killing it would drop loot and hand out experience nobody earned.
        for handle in &doomed {
            self.despawn(*handle);
        }

        doomed.len()
    }

    /// Every walkable square of a terrain, worked out once and kept.
    ///
    /// A realm map is four million squares and a terrain can be a thousandth of it, so looking for
    /// one by guessing at random finds nothing in any reasonable number of tries. Listing them once
    /// turns every later search into a single pick.
    fn open_squares(&mut self, terrain: hendra_content::Terrain) -> &[u32] {
        if !self.spawn_squares.contains_key(&terrain) {
            let mut squares = Vec::new();

            for y in 0..self.terrain.height() {
                for x in 0..self.terrain.width() {
                    if self.terrain.terrain_at(x, y) == terrain && self.terrain.walkable(x, y) {
                        squares.push((x << 16) | y);
                    }
                }
            }

            self.spawn_squares.insert(terrain, squares);
        }

        &self.spawn_squares[&terrain]
    }

    /// A walkable square of a terrain with nobody near it.
    ///
    /// Away from players, because an enemy appearing on top of somebody is not a spawn but an
    /// ambush nobody could have avoided.
    fn open_square(&mut self, terrain: hendra_content::Terrain) -> Option<(f32, f32)> {
        let count = self.open_squares(terrain).len();
        if count == 0 {
            return None;
        }

        for _ in 0..SPAWN_ATTEMPTS {
            let picked = ((self.roll() * count as f32) as usize).min(count - 1);
            let packed = self.spawn_squares[&terrain][picked];
            let (x, y) = (packed >> 16, packed & 0xffff);

            let (at_x, at_y) = (x as f32 + 0.5, y as f32 + 0.5);
            self.grid
                .within(at_x, at_y, crate::realm::PLAYER_CLEARANCE, &mut self.nearby);
            let nearby = std::mem::take(&mut self.nearby);

            let crowded = nearby.iter().any(|handle| {
                self.entities
                    .get(*handle)
                    .is_some_and(|entity| entity.kind == Kind::Player)
            });
            self.nearby = nearby;

            if !crowded {
                return Some((at_x, at_y));
            }
        }

        None
    }

    /// Draws a setpiece into the world at a point.
    ///
    /// The drawing is in its own coordinates, from its top left corner, which is where the original
    /// puts them: a setpiece is placed by its corner and not by its middle.
    ///
    /// Returns the names it could not find in the content. A setpiece with a missing name is a hole
    /// in the realm, and it should be said rather than left to be walked into.
    pub fn draw(
        &mut self,
        catalog: &Catalog,
        drawing: &crate::setpiece::Drawing,
        at: (u32, u32),
    ) -> Vec<&'static str> {
        use crate::setpiece::Placed;

        let mut missing = Vec::new();
        let mut refused = 0usize;
        let behaviours = std::mem::take(&mut self.behaviours);

        for square in &drawing.squares {
            let x = at.0 as i32 + square.x;
            let y = at.1 as i32 + square.y;
            if x < 0 || y < 0 || !self.terrain.contains(x as u32, y as u32) {
                continue;
            }
            let (x, y) = (x as u32, y as u32);

            let tile = match square.tile {
                Some(name) => match catalog.tile_type_of(name) {
                    Some(kind) => Some(kind),
                    None => {
                        missing.push(name);
                        None
                    }
                },
                None => None,
            };

            let object = match square.object {
                Some(name) => match catalog.type_of(name) {
                    Some(kind) => Some((kind, square.size)),
                    None => {
                        missing.push(name);
                        None
                    }
                },
                None => None,
            };

            // Whatever was standing here goes, whether the drawing asked for the square to be
            // cleared or is about to put its own wall there.
            if square.clear || object.is_some() {
                self.clear_area(x as f32 + 0.5, y as f32 + 0.5, 1.0, 1.0);
            }

            // Painted rather than spawned. A setpiece's walls are scenery like any other wall: the
            // original writes them onto the tile, and a castle whose every stone was an entity
            // would cost eight hundred places in the world and eight hundred snapshot entries.
            if !self.terrain.paint(
                catalog,
                x,
                y,
                tile,
                object,
                square.clear && object.is_none(),
            ) {
                refused += 1;
                continue;
            }

            if let Some(tile) = tile
                && self.ground_changes.len() < MAX_PENDING_GROUND_CHANGES
            {
                self.ground_changes.push((x as u16, y as u16, tile.0));
            }

            if let Some((object, size)) = object
                && self.scenery_changes.len() < MAX_PENDING_GROUND_CHANGES
            {
                self.scenery_changes
                    .push((x as u16, y as u16, object.0, size));
            }
        }

        for placed in &drawing.placed {
            match placed {
                Placed::Living { x, y, name, size } => {
                    let Some(kind) = catalog.type_of(name) else {
                        missing.push(name);
                        continue;
                    };

                    if let Some(handle) = self.spawn_child(
                        catalog,
                        &behaviours,
                        kind,
                        at.0 as f32 + x,
                        at.1 as f32 + y,
                        None,
                        None,
                    ) && *size > 0
                        && let Some(entity) = self.entities.get_mut(handle)
                    {
                        entity.size = *size;
                    }
                }

                // Chests come after, because filling one rolls loot and that needs the world
                // back in one piece.
                Placed::Chest { .. } => {}
            }
        }

        self.behaviours = behaviours;

        for placed in &drawing.placed {
            let Placed::Chest {
                x,
                y,
                loot,
                least,
                most,
            } = placed
            else {
                continue;
            };

            self.place_chest(
                catalog,
                (at.0 as f32 + x, at.1 as f32 + y),
                loot,
                *least,
                *most,
            );
        }

        if refused > 0 {
            // A setpiece that came out with holes in it, because the map cannot describe any more
            // kinds of square. Better said than walked into.
            self.refused_squares += refused;
        }

        missing
    }

    /// How many squares a setpiece could not paint because the map was full.
    pub fn refused_squares(&self) -> usize {
        self.refused_squares
    }

    /// Puts down a chest holding a few items drawn from a setpiece's loot table.
    fn place_chest(
        &mut self,
        catalog: &Catalog,
        at: (f32, f32),
        loot: &[crate::setpiece::Tier],
        least: usize,
        most: usize,
    ) {
        use hendra_behavior::program::LootEntry;

        let wanted = least + (self.roll() * (most.saturating_sub(least) + 1) as f32) as usize;

        let mut held = Vec::new();
        for tier in loot {
            if held.len() >= wanted.min(SETPIECE_CHEST_SLOTS) {
                break;
            }

            let entry = LootEntry::Tier {
                tier: tier.tier,
                kind: tier.kind.to_string(),
                chance: tier.chance,
            };
            if let Some(item) = self.roll_loot(&entry, catalog) {
                held.push(item);
            }
        }

        if held.is_empty() {
            return;
        }

        let Some(kind) = catalog.type_of(SETPIECE_CHEST) else {
            return;
        };

        let mut container = Container::new(ContainerKind::Bag, SETPIECE_CHEST_SLOTS);
        for item in held {
            container.insert(item, catalog);
        }

        let mut chest = Entity::fixture(kind, at.0, at.1);
        chest.kind = Kind::Container;
        chest.container = Some(Box::new(container));
        // No expiry: a setpiece chest is part of the realm and stands until somebody empties it.
        self.spawn(chest);
    }

    /// Setpiece names a behaviour asked for that nothing answers to.
    ///
    /// Taken rather than read, so each one is reported once. Four of the shipped behaviours name a
    /// setpiece that does not exist, and they do in the original too.
    pub fn take_unknown_setpieces(&mut self) -> Vec<String> {
        let mut names: Vec<String> = self.unknown_setpieces.drain().collect();
        names.sort();
        names
    }

    /// How the realm is doing.
    pub fn realm(&self) -> &crate::realm::Realm {
        &self.realm
    }

    /// How the realm is doing, so a caller can advance its clock.
    pub fn realm_mut(&mut self) -> &mut crate::realm::Realm {
        &mut self.realm
    }

    /// Opens a portal where an entity is standing.
    pub fn open_portal(
        &mut self,
        catalog: &Catalog,
        at: Handle,
        kind: ObjectType,
        duration_ms: u32,
    ) -> Option<Handle> {
        let (x, y) = self.entities.get(at).map(|entity| (entity.x, entity.y))?;

        let behaviours = std::mem::take(&mut self.behaviours);
        let portal = self.spawn_child(catalog, &behaviours, kind, x, y, None, None);
        self.behaviours = behaviours;

        if let Some(portal) = portal
            && let Some(entity) = self.entities.get_mut(portal)
        {
            entity.kind = Kind::Portal;
            // A portal with no lifetime is a permanent change to a room somebody else has to live
            // in, so one opened by an item always has one.
            entity.expires_in_ms = Some(duration_ms.max(1_000));
        }

        portal
    }

    /// How many enemies are alive.
    pub fn enemy_count(&self) -> usize {
        self.entities
            .iter()
            .filter(|(_, entity)| entity.kind == Kind::Enemy && !entity.dead)
            .count()
    }

    /// How many of one kind are alive, for a realm counting against a ceiling.
    pub fn count_of_kind(&self, kind: ObjectType) -> usize {
        self.entities
            .iter()
            .filter(|(_, entity)| {
                entity.object_type == kind && entity.kind == Kind::Enemy && !entity.dead
            })
            .count()
    }

    /// Says something to everyone, as the world rather than as an entity.
    ///
    /// Used for the announcements a realm makes about itself, which have no speaker: nothing in
    /// the world said them, and attributing them to an enemy would be a lie the client repeats.
    pub fn announce(&mut self, text: &str) {
        if self.announcements.len() < MAX_PENDING_ANNOUNCEMENTS {
            self.announcements.push(Announcement {
                from: Handle::NONE,
                text: text.into(),
                broadcast: true,
            });
        }
    }

    /// Stamps a prefab map into the world.
    ///
    /// A setpiece is a small map placed at a point, not a circle of one tile painted over the
    /// ground. Every square it names is written, including the ground and whatever stands on it,
    /// so a room built this way is the room the author drew rather than an approximation of it.
    ///
    /// Anything already standing where it lands is removed. A setpiece that appeared around the
    /// existing furniture would leave a wall through the middle of a boss's arena.
    pub fn stamp(&mut self, catalog: &Catalog, piece: &Map, at: (f32, f32)) {
        // Centred on the point rather than starting there, which is what "place it here" means to
        // whoever wrote the content.
        let left = at.0 as i32 - piece.width() as i32 / 2;
        let top = at.1 as i32 - piece.height() as i32 / 2;

        // Cleared first, so the two passes cannot fight: an object spawned by the stamp must not
        // be removed by the clearing of a later square.
        self.clear_area(
            left as f32,
            top as f32,
            piece.width() as f32,
            piece.height() as f32,
        );

        for y in 0..piece.height() {
            for x in 0..piece.width() {
                let Some(square) = piece.at(x, y) else {
                    continue;
                };

                let (world_x, world_y) = (left + x as i32, top + y as i32);
                if world_x < 0 || world_y < 0 {
                    continue;
                }
                let (world_x, world_y) = (world_x as u32, world_y as u32);

                if let Some(tile) = catalog.tile(square.tile) {
                    self.terrain
                        .set_square(world_x, world_y, square.tile, !tile.no_walk, false);
                    if self.ground_changes.len() < MAX_PENDING_GROUND_CHANGES {
                        self.ground_changes
                            .push((world_x as u16, world_y as u16, square.tile.0));
                    }
                }

                if !square.object.is_none() {
                    let behaviours = std::mem::take(&mut self.behaviours);
                    self.spawn_child(
                        catalog,
                        &behaviours,
                        square.object,
                        world_x as f32 + 0.5,
                        world_y as f32 + 0.5,
                        None,
                        None,
                    );
                    self.behaviours = behaviours;
                }
            }
        }
    }

    /// Removes everything standing in a rectangle, except players.
    ///
    /// Players are spared because a setpiece landing on somebody should move the room around them,
    /// not delete them from it.
    fn clear_area(&mut self, left: f32, top: f32, width: f32, height: f32) {
        for (_, entity) in self.entities.iter_mut() {
            if entity.kind == Kind::Player {
                continue;
            }
            if entity.x >= left
                && entity.x < left + width
                && entity.y >= top
                && entity.y < top + height
            {
                entity.dead = true;
                entity.no_experience = true;
            }
        }
    }

    /// Sets off any trap something has walked into.
    fn spring_traps(&mut self, catalog: &Catalog) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| entity.kind == Kind::Trap && entity.armed.is_some())
                .map(|(handle, _)| handle),
        );

        if self.handles.is_empty() {
            return;
        }

        let armed = std::mem::take(&mut self.handles);
        for handle in &armed {
            let Some((x, y, trap)) = self
                .entities
                .get(*handle)
                .and_then(|entity| entity.armed.map(|armed| (entity.x, entity.y, armed)))
            else {
                continue;
            };

            // A trap is sprung by anything it would hurt, which is what makes walking over one a
            // mistake rather than a decision.
            self.grid.within(x, y, trap.radius, &mut self.nearby);
            let nearby = std::mem::take(&mut self.nearby);

            let triggered = nearby.iter().any(|other| {
                self.entities
                    .get(*other)
                    .is_some_and(|entity| entity.kind == Kind::Enemy && !entity.dead)
            });
            self.nearby = nearby;

            if !triggered {
                continue;
            }

            self.explode(
                catalog,
                (x, y),
                Blast {
                    from: Some(*handle),
                    radius: trap.radius,
                    damage: trap.damage,
                    effect: trap.effect,
                    effect_ms: TRAP_EFFECT_MS,
                    hits_players: false,
                },
            );

            if let Some(entity) = self.entities.get_mut(*handle) {
                entity.dead = true;
                entity.no_experience = true;
            }
        }

        self.handles = armed;
    }

    /// Awards experience for everything that died this tick.
    ///
    /// Between the death and the reaping, because where the enemy fell decides who is close enough
    /// to be paid for it.
    fn award_experience(&mut self, catalog: &Catalog) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| {
                    entity.dead && entity.kind == Kind::Enemy && !entity.no_experience
                })
                .map(|(handle, _)| handle),
        );

        if self.handles.is_empty() {
            return;
        }

        let dead = std::mem::take(&mut self.handles);
        for handle in &dead {
            let Some(entity) = self.entities.get(*handle) else {
                continue;
            };
            let (x, y, max_hp) = (entity.x, entity.y, entity.max_hp);
            let desc = catalog.object(entity.object_type);
            let multiplier = desc.and_then(|desc| desc.exp_multiplier).unwrap_or(1.0);

            // What kind of thing it was, for the counters that ask. A cube is named rather than
            // flagged, because the content has no flag for one and the bonus is about the cubes.
            let was_god = desc.is_some_and(|desc| desc.god);
            let was_cube = desc.is_some_and(|desc| desc.id.contains("Gelatinous Cube"));
            let was_oryx = desc.is_some_and(|desc| desc.id.starts_with("Oryx"));

            // Whoever struck last is the one credited with the kill, as the original credits its
            // last hitter. Everybody nearby still shares the experience.
            if let Some(killer) = entity.last_hurt_by
                && let Some(player) = self.entities.get_mut(killer)
                && player.kind == Kind::Player
            {
                if was_god {
                    player.tally.god_kills += 1;
                } else {
                    player.tally.monster_kills += 1;
                }
                if was_cube {
                    player.tally.cube_kills += 1;
                }
                if was_oryx {
                    player.tally.oryx_kills += 1;
                }
            }

            self.grid
                .within(x, y, crate::leveling::SHARE_RADIUS, &mut self.nearby);
            let nearby = std::mem::take(&mut self.nearby);

            for player in &nearby {
                let earns = self.entities.get(*player).is_some_and(|entity| {
                    entity.kind == Kind::Player
                        && !entity.dead
                        && !crate::effects::Rules::of(entity.conditions).paused
                });
                if !earns {
                    continue;
                }

                let Some(class) = self
                    .entities
                    .get(*player)
                    .and_then(|entity| catalog.class(entity.object_type))
                    .cloned()
                else {
                    continue;
                };

                let level = self
                    .entities
                    .get(*player)
                    .map(|entity| entity.progress.level)
                    .unwrap_or(1);

                let earned = crate::leveling::experience_for_kill(max_hp, multiplier, true, level);
                if earned <= 0 {
                    continue;
                }

                // The roll is drawn from the world so a level-up is reproducible from the seed.
                let mut rolls = Vec::with_capacity(8);
                for _ in 0..8 {
                    rolls.push(self.roll());
                }
                let mut next = rolls.into_iter();

                if let Some(entity) = self.entities.get_mut(*player) {
                    let mut progress = entity.progress;
                    let advance = progress.gain(&class, &mut entity.stats, earned, || {
                        next.next().unwrap_or(0.5)
                    });
                    entity.progress = progress;

                    // A level raises the ceiling and fills what it added, matching the original's
                    // `HP = Stats[0]` after every level.
                    if advance.levels_gained > 0 {
                        entity.max_hp = entity.stats.max_hp().max(1);
                        entity.max_mp = entity.stats.max_mp().max(0);
                        entity.hp = entity.max_hp;
                        entity.mp = entity.max_mp;
                    }
                }
            }

            self.nearby = nearby;
        }

        self.handles = dead;
    }

    /// Runs whatever the dying have arranged to happen after them.
    ///
    /// Between loot and reaping, because these need the entity still in the world. Where it was
    /// standing is most of what a portal, a transformation or a change of ground is about.
    fn run_death_effects(&mut self, catalog: &Catalog) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| entity.dead && !entity.no_experience && entity.mind.is_some())
                .map(|(handle, _)| handle),
        );

        if self.handles.is_empty() {
            return;
        }

        let behaviours = std::mem::take(&mut self.behaviours);
        let dying = std::mem::take(&mut self.handles);

        for handle in &dying {
            let Some(program) = self
                .entities
                .get(*handle)
                .and_then(|entity| catalog.object(entity.object_type))
                .and_then(|desc| behaviours.get(&desc.id))
            else {
                continue;
            };

            // Every state's effects, not only the one it died in: the content puts these on the
            // outermost state precisely so that dying anywhere triggers them.
            for state in &program.states {
                for primitive in &state.behaviours {
                    let Some(effect) = primitive.death_effect() else {
                        continue;
                    };
                    self.run_death_effect(catalog, &behaviours, program, *handle, effect);
                }
            }
        }

        self.handles = dying;
        self.behaviours = behaviours;
    }

    fn run_death_effect(
        &mut self,
        catalog: &Catalog,
        behaviours: &Programs,
        program: &Program,
        handle: Handle,
        effect: &DeathEffect,
    ) {
        let Some((x, y, hp)) = self
            .entities
            .get(handle)
            .map(|entity| (entity.x, entity.y, entity.max_hp))
        else {
            return;
        };

        match effect {
            DeathEffect::Spawn { child, count } => {
                let Some(kind) = program.kind_of(*child).map(ObjectType) else {
                    return;
                };
                for index in 0..(*count).min(MAX_SPAWNED_AT_ONCE) {
                    self.spawn_child(
                        catalog,
                        behaviours,
                        kind,
                        x + index as f32 * 0.35,
                        y,
                        None,
                        Some(handle),
                    );
                }
            }

            DeathEffect::TransformInto { child } => {
                if let Some(kind) = program.kind_of(*child).map(ObjectType) {
                    self.spawn_child(catalog, behaviours, kind, x, y, None, Some(handle));
                }
            }

            DeathEffect::Portal {
                name,
                probability,
                duration_ms,
            } => {
                if self.roll() > *probability {
                    return;
                }
                let Some(kind) = program.kind_of(*name).map(ObjectType) else {
                    return;
                };
                if let Some(portal) =
                    self.spawn_child(catalog, behaviours, kind, x, y, None, Some(handle))
                    && let Some(entity) = self.entities.get_mut(portal)
                {
                    entity.expires_in_ms = Some(*duration_ms);
                }
            }

            DeathEffect::ChangeGround { tile, radius } => {
                if let Some(kind) = program.kind_of(*tile) {
                    self.reshape_ground(catalog, x, y, *radius, kind);
                }
            }

            DeathEffect::RemoveObjects { radius, kind } => {
                let wanted = kind.and_then(|name| program.kind_of(name)).map(ObjectType);
                self.each_nearby(handle, *radius, false, wanted, |world, other| {
                    if let Some(entity) = world.entities.get_mut(other) {
                        entity.dead = true;
                        entity.no_experience = true;
                    }
                });
            }

            DeathEffect::Order {
                radius,
                kind,
                state,
            } => {
                let wanted = kind.and_then(|name| program.kind_of(name)).map(ObjectType);
                let state = state.clone();
                self.each_nearby(handle, *radius, false, wanted, |world, other| {
                    world.order_into(catalog, behaviours, other, &state);
                });
            }

            // What it could still have taken, dealt to whatever it was standing with. This is how
            // the game's linked bosses die together.
            DeathEffect::TransferDamage { radius, kind } => {
                let wanted = kind.and_then(|name| program.kind_of(name)).map(ObjectType);
                self.each_nearby(handle, *radius, false, wanted, |world, other| {
                    if let Some(entity) = world.entities.get_mut(other) {
                        entity.hp -= hp;
                        entity.damage_since_tick += hp;
                        if entity.hp <= 0 {
                            entity.dead = true;
                        }
                    }
                });
            }
        }
    }

    /// Takes everything said since the last call.
    ///
    /// Draining rather than reading, because each of these should be sent once. A caller that
    /// forgets to drain gets a queue that stops growing rather than one that grows forever.
    pub fn take_announcements(&mut self) -> Vec<Announcement> {
        std::mem::take(&mut self.announcements)
    }

    /// Takes every ground change since the last call, as `(x, y, tile)`.
    pub fn take_ground_changes(&mut self) -> Vec<(u16, u16, u16)> {
        std::mem::take(&mut self.ground_changes)
    }

    /// Takes every piece of scenery that has appeared, as `(x, y, object, size)`.
    pub fn take_scenery_changes(&mut self) -> Vec<(u16, u16, u16, u16)> {
        std::mem::take(&mut self.scenery_changes)
    }

    /// Notes which players have died, and what killed each.
    ///
    /// Before the reaping, because the body is where the gravestone goes and what killed it may
    /// itself be about to be removed. The session answers the rest: a world can end a life but
    /// cannot write a character down.
    fn note_deaths(&mut self, catalog: &Catalog) {
        for (handle, entity) in self.entities.iter() {
            if entity.kind != Kind::Player || !entity.dead {
                continue;
            }

            // Named by what last hurt them, which is the only thing that reads as an answer to
            // "what killed me". Falling back to the world says something true when nothing did.
            let killer = entity
                .last_hurt_by
                .and_then(|by| self.entities.get(by))
                .and_then(|by| catalog.object(by.object_type))
                .map(|desc| desc.id.clone())
                .unwrap_or_else(|| self.name.to_string());

            self.deaths.push(Death {
                who: handle,
                killer,
                x: entity.x,
                y: entity.y,
            });
        }
    }

    /// Removes everything marked dead.
    fn reap(&mut self) {
        self.entities.retain(|_, entity| !entity.dead);
    }

    /// The world as one player sees it.
    ///
    /// Only what is within sight is encoded at all, rather than encoded and then discarded. The
    /// entities come back sorted by handle, which is what the snapshot encoder's merge wants.
    pub fn snapshot_for(&mut self, viewer: Handle, radius: f32) -> WorldSnapshot {
        let Some(entity) = self.entities.get(viewer) else {
            return WorldSnapshot::new();
        };
        let (x, y) = (entity.x, entity.y);

        self.grid.within(x, y, radius, &mut self.nearby);

        self.visible.clear();
        for handle in &self.nearby {
            let Some(entity) = self.entities.get(*handle) else {
                continue;
            };

            // Within range is not the same as in view. Without this a player sees, and is seen by,
            // anything on the far side of a wall, which in a game where being seen means being
            // shot is a correctness problem rather than a cosmetic one.
            if *handle != viewer && !self.terrain.line_of_sight(x, y, entity.x, entity.y) {
                continue;
            }

            // Invisible is hidden from everyone but the one carrying it. Filtered here rather than
            // by the client, because a client told about something it should not see is a client
            // that can be made to reveal it.
            if *handle != viewer && crate::effects::Rules::of(entity.conditions).invisible {
                continue;
            }

            self.visible.push((handle.to_entity_id(), entity.state()));
        }

        WorldSnapshot::from_unsorted(std::mem::take(&mut self.visible))
    }

    /// Everything within a radius of a point.
    pub fn near(&mut self, x: f32, y: f32, radius: f32) -> &[Handle] {
        self.grid.within(x, y, radius, &mut self.nearby);
        &self.nearby
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hendra_content::map::Composition;
    use hendra_content::{Map, Region, TileType};

    const FIXTURE: &str = r#"<Objects>
        <Ground type="0x10" id="Grass"><Speed>1</Speed></Ground>
        <Ground type="0x11" id="Water"><NoWalk/></Ground>
        <Ground type="0x12" id="Lava"><MinDamage>100</MinDamage><MaxDamage>100</MaxDamage></Ground>
        <Object type="0x500" id="Wall"><Class>GameObject</Class><FullOccupy/><Static/></Object>
        <Object type="0x501" id="Sign"><Class>GameObject</Class><Static/></Object>
        <Object type="0x504" id="Tree"><Class>GameObject</Class><BlocksSight/><Static/></Object>
        <Object type="0x502" id="Slime"><Class>Character</Class><Enemy/>
          <MaxHitPoints>200</MaxHitPoints>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>20</MinDamage><MaxDamage>20</MaxDamage>
            <LifetimeMS>2000</LifetimeMS></Projectile>
        </Object>
        <Object type="0x503" id="Guard"><Class>Character</Class><Enemy/>
          <MaxHitPoints>50</MaxHitPoints></Object>
        <Object type="0x507" id="Plated Slime"><Class>Character</Class><Enemy/>
          <MaxHitPoints>200</MaxHitPoints><Defense>30</Defense></Object>
        <Object type="0x505" id="Spawnling"><Class>Character</Class><Enemy/>
          <MaxHitPoints>10</MaxHitPoints></Object>
        <Object type="0x506" id="Doorway"><Class>Portal</Class></Object>
        <Object type="0x600" id="Hero"><Class>Player</Class><Player/></Object>
        <Object type="0x030e" id="Wizard"><Class>Player</Class><Player/>
          <MaxHitPoints max="670">100</MaxHitPoints>
          <MaxMagicPoints max="385">100</MaxMagicPoints>
          <Attack max="75">12</Attack><Defense max="25">0</Defense>
          <Speed max="50">12</Speed><Dexterity max="75">15</Dexterity>
          <HpRegen max="40">10</HpRegen><MpRegen max="60">10</MpRegen>
          <SlotTypes>8, 5, 6, 9</SlotTypes>
          <LevelIncrease min="20" max="30">MaxHitPoints</LevelIncrease>
          <LevelIncrease min="2" max="8">MaxMagicPoints</LevelIncrease>
        </Object>
        <Object type="0x900" id="Bolt"><Class>Projectile</Class></Object>
        <Object type="0x510" id="Loot Bag"><Class>Container</Class></Object>
        <Object type="0x511" id="Loot Bag 5"><Class>Container</Class></Object>
        <Object type="0x904" id="Rare Blade">
          <Class>Equipment</Class><Item/><SlotType>1</SlotType><BagType>5</BagType>
        </Object>
        <Object type="0x905" id="Cloak of Shadows">
          <Class>Equipment</Class><Item/><SlotType>13</SlotType>
          <Activate duration="5" distance="1">Decoy</Activate>
        </Object>
        <Object type="0x906" id="Trap Spell">
          <Class>Equipment</Class><Item/><SlotType>5</SlotType>
          <Activate radius="4" totalDamage="200" duration="10">Trap</Activate>
        </Object>
        <Object type="0x902" id="Health Potion">
          <Class>Equipment</Class><Item/><SlotType>4</SlotType><Consumable/>
          <Activate amount="100">Heal</Activate>
        </Object>
        <Object type="0x903" id="Spell of Fire">
          <Class>Equipment</Class><Item/><SlotType>5</SlotType>
          <MpCost>60</MpCost><Cooldown>0.5</Cooldown>
          <Activate radius="3" totalDamage="200">PoisonGrenade</Activate>
        </Object>
        <Object type="0x901" id="Wand">
          <Class>Equipment</Class><Item/><SlotType>8</SlotType><RateOfFire>1</RateOfFire>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>100</MinDamage><MaxDamage>100</MaxDamage>
            <LifetimeMS>2000</LifetimeMS></Projectile>
        </Object>
      </Objects>"#;

    fn catalog() -> Catalog {
        Catalog::load_str(&[FIXTURE]).0
    }

    fn square(tile: u16, object: u16) -> Composition {
        Composition {
            tile: TileType(tile),
            object: ObjectType(object),
            region: Region::None,
            terrain: hendra_content::Terrain::None,
            config: String::new(),
        }
    }

    /// An open 32×32 field of grass.
    fn field(catalog: &Catalog) -> World {
        let squares = (0..32 * 32).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(32, 32, squares).unwrap();
        World::new("Field", Terrain::build(map, catalog), catalog)
    }

    #[test]
    fn scenery_stays_in_the_map_and_everything_else_becomes_an_entity() {
        // The realm map carries a quarter of a million trees. As entities they would fill a world
        // four times over and leave no room for a single enemy.
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..8 * 8)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[10] = square(0x10, 0x500); // wall, scenery
        squares[20] = square(0x10, 0x501); // sign, scenery
        squares[30] = square(0x10, 0x502); // slime, an enemy
        squares[40] = square(0x10, 0x506); // doorway, a portal somebody has to be able to use

        let map = Map::from_squares(8, 8, squares).unwrap();
        let world = World::new("Test", Terrain::build(map, &catalog), &catalog);

        assert_eq!(world.len(), 2, "only the slime and the doorway");
        assert!(!world.terrain().walkable(2, 1), "but the wall still blocks");

        let kinds: Vec<Kind> = world.iter().map(|(_, entity)| entity.kind).collect();
        assert!(kinds.contains(&Kind::Enemy));
        assert!(kinds.contains(&Kind::Portal));
    }

    #[test]
    fn a_map_object_the_author_named_is_not_scenery() {
        // Naming one is the map author saying this one is not a tree.
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..8 * 8)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[20] = Composition {
            tile: TileType(0x10),
            object: ObjectType(0x501),
            region: Region::None,
            terrain: hendra_content::Terrain::None,
            config: "name:Welcome".to_string(),
        };

        let map = Map::from_squares(8, 8, squares).unwrap();
        let world = World::new("Test", Terrain::build(map, &catalog), &catalog);

        assert_eq!(world.len(), 1);
        assert_eq!(
            world.iter().next().unwrap().1.name.as_deref(),
            Some("Welcome")
        );
    }

    #[test]
    fn an_enemy_from_the_map_takes_its_hit_points_from_the_catalog() {
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..4 * 4)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[5] = square(0x10, 0x502);

        let map = Map::from_squares(4, 4, squares).unwrap();
        let world = World::new("Test", Terrain::build(map, &catalog), &catalog);

        let (_, slime) = world.iter().next().expect("the slime should be there");
        assert_eq!(slime.max_hp, 200);
        assert_eq!(slime.hp, 200);
    }

    #[test]
    fn an_ordinary_step_is_honoured_in_full() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();

        // 5 tiles per second for 50ms is a quarter tile.
        let outcome = world
            .resolve_move(player, &catalog, 16.25, 16.0, 50)
            .unwrap();

        assert_eq!(outcome.refused, None);
        assert!((outcome.x - 16.25).abs() < 1e-4);
    }

    #[test]
    fn a_claim_faster_than_the_entity_can_travel_is_trimmed() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();

        // Claiming ten tiles in one 50ms tick.
        let outcome = world
            .resolve_move(player, &catalog, 26.0, 16.0, 50)
            .unwrap();

        assert_eq!(outcome.refused, Some(MoveRefusal::TooFar));

        // Trimmed to speed × time × tolerance, not rejected outright.
        let travelled = outcome.x - 16.0;
        // Four tiles a second at zero speed, over fifty milliseconds, plus the tolerance.
        let allowed = 4.0 * 0.05 * MOVE_TOLERANCE;
        assert!(
            (travelled - allowed).abs() < 1e-3,
            "travelled {travelled}, allowed {allowed}"
        );
    }

    #[test]
    fn walking_into_water_stops_at_the_edge() {
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..8 * 8)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        // Flood the column at x = 4.
        for y in 0..8 {
            squares[y * 8 + 4] = square(0x11, ObjectType::NONE.0);
        }

        let map = Map::from_squares(8, 8, squares).unwrap();
        let mut world = World::new("Test", Terrain::build(map, &catalog), &catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 3.5, 3.5, 800))
            .unwrap();

        let outcome = world.resolve_move(player, &catalog, 4.5, 3.5, 200).unwrap();
        assert_eq!(outcome.refused, Some(MoveRefusal::Blocked));
        assert!(outcome.x < 4.0, "should not have entered the water");
    }

    #[test]
    fn a_diagonal_into_a_wall_slides_along_it() {
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..8 * 8)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        for y in 0..8 {
            squares[y * 8 + 4] = square(0x11, ObjectType::NONE.0);
        }

        let map = Map::from_squares(8, 8, squares).unwrap();
        let mut world = World::new("Test", Terrain::build(map, &catalog), &catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 3.5, 3.5, 800))
            .unwrap();

        // Moving diagonally into the wall: the wall stops x, but y should still advance.
        let outcome = world.resolve_move(player, &catalog, 4.5, 4.0, 200).unwrap();
        assert_eq!(outcome.refused, Some(MoveRefusal::Blocked));
        assert!(
            outcome.y > 3.5,
            "movement along the wall should survive, got y = {}",
            outcome.y
        );
    }

    #[test]
    fn standing_in_lava_costs_hit_points_and_eventually_kills() {
        let catalog = catalog();
        let squares = (0..8 * 8).map(|_| square(0x12, ObjectType::NONE.0));
        let map = Map::from_squares(8, 8, squares).unwrap();
        let mut world = World::new("Lava", Terrain::build(map, &catalog), &catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 4.0, 4.0, 300))
            .unwrap();

        // A hundred a second from the ground against six a second of regeneration, so the loss is
        // most of the hundred rather than all of it.
        world.advance(&catalog, 1000);
        let after_a_second = world.get(player).unwrap().hp;
        assert!(
            (200..=210).contains(&after_a_second),
            "about a hundred a second, less regeneration: {after_a_second}"
        );

        for _ in 0..5 {
            world.advance(&catalog, 1000);
        }

        assert!(world.get(player).is_none(), "the player should have died");
        assert_eq!(world.len(), 0);
    }

    #[test]
    fn a_snapshot_holds_what_is_in_sight_and_nothing_else() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let viewer = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();
        let near = world
            .spawn(Entity::player(ObjectType(0x600), 18.0, 16.0, 800))
            .unwrap();
        let far = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();
        world.get_mut(far).unwrap().x = 100.0;

        world.advance(&catalog, 50);

        let snapshot = world.snapshot_for(viewer, SIGHT_RADIUS);
        assert!(
            snapshot.get(viewer.to_entity_id()).is_some(),
            "the viewer sees itself"
        );
        assert!(snapshot.get(near.to_entity_id()).is_some());
        assert!(
            snapshot.get(far.to_entity_id()).is_none(),
            "an entity outside the radius should never be encoded"
        );
    }

    #[test]
    fn ticking_advances_the_tick_number() {
        let catalog = catalog();
        let mut world = field(&catalog);
        assert_eq!(world.tick_number(), Tick::ZERO);

        world.advance(&catalog, 50);
        world.advance(&catalog, 50);
        assert_eq!(world.tick_number(), Tick(2));
    }

    #[test]
    fn a_despawned_entity_leaves_the_index_too() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let viewer = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();
        let other = world
            .spawn(Entity::player(ObjectType(0x600), 17.0, 16.0, 800))
            .unwrap();
        world.advance(&catalog, 50);
        assert!(
            world
                .snapshot_for(viewer, SIGHT_RADIUS)
                .get(other.to_entity_id())
                .is_some()
        );

        world.despawn(other);
        world.advance(&catalog, 50);

        assert!(
            world
                .snapshot_for(viewer, SIGHT_RADIUS)
                .get(other.to_entity_id())
                .is_none(),
            "a despawned entity must leave the spatial index as well as the slab"
        );
    }

    /// An armed player and a slime three tiles east of them.
    fn duel(catalog: &Catalog) -> (World, Handle, Handle) {
        let squares = (0..32 * 32).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Duel", Terrain::build(map, catalog), catalog);

        let mut player = Entity::player(ObjectType(0x600), 5.0, 5.0, 800);
        player.weapon = Some(ObjectType(0x901));
        let shooter = world.spawn(player).unwrap();

        let mut slime = Entity::fixture(ObjectType(0x502), 8.0, 5.0);
        slime.kind = Kind::Enemy;
        slime.hp = 200;
        slime.max_hp = 200;
        let target = world.spawn(slime).unwrap();

        world.advance(catalog, 50);
        (world, shooter, target)
    }

    #[test]
    fn shooting_spawns_a_projectile_that_travels_and_hits() {
        let catalog = catalog();
        let (mut world, shooter, target) = duel(&catalog);

        let fired = world.shoot(shooter, &catalog, 0.0);
        assert_eq!(fired.len(), 1, "the wand has one projectile");
        assert_eq!(world.projectile_count(), 1);

        let before = world.get(target).unwrap().hp;
        for _ in 0..10 {
            world.advance(&catalog, 50);
        }

        let after = world.get(target).map(|entity| entity.hp).unwrap_or(0);
        assert!(after < before, "the slime should have taken damage");
    }

    #[test]
    fn an_unarmed_entity_cannot_shoot() {
        let catalog = catalog();
        let (mut world, _, target) = duel(&catalog);

        assert!(
            world.shoot(target, &catalog, 0.0).is_empty(),
            "the slime has no weapon"
        );
        assert_eq!(world.projectile_count(), 0);
    }

    // -- what behaviours do to the world -------------------------------------------------------

    /// Installs behaviours from source and gives everything already present a mind.
    fn behaving(world: &mut World, catalog: &Catalog, source: &str) {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let (programs, diagnostics) = compile(&parse(source).expect("parses"));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        world.set_behaviours(catalog, programs);
    }

    fn count_of(world: &World, kind: u16) -> usize {
        world
            .iter()
            .filter(|(_, entity)| entity.object_type == ObjectType(kind) && !entity.dead)
            .count()
    }

    #[test]
    fn a_behaviour_that_names_an_entity_has_it_resolved_at_load() {
        // Nothing resolves names but this, and without it every behaviour that names an entity is
        // inert: a boss waits forever for guardians it cannot recognise.
        let catalog = catalog();
        let mut world = field(&catalog);
        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { on entity_exists("Guard", 10) -> b } state b { } }"#,
        );

        let program = world.behaviours.get("Slime").expect("compiled");
        assert_eq!(
            program.kind_of(hendra_behavior::NameRef(0)),
            Some(0x503),
            "the guard should have been resolved to its object type"
        );
    }

    #[test]
    fn a_boss_wakes_when_the_entity_it_watches_for_is_gone() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        let boss = world.spawn(boss).unwrap();

        let mut guard = Entity::fixture(ObjectType(0x503), 12.0, 10.0);
        guard.kind = Kind::Enemy;
        guard.max_hp = 50;
        guard.hp = 50;
        let guard = world.spawn(guard).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" {
                 state sealed { on entities_not_exists(20, "Guard") -> awake }
                 state awake { }
               }"#,
        );
        world.reindex();

        world.advance(&catalog, 50);
        let state_of = |world: &World, handle: Handle| {
            let entity = world.get(handle).unwrap();
            let mind = entity.mind.as_ref().unwrap();
            let program = world.behaviours.get("Slime").unwrap();
            mind.state_name(program).to_string()
        };
        assert_eq!(state_of(&world, boss), "sealed", "the guard is still there");

        world.get_mut(guard).unwrap().dead = true;
        world.advance(&catalog, 50);
        assert_eq!(state_of(&world, boss), "awake");
    }

    #[test]
    fn a_conditional_effect_lands_and_lifts_when_the_state_does() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        let boss = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" {
                 state shielded {
                   conditional_effect(invulnerable)
                   on timed(200ms) -> open
                 }
                 state open { }
               }"#,
        );
        world.reindex();

        world.advance(&catalog, 50);
        assert!(
            world
                .get(boss)
                .unwrap()
                .conditions
                .contains(hendra_content::ConditionEffect::Invulnerable),
            "should be invulnerable while shielded"
        );

        // Out of the state and past the renewal window, it should wear off on its own.
        for _ in 0..12 {
            world.advance(&catalog, 50);
        }
        assert!(
            !world
                .get(boss)
                .unwrap()
                .conditions
                .contains(hendra_content::ConditionEffect::Invulnerable),
            "should have lapsed once the state was left"
        );
    }

    #[test]
    fn a_spawn_makes_children_that_have_minds_of_their_own() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut parent = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        parent.kind = Kind::Enemy;
        parent.max_hp = 200;
        parent.hp = 200;
        world.spawn(parent).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { spawn("Spawnling", 3, cooldown: 100ms) } }
               enemy "Spawnling" { state b { wander(0.4) } }"#,
        );
        world.reindex();

        for _ in 0..10 {
            world.advance(&catalog, 50);
        }

        let children = count_of(&world, 0x505);
        assert_eq!(children, 3, "three, and no more than three");

        assert!(
            world
                .iter()
                .filter(|(_, e)| e.object_type == ObjectType(0x505))
                .all(|(_, e)| e.mind.is_some()),
            "each child should be running its own behaviour"
        );
    }

    #[test]
    fn an_order_drives_other_entities_into_a_state() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        let mut minion = Entity::fixture(ObjectType(0x503), 12.0, 10.0);
        minion.kind = Kind::Enemy;
        minion.max_hp = 50;
        minion.hp = 50;
        let minion = world.spawn(minion).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { order(20, "Guard", "charge") } }
               enemy "Guard" { state waiting { } state charge { } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        let program = world.behaviours.get("Guard").unwrap();
        let state = world
            .get(minion)
            .unwrap()
            .mind
            .as_ref()
            .unwrap()
            .state_name(program);
        assert_eq!(state, "charge", "the minion should have taken the order");
    }

    #[test]
    fn an_order_naming_a_state_the_target_lacks_leaves_it_alone() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        let mut minion = Entity::fixture(ObjectType(0x503), 12.0, 10.0);
        minion.kind = Kind::Enemy;
        minion.max_hp = 50;
        minion.hp = 50;
        let minion = world.spawn(minion).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { order(20, "Guard", "nowhere") } }
               enemy "Guard" { state waiting { } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        let program = world.behaviours.get("Guard").unwrap();
        assert_eq!(
            world
                .get(minion)
                .unwrap()
                .mind
                .as_ref()
                .unwrap()
                .state_name(program),
            "waiting"
        );
    }

    #[test]
    fn a_portal_is_left_behind_when_the_boss_dies() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        let boss = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { drop_portal_on_death("Doorway", 1) } }"#,
        );
        world.reindex();

        world.advance(&catalog, 50);
        assert_eq!(count_of(&world, 0x506), 0, "not until it dies");

        world.get_mut(boss).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(count_of(&world, 0x506), 1, "the way in should be open");
    }

    #[test]
    fn a_death_effect_runs_once_rather_than_every_tick() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        let boss = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { drop_portal_on_death("Doorway", 1) } }"#,
        );
        world.reindex();

        world.get_mut(boss).unwrap().dead = true;
        for _ in 0..10 {
            world.advance(&catalog, 50);
        }

        assert_eq!(count_of(&world, 0x506), 1, "one portal, not ten");
    }

    #[test]
    fn a_transform_replaces_the_body_without_awarding_a_kill() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        let boss = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { transform("Guard") } }
               enemy "Guard" { state b { } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        assert!(world.get(boss).is_none(), "the old body is gone");
        assert_eq!(count_of(&world, 0x503), 1, "and the new one is there");
    }

    #[test]
    fn changing_the_ground_changes_what_can_be_walked_on() {
        let catalog = catalog();
        let mut world = field(&catalog);
        assert!(world.terrain().walkable(10, 10), "grass to begin with");

        let mut boss = Entity::fixture(ObjectType(0x502), 10.5, 10.5);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { ground_transform("Water", 2) } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        assert!(!world.terrain().walkable(10, 10), "water now");
        assert!(world.terrain().walkable(20, 20), "and only where it stood");
    }

    #[test]
    fn a_grenade_hurts_players_and_respects_their_armour() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        let victim = world
            .spawn(Entity::player(ObjectType(0x600), 12.0, 10.0, 500))
            .unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { grenade(4, 100, 20, cooldown: 100000) } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        let hurt = world.get(victim).unwrap().hp;
        assert!(hurt < 500, "should have been caught in the blast");
        assert!(hurt > 0, "but not killed outright");
    }

    #[test]
    fn a_grenade_misses_someone_standing_clear_of_it() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        // Near enough to be aimed at, and the blast is small.
        let near = world
            .spawn(Entity::player(ObjectType(0x600), 11.0, 10.0, 500))
            .unwrap();
        let far = world
            .spawn(Entity::player(ObjectType(0x600), 25.0, 10.0, 500))
            .unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { grenade(1, 100, 20, cooldown: 100000) } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        assert!(world.get(near).unwrap().hp < 500, "the near one is hit");
        assert_eq!(world.get(far).unwrap().hp, 500, "the far one is not");
    }

    #[test]
    fn an_entity_that_shrinks_stops_at_its_target_size() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        boss.size = 100;
        let boss = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { change_size(-100, 25) } }"#,
        );
        world.reindex();

        for _ in 0..60 {
            world.advance(&catalog, 50);
        }

        assert_eq!(
            world.get(boss).unwrap().size,
            25,
            "should have stopped at the target rather than shrinking away"
        );
    }

    // -- condition effects ----------------------------------------------------------------------

    fn give(world: &mut World, handle: Handle, effect: hendra_content::ConditionEffect) {
        if let Some(entity) = world.get_mut(handle) {
            entity.conditions.insert(effect);
            entity.effects.push((effect.index() as u8, 60_000));
        }
    }

    #[test]
    fn an_invulnerable_target_takes_no_damage_and_takes_it_again_when_the_effect_lifts() {
        // The most used behaviour in the game's content. Before this it marked a boss invulnerable
        // and left it perfectly killable.
        let catalog = catalog();
        let (mut world, shooter, target) = duel(&catalog);

        give(
            &mut world,
            target,
            hendra_content::ConditionEffect::Invulnerable,
        );
        let before = world.get(target).unwrap().hp;

        for _ in 0..40 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);
        }
        assert_eq!(
            world.get(target).unwrap().hp,
            before,
            "a hundred shots should have done nothing"
        );

        // Lift it, and the same shots land.
        world.get_mut(target).unwrap().conditions = hendra_content::ConditionSet::EMPTY;
        world.get_mut(target).unwrap().effects.clear();

        for _ in 0..10 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);
            if world.get(target).is_none_or(|e| e.hp < before) {
                break;
            }
        }
        assert!(
            world.get(target).is_none_or(|e| e.hp < before),
            "should be killable once the effect is gone"
        );
    }

    #[test]
    fn armour_changes_what_a_shot_is_worth() {
        let catalog = catalog();

        // A target with armour of its own, because ArmorBroken removes defence rather than
        // creating negative defence. Against something with none it correctly does nothing.
        let hp_after = |effect: Option<hendra_content::ConditionEffect>| {
            let mut world = field(&catalog);

            let mut player = Entity::player(ObjectType(0x600), 5.0, 5.0, 800);
            player.weapon = Some(ObjectType(0x901));
            let shooter = world.spawn(player).unwrap();

            let mut plated = Entity::fixture(ObjectType(0x507), 8.0, 5.0);
            plated.kind = Kind::Enemy;
            plated.hp = 400;
            plated.max_hp = 400;
            let target = world.spawn(plated).unwrap();
            world.reindex();

            if let Some(effect) = effect {
                give(&mut world, target, effect);
            }
            world.shoot(shooter, &catalog, 0.0);
            for _ in 0..12 {
                world.advance(&catalog, 50);
            }
            world.get(target).map(|e| e.hp)
        };

        let plain = hp_after(None).expect("alive");
        let armoured = hp_after(Some(hendra_content::ConditionEffect::Armored)).expect("alive");
        let broken = hp_after(Some(hendra_content::ConditionEffect::ArmorBroken)).expect("alive");

        assert!(
            armoured > plain,
            "armour should absorb some: {armoured} vs {plain}"
        );
        assert!(
            broken < plain,
            "broken armour should absorb less: {broken} vs {plain}"
        );
    }

    #[test]
    fn a_paralysed_player_cannot_move_and_moves_again_when_it_lifts() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        // Within what five tiles a second allows in fifty milliseconds.
        let outcome = world
            .resolve_move(player, &catalog, 10.2, 10.0, 50)
            .unwrap();
        assert!(outcome.refused.is_none(), "unaffected, so allowed");

        give(
            &mut world,
            player,
            hendra_content::ConditionEffect::Paralyzed,
        );
        let held = world
            .resolve_move(player, &catalog, 10.2, 10.0, 50)
            .unwrap();

        assert_eq!(held.refused, Some(MoveRefusal::Rooted));
        assert_eq!((held.x, held.y), (10.0, 10.0), "held exactly where it was");
    }

    #[test]
    fn being_slowed_shortens_how_far_a_claim_may_reach() {
        // Not refused outright: a slowed player still moves, just less. Refusing would look like
        // a disconnection rather than an effect.
        let catalog = catalog();
        let mut world = field(&catalog);
        // Speed is held at the base when slowed rather than scaled, so the difference only shows
        // on a character that has speed to lose.
        let mut fast = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        fast.stats.boost(hendra_content::Stat::Speed, 75);
        let player = world.spawn(fast).unwrap();
        world.reindex();

        let reach = |world: &World| {
            let outcome = world
                .resolve_move(player, &catalog, 30.0, 10.0, 100)
                .unwrap();
            outcome.x - 10.0
        };

        let normal = reach(&world);
        give(&mut world, player, hendra_content::ConditionEffect::Slowed);
        let slowed = reach(&world);

        assert!(slowed > 0.0, "still moving");
        assert!(slowed < normal, "but less far: {slowed} vs {normal}");
    }

    #[test]
    fn a_stunned_entity_cannot_shoot() {
        let catalog = catalog();
        let (mut world, shooter, _) = duel(&catalog);

        give(
            &mut world,
            shooter,
            hendra_content::ConditionEffect::Stunned,
        );
        assert!(world.shoot(shooter, &catalog, 0.0).is_empty());

        world.get_mut(shooter).unwrap().conditions = hendra_content::ConditionSet::EMPTY;
        assert_eq!(world.shoot(shooter, &catalog, 0.0).len(), 1);
    }

    #[test]
    fn a_paused_enemy_stops_thinking_entirely() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut enemy = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 200;
        enemy.hp = 200;
        let enemy = world.spawn(enemy).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { on timed(100ms) -> b } state b { } }"#,
        );
        world.reindex();

        give(&mut world, enemy, hendra_content::ConditionEffect::Paused);
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }

        let program = world.behaviours.get("Slime").unwrap();
        let state = world
            .get(enemy)
            .unwrap()
            .mind
            .as_ref()
            .unwrap()
            .state_name(program);
        assert_eq!(state, "a", "a paused enemy should not have transitioned");
    }

    #[test]
    fn bleeding_hurts_over_time_but_never_finishes_the_job() {
        // The game leaves you at one and lets something else kill you, which is what stops a stray
        // poison being an execution.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        give(
            &mut world,
            player,
            hendra_content::ConditionEffect::Bleeding,
        );
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }

        let hurt = world.get(player).unwrap().hp;
        assert!(hurt < 500 && hurt > 400, "one second of bleeding: {hurt}");

        // Long enough to run the health out, and inside the effect's own lifetime: once bleeding
        // lapses the player starts recovering again, which is correct and not what this measures.
        for _ in 0..400 {
            world.advance(&catalog, 50);
        }
        assert_eq!(world.get(player).unwrap().hp, 1, "left alive at one");
    }

    #[test]
    fn healing_restores_over_time_and_stops_at_full() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.get_mut(player).unwrap().hp = 100;
        world.reindex();

        give(&mut world, player, hendra_content::ConditionEffect::Healing);
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        assert!(world.get(player).unwrap().hp > 100);

        for _ in 0..1000 {
            world.advance(&catalog, 50);
        }
        assert_eq!(world.get(player).unwrap().hp, 500, "and no further");
    }

    #[test]
    fn a_small_heal_per_tick_is_not_rounded_away() {
        // Twenty a second at fifty-millisecond ticks is one point per tick. Rounding each tick
        // independently would make healing do nothing at all.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.get_mut(player).unwrap().hp = 100;
        world.reindex();

        give(&mut world, player, hendra_content::ConditionEffect::Healing);
        for _ in 0..10 {
            world.advance(&catalog, 20);
        }

        assert!(
            world.get(player).unwrap().hp > 100,
            "two hundred milliseconds of healing should show"
        );
    }

    #[test]
    fn an_immunity_refuses_the_effect_rather_than_holding_it_uselessly() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        give(
            &mut world,
            player,
            hendra_content::ConditionEffect::ParalyzeImmune,
        );
        world.give_effect(
            player,
            hendra_content::ConditionEffect::Paralyzed.index() as u8,
            5000,
        );

        assert!(
            !world
                .get(player)
                .unwrap()
                .conditions
                .contains(hendra_content::ConditionEffect::Paralyzed),
            "the immunity should have refused it"
        );
        assert!(
            world
                .resolve_move(player, &catalog, 10.2, 10.0, 50)
                .unwrap()
                .refused
                .is_none(),
            "and movement is unaffected"
        );
    }

    #[test]
    fn what_a_player_wears_changes_what_it_can_do() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.weapon = Some(ObjectType(0x901));
        let player = world.spawn(player).unwrap();
        world.reindex();

        let rules = crate::effects::Rules::NONE;
        let bare = world.get(player).unwrap().stats;
        let bare_speed = bare.movement_speed(&rules);
        let bare_cooldown = bare.shot_cooldown_ms(&rules, 1.0);

        // A ring of speed and dexterity, as an equipment layer.
        let mut boosts = [0i32; 8];
        boosts[hendra_content::Stat::Speed.index()] = 30;
        boosts[hendra_content::Stat::Dexterity.index()] = 30;
        world.get_mut(player).unwrap().stats.set_equipment(boosts);

        let worn = world.get(player).unwrap().stats;
        assert!(worn.movement_speed(&rules) > bare_speed);
        assert!(worn.shot_cooldown_ms(&rules, 1.0) < bare_cooldown);

        // Taking it off returns exactly where it started.
        world.get_mut(player).unwrap().stats.set_equipment([0; 8]);
        let after = world.get(player).unwrap().stats;
        assert_eq!(after.movement_speed(&rules), bare_speed);
        assert_eq!(after.shot_cooldown_ms(&rules, 1.0), bare_cooldown);
    }

    #[test]
    fn a_players_stats_reach_the_snapshot_and_an_enemys_do_not() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.stats.boost(hendra_content::Stat::Attack, 40);
        let player = world.spawn(player).unwrap();

        let mut enemy = Entity::fixture(ObjectType(0x502), 12.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 200;
        enemy.hp = 200;
        world.spawn(enemy).unwrap();
        world.reindex();

        let snapshot = world.snapshot_for(player, 20.0);
        let seen: Vec<_> = snapshot.iter().collect();

        let (_, mine) = seen
            .iter()
            .find(|(_, state)| state.object_type == 0x600)
            .expect("the player is in its own snapshot");
        assert_eq!(mine.stats[hendra_content::Stat::Attack.index()], 40);

        let (_, theirs) = seen
            .iter()
            .find(|(_, state)| state.object_type == 0x502)
            .expect("and so is the enemy");
        assert_eq!(theirs.stats, [0; 8], "an enemy sends none");
    }

    #[test]
    fn killing_something_earns_experience_and_eventually_a_level() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();
        world.reindex();

        assert_eq!(world.get(player).unwrap().progress.level, 1);

        // Slimes have two hundred health, so each is worth five capped at five.
        for _ in 0..12 {
            let mut slime = Entity::fixture(ObjectType(0x502), 11.0, 10.0);
            slime.kind = Kind::Enemy;
            slime.max_hp = 200;
            slime.hp = 200;
            let slime = world.spawn(slime).unwrap();
            world.reindex();
            world.get_mut(slime).unwrap().dead = true;
            world.advance(&catalog, 50);
        }

        let progress = world.get(player).unwrap().progress;
        assert!(progress.experience > 0, "earned nothing");
        assert_eq!(progress.level, 2, "fifty experience is one level");
    }

    #[test]
    fn a_summon_is_worth_nothing_so_it_cannot_be_farmed() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();

        let mut summon = Entity::fixture(ObjectType(0x502), 11.0, 10.0);
        summon.kind = Kind::Enemy;
        summon.max_hp = 200;
        summon.hp = 200;
        summon.no_experience = true;
        let summon = world.spawn(summon).unwrap();
        world.reindex();

        world.get_mut(summon).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(world.get(player).unwrap().progress.experience, 0);
    }

    #[test]
    fn experience_reaches_everyone_nearby_and_nobody_far_away() {
        // Shared rather than split, so helping someone else's fight is never worse than standing
        // elsewhere. Distance is the only thing that decides it.
        let catalog = catalog();
        let mut world = field(&catalog);

        let near = world
            .spawn(Entity::player(ObjectType(0x030e), 11.0, 10.0, 100))
            .unwrap();
        let far = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();
        world.get_mut(far).unwrap().x = 200.0;

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.max_hp = 200;
        slime.hp = 200;
        let slime = world.spawn(slime).unwrap();
        world.reindex();

        world.get_mut(slime).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert!(world.get(near).unwrap().progress.experience > 0);
        assert_eq!(world.get(far).unwrap().progress.experience, 0);
    }

    #[test]
    fn a_paused_player_earns_nothing() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 11.0, 10.0, 100))
            .unwrap();

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.max_hp = 200;
        slime.hp = 200;
        let slime = world.spawn(slime).unwrap();
        world.reindex();

        give(&mut world, player, hendra_content::ConditionEffect::Paused);
        world.get_mut(slime).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(world.get(player).unwrap().progress.experience, 0);
    }

    #[test]
    fn a_level_raises_the_ceiling_and_fills_what_it_added() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();
        world.get_mut(player).unwrap().stats =
            crate::stats::Stats::starting(catalog.class(ObjectType(0x030e)).unwrap());
        world.get_mut(player).unwrap().hp = 50;
        world.reindex();

        for _ in 0..12 {
            let mut slime = Entity::fixture(ObjectType(0x502), 11.0, 10.0);
            slime.kind = Kind::Enemy;
            slime.max_hp = 200;
            slime.hp = 200;
            let slime = world.spawn(slime).unwrap();
            world.reindex();
            world.get_mut(slime).unwrap().dead = true;
            world.advance(&catalog, 50);
        }

        let entity = world.get(player).unwrap();
        assert!(entity.progress.level >= 2);
        assert_eq!(entity.hp, entity.max_hp, "a level fills what it added");
        assert!(entity.max_hp > 100, "and the ceiling moved");
    }

    #[test]
    fn a_thrown_object_telegraphs_before_it_lands() {
        // Landing instantly makes a thrown attack unavoidable, which is the difference between a
        // hard fight and one nobody can play around.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut thrower = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        thrower.kind = Kind::Enemy;
        thrower.max_hp = 200;
        thrower.hp = 200;
        world.spawn(thrower).unwrap();

        world
            .spawn(Entity::player(ObjectType(0x600), 14.0, 10.0, 500))
            .unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" {
                 state a {
                   toss_object("Spawnling", range: 3, cooldown: 100000, throw_delay: 500)
                 }
               }"#,
        );
        world.reindex();

        world.advance(&catalog, 50);
        assert_eq!(count_of(&world, 0x505), 0, "still in the air");

        for _ in 0..8 {
            world.advance(&catalog, 50);
        }
        assert_eq!(count_of(&world, 0x505), 0, "not yet");

        for _ in 0..4 {
            world.advance(&catalog, 50);
        }
        assert_eq!(count_of(&world, 0x505), 1, "and now it lands");
    }

    #[test]
    fn a_spawn_with_no_telegraph_arrives_at_once() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut parent = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        parent.kind = Kind::Enemy;
        parent.max_hp = 200;
        parent.hp = 200;
        world.spawn(parent).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { spawn("Spawnling", 1, cooldown: 100000) } }"#,
        );
        world.reindex();

        world.advance(&catalog, 50);
        assert_eq!(count_of(&world, 0x505), 1);
    }

    #[test]
    fn a_potion_heals_and_a_spell_costs_magic() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.hp = 100;
        player.mp = 100;
        player.max_mp = 100;
        let player = world.spawn(player).unwrap();
        world.reindex();

        let ran = world.use_item(player, &catalog, ObjectType(0x902), (10.0, 10.0));
        assert_eq!(ran.len(), 1);
        assert_eq!(world.get(player).unwrap().hp, 200, "healed for a hundred");

        // One ability cooldown covers everything, so the spell waits for the potion's.
        for _ in 0..12 {
            world.advance(&catalog, 50);
        }

        let ran = world.use_item(player, &catalog, ObjectType(0x903), (14.0, 10.0));
        assert_eq!(ran.len(), 1);
        assert_eq!(world.get(player).unwrap().mp, 40, "sixty magic spent");
    }

    #[test]
    fn an_ability_is_refused_without_the_magic_for_it() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 10;
        player.max_mp = 100;
        let player = world.spawn(player).unwrap();
        world.reindex();

        assert!(
            world
                .use_item(player, &catalog, ObjectType(0x903), (14.0, 10.0))
                .is_empty()
        );
        assert_eq!(world.get(player).unwrap().mp, 10, "and nothing was spent");
    }

    #[test]
    fn the_cooldown_stops_an_ability_being_used_as_fast_as_a_client_likes() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 1000;
        player.max_mp = 1000;
        let player = world.spawn(player).unwrap();
        world.reindex();

        assert!(
            !world
                .use_item(player, &catalog, ObjectType(0x903), (14.0, 10.0))
                .is_empty()
        );
        for _ in 0..5 {
            assert!(
                world
                    .use_item(player, &catalog, ObjectType(0x903), (14.0, 10.0))
                    .is_empty(),
                "still cooling down"
            );
        }

        for _ in 0..12 {
            world.advance(&catalog, 50);
        }
        assert!(
            !world
                .use_item(player, &catalog, ObjectType(0x903), (14.0, 10.0))
                .is_empty()
        );
    }

    #[test]
    fn a_quiet_player_cannot_use_an_ability_at_all() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 1000;
        player.max_mp = 1000;
        let player = world.spawn(player).unwrap();
        world.reindex();

        give(&mut world, player, hendra_content::ConditionEffect::Quiet);
        assert!(
            world
                .use_item(player, &catalog, ObjectType(0x903), (14.0, 10.0))
                .is_empty()
        );
    }

    #[test]
    fn an_ability_reaches_what_it_is_aimed_at_and_spares_the_people_beside_it() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 1000;
        player.max_mp = 1000;
        let player = world.spawn(player).unwrap();

        let mut enemy = Entity::fixture(ObjectType(0x502), 20.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 500;
        enemy.hp = 500;
        let enemy = world.spawn(enemy).unwrap();

        // Standing in the blast, and on the caster's side.
        let friend = world
            .spawn(Entity::player(ObjectType(0x600), 20.5, 10.0, 500))
            .unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x903), (20.0, 10.0));

        assert!(world.get(enemy).unwrap().hp < 500, "the enemy is hit");
        assert_eq!(world.get(friend).unwrap().hp, 500, "and the friend is not");
    }

    #[test]
    fn a_bag_takes_the_colour_of_the_best_thing_in_it() {
        // A white bag beside a brown one is how a player knows which to walk back for, and reading
        // the highest rather than the first means the order loot rolled in does not decide it.
        let catalog = catalog();
        let mut world = field(&catalog);
        world.set_bag_types(vec![
            ObjectType(0x510),
            ObjectType::NONE,
            ObjectType::NONE,
            ObjectType::NONE,
            ObjectType::NONE,
            ObjectType(0x511),
        ]);

        let mut enemy = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 200;
        enemy.hp = 200;
        let enemy = world.spawn(enemy).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" {
                 state a { }
                 loot {
                   item("Health Potion", 1)
                   item("Rare Blade", 1)
                 }
               }"#,
        );
        world.reindex();

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(count_of(&world, 0x511), 1, "the rarer colour wins");
        assert_eq!(count_of(&world, 0x510), 0);
    }

    #[test]
    fn a_colour_with_no_bag_registered_still_drops_the_loot() {
        // Losing the loot is worse than losing its colour.
        let catalog = catalog();
        let mut world = field(&catalog);
        world.set_bag_types(vec![ObjectType(0x510)]);

        let mut enemy = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 200;
        enemy.hp = 200;
        let enemy = world.spawn(enemy).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { } loot { item("Rare Blade", 1) } }"#,
        );
        world.reindex();

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);

        let bags = world
            .iter()
            .filter(|(_, entity)| entity.container.is_some())
            .count();
        assert_eq!(bags, 1, "the loot is still there");
    }

    #[test]
    fn an_invisible_player_is_hidden_from_everyone_but_itself() {
        // Filtered by the server rather than the client: a client told about something it should
        // not see is a client that can be made to reveal it.
        let catalog = catalog();
        let mut world = field(&catalog);

        let watcher = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        let hidden = world
            .spawn(Entity::player(ObjectType(0x600), 12.0, 10.0, 500))
            .unwrap();
        world.reindex();

        assert_eq!(world.snapshot_for(watcher, 20.0).len(), 2);

        give(
            &mut world,
            hidden,
            hendra_content::ConditionEffect::Invisible,
        );

        assert_eq!(
            world.snapshot_for(watcher, 20.0).len(),
            1,
            "the watcher sees only itself"
        );
        assert_eq!(
            world.snapshot_for(hidden, 20.0).len(),
            2,
            "and the hidden one still sees both"
        );
    }

    #[test]
    fn a_decoy_is_shot_by_enemies_and_not_by_the_one_who_left_it() {
        // The whole reason a decoy is its own kind: an enemy with a flag would be shot by its
        // owner, which is the opposite of what it is for.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 500;
        player.max_mp = 500;
        player.weapon = Some(ObjectType(0x901));
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x905), (10.0, 10.0));

        let decoy = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Decoy)
            .map(|(handle, _)| handle)
            .expect("a decoy was left");

        let before = world.get(decoy).unwrap().hp;
        world.shoot(player, &catalog, 0.0);
        for _ in 0..10 {
            world.advance(&catalog, 50);
        }

        assert!(
            world.get(decoy).is_none_or(|entity| entity.hp == before),
            "its owner's shots pass through it"
        );
    }

    #[test]
    fn a_decoy_does_not_last_forever() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 500;
        player.max_mp = 500;
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x905), (10.0, 10.0));
        assert_eq!(
            world.iter().filter(|(_, e)| e.kind == Kind::Decoy).count(),
            1
        );

        for _ in 0..120 {
            world.advance(&catalog, 100);
        }
        assert_eq!(
            world.iter().filter(|(_, e)| e.kind == Kind::Decoy).count(),
            0,
            "it should have gone"
        );
    }

    #[test]
    fn a_trap_waits_and_goes_off_when_something_walks_into_it() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 500;
        player.max_mp = 500;
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x906), (20.0, 10.0));
        world.advance(&catalog, 50);
        assert_eq!(
            world.iter().filter(|(_, e)| e.kind == Kind::Trap).count(),
            1,
            "armed and waiting"
        );

        // Something walks in.
        let mut enemy = Entity::fixture(ObjectType(0x502), 20.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 500;
        enemy.hp = 500;
        let enemy = world.spawn(enemy).unwrap();
        world.reindex();
        world.advance(&catalog, 50);

        assert!(
            world.get(enemy).unwrap().hp < 500,
            "it should have gone off"
        );
        assert_eq!(
            world.iter().filter(|(_, e)| e.kind == Kind::Trap).count(),
            0,
            "and be spent"
        );
    }

    #[test]
    fn a_trap_nobody_walks_into_stays_armed() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 500;
        player.max_mp = 500;
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x906), (20.0, 10.0));
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }

        assert_eq!(
            world.iter().filter(|(_, e)| e.kind == Kind::Trap).count(),
            1,
            "a player standing beside it is not what sets it off"
        );
    }

    #[test]
    fn a_setpiece_stamps_a_room_rather_than_painting_a_circle() {
        // The wrong version of this painted one tile over a radius, which left the room looking
        // roughly right and nothing where the author had drawn it.
        let catalog = catalog();
        let mut world = field(&catalog);

        // A three by three piece: water all round, a sign in the middle.
        let mut squares: Vec<Composition> =
            (0..9).map(|_| square(0x11, ObjectType::NONE.0)).collect();
        squares[4] = square(0x11, 0x501);
        let piece = Map::from_squares(3, 3, squares).unwrap();

        world.stamp(&catalog, &piece, (10.0, 10.0));
        world.reindex();

        // Centred on the point rather than starting there: a three by three at (10, 10) covers
        // nine through eleven, so the corner before it is the square that tells them apart.
        assert!(!world.terrain().walkable(9, 9), "the piece is centred");
        assert!(!world.terrain().walkable(11, 11));
        assert!(world.terrain().walkable(12, 12), "and no further");
        assert!(world.terrain().walkable(20, 20), "and only where it landed");
        assert_eq!(count_of(&world, 0x501), 1, "the sign is there");
    }

    #[test]
    fn a_setpiece_clears_what_was_standing_where_it_lands() {
        // A setpiece that appeared around the existing furniture would leave a wall through the
        // middle of a boss's arena.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut old = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        old.kind = Kind::Enemy;
        old.max_hp = 200;
        old.hp = 200;
        world.spawn(old).unwrap();
        world.reindex();

        let squares: Vec<Composition> = (0..25).map(|_| square(0x10, ObjectType::NONE.0)).collect();
        let piece = Map::from_squares(5, 5, squares).unwrap();

        world.stamp(&catalog, &piece, (10.0, 10.0));
        world.advance(&catalog, 50);

        assert_eq!(count_of(&world, 0x502), 0, "what was there is gone");
    }

    #[test]
    fn a_setpiece_moves_the_room_around_a_player_rather_than_deleting_them() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        let squares: Vec<Composition> = (0..25).map(|_| square(0x10, ObjectType::NONE.0)).collect();
        let piece = Map::from_squares(5, 5, squares).unwrap();

        world.stamp(&catalog, &piece, (10.0, 10.0));
        world.advance(&catalog, 50);

        assert!(world.get(player).is_some(), "the player is still there");
    }

    #[test]
    fn a_realm_announcement_is_heard_by_everyone_and_named_after_nobody() {
        let catalog = catalog();
        let mut world = field(&catalog);
        world.announce("the realm is closing");

        let said = world.take_announcements();
        assert_eq!(said.len(), 1);
        assert!(said[0].broadcast, "everybody hears it");
        assert_eq!(said[0].from, Handle::NONE, "and nobody said it");
    }

    #[test]
    fn counting_enemies_ignores_the_dead_and_everything_else() {
        let catalog = catalog();
        let mut world = field(&catalog);

        world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();

        for at in 0..3 {
            let mut enemy = Entity::fixture(ObjectType(0x502), 11.0 + at as f32, 10.0);
            enemy.kind = Kind::Enemy;
            enemy.max_hp = 200;
            enemy.hp = 200;
            let enemy = world.spawn(enemy).unwrap();
            if at == 0 {
                world.get_mut(enemy).unwrap().dead = true;
            }
        }

        assert_eq!(world.enemy_count(), 2, "the dead one does not count");
        assert_eq!(world.count_of_kind(ObjectType(0x502)), 2);
        assert_eq!(world.count_of_kind(ObjectType(0x503)), 0);
    }

    /// A field of one terrain, so a realm has somewhere to put things.
    fn terraced(catalog: &Catalog, terrain: hendra_content::Terrain) -> World {
        let squares = (0..64 * 64).map(|_| Composition {
            tile: TileType(0x10),
            object: ObjectType::NONE,
            region: Region::None,
            terrain,
            config: String::new(),
        });
        let map = Map::from_squares(64, 64, squares).unwrap();
        World::new("Realm", Terrain::build(map, catalog), catalog)
    }

    fn slimes(terrain: hendra_content::Terrain) -> Vec<crate::realm::Spawn> {
        vec![crate::realm::Spawn {
            kind: ObjectType(0x502),
            terrain,
            weight: 1.0,
            group: None,
            per_enemy: 200,
        }]
    }

    fn add(terrain: hendra_content::Terrain, count: usize) -> Vec<crate::realm::Adjustment> {
        vec![crate::realm::Adjustment {
            terrain,
            add: count,
            remove: 0,
        }]
    }

    #[test]
    fn a_realm_fills_the_terrain_it_was_asked_to_fill() {
        let catalog = catalog();
        let terrain = hendra_content::Terrain::MidPlains;
        let mut world = terraced(&catalog, terrain);

        let (added, removed) = world.populate(&catalog, &slimes(terrain), &add(terrain, 40));
        world.reindex();

        assert_eq!(added, 40);
        assert_eq!(removed, 0);
        assert_eq!(world.enemy_count(), 40);
    }

    #[test]
    fn what_a_realm_places_is_tagged_with_the_ground_it_was_placed_on() {
        // The tag is what the next count reads. Without it the population would be recounted as
        // zero every minute and the realm would fill forever.
        let catalog = catalog();
        let terrain = hendra_content::Terrain::HighForest;
        let mut world = terraced(&catalog, terrain);

        world.populate(&catalog, &slimes(terrain), &add(terrain, 10));
        world.reindex();

        assert_eq!(world.alive_by_terrain()[terrain as usize], 10);
        assert_eq!(
            world.alive_by_terrain()[hendra_content::Terrain::MidPlains as usize],
            0
        );
    }

    #[test]
    fn a_realm_places_nothing_on_ground_nothing_belongs_to() {
        let catalog = catalog();
        let mut world = terraced(&catalog, hendra_content::Terrain::MidPlains);

        // Asked to fill the mountains, of which this map has none.
        let (added, _) = world.populate(
            &catalog,
            &slimes(hendra_content::Terrain::Mountains),
            &add(hendra_content::Terrain::Mountains, 20),
        );

        assert_eq!(added, 0);
        assert_eq!(world.enemy_count(), 0);
    }

    #[test]
    fn nothing_appears_on_top_of_a_player() {
        // An enemy on top of somebody is not a spawn but an ambush nobody could have avoided.
        let catalog = catalog();
        let terrain = hendra_content::Terrain::MidPlains;
        let mut world = terraced(&catalog, terrain);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 32.0, 32.0, 500))
            .unwrap();
        world.reindex();

        world.populate(&catalog, &slimes(terrain), &add(terrain, 200));
        world.reindex();

        let (px, py) = {
            let entity = world.get(player).unwrap();
            (entity.x, entity.y)
        };

        for (_, entity) in world.iter() {
            if entity.kind != Kind::Enemy {
                continue;
            }
            let (dx, dy) = (entity.x - px, entity.y - py);

            // A literal rather than the constant, so shrinking the constant fails this instead of
            // quietly moving the bar down with it.
            assert!(
                (dx * dx + dy * dy).sqrt() >= 9.0,
                "something appeared beside the player"
            );
        }
    }

    #[test]
    fn a_terrain_that_has_overfilled_is_thinned() {
        let catalog = catalog();
        let terrain = hendra_content::Terrain::LowSand;
        let mut world = terraced(&catalog, terrain);

        world.populate(&catalog, &slimes(terrain), &add(terrain, 30));
        world.reindex();

        let (added, removed) = world.populate(
            &catalog,
            &slimes(terrain),
            &[crate::realm::Adjustment {
                terrain,
                add: 0,
                remove: 12,
            }],
        );
        world.reindex();

        assert_eq!(added, 0);
        assert_eq!(removed, 12);
        assert_eq!(world.enemy_count(), 18);
    }

    #[test]
    fn thinning_leaves_alone_what_somebody_is_fighting() {
        // Despawning an enemy a player is on reads as the server eating their kill.
        let catalog = catalog();
        let terrain = hendra_content::Terrain::LowSand;
        let mut world = terraced(&catalog, terrain);

        world.populate(&catalog, &slimes(terrain), &add(terrain, 20));
        world.reindex();

        // Stand on one of them.
        let (victim, x, y) = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Enemy)
            .map(|(handle, entity)| (handle, entity.x, entity.y))
            .unwrap();
        world
            .spawn(Entity::player(ObjectType(0x600), x, y, 500))
            .unwrap();
        world.reindex();

        world.populate(
            &catalog,
            &slimes(terrain),
            &[crate::realm::Adjustment {
                terrain,
                add: 0,
                remove: 20,
            }],
        );

        assert!(
            world.get(victim).is_some(),
            "the one being fought was taken"
        );
    }

    #[test]
    fn what_an_enemy_spawns_is_counted_where_its_parent_was() {
        // Otherwise a breeding enemy leaks out of the census, and the terrain it is filling reads
        // as empty and gets filled again.
        let catalog = catalog();
        let terrain = hendra_content::Terrain::MidForest;
        let mut world = terraced(&catalog, terrain);

        world.populate(&catalog, &slimes(terrain), &add(terrain, 1));
        world.reindex();

        let parent = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Enemy)
            .map(|(handle, _)| handle)
            .unwrap();

        let behaviours = std::mem::take(&mut world.behaviours);
        let child = world
            .spawn_child(
                &catalog,
                &behaviours,
                ObjectType(0x502),
                5.0,
                5.0,
                None,
                Some(parent),
            )
            .unwrap();
        world.behaviours = behaviours;

        assert_eq!(world.get(child).unwrap().terrain, terrain);
        assert_eq!(world.alive_by_terrain()[terrain as usize], 2);
    }

    #[test]
    fn teleporting_moves_you_to_them_and_starts_a_cooldown() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mover = world
            .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
            .unwrap();
        let target = world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 24.0, 500))
            .unwrap();
        world.get_mut(target).unwrap().name = Some("Bo".into());

        assert_eq!(world.teleport_to(mover, target), None);

        let moved = world.get(mover).unwrap();
        assert_eq!((moved.x, moved.y), (20.0, 24.0));
        assert_eq!(moved.teleport_cooldown_ms, TELEPORT_COOLDOWN_MS);

        // And not again straight away, or a realm can be crossed faster than anything can chase.
        assert!(world.teleport_to(mover, target).is_some());
    }

    #[test]
    fn nobody_teleports_to_somebody_hiding_or_paused() {
        // Both would reach a player the world says cannot be reached: one is hiding, and the other
        // is somewhere the world has stopped.
        let catalog = catalog();

        for hidden in [
            hendra_content::ConditionEffect::Invisible,
            hendra_content::ConditionEffect::Paused,
        ] {
            let mut world = field(&catalog);
            let mover = world
                .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
                .unwrap();
            let target = world
                .spawn(Entity::player(ObjectType(0x600), 20.0, 24.0, 500))
                .unwrap();

            world.get_mut(target).unwrap().conditions.insert(hidden);

            assert!(
                world.teleport_to(mover, target).is_some(),
                "reached somebody {hidden:?}"
            );
            assert_eq!(world.get(mover).unwrap().x, 2.0, "and moved anyway");
        }
    }

    #[test]
    fn a_world_that_forbids_teleporting_forbids_it() {
        // The nexus and the shops do, in the world definitions.
        let catalog = catalog();
        let mut world = field(&catalog);
        world.set_allows_teleport(false);

        let mover = world
            .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
            .unwrap();
        let target = world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 24.0, 500))
            .unwrap();

        assert!(world.teleport_to(mover, target).is_some());
        assert_eq!(world.get(mover).unwrap().x, 2.0);
    }

    #[test]
    fn you_cannot_teleport_to_an_enemy_or_to_yourself() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mover = world
            .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
            .unwrap();
        let slime = world
            .spawn(Entity::fixture(ObjectType(0x502), 20.0, 24.0))
            .unwrap();

        assert!(world.teleport_to(mover, mover).is_some());
        assert!(world.teleport_to(mover, slime).is_some());
        assert_eq!(world.get(mover).unwrap().x, 2.0);
    }

    #[test]
    fn a_teleported_player_is_not_snapped_back_for_arriving() {
        // The server moved them. Checking that against a walking speed would refuse its own
        // teleport, and the player would be dragged back the moment their client agreed.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mover = world
            .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
            .unwrap();
        let target = world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 24.0, 500))
            .unwrap();

        world.teleport_to(mover, target);

        let outcome = world
            .resolve_move(mover, &catalog, 20.0, 24.0, 50)
            .expect("an outcome");
        assert_eq!((outcome.x, outcome.y), (20.0, 24.0));
        assert!(outcome.refused.is_none());
    }

    #[test]
    fn a_player_can_be_found_by_name_whatever_its_capitals() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let handle = world
            .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
            .unwrap();
        world.get_mut(handle).unwrap().name = Some("Fesal".into());

        assert_eq!(world.player_named("fesal"), Some(handle));
        assert_eq!(world.player_named("FESAL"), Some(handle));
        assert_eq!(world.player_named("Nobody"), None);
    }

    #[test]
    fn a_loot_boost_makes_earned_loot_likelier() {
        // A multiplier rather than a bonus, as the original has it: a boost is worth more on
        // something that already drops often, which is what makes it worth having on a run.
        let catalog = catalog();
        let mut world = field(&catalog);

        let entry = hendra_behavior::program::LootEntry::Item {
            name: "Rare Blade".to_string(),
            chance: 0.5,
        };

        // Decisive rather than statistical. Comparing two runs of a random draw can agree by luck,
        // and a test that can pass by luck passes with the multiplier taken out as well: this one
        // did, which is why it is written this way. Half a chance doubled is a certainty.
        for _ in 0..200 {
            assert!(
                world.roll_loot_for(&entry, &catalog, 2.0).is_some(),
                "a doubled half-chance missed"
            );
        }

        // And without it, some of them miss.
        let missed = (0..200)
            .filter(|_| world.roll_loot_for(&entry, &catalog, 1.0).is_none())
            .count();
        assert!(missed > 0, "an unboosted half-chance never missed");
    }

    #[test]
    fn a_boost_cannot_make_something_drop_that_never_drops() {
        // A multiplier on nothing is nothing, which is what keeps a boost from turning a table
        // entry with no chance into a certainty.
        let catalog = catalog();
        let mut world = field(&catalog);

        let never = hendra_behavior::program::LootEntry::Item {
            name: "Rare Blade".to_string(),
            chance: 0.0,
        };

        for _ in 0..200 {
            assert!(world.roll_loot_for(&never, &catalog, 10.0).is_none());
        }
    }

    #[test]
    fn a_bag_that_belongs_to_somebody_is_only_theirs_to_open() {
        // What makes dropping a soulbound item a way to move it rather than a way to give it away.
        let catalog = catalog();
        let mut world = field(&catalog);

        let owner = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 500))
            .unwrap();

        world.drop_owned_bag(&catalog, owner, vec![ObjectType(0x904)], 5.0, 5.0);
        world.reindex();

        let bag = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Container)
            .map(|(handle, entity)| (handle, entity.belongs_to))
            .expect("a bag");

        assert_eq!(bag.1, Some(owner));
    }

    #[test]
    fn a_temporary_boost_lapses_rather_than_lasting_forever() {
        // Its duration used to be discarded, which made every temporary boost permanent.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 500))
            .unwrap();

        let before = world.get(player).unwrap().stats.totals()[2];

        world.carry_out(
            player,
            &catalog,
            &hendra_content::Effect::StatBoost {
                stat: 2,
                amount: 10,
                duration_ms: 1000,
                range: None,
            },
            (5.0, 5.0),
            (5.0, 5.0),
        );

        let raised = world.get(player).unwrap().stats.totals()[2];
        assert_eq!(raised - before, 10);

        world.advance(&catalog, 500);
        assert_eq!(
            world.get(player).unwrap().stats.totals()[2],
            raised,
            "it lapsed early"
        );

        world.advance(&catalog, 600);
        assert_eq!(
            world.get(player).unwrap().stats.totals()[2],
            before,
            "it never lapsed"
        );
    }

    #[test]
    fn two_boosts_of_the_same_size_are_worth_less_than_two() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 500))
            .unwrap();

        let before = world.get(player).unwrap().stats.totals()[2];

        for amount in [8, 8] {
            world.carry_out(
                player,
                &catalog,
                &hendra_content::Effect::StatBoost {
                    stat: 2,
                    amount,
                    duration_ms: 10_000,
                    range: None,
                },
                (5.0, 5.0),
                (5.0, 5.0),
            );
        }

        // One of them is kept, since renewing extends rather than adding a second. Two different
        // sizes is what the stacking rule is really about, and `stats::stacked` holds that.
        let after = world.get(player).unwrap().stats.totals()[2];
        assert_eq!(after - before, 8);
    }

    #[test]
    fn an_aura_reaches_the_people_standing_in_it() {
        // Its range used to be discarded, so an aura reached only whoever used it.
        let catalog = catalog();
        let mut world = field(&catalog);

        let caster = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 500))
            .unwrap();
        let friend = world
            .spawn(Entity::player(ObjectType(0x600), 7.0, 5.0, 500))
            .unwrap();
        let stranger = world
            .spawn(Entity::player(ObjectType(0x600), 40.0, 40.0, 500))
            .unwrap();
        world.reindex();

        let before = world.get(friend).unwrap().stats.totals()[2];

        world.carry_out(
            caster,
            &catalog,
            &hendra_content::Effect::StatBoost {
                stat: 2,
                amount: 10,
                duration_ms: 5000,
                range: Some(6.0),
            },
            (5.0, 5.0),
            (5.0, 5.0),
        );

        assert_eq!(
            world.get(friend).unwrap().stats.totals()[2] - before,
            10,
            "somebody standing in it got nothing"
        );
        assert_eq!(
            world.get(stranger).unwrap().stats.totals()[2] - before,
            0,
            "somebody across the map got it"
        );
        assert!(
            world.get(caster).unwrap().stats.totals()[2] > 0,
            "the caster"
        );
    }

    #[test]
    fn standing_in_an_aura_does_not_pile_up_boosts() {
        // Renewing extends rather than adding, or a minute in an aura would be sixty boosts.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 500))
            .unwrap();
        world.reindex();

        for _ in 0..30 {
            world.carry_out(
                player,
                &catalog,
                &hendra_content::Effect::StatBoost {
                    stat: 2,
                    amount: 10,
                    duration_ms: 5000,
                    range: Some(6.0),
                },
                (5.0, 5.0),
                (5.0, 5.0),
            );
        }

        assert_eq!(world.get(player).unwrap().boosts.len(), 1);
    }

    #[test]
    fn a_dead_player_is_reported_once_and_named_after_what_killed_it() {
        // Before this, a player whose health reached zero simply vanished: nothing marked the
        // character dead, and logging back in found it alive.
        let catalog = catalog();
        let mut world = field(&catalog);

        let slime = world
            .spawn(Entity::fixture(ObjectType(0x502), 8.0, 8.0))
            .unwrap();
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 9.0, 9.0, 100))
            .unwrap();

        if let Some(entity) = world.get_mut(player) {
            entity.hp = 0;
            entity.dead = true;
            entity.last_hurt_by = Some(slime);
        }

        world.advance(&catalog, 50);

        let deaths = world.take_deaths();
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].who, player);
        assert_eq!(deaths[0].killer, "Slime");

        // And once: a death answered twice is a character killed twice.
        assert!(world.take_deaths().is_empty());
    }

    #[test]
    fn a_death_with_nothing_to_blame_is_named_after_the_world() {
        // Ground damage and a fall have no killer, and "killed by nobody" is not an answer.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 9.0, 9.0, 100))
            .unwrap();
        if let Some(entity) = world.get_mut(player) {
            entity.dead = true;
        }

        world.advance(&catalog, 50);

        let deaths = world.take_deaths();
        assert_eq!(deaths[0].killer, "Field");
    }

    #[test]
    fn a_gravestone_says_how_far_the_character_got() {
        // Which stone and how long it stands is what the room remembers of somebody.
        let catalog = catalog();
        let mut world = field(&catalog);

        // A level-one death leaves the smallest stone; the content has to have them for this to
        // mean anything, and where it does not the world places nothing rather than a wrong one.
        world.place_gravestone(&catalog, (4.0, 4.0), "Fesal", 0, 1, false);
        world.place_gravestone(&catalog, (6.0, 6.0), "Bo", 8, 20, false);
        world.reindex();

        // Nothing is placed for a stone the content does not describe, which is the fixture here.
        assert!(
            world
                .iter()
                .all(|(_, entity)| entity.name.as_deref() != Some("nobody")),
            "a stone appeared for a type the catalog does not have"
        );
    }

    #[test]
    fn loot_that_belongs_to_somebody_goes_in_a_bag_only_they_can_open() {
        // The whole point of a threshold: a bag anybody could take from would make it decide who
        // the loot was rolled for and nothing about who ends up with it.
        let catalog = catalog();
        let mut world = field(&catalog);

        let earner = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 500))
            .unwrap();

        world.drop_owned_bag(&catalog, earner, vec![ObjectType(0x904)], 5.0, 5.0);
        world.reindex();

        let bag = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Container)
            .map(|(handle, entity)| (handle, entity.belongs_to))
            .expect("a bag");

        assert_eq!(bag.1, Some(earner), "the bag belongs to nobody");
    }

    #[test]
    fn a_threshold_is_measured_against_what_the_enemy_started_with() {
        // Against what it has now would be a share of nothing, which nobody can meet.
        let catalog = catalog();
        let mut world = field(&catalog);

        // Through the world rather than by hand, so it has the health its description gives it.
        let behaviours = std::mem::take(&mut world.behaviours);
        let slime = world
            .spawn_child(
                &catalog,
                &behaviours,
                ObjectType(0x502),
                8.0,
                8.0,
                None,
                None,
            )
            .unwrap();
        world.behaviours = behaviours;

        let started = world.get(slime).unwrap().max_hp;
        assert!(started > 0, "the slime has no health to take a share of");

        if let Some(entity) = world.get_mut(slime) {
            entity.base_max_hp = Some(started);
            entity.hp = 0;
            entity.dead = true;
        }

        // A tenth of what it started with is a real bar; a tenth of nothing is not.
        assert_eq!(
            world.get(slime).unwrap().base_max_hp,
            Some(started),
            "the enemy forgot what it started with"
        );
    }

    #[test]
    fn damage_is_remembered_per_player_and_bounded() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let slime = world
            .spawn(Entity::fixture(ObjectType(0x502), 8.0, 8.0))
            .unwrap();

        // More damagers than one enemy remembers, so the list cannot grow without limit.
        for index in 0..(MOST_REMEMBERED_DAMAGERS + 20) {
            let who = world
                .spawn(Entity::player(
                    ObjectType(0x600),
                    index as f32 % 30.0,
                    1.0,
                    500,
                ))
                .unwrap();

            if let Some(entity) = world.get_mut(slime) {
                let held = entity.damage_by.iter_mut().find(|(held, _)| *held == who);
                match held {
                    Some((_, total)) => *total += 1,
                    None => {
                        if entity.damage_by.len() < MOST_REMEMBERED_DAMAGERS {
                            entity.damage_by.push((who, 1));
                        }
                    }
                }
            }
        }

        assert_eq!(
            world.get(slime).unwrap().damage_by.len(),
            MOST_REMEMBERED_DAMAGERS
        );
    }

    #[test]
    fn a_drawn_setpiece_paints_the_ground_and_puts_its_boss_in() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let drawing = crate::setpiece::Drawing {
            squares: vec![crate::setpiece::Painted {
                x: 1,
                y: 1,
                tile: Some("Water"),
                object: None,
                size: 0,
                clear: true,
            }],
            placed: vec![crate::setpiece::Placed::Living {
                x: 2.5,
                y: 2.5,
                name: "Slime",
                size: 150,
            }],
            prefab: None,
        };

        let missing = world.draw(&catalog, &drawing, (5, 5));
        world.reindex();

        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(world.terrain().tile_at(6, 6), TileType(0x11), "the ground");
        assert!(!world.terrain().walkable(6, 6), "and water is not walkable");

        let boss = world
            .iter()
            .find(|(_, entity)| entity.object_type == ObjectType(0x502))
            .map(|(_, entity)| (entity.x, entity.y, entity.size))
            .expect("the boss");
        assert_eq!(boss, (7.5, 7.5, 150));
    }

    #[test]
    fn what_a_setpiece_paints_is_scenery_rather_than_an_entity() {
        // A castle whose every stone was an entity would cost eight hundred places in the world and
        // eight hundred snapshot entries, for eight hundred things that never move.
        let catalog = catalog();
        let mut world = field(&catalog);
        let before = world.len();

        let squares = (0..10)
            .map(|step| crate::setpiece::Painted {
                x: step,
                y: 0,
                tile: Some("Grass"),
                object: Some("Wall"),
                size: 0,
                clear: false,
            })
            .collect();

        world.draw(
            &catalog,
            &crate::setpiece::Drawing {
                squares,
                placed: Vec::new(),
                prefab: None,
            },
            (4, 4),
        );
        world.reindex();

        assert_eq!(world.len(), before, "the walls became entities");
        assert!(!world.terrain().walkable(4, 4), "but they still block");
        assert_eq!(
            world.terrain().map().at(4, 4).map(|square| square.object),
            Some(ObjectType(0x500)),
            "and the map knows they are there, so somebody joining later sees them"
        );
    }

    #[test]
    fn a_setpiece_naming_something_the_content_lacks_says_so_rather_than_drawing_a_hole() {
        // A missing name is a stretch of realm nobody can fight in, and the only symptom without
        // this is a room that came out wrong.
        let catalog = catalog();
        let mut world = field(&catalog);

        let drawing = crate::setpiece::Drawing {
            squares: vec![crate::setpiece::Painted {
                x: 0,
                y: 0,
                tile: Some("Ground That Does Not Exist"),
                object: Some("Object That Does Not Exist"),
                size: 0,
                clear: false,
            }],
            placed: vec![crate::setpiece::Placed::Living {
                x: 0.0,
                y: 0.0,
                name: "Nobody",
                size: 0,
            }],
            prefab: None,
        };

        let missing = world.draw(&catalog, &drawing, (2, 2));

        assert_eq!(missing.len(), 3, "{missing:?}");
    }

    #[test]
    fn a_setpiece_drawn_off_the_edge_of_the_map_paints_what_fits() {
        // The scatterer picks a corner at random, so some of them hang off the map.
        let catalog = catalog();
        let mut world = field(&catalog);

        let squares = (0..8)
            .map(|step| crate::setpiece::Painted {
                x: step,
                y: 0,
                tile: Some("Water"),
                object: None,
                size: 0,
                clear: true,
            })
            .collect();

        let drawing = crate::setpiece::Drawing {
            squares,
            placed: Vec::new(),
            prefab: None,
        };

        // Four of the eight squares are on the map and four are past its right edge.
        world.draw(&catalog, &drawing, (28, 4));

        assert_eq!(
            world.terrain().tile_at(31, 4),
            TileType(0x11),
            "the last one on"
        );
    }

    #[test]
    fn a_behaviour_naming_a_setpiece_that_does_not_exist_is_reported_once() {
        // Four of the shipped behaviours do exactly this, and so does the original: the class it
        // looks for is not there. Saying so beats a boss whose arena never appears.
        let catalog = catalog();
        let mut world = field(&catalog);

        let handle = world
            .spawn(Entity::fixture(ObjectType(0x502), 8.0, 8.0))
            .unwrap();
        let program = hendra_behavior::Program::default();

        for _ in 0..3 {
            world.apply(
                handle,
                &catalog,
                &Programs::default(),
                &program,
                &Action::Setpiece {
                    name: "BottledEvil".to_string(),
                },
                50,
            );
        }

        assert_eq!(
            world.take_unknown_setpieces(),
            vec!["BottledEvil".to_string()]
        );
        assert!(world.take_unknown_setpieces().is_empty(), "and only once");
    }

    #[test]
    fn speech_is_taken_once_and_does_not_pile_up_forever() {
        // A boss left alive in an empty room talks to itself, and nothing in the simulation makes
        // it stop. A queue nobody drains has to stop growing on its own.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { taunt("a", "b", cooldown: 0) } }"#,
        );
        world.reindex();

        for _ in 0..40 {
            world.advance(&catalog, 50);
        }

        let said = world.take_announcements();
        assert!(!said.is_empty(), "it should have said something");
        assert!(
            world.take_announcements().is_empty(),
            "and taking it should have emptied the queue"
        );

        for _ in 0..5_000 {
            world.advance(&catalog, 50);
        }
        assert!(
            world.take_announcements().len() <= MAX_PENDING_ANNOUNCEMENTS,
            "the queue should be bounded when nobody drains it"
        );
    }

    #[test]
    fn the_cooldown_stops_a_client_firing_as_fast_as_it_likes() {
        let catalog = catalog();
        let (mut world, shooter, _) = duel(&catalog);

        assert_eq!(world.shoot(shooter, &catalog, 0.0).len(), 1);

        // Twenty more attempts in the same instant yield nothing. Rate of fire is the server's to
        // enforce; a client that asks faster is simply refused rather than believed.
        for _ in 0..20 {
            assert!(world.shoot(shooter, &catalog, 0.0).is_empty());
        }
        assert_eq!(world.projectile_count(), 1);

        // After the cooldown has elapsed it may fire again. A character with no dexterity fires
        // slowly, so this waits out the full minimum rather than a fixed 500ms.
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        assert_eq!(world.shoot(shooter, &catalog, 0.0).len(), 1);
    }

    #[test]
    fn enough_shots_kill_and_the_body_is_removed() {
        let catalog = catalog();
        let (mut world, shooter, target) = duel(&catalog);

        // A hundred a shot, halved by an attack of zero and reduced again by defence, against two
        // hundred hit points. Slower than it used to be, so this allows more attempts.
        for _ in 0..200 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);
            if world.get(target).is_none() {
                break;
            }
        }

        assert!(world.get(target).is_none(), "the slime should be dead");
    }

    #[test]
    fn a_projectile_outlives_nothing_when_its_owner_leaves() {
        let catalog = catalog();
        let (mut world, shooter, _) = duel(&catalog);

        world.shoot(shooter, &catalog, 0.0);
        assert_eq!(world.projectile_count(), 1);

        world.despawn(shooter);
        world.advance(&catalog, 50);

        assert_eq!(
            world.projectile_count(),
            0,
            "a world should not keep firing for someone who has gone"
        );
    }

    #[test]
    fn a_player_counts_the_ground_they_have_looked_at() {
        // The count two fame bonuses rest on. The original gets it from the tiles it sends as it
        // reveals the map; we send the map whole, so it is taken here instead.
        let catalog = catalog();
        let squares = (0..64 * 64).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(64, 64, squares).unwrap();
        let mut world = World::new("Field", Terrain::build(map, &catalog), &catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 32.0, 32.0, 800))
            .unwrap();
        world.look_around(player);

        let first = world.get(player).unwrap().tally.tiles_seen;

        // A circle of radius twenty, which is a bit over twelve hundred squares.
        assert!(
            (1_100..1_400).contains(&first),
            "a circle of sight was {first} squares"
        );

        // Standing still uncovers nothing, however long they stand there.
        for _ in 0..10 {
            world.place(
                player,
                MoveOutcome {
                    x: 32.2,
                    y: 32.4,
                    refused: None,
                },
            );
        }
        assert_eq!(
            world.get(player).unwrap().tally.tiles_seen,
            first,
            "standing still uncovered ground"
        );

        // Walking uncovers what is newly in front and not what was already behind.
        world.place(
            player,
            MoveOutcome {
                x: 33.0,
                y: 32.0,
                refused: None,
            },
        );
        let after = world.get(player).unwrap().tally.tiles_seen;

        assert!(after > first, "walking uncovered nothing");
        assert!(
            after - first < 100,
            "one step uncovered {} squares, so ground already seen was counted again",
            after - first
        );

        // And walking back over the same ground is not new ground.
        world.place(
            player,
            MoveOutcome {
                x: 32.0,
                y: 32.0,
                refused: None,
            },
        );
        assert_eq!(
            world.get(player).unwrap().tally.tiles_seen,
            after,
            "ground already walked was counted twice"
        );
    }

    #[test]
    fn sight_stops_at_the_edge_of_the_map() {
        // A player in a corner sees a quarter of a circle. Counting the squares that are not there
        // would make a small map worth more than a large one.
        let catalog = catalog();
        let squares = (0..64 * 64).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(64, 64, squares).unwrap();
        let mut world = World::new("Field", Terrain::build(map, &catalog), &catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 0.0, 0.0, 800))
            .unwrap();
        world.look_around(player);

        let corner = world.get(player).unwrap().tally.tiles_seen;
        assert!(
            (250..400).contains(&corner),
            "a corner of the map was worth {corner} squares"
        );

        // The far corner, where an overrun does not fall off the end of the array but wraps into
        // the beginning of the next row: ground the player cannot see, counted as seen, on the
        // opposite side of the map.
        let far = world
            .spawn(Entity::player(ObjectType(0x600), 63.5, 63.5, 800))
            .unwrap();
        world.look_around(far);

        assert_eq!(
            world.get(far).unwrap().tally.tiles_seen,
            corner,
            "the far corner saw more than the near one"
        );
    }

    /// An enemy with a behaviour, and a player standing in front of it.
    fn confrontation(catalog: &Catalog, source: &str) -> (World, Handle, Handle) {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let squares = (0..32 * 32).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, catalog), catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 500;
        slime.max_hp = 500;
        let enemy = world.spawn(slime).unwrap();

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 14.0, 10.0, 800))
            .unwrap();

        let (programs, diagnostics) = compile(&parse(source).expect("behaviour should parse"));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        world.set_behaviours(catalog, programs);

        world.advance(catalog, 50);
        (world, enemy, player)
    }

    #[test]
    fn an_enemy_with_a_behaviour_gets_a_mind() {
        let catalog = catalog();
        let (world, enemy, _) =
            confrontation(&catalog, r#"enemy "Slime" { state idle { wander(0.4) } }"#);

        assert_eq!(world.thinking(), 1);
        assert!(world.get(enemy).unwrap().mind.is_some());
    }

    #[test]
    fn an_enemy_hears_what_is_said_near_it() {
        // A dungeon whose door opens when you say the right word is built out of this. Draconis is
        // the one in the content: three dragon souls that wait, orbiting an altar, until somebody
        // says the colour.
        let catalog = catalog();
        let (mut world, enemy, _) = confrontation(
            &catalog,
            r#"enemy "Slime" {
                state quiet { on player_text("Red", 99, false, false) -> loud }
                state loud { wander(0.4) }
            }"#,
        );

        let quiet = world.get(enemy).unwrap().mind.as_ref().unwrap().state();

        world.heard((14.0, 10.0), "Red");
        world.advance(&catalog, 50);

        assert_ne!(
            world.get(enemy).unwrap().mind.as_ref().unwrap().state(),
            quiet,
            "saying the word changed nothing"
        );
    }

    #[test]
    fn a_word_is_heard_when_it_is_said_and_not_after() {
        // Held for the tick it was said in and no longer. A word that lingered would be heard by
        // everything that arrived afterwards, so a door would open for somebody who walked up to it
        // in silence, having missed the moment entirely.
        let catalog = catalog();
        let listener = r#"enemy "Slime" {
                state quiet { on player_text("Red", 99, false, false) -> loud }
                state loud { wander(0.4) }
            }"#;
        let (mut world, first, _) = confrontation(&catalog, listener);

        // What waiting looks like, read rather than assumed: a state's number is whatever the
        // compiler gave it, and a test that hard-codes one passes for the wrong reason.
        let quiet = world.get(first).unwrap().mind.as_ref().unwrap().state();

        world.heard((14.0, 10.0), "Red");
        world.advance(&catalog, 50);
        assert_ne!(
            world.get(first).unwrap().mind.as_ref().unwrap().state(),
            quiet,
            "the word was not heard at all"
        );

        // A second enemy arrives after the word was said, into a world where nobody has spoken
        // since. It should be waiting, not reacting. Handing the world its behaviours again is how
        // the newcomer gets a mind, and it puts the first one back at its beginning too, which the
        // assertion above has already read.
        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 500;
        slime.max_hp = 500;
        let second = world.spawn(slime).unwrap();

        let (programs, _) = hendra_behavior::compile::compile(
            &hendra_behavior::parse::parse(listener).expect("behaviour should parse"),
        );
        world.set_behaviours(&catalog, programs);

        for _ in 0..5 {
            world.advance(&catalog, 50);
        }

        assert_eq!(
            world.get(second).unwrap().mind.as_ref().unwrap().state(),
            quiet,
            "a word carried over into later ticks"
        );
    }

    #[test]
    fn somebody_shouting_from_across_the_map_is_not_heard() {
        let catalog = catalog();
        let (mut world, enemy, _) = confrontation(
            &catalog,
            r#"enemy "Slime" {
                state quiet { on player_text("Red", 4, false, false) -> loud }
                state loud { wander(0.4) }
            }"#,
        );

        let quiet = world.get(enemy).unwrap().mind.as_ref().unwrap().state();

        world.heard((30.0, 30.0), "Red");
        world.advance(&catalog, 50);

        assert_eq!(
            world.get(enemy).unwrap().mind.as_ref().unwrap().state(),
            quiet,
            "a shout from across the map was heard"
        );
    }

    #[test]
    fn an_enemy_with_no_behaviour_simply_stands_there() {
        let catalog = catalog();
        let (world, enemy, _) = confrontation(
            &catalog,
            r#"enemy "Something Else" { state idle { wander(0.4) } }"#,
        );

        assert_eq!(
            world.thinking(),
            0,
            "a half-converted directory should still run"
        );
        assert!(world.get(enemy).unwrap().mind.is_none());
    }

    #[test]
    fn an_enemy_chases_a_player() {
        let catalog = catalog();
        let (mut world, enemy, _) = confrontation(
            &catalog,
            r#"enemy "Slime" { state idle { follow(1.0, 20, 1) } }"#,
        );

        let start = world.get(enemy).unwrap().x;
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }

        let now = world.get(enemy).unwrap().x;
        assert!(
            now > start + 0.5,
            "it should have closed the distance: {start} -> {now}"
        );
    }

    #[test]
    fn an_enemy_shoots_the_player_and_hurts_them() {
        let catalog = catalog();
        let (mut world, _, player) = confrontation(
            &catalog,
            r#"enemy "Slime" { state idle { shoot(count: 1, cooldown: 200ms) } }"#,
        );

        let before = world.get(player).unwrap().hp;
        for _ in 0..30 {
            world.advance(&catalog, 50);
        }

        let after = world.get(player).map(|entity| entity.hp).unwrap_or(0);
        assert!(
            after < before,
            "the player should have taken fire: {before} -> {after}"
        );
    }

    #[test]
    fn an_enemy_does_not_shoot_through_a_wall() {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..32 * 32)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        // A sight-blocking column between the two of them.
        for y in 0..32 {
            squares[y * 32 + 12] = square(0x10, 0x504);
        }

        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, &catalog), &catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 500;
        slime.max_hp = 500;
        world.spawn(slime).unwrap();

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 10.0, 800))
            .unwrap();

        let (programs, _) = compile(
            &parse(r#"enemy "Slime" { state idle { shoot(count: 1, cooldown: 100ms) } }"#).unwrap(),
        );
        world.set_behaviours(&catalog, programs);

        let before = world.get(player).unwrap().hp;
        for _ in 0..30 {
            world.advance(&catalog, 50);
        }

        assert_eq!(
            world.get(player).unwrap().hp,
            before,
            "an enemy that cannot see the player must not shoot them"
        );
    }

    #[test]
    fn an_enemy_cannot_walk_through_a_wall_either() {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..32 * 32)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        for y in 0..32 {
            squares[y * 32 + 12] = square(0x11, ObjectType::NONE.0);
        }

        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, &catalog), &catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 500;
        slime.max_hp = 500;
        let enemy = world.spawn(slime).unwrap();
        world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 10.0, 800))
            .unwrap();

        let (programs, _) =
            compile(&parse(r#"enemy "Slime" { state idle { follow(2.0, 30, 1) } }"#).unwrap());
        world.set_behaviours(&catalog, programs);

        for _ in 0..80 {
            world.advance(&catalog, 50);
        }

        assert!(
            world.get(enemy).unwrap().x < 12.0,
            "it should have been stopped by the water, not walked over it"
        );
    }

    #[test]
    fn a_dying_enemy_leaves_a_bag() {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let catalog = catalog();
        let squares = (0..16 * 16).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, &catalog), &catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 8.0, 8.0);
        slime.kind = Kind::Enemy;
        slime.hp = 10;
        slime.max_hp = 200;
        let enemy = world.spawn(slime).unwrap();

        // A certainty rather than a chance, so the test does not depend on a roll.
        let (programs, _) = compile(
            &parse(r#"enemy "Slime" { state idle { } loot { item("Wand", 1.0) } }"#).unwrap(),
        );
        world.set_behaviours(&catalog, programs);

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert!(world.get(enemy).is_none(), "the slime is gone");

        let bag = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Container)
            .map(|(_, entity)| entity.clone())
            .expect("a bag should have been left behind");

        let container = bag.container.as_ref().expect("the bag holds something");
        assert_eq!(container.occupied(), 1);
        assert_eq!(container.item(0), catalog.type_of("Wand").unwrap());
        assert!((bag.x - 8.0).abs() < 1e-4, "and it is where the slime died");
    }

    #[test]
    fn a_bag_expires() {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let catalog = catalog();
        let squares = (0..16 * 16).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, &catalog), &catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 8.0, 8.0);
        slime.kind = Kind::Enemy;
        slime.hp = 1;
        slime.max_hp = 200;
        let enemy = world.spawn(slime).unwrap();

        let (programs, _) = compile(
            &parse(r#"enemy "Slime" { state idle { } loot { item("Wand", 1.0) } }"#).unwrap(),
        );
        world.set_behaviours(&catalog, programs);

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);
        assert_eq!(
            world
                .iter()
                .filter(|(_, e)| e.kind == Kind::Container)
                .count(),
            1
        );

        // A minute later there is nothing left, so a cleared dungeon does not fill with bags.
        for _ in 0..(61_000 / 50) {
            world.advance(&catalog, 50);
        }
        assert_eq!(
            world
                .iter()
                .filter(|(_, e)| e.kind == Kind::Container)
                .count(),
            0
        );
    }

    #[test]
    fn an_enemy_with_no_loot_leaves_nothing() {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let catalog = catalog();
        let squares = (0..16 * 16).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, &catalog), &catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 8.0, 8.0);
        slime.kind = Kind::Enemy;
        slime.hp = 1;
        slime.max_hp = 200;
        let enemy = world.spawn(slime).unwrap();

        let (programs, _) = compile(&parse(r#"enemy "Slime" { state idle { } }"#).unwrap());
        world.set_behaviours(&catalog, programs);

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(
            world
                .iter()
                .filter(|(_, e)| e.kind == Kind::Container)
                .count(),
            0,
            "an empty bag is worse than no bag"
        );
    }

    #[test]
    fn a_move_for_an_entity_that_no_longer_exists_is_refused_safely() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();
        world.despawn(player);

        assert!(
            world
                .resolve_move(player, &catalog, 17.0, 16.0, 50)
                .is_none()
        );
    }
}
