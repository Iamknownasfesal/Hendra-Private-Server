//! Terrain, reduced to the questions the tick actually asks.
//!
//! A map square carries a ground type, which carries a descriptor, which says whether it can be
//! walked on. Following that chain during movement means two indirections and a branch for every
//! step of every entity, every tick, for an answer that cannot change while the world is running.
//!
//! So it is answered once, at load, into bitsets: one bit per square for walkable, one for blocks
//! sight. A 2048×2048 realm costs 512 KB per bitset and turns the movement check into a shift and a
//! mask. Speed and damage stay behind the descriptor, because those are read only when an entity is
//! actually standing somewhere and are not worth the memory.

use hendra_content::{Catalog, Map, ObjectType, TileType};

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

    fn put(&mut self, x: u32, y: u32, on: bool) {
        let bit = self.at(x, y);
        if on {
            self.words[bit / 64] |= 1u64 << (bit % 64);
        } else {
            self.words[bit / 64] &= !(1u64 << (bit % 64));
        }
    }

    fn clear_bit(&mut self, x: u32, y: u32) {
        self.put(x, y, false);
    }

    #[inline]
    fn get(&self, x: u32, y: u32) -> bool {
        let bit = self.at(x, y);
        self.words[bit / 64] & (1u64 << (bit % 64)) != 0
    }

    fn count(&self) -> usize {
        self.words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }
}

/// How many tiles wide a blocker-summary region is.
///
/// A coarse second grid recording only "does this region contain anything that blocks sight". Most
/// of most maps is open ground, and a sight test across open ground can then be answered by a
/// handful of bitset lookups instead of walking twenty squares.
const REGION: u32 = 8;

/// The terrain of one world.
pub struct Terrain {
    width: u32,
    height: u32,
    walkable: BitGrid,
    blocks_sight: BitGrid,

    /// One bit per [`REGION`]-sized block, set when anything in it blocks sight.
    blocker_regions: BitGrid,
    region_columns: u32,
    region_rows: u32,

    /// Whether the map blocks sight anywhere at all.
    any_blockers: bool,

    /// What each square is, so it can be told to a client.
    ///
    /// Two bytes a square, which is eight megabytes for the largest map in the game. Kept because
    /// there is no other way to send the ground, and derived from the map rather than stored twice:
    /// the map itself is dropped once this is built.
    tiles: Vec<TileType>,

    map: Map,
}

impl Terrain {
    /// Precomputes everything the tick needs from a map.
    pub fn build(map: Map, catalog: &Catalog) -> Terrain {
        let (width, height) = (map.width(), map.height());
        let mut walkable = BitGrid::new(width, height);
        let mut blocks_sight = BitGrid::new(width, height);
        let mut tiles = vec![TileType(0); (width as usize) * (height as usize)];

        for y in 0..height {
            for x in 0..width {
                let Some(square) = map.at(x, y) else { continue };
                tiles[(y as usize) * (width as usize) + x as usize] = square.tile;

                // A square is walkable when its ground allows it and nothing standing there
                // objects. Absent ground is not walkable: that is how maps spell a hole.
                let ground_ok = catalog.tile(square.tile).is_some_and(|tile| !tile.no_walk);

                let object = catalog.object(square.object);
                let object_blocks =
                    object.is_some_and(|desc| desc.full_occupy || desc.occupy_square);

                if ground_ok && !object_blocks {
                    walkable.set(x, y);
                }

                if object.is_some_and(|desc| desc.blocks_sight) {
                    blocks_sight.set(x, y);
                }
            }
        }

        // Summarise the blockers into coarse regions, so an open sight line can be answered without
        // walking it.
        let region_columns = width.div_ceil(REGION);
        let region_rows = height.div_ceil(REGION);
        let mut blocker_regions = BitGrid::new(region_columns, region_rows);
        let mut any_blockers = false;

        for y in 0..height {
            for x in 0..width {
                if blocks_sight.get(x, y) {
                    blocker_regions.set(x / REGION, y / REGION);
                    any_blockers = true;
                }
            }
        }

        Terrain {
            width,
            height,
            walkable,
            blocks_sight,
            blocker_regions,
            region_columns,
            region_rows,
            any_blockers,
            tiles,
            map,
        }
    }

    /// Whether anything between two points could possibly block sight.
    ///
    /// A conservative test over the coarse grid: false means definitely clear, true means walk it
    /// properly. Checking the segment's bounding box rather than the regions it actually crosses
    /// costs a few extra lookups and keeps this simple enough to be obviously correct.
    fn could_block(&self, from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> bool {
        if !self.any_blockers {
            return false;
        }

        let region = |value: f32, limit: u32| -> u32 {
            (value.max(0.0) as u32 / REGION).min(limit.saturating_sub(1))
        };

        let min_column = region(from_x.min(to_x), self.region_columns);
        let max_column = region(from_x.max(to_x), self.region_columns);
        let min_row = region(from_y.min(to_y), self.region_rows);
        let max_row = region(from_y.max(to_y), self.region_rows);

        for row in min_row..=max_row {
            for column in min_column..=max_column {
                if self.blocker_regions.get(column, row) {
                    return true;
                }
            }
        }
        false
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

    /// What one square is.
    /// Paints one square: its ground, what stands on it, and how big that is.
    ///
    /// The map is changed as well as the collision bitmap, because the map is what a player joining
    /// later is told the world looks like. Changing only the bitmap gives a wall that blocks but
    /// cannot be seen.
    pub fn paint(
        &mut self,
        catalog: &Catalog,
        x: u32,
        y: u32,
        tile: Option<TileType>,
        object: Option<(ObjectType, u16)>,
        clear: bool,
    ) -> bool {
        if !self.contains(x, y) {
            return false;
        }

        let mut square = match self.map.at(x, y) {
            Some(square) => square.clone(),
            None => return false,
        };

        if let Some(tile) = tile {
            square.tile = tile;
        }
        if clear {
            square.object = ObjectType::NONE;
            square.config = String::new();
        }
        if let Some((object, size)) = object {
            square.object = object;
            square.config = if size > 0 {
                format!("size:{size}")
            } else {
                String::new()
            };
        }

        let ground_ok = catalog
            .tile(square.tile)
            .is_some_and(|ground| !ground.no_walk);
        let standing = catalog.object(square.object);
        let blocked = standing.is_some_and(|desc| desc.full_occupy || desc.occupy_square);
        let blocks_sight = standing.is_some_and(|desc| desc.blocks_sight);

        let tile_type = square.tile;

        // A map already naming as many distinct squares as an index can hold cannot take another.
        // Refusing and saying so beats repainting somebody else's square.
        if !self.map.set(x, y, square) {
            return false;
        }

        if let Some(held) = self.tiles.get_mut((y * self.width + x) as usize) {
            *held = tile_type;
        }

        self.walkable.put(x, y, ground_ok && !blocked);
        self.blocks_sight.put(x, y, blocks_sight);

        if blocks_sight {
            self.blocker_regions.set(x / REGION, y / REGION);
        } else {
            self.rebuild_region(x / REGION, y / REGION);
        }

        true
    }

    /// What terrain a square is, which is what decides who may be spawned on it.
    pub fn terrain_at(&self, x: u32, y: u32) -> hendra_content::Terrain {
        self.map
            .at(x, y)
            .map(|square| square.terrain)
            .unwrap_or_default()
    }

    /// How many squares of each terrain the map has, counted once.
    ///
    /// A realm's population is drawn from this: a terrain gives each of its enemies a fixed number
    /// of squares, so how much of the map is mountain decides how many gods live on it.
    pub fn terrain_census(&self) -> [u32; hendra_content::TERRAIN_COUNT] {
        let mut counts = [0u32; hendra_content::TERRAIN_COUNT];

        for y in 0..self.height {
            for x in 0..self.width {
                counts[self.terrain_at(x, y) as usize] += 1;
            }
        }

        counts
    }

    pub fn tile_at(&self, x: u32, y: u32) -> TileType {
        if !self.contains(x, y) {
            return TileType(0);
        }
        self.tiles
            .get((y * self.width + x) as usize)
            .copied()
            .unwrap_or(TileType(0))
    }

    /// What each square of a row is, run-length encoded.
    ///
    /// A map is mostly the same square repeated, so runs are the difference between a strip that
    /// fits in a message and one that does not. Returns nothing for a row outside the map.
    pub fn row_runs(&self, y: u32, from_x: u32, width: u32) -> Vec<(u16, u16)> {
        if y >= self.height || from_x >= self.width {
            return Vec::new();
        }

        let end = (from_x + width).min(self.width);
        let mut runs: Vec<(u16, u16)> = Vec::new();

        for x in from_x..end {
            let tile = self.tile_at(x, y).0;
            match runs.last_mut() {
                Some((count, held)) if *held == tile && *count < u16::MAX => *count += 1,
                _ => runs.push((1, tile)),
            }
        }

        runs
    }

    /// Changes what one square is, for behaviours that reshape the ground.
    ///
    /// The blocker regions are rebuilt for the square's own region rather than for the whole map,
    /// because a boss paving a floor does it a square at a time and rebuilding everything each
    /// time would cost more than the tick has.
    pub fn set_square(
        &mut self,
        x: u32,
        y: u32,
        tile: TileType,
        walkable: bool,
        blocks_sight: bool,
    ) {
        if !self.contains(x, y) {
            return;
        }

        if let Some(held) = self.tiles.get_mut((y * self.width + x) as usize) {
            *held = tile;
        }

        self.walkable.put(x, y, walkable);
        self.blocks_sight.put(x, y, blocks_sight);

        if blocks_sight {
            self.blocker_regions.set(x / REGION, y / REGION);
        } else {
            self.rebuild_region(x / REGION, y / REGION);
        }
    }

    /// Recomputes whether one coarse region contains anything that blocks sight.
    fn rebuild_region(&mut self, region_x: u32, region_y: u32) {
        let (from_x, from_y) = (region_x * REGION, region_y * REGION);
        for y in from_y..(from_y + REGION).min(self.height) {
            for x in from_x..(from_x + REGION).min(self.width) {
                if self.blocks_sight.get(x, y) {
                    return;
                }
            }
        }
        self.blocker_regions.clear_bit(region_x, region_y);
    }

    /// Whether a square stops sight passing through it.
    #[inline]
    pub fn blocks_sight(&self, x: u32, y: u32) -> bool {
        // Outside the map blocks sight, so a ray leaving the world stops rather than running on.
        !self.contains(x, y) || self.blocks_sight.get(x, y)
    }

    /// Whether a position, which is continuous rather than a square, can be occupied.
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

    /// Whether one point can see another.
    ///
    /// Walks the squares between the two and stops at the first that blocks. The endpoints are
    /// excluded: standing on a sight-blocking square must not hide you from yourself,
    /// and something standing *on* a tree is visible even though the tree blocks what is behind it.
    ///
    /// This is a supercover walk rather than a Bresenham line, so it visits every square the segment
    /// touches, including the ones it merely clips at a corner. A thin Bresenham line slips
    /// diagonally between two walls that meet at a corner, which players notice immediately because
    /// it lets them see and be seen through what is visibly a solid join.
    pub fn line_of_sight(&self, from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> bool {
        let (mut x, mut y) = (from_x.floor() as i64, from_y.floor() as i64);
        let (target_x, target_y) = (to_x.floor() as i64, to_y.floor() as i64);

        if x == target_x && y == target_y {
            return true;
        }

        // Both ends must be on the map before the coarse test can vouch for the space between them.
        let inside = |x: f32, y: f32| {
            x >= 0.0 && y >= 0.0 && (x as u32) < self.width && (y as u32) < self.height
        };
        if inside(from_x, from_y)
            && inside(to_x, to_y)
            && !self.could_block(from_x, from_y, to_x, to_y)
        {
            return true;
        }

        let (dx, dy) = (to_x - from_x, to_y - from_y);
        let (step_x, step_y) = (dx.signum() as i64, dy.signum() as i64);

        // Distance along the ray to the next square boundary on each axis, and how much distance
        // one whole square costs. An infinite delta means the ray never crosses that axis.
        let delta_x = if dx == 0.0 {
            f32::INFINITY
        } else {
            (1.0 / dx).abs()
        };
        let delta_y = if dy == 0.0 {
            f32::INFINITY
        } else {
            (1.0 / dy).abs()
        };

        let mut next_x = if dx == 0.0 {
            f32::INFINITY
        } else if dx > 0.0 {
            ((x + 1) as f32 - from_x) / dx
        } else {
            (from_x - x as f32) / -dx
        };
        let mut next_y = if dy == 0.0 {
            f32::INFINITY
        } else if dy > 0.0 {
            ((y + 1) as f32 - from_y) / dy
        } else {
            (from_y - y as f32) / -dy
        };

        // Bounded so a ray that somehow fails to terminate cannot spin. The bound is the taxicab
        // distance, which no correct walk exceeds.
        let limit = ((target_x - x).abs() + (target_y - y).abs() + 2) as usize;

        for _ in 0..limit {
            if next_x < next_y {
                next_x += delta_x;
                x += step_x;
            } else {
                next_y += delta_y;
                y += step_y;
            }

            if x == target_x && y == target_y {
                return true;
            }

            if x < 0 || y < 0 || self.blocks_sight(x as u32, y as u32) {
                return false;
            }
        }

        // Ran out of steps without arriving: treat as blocked rather than claim a view that was
        // never traced.
        false
    }

    /// The sight walk with the coarse short-circuit skipped.
    ///
    /// Exists so a test can check that the optimisation only ever saves work, never changes an
    /// answer, which is the failure mode of a conservative filter that turns out not to be.
    #[cfg(test)]
    fn line_of_sight_walked(&self, from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> bool {
        let (mut x, mut y) = (from_x.floor() as i64, from_y.floor() as i64);
        let (target_x, target_y) = (to_x.floor() as i64, to_y.floor() as i64);
        if x == target_x && y == target_y {
            return true;
        }

        let (dx, dy) = (to_x - from_x, to_y - from_y);
        let (step_x, step_y) = (dx.signum() as i64, dy.signum() as i64);
        let delta_x = if dx == 0.0 {
            f32::INFINITY
        } else {
            (1.0 / dx).abs()
        };
        let delta_y = if dy == 0.0 {
            f32::INFINITY
        } else {
            (1.0 / dy).abs()
        };

        let mut next_x = if dx == 0.0 {
            f32::INFINITY
        } else if dx > 0.0 {
            ((x + 1) as f32 - from_x) / dx
        } else {
            (from_x - x as f32) / -dx
        };
        let mut next_y = if dy == 0.0 {
            f32::INFINITY
        } else if dy > 0.0 {
            ((y + 1) as f32 - from_y) / dy
        } else {
            (from_y - y as f32) / -dy
        };

        let limit = ((target_x - x).abs() + (target_y - y).abs() + 2) as usize;
        for _ in 0..limit {
            if next_x < next_y {
                next_x += delta_x;
                x += step_x;
            } else {
                next_y += delta_y;
                y += step_y;
            }
            if x == target_x && y == target_y {
                return true;
            }
            if x < 0 || y < 0 || self.blocks_sight(x as u32, y as u32) {
                return false;
            }
        }
        false
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
    #[test]
    fn a_row_of_one_tile_is_one_run() {
        // A map is mostly the same square repeated, which is the difference between a strip that
        // fits in a message and one that does not.
        let catalog = catalog();
        let squares = (0..16 * 4).map(|_| square(0x10, 0));
        let terrain = Terrain::build(Map::from_squares(16, 4, squares).unwrap(), &catalog);

        let runs = terrain.row_runs(0, 0, 16);
        assert_eq!(runs, vec![(16, 0x10)]);
    }

    #[test]
    fn a_row_that_changes_is_several_runs_in_order() {
        let catalog = catalog();
        let mut squares: Vec<_> = (0..8 * 2).map(|_| square(0x10, 0)).collect();
        squares[2] = square(0x11, 0);
        squares[3] = square(0x11, 0);
        let terrain = Terrain::build(Map::from_squares(8, 2, squares).unwrap(), &catalog);

        assert_eq!(
            terrain.row_runs(0, 0, 8),
            vec![(2, 0x10), (2, 0x11), (4, 0x10)]
        );
    }

    #[test]
    fn every_row_together_is_the_whole_map() {
        let catalog = catalog();
        let squares = (0..8 * 6).map(|_| square(0x10, 0));
        let terrain = Terrain::build(Map::from_squares(8, 6, squares).unwrap(), &catalog);

        let total: u32 = (0..terrain.height())
            .flat_map(|y| terrain.row_runs(y, 0, terrain.width()))
            .map(|(count, _)| count as u32)
            .sum();

        assert_eq!(total, 8 * 6, "no square is left out or sent twice");
    }

    #[test]
    fn a_row_outside_the_map_is_nothing_rather_than_a_panic() {
        let catalog = catalog();
        let squares = (0..4 * 4).map(|_| square(0x10, 0));
        let terrain = Terrain::build(Map::from_squares(4, 4, squares).unwrap(), &catalog);

        assert!(terrain.row_runs(99, 0, 4).is_empty());
        assert!(terrain.row_runs(0, 99, 4).is_empty());
    }

    #[test]
    fn changing_the_ground_changes_what_is_sent() {
        // Otherwise a client joining after a boss reshaped the room would be told the old map.
        let catalog = catalog();
        let squares = (0..8 * 2).map(|_| square(0x10, 0));
        let mut terrain = Terrain::build(Map::from_squares(8, 2, squares).unwrap(), &catalog);

        terrain.set_square(3, 0, TileType(0x11), false, false);

        assert_eq!(
            terrain.row_runs(0, 0, 8),
            vec![(3, 0x10), (1, 0x11), (4, 0x10)]
        );
    }

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
        assert!(terrain.walkable(3, 0), "but you can still walk under it");
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
            vec![
                square(0x10, ObjectType::NONE.0),
                square(0x12, ObjectType::NONE.0),
            ],
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

    /// A 16×16 field of grass, with sight-blocking trees at the given squares.
    fn walled(trees: &[(u32, u32)]) -> Terrain {
        let mut squares: Vec<Composition> = (0..16 * 16)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        for (x, y) in trees {
            squares[(*y as usize) * 16 + (*x as usize)] = square(0x10, 0x501);
        }
        Terrain::build(Map::from_squares(16, 16, squares).unwrap(), &catalog())
    }

    #[test]
    fn open_ground_is_visible_across() {
        let terrain = walled(&[]);
        assert!(terrain.line_of_sight(1.5, 1.5, 14.5, 1.5));
        assert!(terrain.line_of_sight(1.5, 1.5, 14.5, 14.5));
        assert!(
            terrain.line_of_sight(14.5, 14.5, 1.5, 1.5),
            "and back again"
        );
    }

    #[test]
    fn a_wall_blocks_what_is_behind_it() {
        let wall: Vec<(u32, u32)> = (0..16).map(|y| (8, y)).collect();
        let terrain = walled(&wall);

        assert!(
            !terrain.line_of_sight(2.5, 8.5, 13.5, 8.5),
            "straight through"
        );
        assert!(
            !terrain.line_of_sight(2.5, 2.5, 13.5, 13.5),
            "diagonally through"
        );
        assert!(
            terrain.line_of_sight(2.5, 8.5, 6.5, 8.5),
            "short of the wall"
        );
    }

    #[test]
    fn sight_is_symmetric() {
        // Asymmetric visibility is the classic ray-casting bug: A sees B but B does not see A, so
        // one of them is shot by something they cannot see.
        let terrain = walled(&[(6, 6), (7, 6), (6, 7)]);

        for a in [(2.5f32, 2.5f32), (3.2, 9.8), (11.5, 4.5), (9.1, 9.1)] {
            for b in [(12.5f32, 12.5f32), (5.5, 11.5), (10.5, 2.5), (1.5, 7.5)] {
                assert_eq!(
                    terrain.line_of_sight(a.0, a.1, b.0, b.1),
                    terrain.line_of_sight(b.0, b.1, a.0, a.1),
                    "{a:?} and {b:?} disagree about seeing each other"
                );
            }
        }
    }

    #[test]
    fn a_diagonal_join_cannot_be_seen_through() {
        // Two walls meeting at a corner. A thin line slips between them; a supercover walk does not.
        let terrain = walled(&[(8, 7), (7, 8)]);
        assert!(
            !terrain.line_of_sight(7.5, 7.5, 8.5, 8.5),
            "the corner between two walls is not a gap"
        );
    }

    #[test]
    fn standing_on_cover_does_not_hide_you() {
        let terrain = walled(&[(8, 8)]);

        assert!(
            terrain.line_of_sight(5.5, 8.5, 8.5, 8.5),
            "the tree itself is visible"
        );
        assert!(
            terrain.line_of_sight(8.5, 8.5, 8.5, 8.5),
            "and it can see itself"
        );
        assert!(
            !terrain.line_of_sight(5.5, 8.5, 11.5, 8.5),
            "but not past it"
        );
    }

    #[test]
    fn a_ray_leaving_the_map_is_blocked() {
        let terrain = walled(&[]);
        assert!(!terrain.line_of_sight(1.5, 1.5, -5.0, 1.5));
        assert!(!terrain.line_of_sight(1.5, 1.5, 40.0, 40.0));
    }

    #[test]
    fn the_coarse_filter_saves_work_without_changing_answers() {
        // The optimisation must be conservative: it may only skip a walk it can prove is clear.
        // Blockers are scattered so that some region boxes contain one and some do not.
        let trees: Vec<(u32, u32)> = vec![
            (3, 3),
            (4, 3),
            (8, 7),
            (7, 8),
            (12, 2),
            (2, 12),
            (9, 9),
            (10, 9),
            (14, 14),
        ];
        let terrain = walled(&trees);

        let mut checked = 0usize;
        let mut x = 0.5f32;
        while x < 16.0 {
            let mut y = 0.5f32;
            while y < 16.0 {
                for target in [
                    (0.5f32, 0.5f32),
                    (15.5, 15.5),
                    (8.5, 1.5),
                    (1.5, 8.5),
                    (11.3, 6.7),
                    (6.7, 11.3),
                ] {
                    assert_eq!(
                        terrain.line_of_sight(x, y, target.0, target.1),
                        terrain.line_of_sight_walked(x, y, target.0, target.1),
                        "({x}, {y}) to {target:?} disagreed"
                    );
                    checked += 1;
                }
                y += 1.7;
            }
            x += 1.3;
        }

        assert!(
            checked > 400,
            "the sweep should be broad, checked {checked}"
        );
    }

    #[test]
    fn an_open_map_needs_no_walk_at_all() {
        let terrain = walled(&[]);
        assert!(!terrain.any_blockers, "nothing on this map blocks sight");
        assert!(terrain.line_of_sight(0.5, 0.5, 15.5, 15.5));
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
