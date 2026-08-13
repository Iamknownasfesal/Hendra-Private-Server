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

use hendra_content::{Catalog, ConditionSet, ObjectType};
use hendra_net::{EntityId, EntityState, Tick, WorldSnapshot};

use crate::grid::Grid;
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
    grid: Grid,
    tick: Tick,

    /// Reused between ticks so a warm world allocates nothing.
    handles: Vec<Handle>,
    nearby: Vec<Handle>,
    visible: Vec<(EntityId, EntityState)>,
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
            grid,
            tick: Tick::ZERO,
            handles: Vec::new(),
            nearby: Vec::new(),
            visible: Vec::new(),
        };
        world.reindex();
        world
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

    /// Applies a resolved move.
    pub fn place(&mut self, handle: Handle, outcome: MoveOutcome) {
        if let Some(entity) = self.entities.get_mut(handle) {
            entity.x = outcome.x;
            entity.y = outcome.y;
        }
    }

    /// Advances the world by one tick.
    pub fn advance(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        self.tick = self.tick.next();

        self.apply_hazards(catalog, elapsed_ms);
        self.reap();
        self.reindex();
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
            if let Some(entity) = self.entities.get(*handle) {
                self.visible.push((handle.to_entity_id(), entity.state()));
            }
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
        <Object type="0x502" id="Slime"><Class>Character</Class><Enemy/><MaxHitPoints>200</MaxHitPoints></Object>
        <Object type="0x600" id="Hero"><Class>Player</Class><Player/></Object>
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

    #[test]
    fn a_move_for_an_entity_that_no_longer_exists_is_refused_safely() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world.spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800)).unwrap();
        world.despawn(player);

        assert!(world.resolve_move(player, &catalog, 17.0, 16.0, 50).is_none());
    }
}
