//! What one player can see of the map they are standing on.
//!
//! `realm/Sight.cs` in the original. One object answers the whole question: `GetSightCircle` returns
//! a set of squares, and `SendUpdate` drives the ground it sends, the scenery it sends and the
//! entities it draws from that one set (`Player.Update.cs:130-165`, `:278`). Everything here exists
//! so that the same is true on this server -- a wall that hides an enemy also stops the floor behind
//! it being drawn, and stops it counting towards the exploration fame.
//!
//! A world picks one of four algorithms by its `blocking` field, and they are genuinely different
//! shapes rather than one shape with a switch:
//!
//! - `0` sees the whole disc, walls and all. The realm, the nexus, the vault, and every world built
//!   in code, because `World`'s constructor leaves the field at zero.
//! - `1` floods out from the player and stops at anything that blocks sight, so you see the room you
//!   are standing in and nothing of the next one. Twenty-five of the content's worlds ask for this.
//! - `2` casts fifty-five rays. No world in the content asks for it, and it is here because the
//!   original has it.
//! - `3` labels every connected open area with a prime at load and asks whether the square shares
//!   the player's label. Three worlds ask for it: the Mad Lab, the Sewers and the Shatters.

use std::sync::LazyLock;

use crate::tiles::Terrain;

/// How far a player can see, in tiles. `Player.Radius` (`Player.Update.cs:64`).
pub const RADIUS: i32 = 20;

/// `Player.RadiusSqr` (`Player.Update.cs:65`), which is what every range test actually compares.
pub const RADIUS_SQR: i32 = RADIUS * RADIUS;

/// The eight neighbours of a square, in the original's order (`Sight.cs:26-36`).
///
/// The order decides nothing about the result -- the flood fill's frontier is a queue and its
/// membership test is a set -- but it decides the order squares are uncovered in, and that is the
/// order they reach the wire in.
const SURROUNDING: [(i32, i32); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

/// How far apart two samples along a ray are, in tiles (`Sight.cs:23`).
const RAY_STEP: f32 = 0.1;

/// The angle between one ray and the next, in radians (`Sight.cs:24`).
///
/// `2.30f / Radius`, which is 0.115 and gives fifty-five rays over a full turn. Wide enough that a
/// bare raycast leaves gaps at twenty tiles, which is why each unblocked sample also lights its
/// eight neighbours.
const ANGLE_STEP: f32 = 2.30 / RADIUS as f32;

/// How many distinct open areas a map may be labelled with (`MaxNumRegions`, `Sight.cs:18`).
const MAX_REGIONS: usize = 2048;

/// How a world decides what a player can see.
///
/// `Sight.GetSightCircle` picks between these on the world's `blocking` value, and the base `World`
/// constructor sets it to zero -- so the realm, the nexus, the vault and everything else built in
/// code show whatever is within twenty tiles, wall or no wall. Only a world loaded from a proto asks
/// for anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sight {
    /// The whole disc, walls ignored. `blocking: 0`, and the default.
    #[default]
    Unblocked,

    /// The room you are standing in, found by flooding out to the walls. `blocking: 1`, and what
    /// twenty-five of the content's worlds ask for.
    Room,

    /// Whatever fifty-five rays reach. `blocking: 2`, which no world in the content selects.
    Line,

    /// Whatever shares the open area you are standing in. `blocking: 3`.
    Region,
}

impl Sight {
    pub fn from_blocking(blocking: i32) -> Sight {
        match blocking {
            1 => Sight::Room,
            2 => Sight::Line,
            3 => Sight::Region,
            _ => Sight::Unblocked,
        }
    }

    /// Whether anything at all stands between a viewer and what they might see.
    pub fn occludes(self) -> bool {
        self != Sight::Unblocked
    }
}

/// Every offset within twenty tiles, as `InitUnblockedView` builds it (`Sight.cs:64-93`).
///
/// The original walks an outward spiral and keeps the offsets inside the radius; the spiral covers
/// the square that encloses the disc exactly once, so what comes out is the disc itself -- 1257
/// offsets, which is the `AppoxAreaOfSight` the original sizes its buffers by. Built the same way
/// rather than by two nested loops so that the order squares are uncovered in is the original's:
/// nearest first, which is what a client wants drawn first.
static UNBLOCKED_VIEW: LazyLock<Vec<(i32, i32)>> = LazyLock::new(|| {
    let mut view = vec![(0, 0)];
    let push = |view: &mut Vec<(i32, i32)>, x: i32, y: i32| {
        if x * x + y * y <= RADIUS_SQR {
            view.push((x, y));
        }
    };

    let (mut x, mut y) = (0i32, 0i32);
    let (mut i, mut j) = (1i32, 2i32);

    loop {
        x += 1;
        push(&mut view, x, y);
        for _ in 0..i {
            y -= 1;
            push(&mut view, x, y);
        }
        for _ in 0..j {
            x -= 1;
            push(&mut view, x, y);
        }
        for _ in 0..j {
            y += 1;
            push(&mut view, x, y);
        }
        for _ in 0..j {
            x += 1;
            push(&mut view, x, y);
        }

        i += 2;
        j += 2;
        if j > 2 * RADIUS {
            break;
        }
    }

    view
});

/// The fifty-five rays a `blocking: 2` world sees along, as `InitSightRays` builds them
/// (`Sight.cs:95-119`).
///
/// Each ray is sampled every tenth of a tile out to twenty and reduced to the distinct squares it
/// passes through, in order, so walking one and stopping at the first blocker stops it at the right
/// square. The angle is accumulated in single precision because the original's is, and the
/// accumulated error is what decides whether the fifty-sixth ray exists: it does not.
static SIGHT_RAYS: LazyLock<Vec<Vec<(i32, i32)>>> = LazyLock::new(|| {
    let mut rays = Vec::new();
    let mut angle = 0.0f32;

    while angle < std::f32::consts::TAU {
        let mut ray: Vec<(i32, i32)> = Vec::with_capacity(RADIUS as usize);
        let mut distance = RAY_STEP;

        while distance < RADIUS as f32 {
            let point = (
                (distance as f64 * (angle as f64).cos()) as i32,
                (distance as f64 * (angle as f64).sin()) as i32,
            );
            if !ray.contains(&point) {
                ray.push(point);
            }
            distance += RAY_STEP;
        }

        rays.push(ray);
        angle += ANGLE_STEP;
    }

    rays
});

/// How wide the box that holds any sight circle is.
///
/// Every algorithm is bounded by the radius: the disc and the flood fill by the `RadiusSqr` test,
/// and the rays by their own length of nineteen squares plus the one square their neighbours reach.
/// So a circle never leaves a 41x41 box around the player, and membership can be a bitmap of that
/// box rather than a hash of coordinates.
const SPAN: i32 = RADIUS * 2 + 1;

/// The squares one player can currently see.
///
/// `Sight._sCircle` (`Sight.cs:41`), and cached the same way: recomputed when the player's square
/// changes or when something that blocks sight near them is taken away (`Player.Move`,
/// `Player.cs:1108-1116`; `World.LeaveWorld`, `World.cs:397-405`), and otherwise reused. It is the
/// one answer that the ground pass, the scenery pass and the entity pass all read.
#[derive(Debug, Clone)]
pub struct SightCircle {
    /// Where the circle was taken from, as `Sight.LastX`/`LastY`. `None` until one has been taken.
    at: Option<(i32, i32)>,

    /// Whether the map under the circle changed since it was taken, which forces a fresh one even
    /// though the player has not moved. `Sight.UpdateCount` (`Sight.cs:45`).
    stale: bool,

    /// One bit per square of the 41x41 box centred on [`Self::at`], set for the squares in view.
    marks: Vec<u64>,

    /// The same squares in map coordinates, in the order they were found.
    tiles: Vec<(u32, u32)>,

    /// The flood fill's frontier, as `(x, y, generation)`. Held between calls so a circle costs no
    /// allocation to take.
    frontier: Vec<(i32, i32, i32)>,
}

impl Default for SightCircle {
    fn default() -> SightCircle {
        SightCircle::new()
    }
}

impl SightCircle {
    pub fn new() -> SightCircle {
        SightCircle {
            at: None,
            stale: true,
            marks: vec![0; ((SPAN * SPAN) as usize).div_ceil(64)],
            tiles: Vec::with_capacity(UNBLOCKED_VIEW.len()),
            frontier: Vec::with_capacity(UNBLOCKED_VIEW.len()),
        }
    }

    /// Every square in view, in the order the algorithm found them.
    pub fn tiles(&self) -> &[(u32, u32)] {
        &self.tiles
    }

    /// Where this circle was taken from.
    pub fn taken_at(&self) -> Option<(i32, i32)> {
        self.at
    }

    /// Whether a square is in view.
    ///
    /// This is `visibleTiles.Contains(new IntPoint((int)i.X, (int)i.Y))`, which is how the original
    /// decides whether an entity is drawn (`Player.Update.cs:209`, `:246`).
    pub fn contains(&self, x: u32, y: u32) -> bool {
        let Some((at_x, at_y)) = self.at else {
            return false;
        };
        let (dx, dy) = (x as i32 - at_x, y as i32 - at_y);
        if dx < -RADIUS || dx > RADIUS || dy < -RADIUS || dy > RADIUS {
            return false;
        }
        let index = ((dy + RADIUS) * SPAN + (dx + RADIUS)) as usize;
        self.marks[index / 64] & (1u64 << (index % 64)) != 0
    }

    /// Forces the next look to take a fresh circle even from the same square.
    ///
    /// `plr.Sight.UpdateCount++`, which the original does to everyone within the radius of a
    /// sight-blocking object that has just been removed (`World.cs:402-405`). Without it a player
    /// standing still when a wall comes down keeps the view that had the wall in it.
    pub fn go_stale(&mut self) {
        self.stale = true;
    }

    /// Brings the circle up to date for a player standing on a square, and says whether it changed.
    ///
    /// The early-out is `GetSightCircle`'s: `UpdateCount <= 0` returns the circle as it stands
    /// (`Sight.cs:137-138`), and `Player.Move` only raises the count when the player's integer
    /// square changes (`Player.cs:1108-1116`).
    pub fn refresh(&mut self, mode: Sight, terrain: &Terrain, at_x: i32, at_y: i32) -> bool {
        if !self.stale && self.at == Some((at_x, at_y)) {
            return false;
        }

        self.stale = false;
        self.at = Some((at_x, at_y));
        self.marks.fill(0);
        self.tiles.clear();

        match mode {
            Sight::Unblocked => self.unblocked(terrain, at_x, at_y),
            Sight::Room => self.room(terrain, at_x, at_y),
            Sight::Line => self.line(terrain, at_x, at_y),
            Sight::Region => self.region(terrain, at_x, at_y),
        }

        true
    }

    /// Marks a square, and says whether it was not already marked.
    ///
    /// This is the `HashSet.Add` the original's algorithms all lean on: a square already in the
    /// circle is neither added twice nor expanded twice.
    fn mark(&mut self, terrain: &Terrain, x: i32, y: i32) -> bool {
        let Some((at_x, at_y)) = self.at else {
            return false;
        };
        // Off the map is not in view. `Wmap.Contains` (`Player.Update.cs` calls it through
        // `map.Contains`) is the only bounds test the original does, and it is done everywhere.
        if x < 0 || y < 0 || x as u32 >= terrain.width() || y as u32 >= terrain.height() {
            return false;
        }

        let (dx, dy) = (x - at_x, y - at_y);
        if dx < -RADIUS || dx > RADIUS || dy < -RADIUS || dy > RADIUS {
            return false;
        }

        let index = ((dy + RADIUS) * SPAN + (dx + RADIUS)) as usize;
        let (word, bit) = (index / 64, 1u64 << (index % 64));
        if self.marks[word] & bit != 0 {
            return false;
        }

        self.marks[word] |= bit;
        self.tiles.push((x as u32, y as u32));
        true
    }

    /// Whether a square is already in the circle, without adding it.
    fn marked(&self, x: i32, y: i32) -> bool {
        let Some((at_x, at_y)) = self.at else {
            return false;
        };
        let (dx, dy) = (x - at_x, y - at_y);
        if dx < -RADIUS || dx > RADIUS || dy < -RADIUS || dy > RADIUS {
            return false;
        }
        let index = ((dy + RADIUS) * SPAN + (dx + RADIUS)) as usize;
        self.marks[index / 64] & (1u64 << (index % 64)) != 0
    }

    /// `CalcUnblockedSight` (`Sight.cs:173-186`): every square of the disc that the map has.
    fn unblocked(&mut self, terrain: &Terrain, at_x: i32, at_y: i32) {
        for (dx, dy) in UNBLOCKED_VIEW.iter() {
            self.mark(terrain, at_x + dx, at_y + dy);
        }
    }

    /// `CalcBlockedRoomSight` (`Sight.cs:207-249`): flood out from the player, stopping at walls.
    ///
    /// The frontier starts with the player's own square and grows by eight-connected neighbours.
    /// A neighbour that blocks sight is added to the circle -- you see the wall -- but is never
    /// expanded from, so nothing behind it is reached. Two limits stop it: the `RadiusSqr` test
    /// against the player's square, and a generation count that refuses to expand past twenty steps,
    /// which is what makes a corridor that doubles back cost its own length rather than its distance.
    ///
    /// The player's own square is put on the frontier but never marked, and reaches the circle only
    /// when a neighbour expands back into it. A player walled in on all eight sides therefore does
    /// not see the square they are standing on. That is the original's behaviour and it is kept.
    fn room(&mut self, terrain: &Terrain, at_x: i32, at_y: i32) {
        let mut frontier = std::mem::take(&mut self.frontier);
        frontier.clear();
        frontier.push((at_x, at_y, 0));

        let mut index = 0usize;
        while index < frontier.len() {
            let (tile_x, tile_y, generation) = frontier[index];
            index += 1;

            if generation > RADIUS {
                continue;
            }

            for (step_x, step_y) in SURROUNDING {
                let (x, y) = (tile_x + step_x, tile_y + step_y);
                let (dx, dy) = (at_x - x, at_y - y);

                if self.marked(x, y)
                    || x < 0
                    || x as u32 >= terrain.width()
                    || y < 0
                    || y as u32 >= terrain.height()
                    || dx * dx + dy * dy > RADIUS_SQR
                {
                    continue;
                }

                self.mark(terrain, x, y);

                if terrain.blocks_sight(x as u32, y as u32) {
                    continue;
                }

                frontier.push((x, y, generation + 1));
            }
        }

        self.frontier = frontier;
    }

    /// `CalcBlockedLineOfSight` (`Sight.cs:251-275`): fifty-five rays, each stopping at a wall.
    ///
    /// Every square a ray passes is added; the ray ends at the first that blocks sight, and every
    /// square it passed that did not also lights its eight neighbours. The dilation is what fills
    /// the gaps fifty-five rays leave at twenty tiles, and it is also why this shape is not the
    /// disc: it reaches a square diagonally past a corner that no ray could have touched.
    fn line(&mut self, terrain: &Terrain, at_x: i32, at_y: i32) {
        for ray in SIGHT_RAYS.iter() {
            for (step_x, step_y) in ray {
                let (x, y) = (at_x + step_x, at_y + step_y);

                if x < 0 || y < 0 || x as u32 >= terrain.width() || y as u32 >= terrain.height() {
                    continue;
                }

                self.mark(terrain, x, y);

                if terrain.blocks_sight(x as u32, y as u32) {
                    break;
                }

                for (around_x, around_y) in SURROUNDING {
                    self.mark(terrain, x + around_x, y + around_y);
                }
            }
        }
    }

    /// `CalcRegionBlockSight` (`Sight.cs:188-205`): the disc, filtered to the player's open area.
    ///
    /// Every square carries a label, and a square is in view when its label divides evenly by the
    /// label of the square the player stands on. Open squares carry one prime each, so that test is
    /// "the same area"; a wall carries the product of the primes of every area that touches it, so
    /// it is visible from all of them. See [`Terrain::calc_region_blocks`].
    fn region(&mut self, terrain: &Terrain, at_x: i32, at_y: i32) {
        let standing = if at_x < 0
            || at_y < 0
            || at_x as u32 >= terrain.width()
            || at_y as u32 >= terrain.height()
        {
            1
        } else {
            terrain.sight_region(at_x as u32, at_y as u32)
        };

        // A label of zero would make every remainder zero and show the whole disc. The original
        // cannot produce one -- every square starts at one -- and neither can this, but the division
        // below is only safe because of it.
        let standing = if standing == 0 { 1 } else { standing };

        for (dx, dy) in UNBLOCKED_VIEW.iter() {
            let (x, y) = (at_x + dx, at_y + dy);
            if x < 0 || y < 0 || x as u32 >= terrain.width() || y as u32 >= terrain.height() {
                continue;
            }

            if terrain.sight_region(x as u32, y as u32) % standing == 0 {
                self.mark(terrain, x, y);
            }
        }
    }
}

/// The first [`MAX_REGIONS`] primes, which is what an open area is labelled with.
///
/// `MathsUtils.GeneratePrimes(MaxNumRegions)` (`Sight.cs:61`, `Utils.cs:48-61`). A map with more
/// open areas than there are labels runs off the end of this list; the original throws there, and
/// this stops labelling instead, because a dungeon that fails to load is worse than one whose
/// two-thousand-and-forty-ninth cupboard is dark.
pub(crate) static REGION_PRIMES: LazyLock<Vec<i64>> = LazyLock::new(|| {
    let mut primes: Vec<i64> = Vec::with_capacity(MAX_REGIONS);
    let mut candidate = 2i64;
    while primes.len() < MAX_REGIONS {
        if primes
            .iter()
            .take_while(|prime| *prime * *prime <= candidate)
            .all(|prime| candidate % prime != 0)
        {
            primes.push(candidate);
        }
        candidate += 1;
    }
    primes
});

/// The neighbour offsets the region labelling walks, which are the sight circle's own.
pub(crate) const REGION_NEIGHBOURS: [(i32, i32); 8] = SURROUNDING;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unblocked_view_is_the_disc_the_original_spirals_out() {
        // `AppoxAreaOfSight` is `(int)(pi * 20 * 20 + 1)`, and the spiral produces exactly that
        // many offsets -- so the original's buffer sizing and its view agree, and the view is the
        // plain disc rather than an approximation of one.
        assert_eq!(UNBLOCKED_VIEW.len(), 1257);

        let mut disc: Vec<(i32, i32)> = Vec::new();
        for y in -RADIUS..=RADIUS {
            for x in -RADIUS..=RADIUS {
                if x * x + y * y <= RADIUS_SQR {
                    disc.push((x, y));
                }
            }
        }

        let mut spiralled = UNBLOCKED_VIEW.clone();
        spiralled.sort_unstable();
        spiralled.dedup();
        disc.sort_unstable();
        assert_eq!(spiralled, disc, "the spiral covers the disc exactly once");

        assert_eq!(UNBLOCKED_VIEW[0], (0, 0), "nearest first");
    }

    #[test]
    fn fifty_five_rays_cover_the_turn() {
        // `2.30f / 20` accumulated in single precision runs out at 6.325 radians, which is one step
        // past a full turn -- so there are fifty-five rays and the last of them overlaps the first.
        assert_eq!(SIGHT_RAYS.len(), 55);

        // Each ray is the distinct squares between the player and twenty tiles out, so it starts on
        // the player's own square and ends one short of the radius.
        assert_eq!(SIGHT_RAYS[0].first(), Some(&(0, 0)));
        assert_eq!(SIGHT_RAYS[0].last(), Some(&(19, 0)));
    }

    #[test]
    fn the_primes_are_the_first_two_thousand_and_forty_eight() {
        assert_eq!(REGION_PRIMES.len(), MAX_REGIONS);
        assert_eq!(REGION_PRIMES[0], 2);
        assert_eq!(REGION_PRIMES[1], 3);
        assert_eq!(REGION_PRIMES[2], 5);
        assert_eq!(*REGION_PRIMES.last().unwrap(), 17863);
    }
}
