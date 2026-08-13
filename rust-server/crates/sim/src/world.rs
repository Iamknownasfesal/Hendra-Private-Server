//! One world: what is in it, and what happens to it each tick.
//!
//! A world is owned outright by whatever ticks it. Nothing here is behind a lock, and nothing is
//! shared with another world — the C# server reached for `ConcurrentDictionary` on five collections
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

use hendra_behavior::program::{Action, Nearby, Programs, Senses};
use hendra_behavior::run::Mind;
use hendra_content::{Catalog, ConditionSet, ObjectType};
use hendra_net::{EntityId, EntityState, Tick, WorldSnapshot};

use crate::grid::Grid;
use crate::inventory::{Container, ContainerKind};
use crate::projectile::{Hit, Projectile, Projectiles};
use crate::slab::{Handle, Slab};
use crate::tiles::Terrain;

/// How far a player can see, in tiles. Matches the client's view distance.
pub const SIGHT_RADIUS: f32 = 20.0;

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
}

impl Kind {
    pub fn is_alive_kind(self) -> bool {
        matches!(self, Kind::Player | Kind::Enemy)
    }
}

/// Anything that occupies a position in a world.
#[derive(Debug, Clone)]
pub struct Entity {
    pub object_type: ObjectType,
    pub kind: Kind,

    pub x: f32,
    pub y: f32,

    pub hp: i32,
    pub max_hp: i32,
    pub mp: i32,
    pub max_mp: i32,

    pub conditions: ConditionSet,
    pub size: u16,
    pub name: Option<Box<str>>,

    /// Base movement, in tiles per second, before ground and conditions.
    pub speed: f32,

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
}

impl Entity {
    /// A fixture read off the map.
    pub fn fixture(object_type: ObjectType, x: f32, y: f32) -> Entity {
        Entity {
            object_type,
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
            speed: 0.0,
            weapon: None,
            cooldown_ms: 0,
            spawn_x: x,
            spawn_y: y,
            mind: None,
            container: None,
            expires_in_ms: None,
            dead: false,
        }
    }

    pub fn player(object_type: ObjectType, x: f32, y: f32, max_hp: i32) -> Entity {
        Entity {
            object_type,
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
            speed: 5.0,
            weapon: None,
            cooldown_ms: 0,
            spawn_x: x,
            spawn_y: y,
            mind: None,
            container: None,
            expires_in_ms: None,
            dead: false,
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
/// Not generosity toward cheating — the claim is clamped either way. It absorbs the ordinary
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

    /// Reused between ticks so a warm world allocates nothing.
    handles: Vec<Handle>,
    nearby: Vec<Handle>,
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
            // Static scenery that blocks a square is already in the collision bitmap, so putting it
            // in the entity list too would cost a snapshot entry per wall for no gain.
            let desc = catalog.object(square.object);
            if desc.is_some_and(|desc| desc.full_occupy && desc.static_object && !desc.enemy) {
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
            handles: Vec::new(),
            nearby: Vec::new(),
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
    /// half-converted content directory should look like — most of the dungeon working.
    pub fn set_behaviours(&mut self, catalog: &Catalog, behaviours: Programs) {
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
            let Some(id) = catalog.object(entity.object_type).map(|desc| desc.id.clone()) else {
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

    /// Rebuilds the spatial index from current positions.
    fn reindex(&mut self) {
        // Collected first because `rebuild` takes `&mut self.grid` while reading `self.entities`.
        self.handles.clear();
        self.handles.extend(self.entities.iter().map(|(handle, _)| handle));

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
    /// is trimmed rather than rejected outright — a rejected move makes a laggy player rubber-band,
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

        let ground = self.terrain.speed_at(catalog, entity.x, entity.y);
        let allowed = entity.speed * ground * (elapsed_ms as f32 / 1000.0) * MOVE_TOLERANCE;

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
        // rather than stopping dead — which is what a player expects and what the client's own
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
    /// the entity's speed would be checking our own arithmetic. Terrain still applies — an enemy
    /// walks through a wall no more than a player does — and it slides along one rather than
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
        if let Some(entity) = self.entities.get_mut(handle) {
            entity.x = outcome.x;
            entity.y = outcome.y;
        }
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
    /// follows — where the shot goes, what it strikes, what that costs — is the server's.
    pub fn shoot(&mut self, handle: Handle, catalog: &Catalog, angle: f32) -> Vec<Handle> {
        let mut fired = Vec::new();

        let Some(entity) = self.entities.get(handle) else {
            return fired;
        };
        if entity.cooldown_ms > 0 || entity.dead {
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
        let cooldown = (500.0 / rate) as u32;

        for shot in &desc.projectiles {
            let roll = self.roll();
            let projectile =
                Projectile::from_desc(handle, from_player, shot, x, y, angle, roll);
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

        self.cool_weapons(elapsed_ms);
        self.think(catalog, elapsed_ms);
        self.apply_hazards(catalog, elapsed_ms);
        self.advance_projectiles(catalog, elapsed_ms);
        self.expire(elapsed_ms);
        self.drop_loot(catalog);
        self.reap();
        self.reindex();
    }

    /// Runs every enemy's behaviour and applies what it asked for.
    fn think(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| entity.mind.is_some() && !entity.dead)
                .map(|(handle, _)| handle),
        );

        for index in 0..self.handles.len() {
            let handle = self.handles[index];

            let Some(senses) = self.senses_for(handle) else {
                continue;
            };

            // The mind comes out of the entity for the duration of the tick, because running it
            // needs the world that the entity is part of.
            let Some(mut mind) = self.entities.get_mut(handle).and_then(|e| e.mind.take()) else {
                continue;
            };

            let Some(program) = self
                .entities
                .get(handle)
                .and_then(|entity| catalog.object(entity.object_type))
                .and_then(|desc| self.behaviours.get(&desc.id))
            else {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.mind = Some(mind);
                }
                continue;
            };

            let mut actions = std::mem::take(&mut self.actions);
            mind.tick(program, &senses, elapsed_ms, &mut actions);

            for action in &actions {
                self.apply(handle, catalog, action, elapsed_ms);
            }

            self.actions = actions;
            if let Some(entity) = self.entities.get_mut(handle) {
                entity.mind = Some(mind);
            }
        }
    }

    /// What an entity can perceive.
    fn senses_for(&mut self, handle: Handle) -> Option<Senses> {
        let entity = self.entities.get(handle)?;
        let (x, y, hp, max_hp) = (entity.x, entity.y, entity.hp, entity.max_hp);
        let (spawn_x, spawn_y) = (entity.spawn_x, entity.spawn_y);

        self.grid.within(x, y, SIGHT_RADIUS, &mut self.nearby);

        let mut nearest: Option<Nearby> = None;
        for found in &self.nearby {
            let Some(other) = self.entities.get(*found) else {
                continue;
            };
            if other.kind != Kind::Player || other.dead {
                continue;
            }

            let (dx, dy) = (other.x - x, other.y - y);
            let distance = (dx * dx + dy * dy).sqrt();

            // Behind a wall is not in sight, so an enemy does not chase or shoot through cover.
            if !self.terrain.line_of_sight(x, y, other.x, other.y) {
                continue;
            }

            if nearest.is_none_or(|closest| distance < closest.distance) {
                nearest = Some(Nearby {
                    x: other.x,
                    y: other.y,
                    distance,
                });
            }
        }

        Some(Senses {
            x,
            y,
            hp,
            max_hp,
            spawn_x,
            spawn_y,
            nearest_player: nearest,
        })
    }

    /// Carries out one thing a behaviour asked for.
    fn apply(&mut self, handle: Handle, catalog: &Catalog, action: &Action, elapsed_ms: u32) {
        match action {
            Action::Move { angle, speed } => {
                let Some(entity) = self.entities.get(handle) else {
                    return;
                };
                // Behaviours quote speed the way the content does, in tenths of a tile per second.
                let distance = speed * 10.0 * (elapsed_ms as f32 / 1000.0);
                let (to_x, to_y) = (
                    entity.x + angle.cos() * distance,
                    entity.y + angle.sin() * distance,
                );

                // Terrain still applies, so an enemy cannot walk through a wall — but the speed
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
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.hp = (entity.hp + amount).min(entity.max_hp);
                }
            }

            // Spawning children needs the catalog and a position; deliberately not implemented
            // until the loot and spawn rules that go with it are.
            Action::Spawn { .. } => {}

            Action::Vanish => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.dead = true;
                }
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
    }

    fn cool_weapons(&mut self, elapsed_ms: u32) {
        for (_, entity) in self.entities.iter_mut() {
            entity.cooldown_ms = entity.cooldown_ms.saturating_sub(elapsed_ms);
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
            if let Some(target) = self.entities.get_mut(hit.target) {
                target.hp -= hit.damage;
                if target.hp <= 0 {
                    target.dead = true;
                }
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
        self.handles.extend(self.entities.iter().filter_map(|(handle, entity)| {
            entity.kind.is_alive_kind().then_some(handle)
        }));

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

            let Some(id) = catalog.object(entity.object_type).map(|desc| desc.id.clone()) else {
                continue;
            };
            let Some(program) = self.behaviours.get(&id) else {
                continue;
            };

            let mut dropped = Vec::new();
            for entry in program.loot.clone() {
                if let Some(item) = self.roll_loot(&entry, catalog) {
                    dropped.push(item);
                }
            }

            if dropped.is_empty() {
                continue;
            }

            let mut container = Container::new(ContainerKind::Bag, 8);
            for item in dropped {
                container.insert(item, catalog);
            }

            let mut bag = Entity::fixture(self.bag_type, x, y);
            bag.kind = Kind::Container;
            bag.container = Some(Box::new(container));
            // Long enough to walk back for, short enough that a dungeon does not fill with bags.
            bag.expires_in_ms = Some(60_000);
            self.spawn(bag);
        }
    }

    /// Decides whether one loot entry drops, and what.
    fn roll_loot(
        &mut self,
        entry: &hendra_behavior::program::LootEntry,
        catalog: &Catalog,
    ) -> Option<ObjectType> {
        use hendra_behavior::program::LootEntry;

        match entry {
            LootEntry::Item { name, chance } => {
                if self.roll() > *chance {
                    return None;
                }
                catalog.type_of(name)
            }

            LootEntry::Tier {
                tier,
                kind,
                chance,
            } => {
                if self.roll() > *chance {
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
            // anything on the far side of a wall — which in a game where being seen means being
            // shot is a correctness problem rather than a cosmetic one.
            if *handle != viewer && !self.terrain.line_of_sight(x, y, entity.x, entity.y) {
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
        <Object type="0x600" id="Hero"><Class>Player</Class><Player/></Object>
        <Object type="0x900" id="Bolt"><Class>Projectile</Class></Object>
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
    fn map_objects_become_entities_but_walls_do_not() {
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..8 * 8)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[10] = square(0x10, 0x500); // wall — collision only
        squares[20] = square(0x10, 0x501); // sign — an entity
        squares[30] = square(0x10, 0x502); // slime — an entity

        let map = Map::from_squares(8, 8, squares).unwrap();
        let world = World::new("Test", Terrain::build(map, &catalog), &catalog);

        assert_eq!(
            world.len(),
            2,
            "the wall belongs in the collision bitmap, not the entity list"
        );
        assert!(!world.terrain().walkable(2, 1), "but it still blocks");

        let kinds: Vec<Kind> = world.iter().map(|(_, entity)| entity.kind).collect();
        assert!(kinds.contains(&Kind::Fixture));
        assert!(kinds.contains(&Kind::Enemy));
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
        let player = world.spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800)).unwrap();

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
        let player = world.spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800)).unwrap();

        // Claiming ten tiles in one 50ms tick.
        let outcome = world
            .resolve_move(player, &catalog, 26.0, 16.0, 50)
            .unwrap();

        assert_eq!(outcome.refused, Some(MoveRefusal::TooFar));

        // Trimmed to speed × time × tolerance, not rejected outright.
        let travelled = outcome.x - 16.0;
        let allowed = 5.0 * 0.05 * MOVE_TOLERANCE;
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
        let player = world.spawn(Entity::player(ObjectType(0x600), 3.5, 3.5, 800)).unwrap();

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
        let player = world.spawn(Entity::player(ObjectType(0x600), 3.5, 3.5, 800)).unwrap();

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

        let player = world.spawn(Entity::player(ObjectType(0x600), 4.0, 4.0, 300)).unwrap();

        world.advance(&catalog, 1000);
        assert_eq!(world.get(player).unwrap().hp, 200, "100 damage per second");

        world.advance(&catalog, 1000);
        world.advance(&catalog, 1000);

        assert!(world.get(player).is_none(), "the player should have died");
        assert_eq!(world.len(), 0);
    }

    #[test]
    fn a_snapshot_holds_what_is_in_sight_and_nothing_else() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let viewer = world.spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800)).unwrap();
        let near = world.spawn(Entity::player(ObjectType(0x600), 18.0, 16.0, 800)).unwrap();
        let far = world.spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800)).unwrap();
        world.get_mut(far).unwrap().x = 100.0;

        world.advance(&catalog, 50);

        let snapshot = world.snapshot_for(viewer, SIGHT_RADIUS);
        assert!(snapshot.get(viewer.to_entity_id()).is_some(), "the viewer sees itself");
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

        let viewer = world.spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800)).unwrap();
        let other = world.spawn(Entity::player(ObjectType(0x600), 17.0, 16.0, 800)).unwrap();
        world.advance(&catalog, 50);
        assert!(world.snapshot_for(viewer, SIGHT_RADIUS).get(other.to_entity_id()).is_some());

        world.despawn(other);
        world.advance(&catalog, 50);

        assert!(
            world.snapshot_for(viewer, SIGHT_RADIUS).get(other.to_entity_id()).is_none(),
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

        // After the cooldown has elapsed it may fire again.
        for _ in 0..12 {
            world.advance(&catalog, 50);
        }
        assert_eq!(world.shoot(shooter, &catalog, 0.0).len(), 1);
    }

    #[test]
    fn enough_shots_kill_and_the_body_is_removed() {
        let catalog = catalog();
        let (mut world, shooter, target) = duel(&catalog);

        // 100 damage a shot against 10 defence and 200 hit points: three shots is plenty.
        for _ in 0..40 {
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
        let (world, enemy, _) = confrontation(
            &catalog,
            r#"enemy "Slime" { state idle { wander(0.4) } }"#,
        );

        assert_eq!(world.thinking(), 1);
        assert!(world.get(enemy).unwrap().mind.is_some());
    }

    #[test]
    fn an_enemy_with_no_behaviour_simply_stands_there() {
        let catalog = catalog();
        let (world, enemy, _) = confrontation(
            &catalog,
            r#"enemy "Something Else" { state idle { wander(0.4) } }"#,
        );

        assert_eq!(world.thinking(), 0, "a half-converted directory should still run");
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
        assert!(now > start + 0.5, "it should have closed the distance: {start} -> {now}");
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
        assert!(after < before, "the player should have taken fire: {before} -> {after}");
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

        let (programs, _) = compile(
            &parse(r#"enemy "Slime" { state idle { follow(2.0, 30, 1) } }"#).unwrap(),
        );
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
        assert_eq!(world.iter().filter(|(_, e)| e.kind == Kind::Container).count(), 1);

        // A minute later there is nothing left, so a cleared dungeon does not fill with bags.
        for _ in 0..(61_000 / 50) {
            world.advance(&catalog, 50);
        }
        assert_eq!(world.iter().filter(|(_, e)| e.kind == Kind::Container).count(), 0);
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
            world.iter().filter(|(_, e)| e.kind == Kind::Container).count(),
            0,
            "an empty bag is worse than no bag"
        );
    }

    #[test]
    fn a_move_for_an_entity_that_no_longer_exists_is_refused_safely() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world.spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800)).unwrap();
        world.despawn(player);

        assert!(world.resolve_move(player, &catalog, 17.0, 16.0, 50).is_none());
    }
}
