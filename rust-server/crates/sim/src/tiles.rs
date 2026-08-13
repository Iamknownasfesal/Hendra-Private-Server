//! Terrain, reduced to the questions the tick actually asks.
//!
//! A map square carries a ground type, which carries a descriptor, which says whether it can be
//! walked on. Following that chain during movement means two indirections and a branch for every
//! step of every entity, every tick — for an answer that cannot change while the world is running.
//!
//! So it is answered once, at load, into bitsets: one bit per square for walkable, one for blocks
//! sight. A 2048×2048 realm costs 512 KB per bitset and turns the movement check into a shift and a
//! mask. Speed and damage stay behind the descriptor, because those are read only when an entity is
//! actually standing somewhere and are not worth the memory.

use hendra_content::{Catalog, Map};

/// A bit per square.
struct BitGrid {
    words: Vec<u64>,
    width: u32,
}

impl BitGrid {
    fn new(width: u32, height: u32) -> BitGrid {
        let squares = (width as usize) * (height as usize);
        BitGrid {
            words: vec![0; squares.div_ceil(64)],
            width,
        }
    }

    #[inline]
    fn at(&self, x: u32, y: u32) -> usize {
        (y as usize) * (self.width as usize) + (x as usize)
    }

    fn set(&mut self, x: u32, y: u32) {
        let bit = self.at(x, y);
        self.words[bit / 64] |= 1u64 << (bit % 64);
    }

    #[inline]
    fn get(&self, x: u32, y: u32) -> bool {
        let bit = self.at(x, y);
        self.words[bit / 64] & (1u64 << (bit % 64)) != 0
    }

    fn count(&self) -> usize {
        self.words.iter().map(|word| word.count_ones() as usize).sum()
    }
}

/// The terrain of one world.
pub struct Terrain {
    width: u32,
    height: u32,
    walkable: BitGrid,
    blocks_sight: BitGrid,
    map: Map,
}

impl Terrain {
    /// Precomputes everything the tick needs from a map.
    pub fn build(map: Map, catalog: &Catalog) -> Terrain {
        let (width, height) = (map.width(), map.height());
        let mut walkable = BitGrid::new(width, height);
        let mut blocks_sight = BitGrid::new(width, height);

        for y in 0..height {
            for x in 0..width {
                let Some(square) = map.at(x, y) else { continue };

                // A square is walkable when its ground allows it and nothing standing there
                // objects. Absent ground is not walkable: that is how maps spell a hole.
                let ground_ok = catalog
                    .tile(square.tile)
                    .is_some_and(|tile| !tile.no_walk);

                let object = catalog.object(square.object);
                let object_blocks = object.is_some_and(|desc| desc.full_occupy || desc.occupy_square);

                if ground_ok && !object_blocks {
                    walkable.set(x, y);
                }

                if object.is_some_and(|desc| desc.blocks_sight) {
                    blocks_sight.set(x, y);
                }
            }
        }

        Terrain {
            width,
            height,
            walkable,
            blocks_sight,
            map,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn map(&self) -> &Map {
        &self.map
    }

    #[inline]
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x < self.width && y < self.height
    }

    /// Whether a square can be stood on. Outside the map is never walkable.
    #[inline]
    pub fn walkable(&self, x: u32, y: u32) -> bool {
        self.contains(x, y) && self.walkable.get(x, y)
    }

    /// Whether a square stops sight passing through it.
    #[inline]
    pub fn blocks_sight(&self, x: u32, y: u32) -> bool {
        // Outside the map blocks sight, so a ray leaving the world stops rather than running on.
        !self.contains(x, y) || self.blocks_sight.get(x, y)
    }

    /// Whether a position — which is continuous, not a square — can be occupied.
    #[inline]
    pub fn walkable_at(&self, x: f32, y: f32) -> bool {
        if x < 0.0 || y < 0.0 {
            return false;
        }
        self.walkable(x as u32, y as u32)
    }

    /// The movement multiplier of the ground at a position, 1.0 being ordinary.
    pub fn speed_at(&self, catalog: &Catalog, x: f32, y: f32) -> f32 {
        if x < 0.0 || y < 0.0 {
            return 1.0;
        }
        self.map
            .at(x as u32, y as u32)
            .and_then(|square| catalog.tile(square.tile))
            .map(|tile| tile.speed)
            .unwrap_or(1.0)
    }

    /// Damage the ground deals per second at a position, if any.
    pub fn hazard_at(&self, catalog: &Catalog, x: f32, y: f32) -> Option<(i32, i32)> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let tile = catalog.tile(self.map.at(x as u32, y as u32)?.tile)?;
        tile.hurts().then_some((tile.min_damage, tile.max_damage))
    }

    /// How many squares can be walked on. Useful for a sanity check after loading a map.
    pub fn walkable_count(&self) -> usize {
        self.walkable.count()
    }

    pub fn sight_blocking_count(&self) -> usize {
        self.blocks_sight.count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hendra_content::map::Composition;
    use hendra_content::{ObjectType, Region, TileType};

    /// Walkable grass, unwalkable water, damaging lava, a wall, and a sight-blocking tree.
    const FIXTURE: &str = r#"<Objects>
        <Ground type="0x10" id="Grass"><Speed>1</Speed></Ground>
        <Ground type="0x11" id="Water"><NoWalk/><Speed>0.5</Speed></Ground>
        <Ground type="0x12" id="Lava"><MinDamage>20</MinDamage><MaxDamage>40</MaxDamage></Ground>
        <Object type="0x500" id="Wall"><Class>GameObject</Class><FullOccupy/><Static/></Object>
        <Object type="0x501" id="Tree"><Class>GameObject</Class><BlocksSight/><Static/></Object>
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

    /// A 4×1 strip: grass, water, wall on grass, tree on grass.
    fn strip() -> Terrain {
        let map = Map::from_squares(
            4,
            1,
            vec![
                square(0x10, ObjectType::NONE.0),
                square(0x11, ObjectType::NONE.0),
                square(0x10, 0x500),
                square(0x10, 0x501),
            ],
        )
        .unwrap();
        Terrain::build(map, &catalog())
    }

    #[test]
    fn ground_decides_walkability() {
        let terrain = strip();
        assert!(terrain.walkable(0, 0), "grass");
        assert!(!terrain.walkable(1, 0), "water is NoWalk");
    }

    #[test]
    fn an_occupying_object_blocks_otherwise_good_ground() {
        let terrain = strip();
        assert!(!terrain.walkable(2, 0), "a wall stands on walkable grass");
    }

    #[test]
    fn sight_blocking_is_separate_from_walking() {
        let terrain = strip();
        assert!(terrain.blocks_sight(3, 0), "a tree blocks sight");
        assert!(
            terrain.walkable(3, 0),
            "but you can still walk under it"
        );
        assert!(!terrain.blocks_sight(0, 0), "open grass does not");
    }

    #[test]
    fn outside_the_map_is_unwalkable_and_opaque() {
        let terrain = strip();
        assert!(!terrain.walkable(4, 0));
        assert!(!terrain.walkable(0, 1));
        assert!(!terrain.walkable_at(-1.0, 0.0));
        assert!(!terrain.walkable_at(0.0, -1.0));

        // Opaque, so a sight ray leaving the world stops instead of running to the horizon.
        assert!(terrain.blocks_sight(4, 0));
        assert!(terrain.blocks_sight(99, 99));
    }

    #[test]
    fn continuous_positions_resolve_to_their_square() {
        let terrain = strip();
        assert!(terrain.walkable_at(0.0, 0.0));
        assert!(terrain.walkable_at(0.99, 0.99));
        assert!(!terrain.walkable_at(1.5, 0.5), "inside the water square");
    }

    #[test]
    fn ground_speed_and_hazards_come_from_the_descriptor() {
        let catalog = catalog();
        let map = Map::from_squares(
            2,
            1,
            vec![square(0x10, ObjectType::NONE.0), square(0x12, ObjectType::NONE.0)],
        )
        .unwrap();
        let terrain = Terrain::build(map, &catalog);

        assert_eq!(terrain.speed_at(&catalog, 0.5, 0.5), 1.0);
        assert_eq!(terrain.hazard_at(&catalog, 0.5, 0.5), None);
        assert_eq!(terrain.hazard_at(&catalog, 1.5, 0.5), Some((20, 40)));
    }

    #[test]
    fn the_counts_match_what_the_map_holds() {
        let terrain = strip();
        assert_eq!(terrain.walkable_count(), 2, "grass and the tree square");
        assert_eq!(terrain.sight_blocking_count(), 1);
        assert_eq!(terrain.width(), 4);
        assert_eq!(terrain.height(), 1);
    }

    #[test]
    fn a_large_map_costs_one_bit_per_square() {
        // 512×512 is a quarter of a million squares; the bitsets should be tens of kilobytes, not
        // megabytes, which is the whole reason for precomputing them this way.
        let squares = (0..512 * 512).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(512, 512, squares).unwrap();
        let terrain = Terrain::build(map, &catalog());

        assert_eq!(terrain.walkable_count(), 512 * 512);
        assert_eq!(
            terrain.walkable.words.len() * 8,
            512 * 512 / 8,
            "one bit per square"
        );
    }
}
