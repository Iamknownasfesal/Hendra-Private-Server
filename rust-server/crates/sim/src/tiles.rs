//! Terrain, reduced to the questions the tick actually asks.
//!
//! A map square carries a ground type, which carries a descriptor, which says whether it can be
//! walked on. Following that chain during movement means two indirections and a branch for every
//! step of every entity, every tick, for an answer that cannot change while the world is running.
//!
//! So it is answered once, at load, into bitsets: one bit per square for each of the four things
//! movement asks about, one for blocks sight, and two for what a bullet reaching the square runs
//! into. A 2048×2048 realm costs 512 KB per bitset and turns the movement check into a shift and a
//! mask. Speed and damage stay behind the descriptor, because those are read only when an entity is
//! actually standing somewhere and are not worth the memory.
//!
//! The four movement bits are kept apart rather than folded together because the original asks two
//! different questions of them. `NoWalk` and `OccupySquare` (`EnemyOccupySquare` for anything the
//! server walks itself) refuse the square a body would land on; `FullOccupy` refuses the *four
//! neighbours* a body's half-tile reach overlaps, and never the square it is standing on. Folding
//! them into one "can this be stood on" bit answers a question neither side of the original asks.

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

/// What a square does to a shot crossing it.
///
/// Returns whether it stops every bullet, and whether it stops one that does not pass cover. The
/// rule is the client's, since what stops a bullet is what the player watched stop it: a square
/// with no ground at all ends the flight (`Projectile.as:217`, `Square.StopsProjectiles`), and so
/// does an object that occupies the square against enemies, or one that occupies it at all when
/// the shot does not pass cover (`Projectile.as:229-232`). An object that is itself a target does
/// not shelter anything, which is what lets a shot reach an enemy standing in a doorway.
///
/// Ground never stops a bullet, whatever it is: shots cross water and lava.
fn shot_stoppers(no_ground: bool, object: Option<&hendra_content::ObjectDesc>) -> (bool, bool) {
    let object = object.filter(|desc| !desc.enemy);

    (
        no_ground || object.is_some_and(|desc| desc.enemy_occupy_square),
        object.is_some_and(|desc| desc.occupy_square),
    )
}

/// How many tiles wide a blocker-summary region is.
///
/// A coarse second grid recording only "does this region contain anything that blocks sight". Most
/// of most maps is open ground, and a sight test across open ground can then be answered by a
/// handful of bitset lookups instead of walking twenty squares.
const REGION: u32 = 8;

/// Whose movement a collision test is being asked about.
///
/// The original asks the same shape of question for everything that moves but reads a different
/// flag for the square a body lands on. A player's collision runs on the client, where
/// `Square.isWalkable` (`Square.as:78`) reads `OccupySquare`; everything the server walks itself
/// goes through `Entity.TileOccupied` (`Entity.cs:510-534`), which reads `EnemyOccupySquare`. The
/// two are not the same set: 159 of the shipped objects carry the first without the second, and
/// they are the ones a player is stopped by and an enemy walks straight through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Walker {
    /// A player, held to the rule their own client enforces.
    Player,

    /// Anything the server walks itself: enemies, pets, decoys, anything a behaviour moves.
    Entity,
}

/// The terrain of one world.
pub struct Terrain {
    width: u32,
    height: u32,

    /// Ground that can be stood on: the square has a descriptor and it is not `NoWalk`.
    ground: BitGrid,

    /// Squares carrying an object with `OccupySquare`, which stops a player landing on them.
    occupies: BitGrid,

    /// Squares carrying an object with `EnemyOccupySquare`, which stops anything the server walks.
    enemy_occupies: BitGrid,

    /// Squares carrying an object with `FullOccupy`, which stops a body coming within half a tile.
    full_occupy: BitGrid,

    blocks_sight: BitGrid,

    /// Squares that end any shot: nothing there at all, or something in the way of every bullet.
    stops_shots: BitGrid,

    /// Squares that end a shot which does not pass cover.
    stops_covered_shots: BitGrid,

    /// One bit per [`REGION`]-sized block, set when anything in it blocks sight.
    blocker_regions: BitGrid,
    region_columns: u32,
    region_rows: u32,

    /// Whether the map blocks sight anywhere at all.
    any_blockers: bool,

    /// Which open area each square belongs to, for a `blocking: 3` world.
    ///
    /// `WmapTile.SightRegion` (`Wmap.cs:114`), which starts at one on every square and is filled in
    /// by [`Self::calc_region_blocks`]. Empty on every other world, because the original only labels
    /// a map whose `Blocking` is three (`World.FromWorldMap`, `World.cs:302-303`) and the labelling
    /// is a whole-map flood fill.
    sight_regions: Vec<i64>,

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
        let mut ground_ok_grid = BitGrid::new(width, height);
        let mut occupies = BitGrid::new(width, height);
        let mut enemy_occupies = BitGrid::new(width, height);
        let mut full_occupy = BitGrid::new(width, height);
        let mut blocks_sight = BitGrid::new(width, height);
        let mut stops_shots = BitGrid::new(width, height);
        let mut stops_covered_shots = BitGrid::new(width, height);
        let mut tiles = vec![TileType(0); (width as usize) * (height as usize)];

        for y in 0..height {
            for x in 0..width {
                let Some(square) = map.at(x, y) else {
                    // A square the map does not have is off the edge of the world as far as a
                    // bullet is concerned, and a bullet reaching one stops.
                    stops_shots.set(x, y);
                    continue;
                };
                tiles[(y as usize) * (width as usize) + x as usize] = square.tile;

                // Ground that can be stood on. Absent ground is not walkable: that is how maps
                // spell a hole.
                let ground = catalog.tile(square.tile);
                if ground.is_some_and(|tile| !tile.no_walk) {
                    ground_ok_grid.set(x, y);
                }

                // What stands on the square, recorded a flag at a time, because the destination
                // test and the neighbour test read different ones.
                let object = catalog.object(square.object);
                if object.is_some_and(|desc| desc.occupy_square) {
                    occupies.set(x, y);
                }
                if object.is_some_and(|desc| desc.enemy_occupy_square) {
                    enemy_occupies.set(x, y);
                }
                if object.is_some_and(|desc| desc.full_occupy) {
                    full_occupy.set(x, y);
                }

                if object.is_some_and(|desc| desc.blocks_sight) {
                    blocks_sight.set(x, y);
                }

                let (stops, stops_covered) = shot_stoppers(ground.is_none(), object);
                if stops {
                    stops_shots.set(x, y);
                }
                if stops_covered {
                    stops_covered_shots.set(x, y);
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
            ground: ground_ok_grid,
            occupies,
            enemy_occupies,
            full_occupy,
            blocks_sight,
            stops_shots,
            stops_covered_shots,
            blocker_regions,
            region_columns,
            region_rows,
            any_blockers,
            sight_regions: Vec::new(),
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

    /// Whether a square can be stood on at all. Outside the map is never walkable.
    ///
    /// The coarse answer, for choosing somewhere to put a body rather than for resolving a move: a
    /// square nothing at all objects to. Movement itself goes through [`Terrain::region_unblocked`],
    /// which asks the two finer questions the original asks.
    ///
    /// `World.IsPassable` with `spawning: true` (`World.cs:454-474`), which is what every caller in
    /// the original passes bar one. [`Terrain::passable`] is that one.
    #[inline]
    pub fn walkable(&self, x: u32, y: u32) -> bool {
        self.contains(x, y)
            && self.ground.get(x, y)
            && !self.occupies.get(x, y)
            && !self.full_occupy.get(x, y)
    }

    /// Whether a square is passable in the weaker sense the realm's own spawner uses.
    ///
    /// `World.IsPassable` left at its default `spawning: false` (`World.cs:454-474`), which drops
    /// the `OccupySquare` clause and keeps `NoWalk`, `FullOccupy` and `EnemyOccupySquare`. Only
    /// `Oryx.Spawn` reads it that way — it calls `IsPassable(pt.X, pt.Y)` bare at `Oryx.cs:559` and
    /// `:581` — and the difference is not small: an object that occupies its square against players
    /// but not against enemies is somewhere a realm enemy may be placed, and trees, cacti and rocks
    /// are all such objects. A realm that refuses them puts nothing in its forests.
    #[inline]
    pub fn passable(&self, x: u32, y: u32) -> bool {
        self.contains(x, y)
            && self.ground.get(x, y)
            && !self.enemy_occupies.get(x, y)
            && !self.full_occupy.get(x, y)
    }

    /// Whether a square holds something no body may come within half a tile of.
    #[inline]
    pub fn full_occupy(&self, x: u32, y: u32) -> bool {
        !self.contains(x, y) || self.full_occupy.get(x, y)
    }

    /// Whether the square a body would land on refuses it.
    ///
    /// `Entity.TileOccupied` (`Entity.cs:510-534`) and, for a player, the first clause of the
    /// client's `isValidPosition` (`Player.as:574-579`) reading `Square.isWalkable`. Off the map is
    /// occupied. The truncation is the original's: `(int)x` rounds toward zero, so a position
    /// between −1 and 0 lands on square 0 rather than off the edge, on both sides of the wire.
    #[inline]
    fn tile_occupied(&self, walker: Walker, x: f32, y: f32) -> bool {
        let (tile_x, tile_y) = (x as i32, y as i32);
        if tile_x < 0 || tile_y < 0 || !self.contains(tile_x as u32, tile_y as u32) {
            return true;
        }
        let (tile_x, tile_y) = (tile_x as u32, tile_y as u32);

        if !self.ground.get(tile_x, tile_y) {
            return true;
        }

        match walker {
            Walker::Player => self.occupies.get(tile_x, tile_y),
            Walker::Entity => self.enemy_occupies.get(tile_x, tile_y),
        }
    }

    /// Whether a square holds something a body may not come within half a tile of.
    ///
    /// `Entity.TileFullOccupied` (`Entity.cs:536-551`) and the client's `isFullOccupy`
    /// (`Player.as:634-637`). Off the map counts, which is what puts the same half-tile skin on the
    /// edge of the world as on a wall.
    #[inline]
    fn tile_full_occupied(&self, x: f32, y: f32) -> bool {
        let (tile_x, tile_y) = (x as i32, y as i32);
        if tile_x < 0 || tile_y < 0 || !self.contains(tile_x as u32, tile_y as u32) {
            return true;
        }

        self.full_occupy.get(tile_x as u32, tile_y as u32)
    }

    /// Whether a body may stand at an exact position.
    ///
    /// `Entity.RegionUnblocked` (`Entity.cs:447-508`) and `Player.isValidPosition`
    /// (`Player.as:574-638`), which are the same routine. A body is a point that reaches half a
    /// tile in every direction: as well as the square it lands on it has to clear whichever of up
    /// to four neighbours that reach overlaps, chosen by which side of the half-tile lines the
    /// position falls on. Sitting exactly on a line reaches nothing across it, which is why a
    /// player lined up on the half-tile fits through a one-square doorway and a player half a tile
    /// off does not.
    ///
    /// `from` is where the body started this move. Its own square is exempt from the landing test
    /// for a player, which is the client's rule (`Player.as:576`, `square_ == _local3`): without it
    /// somebody standing where an object has just appeared could never cross a half-tile line again
    /// and would be frozen where they stand. Nothing the server walks gets that exemption, because
    /// `RegionUnblocked` does not give it one.
    pub fn region_unblocked(
        &self,
        walker: Walker,
        from_x: f32,
        from_y: f32,
        x: f32,
        y: f32,
    ) -> bool {
        let own_square =
            walker == Walker::Player && x as i32 == from_x as i32 && y as i32 == from_y as i32;

        if !own_square && self.tile_occupied(walker, x, y) {
            return false;
        }

        let x_frac = x - (x as i32) as f32;
        let y_frac = y - (y as i32) as f32;

        if x_frac < 0.5 {
            if self.tile_full_occupied(x - 1.0, y) {
                return false;
            }

            if y_frac < 0.5 {
                !(self.tile_full_occupied(x, y - 1.0) || self.tile_full_occupied(x - 1.0, y - 1.0))
            } else if y_frac > 0.5 {
                !(self.tile_full_occupied(x, y + 1.0) || self.tile_full_occupied(x - 1.0, y + 1.0))
            } else {
                true
            }
        } else if x_frac > 0.5 {
            if self.tile_full_occupied(x + 1.0, y) {
                return false;
            }

            if y_frac < 0.5 {
                !(self.tile_full_occupied(x, y - 1.0) || self.tile_full_occupied(x + 1.0, y - 1.0))
            } else if y_frac > 0.5 {
                !(self.tile_full_occupied(x, y + 1.0) || self.tile_full_occupied(x + 1.0, y + 1.0))
            } else {
                true
            }
        } else if y_frac < 0.5 {
            !self.tile_full_occupied(x, y - 1.0)
        } else if y_frac > 0.5 {
            !self.tile_full_occupied(x, y + 1.0)
        } else {
            true
        }
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

        self.put(catalog, x, y, square)
    }

    /// Writes one whole square: its ground, its object, its settings, its region and its terrain.
    ///
    /// The single road by which a square changes after the world is built. Everything the tick asks
    /// about a square is recomputed here from the composition, so the collision bitmaps and the map
    /// a late joiner is sent can never come apart — a wall that blocks but cannot be seen, or is
    /// seen but does not block, is the shape of that going wrong.
    ///
    /// Returns whether the square was taken. A map already naming as many distinct squares as an
    /// index can hold cannot take another, and refusing beats repainting somebody else's square.
    pub fn put(
        &mut self,
        catalog: &Catalog,
        x: u32,
        y: u32,
        square: hendra_content::Composition,
    ) -> bool {
        if !self.contains(x, y) {
            return false;
        }

        let ground = catalog.tile(square.tile);
        let ground_ok = ground.is_some_and(|tile| !tile.no_walk);
        let standing = catalog.object(square.object);
        let occupies = standing.is_some_and(|desc| desc.occupy_square);
        let enemy_occupies = standing.is_some_and(|desc| desc.enemy_occupy_square);
        let full_occupy = standing.is_some_and(|desc| desc.full_occupy);
        let blocks_sight = standing.is_some_and(|desc| desc.blocks_sight);
        let (stops_shots, stops_covered_shots) = shot_stoppers(ground.is_none(), standing);

        let tile_type = square.tile;

        if !self.map.set(x, y, square) {
            return false;
        }

        if let Some(held) = self.tiles.get_mut((y * self.width + x) as usize) {
            *held = tile_type;
        }

        self.ground.put(x, y, ground_ok);
        self.occupies.put(x, y, occupies);
        self.enemy_occupies.put(x, y, enemy_occupies);
        self.full_occupy.put(x, y, full_occupy);
        self.blocks_sight.put(x, y, blocks_sight);
        self.stops_shots.put(x, y, stops_shots);
        self.stops_covered_shots.put(x, y, stops_covered_shots);

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

    /// Changes the ground of one square, for behaviours that reshape it.
    ///
    /// Only the ground changes. What stands on the square keeps whatever it occupies, whatever it
    /// stops and whatever it hides, because `GroundTransform` rewrites `TileId`, `Spawned` and
    /// `UpdateCount` and nothing else (`GroundTransform.cs:72-74`): a boss paving a floor with lava
    /// does not put up a wall, and does not take one down either. Sight in particular is left
    /// alone — it belongs to the object, and clearing it here would let every enemy in the room see
    /// through a wall the moment the floor beneath it changed.
    ///
    /// What a bullet does at the square is recomputed, because a square with no ground at all ends
    /// a shot and one with ground does not, and that is the ground's to say.
    ///
    /// The map is rewritten as well as the bitmaps: it is what a player joining later is sent, and
    /// what the next [`Terrain::paint`] of the same square reads its other fields from. A map that
    /// cannot name another distinct square keeps the old ground rather than losing the change
    /// entirely — the bitmaps and the tile ids are already right by then.
    pub fn set_ground(&mut self, catalog: &Catalog, x: u32, y: u32, tile: TileType) {
        if !self.contains(x, y) {
            return;
        }

        if let Some(held) = self.tiles.get_mut((y * self.width + x) as usize) {
            *held = tile;
        }

        let ground = catalog.tile(tile);
        self.ground
            .put(x, y, ground.is_some_and(|desc| !desc.no_walk));

        let object = self
            .map
            .at(x, y)
            .map(|square| square.object)
            .unwrap_or(ObjectType::NONE);
        let (stops_shots, stops_covered_shots) =
            shot_stoppers(ground.is_none(), catalog.object(object));
        self.stops_shots.put(x, y, stops_shots);
        self.stops_covered_shots.put(x, y, stops_covered_shots);

        if let Some(square) = self.map.at(x, y) {
            let mut square = square.clone();
            square.tile = tile;
            self.map.set(x, y, square);
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

    /// Which open area a square belongs to, as a product of primes.
    ///
    /// One on a map that was never labelled, which makes every remainder zero and every square
    /// visible -- the same answer `WmapTile.SightRegion`'s initial value gives on a world whose
    /// `Blocking` is not three.
    #[inline]
    pub fn sight_region(&self, x: u32, y: u32) -> i64 {
        if !self.contains(x, y) {
            return 1;
        }
        self.sight_regions
            .get((y * self.width + x) as usize)
            .copied()
            .unwrap_or(1)
    }

    /// Whether this map carries the labels a `blocking: 3` world sees by.
    pub fn has_sight_regions(&self) -> bool {
        !self.sight_regions.is_empty()
    }

    /// Labels every connected open area of the map with a prime of its own.
    ///
    /// `Sight.CalcRegionBlocks` (`Sight.cs:277-289`), which the original runs once, at load, and
    /// only for a world whose `Blocking` is three (`World.cs:302-303`). Squares are visited column
    /// by column and every unlabelled open square starts a fresh flood fill under the next prime.
    ///
    /// A square that blocks sight is not given a label of its own: it is multiplied by the prime of
    /// every area that reaches it, so it divides evenly by all of them and is drawn from all of
    /// them. That is what makes a wall visible from both the rooms it separates while neither room
    /// sees the other.
    ///
    /// A map with more open areas than there are primes stops being labelled at that point rather
    /// than throwing as the original does; the squares past it keep the label one, which shows them
    /// from everywhere. There is no map in the content anywhere near the limit.
    pub fn calc_region_blocks(&mut self) {
        let squares = (self.width as usize) * (self.height as usize);
        self.sight_regions = vec![1i64; squares];

        let mut visited: Vec<bool> = vec![false; squares];
        let mut frontier: Vec<(u32, u32)> = Vec::new();
        let mut touched: Vec<usize> = Vec::new();
        let mut next = 0usize;

        for x in 0..self.width {
            for y in 0..self.height {
                let index = (y * self.width + x) as usize;
                if self.sight_regions[index] != 1 || self.blocks_sight.get(x, y) {
                    continue;
                }

                let Some(prime) = crate::sight::REGION_PRIMES.get(next).copied() else {
                    return;
                };
                next += 1;

                // The visited set is per area, as the original's `VisibleTilesSet` is: a wall is
                // reached once per area that touches it, and multiplied once for each.
                for index in touched.drain(..) {
                    visited[index] = false;
                }

                self.sight_regions[index] = prime;
                frontier.clear();
                frontier.push((x, y));

                let mut at = 0usize;
                while at < frontier.len() {
                    let (from_x, from_y) = frontier[at];
                    at += 1;

                    for (step_x, step_y) in crate::sight::REGION_NEIGHBOURS {
                        let (to_x, to_y) =
                            (from_x as i64 + step_x as i64, from_y as i64 + step_y as i64);
                        if to_x < 0
                            || to_y < 0
                            || to_x as u32 >= self.width
                            || to_y as u32 >= self.height
                        {
                            continue;
                        }
                        let (to_x, to_y) = (to_x as u32, to_y as u32);

                        let index = (to_y * self.width + to_x) as usize;
                        if visited[index] {
                            continue;
                        }
                        visited[index] = true;
                        touched.push(index);

                        if self.blocks_sight.get(to_x, to_y) {
                            self.sight_regions[index] =
                                self.sight_regions[index].saturating_mul(prime);
                            continue;
                        }

                        self.sight_regions[index] = prime;
                        frontier.push((to_x, to_y));
                    }
                }
            }
        }
    }

    /// Relabels a square that has stopped blocking sight, and joins the areas it separated.
    ///
    /// `Sight.UpdateRegion` (`Sight.cs:329-391`). The square takes the smallest label of the open
    /// squares around it; every wall still touching it is re-multiplied onto that label; and every
    /// square anywhere on the map carrying one of the other labels is moved onto it, because the
    /// areas the wall kept apart are now one area.
    ///
    /// The last step is a full sweep of the map, and it is the original's -- a wall coming down in
    /// the Shatters really does cost one pass over the whole map there.
    ///
    /// Neighbours off the edge of the map are skipped. The original indexes them without a bounds
    /// test and would throw; there is no behaviour to match in a throw.
    pub fn update_sight_region(&mut self, x: u32, y: u32) {
        if self.sight_regions.is_empty() || !self.contains(x, y) {
            return;
        }

        let mut connected: Vec<i64> = Vec::with_capacity(8);
        for (step_x, step_y) in crate::sight::REGION_NEIGHBOURS {
            let (near_x, near_y) = (x as i64 + step_x as i64, y as i64 + step_y as i64);
            if near_x < 0
                || near_y < 0
                || near_x as u32 >= self.width
                || near_y as u32 >= self.height
            {
                continue;
            }
            let (near_x, near_y) = (near_x as u32, near_y as u32);
            if !self.blocks_sight.get(near_x, near_y) {
                connected.push(self.sight_region(near_x, near_y));
            }
        }

        let here = (y * self.width + x) as usize;
        let Some(joined) = connected.iter().copied().min() else {
            self.sight_regions[here] = 1;
            return;
        };
        self.sight_regions[here] = joined;

        // Walls still standing around the square are shown from the joined area too.
        for (step_x, step_y) in crate::sight::REGION_NEIGHBOURS {
            let (near_x, near_y) = (x as i64 + step_x as i64, y as i64 + step_y as i64);
            if near_x < 0
                || near_y < 0
                || near_x as u32 >= self.width
                || near_y as u32 >= self.height
            {
                continue;
            }
            let (near_x, near_y) = (near_x as u32, near_y as u32);
            if !self.blocks_sight.get(near_x, near_y) {
                continue;
            }

            let index = (near_y * self.width + near_x) as usize;
            for label in &connected {
                if *label != 0 && self.sight_regions[index] % *label == 0 {
                    self.sight_regions[index] /= *label;
                }
            }
            self.sight_regions[index] = self.sight_regions[index].saturating_mul(joined);
        }

        // And every square of every area the wall was keeping apart moves onto the joined label.
        for label in connected {
            if label == joined || label == 0 {
                continue;
            }

            for held in self.sight_regions.iter_mut() {
                if *held % label != 0 {
                    continue;
                }
                *held /= label;
                *held = held.saturating_mul(joined);
            }
        }
    }

    /// Whether a square stops sight passing through it.
    #[inline]
    pub fn blocks_sight(&self, x: u32, y: u32) -> bool {
        // Outside the map blocks sight, so a ray leaving the world stops rather than running on.
        !self.contains(x, y) || self.blocks_sight.get(x, y)
    }

    /// Whether a bullet reaching this point would have broken against something.
    ///
    /// `Projectile.Blocked` (`Projectile.cs:321-345`) and the client's own test
    /// (`Projectile.as:217-232`), which is the one the player watched happen: off the map, a square
    /// with no ground, an object that occupies the square against enemies, or one that occupies it
    /// at all when the shot does not pass cover. Sight is not the question — a fence stops a bullet
    /// and does not block the view, and both are in the shipped content.
    #[inline]
    pub fn stops_shot(&self, x: f32, y: f32, passes_cover: bool) -> bool {
        if x < 0.0 || y < 0.0 {
            return true;
        }
        let (x, y) = (x as u32, y as u32);
        if !self.contains(x, y) {
            return true;
        }

        self.stops_shots.get(x, y) || !passes_cover && self.stops_covered_shots.get(x, y)
    }

    /// Whether a position, which is continuous rather than a square, can be occupied.
    #[inline]
    pub fn walkable_at(&self, x: f32, y: f32) -> bool {
        if x < 0.0 || y < 0.0 {
            return false;
        }
        self.walkable(x as u32, y as u32)
    }

    /// [`Terrain::passable`] at a position rather than a square.
    pub fn passable_at(&self, x: f32, y: f32) -> bool {
        if x < 0.0 || y < 0.0 {
            return false;
        }
        self.passable(x as u32, y as u32)
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

    /// The object standing on a square, if any.
    pub fn object_at(&self, x: f32, y: f32) -> Option<hendra_content::ObjectType> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let object = self.map.at(x as u32, y as u32)?.object;
        (!object.is_none()).then_some(object)
    }

    /// Which of the laboratory's two waters a position stands in, if either.
    ///
    /// `Some(true)` for the green water that hexes, `Some(false)` for the blue water that washes
    /// it off. `CheckLabConditions` (`MoveHandler.cs:41-84`) reads the raw tile identifier rather
    /// than anything the descriptors say, and refuses both when the square carries an object at
    /// all — a bridge or a wall laid over the water is dry ground.
    pub fn lab_water_at(&self, x: f32, y: f32) -> Option<bool> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let square = self.map.at(x as u32, y as u32)?;
        if !square.object.is_none() {
            return None;
        }
        match square.tile.0 {
            0xa9 | 0x82 => Some(true),
            0xa7 | 0x83 => Some(false),
            _ => None,
        }
    }

    /// Damage the ground deals per second at a position, if any.
    ///
    /// Read from the tile as it stands now rather than from the map the world was built from. The
    /// original has one of each square — `GroundTransform` writes `tile.TileId` in place
    /// (`GroundTransform.cs:73`) and `ChangeGroundOnDeath` puts a rewritten clone back
    /// (`ChangeGroundOnDeath.cs:38`, `:47`) — and burns off the same one it paints, so lava laid
    /// down mid-fight has to burn what stands in it.
    pub fn hazard_at(&self, catalog: &Catalog, x: f32, y: f32) -> Option<(i32, i32)> {
        if x < 0.0 || y < 0.0 || !self.contains(x as u32, y as u32) {
            return None;
        }
        let tile = catalog.tile(self.tile_at(x as u32, y as u32))?;
        tile.hurts().then_some((tile.min_damage, tile.max_damage))
    }

    /// What the ground at a position is called.
    ///
    /// What names a death by it: `ApplyGroundDamage` passes `tileDesc.ObjectId` as the killer
    /// (`Player.Ground.cs:111`), so somebody burned to death in lava was killed by "Lava" rather
    /// than by the world they were standing in. Read from the square as it stands, for the same
    /// reason [`Terrain::hazard_at`] is.
    pub fn tile_name_at<'a>(&self, catalog: &'a Catalog, x: f32, y: f32) -> Option<&'a str> {
        if x < 0.0 || y < 0.0 || !self.contains(x as u32, y as u32) {
            return None;
        }
        let tile = catalog.tile(self.tile_at(x as u32, y as u32))?;
        Some(&tile.id)
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
        self.ground
            .words
            .iter()
            .zip(&self.occupies.words)
            .zip(&self.full_occupy.words)
            .map(|((ground, occupies), full)| (ground & !occupies & !full).count_ones() as usize)
            .sum()
    }

    /// How many squares carry an object with each of the three occupancy flags, in the order
    /// `OccupySquare`, `EnemyOccupySquare`, `FullOccupy`. What a map's collision is actually made
    /// of, for a harness that wants to say which of the three moved an answer.
    pub fn occupancy_counts(&self) -> (usize, usize, usize) {
        (
            self.occupies.count(),
            self.enemy_occupies.count(),
            self.full_occupy.count(),
        )
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

        terrain.set_ground(&catalog, 3, 0, TileType(0x11));

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
        <Object type="0x502" id="Bramble"><Class>GameObject</Class><OccupySquare/><Static/></Object>
        <Object type="0x503" id="Pillar"><Class>GameObject</Class>
          <OccupySquare/><EnemyOccupySquare/><FullOccupy/><Static/></Object>
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

    /// A 5×5 field of grass with one object in the middle.
    fn field_with(object: u16) -> Terrain {
        let catalog = catalog();
        let mut squares: Vec<_> = (0..5 * 5).map(|_| square(0x10, 0)).collect();
        squares[2 * 5 + 2] = square(0x10, object);
        Terrain::build(Map::from_squares(5, 5, squares).unwrap(), &catalog)
    }

    #[test]
    fn a_square_a_player_is_stopped_by_is_not_one_an_enemy_is() {
        // `Square.isWalkable` reads `OccupySquare` and `Entity.TileOccupied` reads
        // `EnemyOccupySquare`. 159 of the shipped objects carry the first without the second, and
        // one bit for both makes every one of them stop an enemy that should walk over it.
        let terrain = field_with(0x502);

        assert!(
            !terrain.region_unblocked(Walker::Player, 1.5, 2.5, 2.5, 2.5),
            "a player is stopped by what occupies the square"
        );
        assert!(
            terrain.region_unblocked(Walker::Entity, 1.5, 2.5, 2.5, 2.5),
            "and an enemy is not, because nothing there occupies it against enemies"
        );
    }

    #[test]
    fn a_full_occupy_square_keeps_a_body_half_a_tile_away() {
        // `TileFullOccupied` is asked about the neighbours a body's reach overlaps, never about the
        // square it lands on. So the square beside a wall can be stood on, but only on the far half
        // of it, and exactly on the half-tile line the reach crosses nothing.
        let terrain = field_with(0x503);

        assert!(
            terrain.region_unblocked(Walker::Player, 1.5, 2.5, 1.4, 2.5),
            "the far half of the square beside it is clear"
        );
        assert!(
            terrain.region_unblocked(Walker::Player, 1.5, 2.5, 1.5, 2.5),
            "and so is the line itself"
        );
        assert!(
            !terrain.region_unblocked(Walker::Player, 1.5, 2.5, 1.6, 2.5),
            "but the near half reaches into the wall"
        );
    }

    #[test]
    fn a_body_cannot_slip_through_a_corner_join() {
        // Two walls meeting at a corner leave a diagonal that is nothing at all to a point and shut
        // to anything with a half-tile reach. Without the neighbour test every corner in the game
        // is a shortcut.
        let catalog = catalog();
        let mut squares: Vec<_> = (0..5 * 5).map(|_| square(0x10, 0)).collect();
        squares[2 * 5 + 3] = square(0x10, 0x503);
        squares[3 * 5 + 2] = square(0x10, 0x503);
        let terrain = Terrain::build(Map::from_squares(5, 5, squares).unwrap(), &catalog);

        assert!(
            terrain.walkable(2, 2) && terrain.walkable(3, 3),
            "both ends of the diagonal can be stood on"
        );
        assert!(
            !terrain.region_unblocked(Walker::Player, 2.5, 2.5, 2.9, 2.9),
            "and the gap between them cannot be passed"
        );
    }

    #[test]
    fn the_far_edge_of_the_map_has_the_same_skin_as_a_wall_and_the_near_edge_has_none() {
        // `TileFullOccupied` answers true off the map, so the world's edge holds a body off exactly
        // as a wall does. Only at the far edge, though: the neighbour is named as `x - 1` in
        // floating point and truncated toward zero, so at the near edge the lookup lands back on
        // square 0 instead of off the map and nothing is in the way. The original does this on both
        // sides of the wire and the asymmetry is its own.
        let terrain = field_with(0);

        assert!(
            !terrain.region_unblocked(Walker::Player, 4.5, 2.5, 4.6, 2.5),
            "the outer half of the last square reaches off the map"
        );
        assert!(
            terrain.region_unblocked(Walker::Player, 0.5, 2.5, 0.4, 2.5),
            "and the outer half of the first square does not, because -0.6 truncates to square 0"
        );
    }

    #[test]
    fn a_player_standing_where_an_object_appeared_is_not_frozen_there() {
        // The client's own escape (`Player.as:576`): the square a player is already in is exempt
        // from the landing test. Without it somebody an object has been dropped on could never
        // cross a half-tile line again. Nothing the server walks gets the exemption, because
        // `RegionUnblocked` does not give it one.
        let terrain = field_with(0x503);

        assert!(
            terrain.region_unblocked(Walker::Player, 2.5, 2.5, 2.9, 2.5),
            "a player can still move inside the square they are standing in"
        );
        assert!(
            !terrain.region_unblocked(Walker::Entity, 2.5, 2.5, 2.9, 2.5),
            "an enemy has no such exemption"
        );
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
            terrain.ground.words.len() * 8,
            512 * 512 / 8,
            "one bit per square"
        );
    }
}
