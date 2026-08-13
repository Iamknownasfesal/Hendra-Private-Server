//! A uniform spatial grid, rebuilt once per tick.
//!
//! Every expensive question the simulation asks is "what is near this point?": what a player can
//! see, what a bullet just hit, what an enemy might chase. On a tile map with a bounded query
//! radius, a uniform grid answers that better than a tree: no rebalancing, no pointer chasing, and
//! a rebuild that is a counting sort.
//!
//! # Layout
//!
//! Cells are not `Vec`s. A `Vec` per cell would mean thousands of allocations per tick and a heap
//! walk per query. Instead the rebuild counts how many entities fall in each cell, prefix-sums the
//! counts into offsets, and scatters entries into one contiguous array, the same shape a sparse
//! matrix uses. A query then walks a handful of contiguous slices.
//!
//! ```text
//!   starts:  [0, 0, 2, 2, 5, ...]      one per cell, plus a final total
//!   entries: [e,e | | e,e,e | ...]     entities, grouped by cell
//! ```
//!
//! Both arrays are reused between ticks, so a warm grid allocates nothing.

use crate::slab::Handle;

/// How many tiles a cell spans, by default.
///
/// Chosen against the two query sizes that actually occur. A sight query of radius 20 touches about
/// forty-nine cells; a collision query of radius 1 touches four. Smaller cells would make sight
/// queries scan hundreds of cells, larger ones would make every collision query examine entities
/// far outside its radius.
pub const DEFAULT_CELL_SIZE: f32 = 8.0;

#[derive(Debug, Clone, Copy)]
struct Entry {
    handle: Handle,
    x: f32,
    y: f32,
}

impl Entry {
    const EMPTY: Entry = Entry {
        handle: Handle::NONE,
        x: 0.0,
        y: 0.0,
    };
}

/// A spatial index over one world.
pub struct Grid {
    cell_size: f32,
    cols: usize,
    rows: usize,

    /// Offset into `entries` where each cell begins, with a final entry holding the total.
    starts: Vec<u32>,
    entries: Vec<Entry>,

    /// Scratch for the counting sort, kept between rebuilds.
    counts: Vec<u32>,

    /// Entities as they arrived, before being grouped. Kept so the scatter pass has somewhere to
    /// read from that is not the array it is writing to.
    pending: Vec<Entry>,
}

impl Grid {
    pub fn new(width: u32, height: u32) -> Grid {
        Grid::with_cell_size(width, height, DEFAULT_CELL_SIZE)
    }

    pub fn with_cell_size(width: u32, height: u32, cell_size: f32) -> Grid {
        let cell_size = cell_size.max(1.0);
        let cols = ((width as f32 / cell_size).ceil() as usize).max(1);
        let rows = ((height as f32 / cell_size).ceil() as usize).max(1);

        Grid {
            cell_size,
            cols,
            rows,
            starts: vec![0; cols * rows + 1],
            entries: Vec::new(),
            counts: vec![0; cols * rows],
            pending: Vec::new(),
        }
    }

    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    pub fn cells(&self) -> usize {
        self.cols * self.rows
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Which cell a position falls in, clamped to the grid.
    ///
    /// Clamping rather than rejecting: an entity slightly outside the map, mid-knockback or on a
    /// map whose objects sit on the boundary, should still be findable. Rejecting it instead is
    /// that it silently vanishes from every query.
    #[inline]
    fn cell_of(&self, x: f32, y: f32) -> (usize, usize) {
        let column = (x / self.cell_size).floor().max(0.0) as usize;
        let row = (y / self.cell_size).floor().max(0.0) as usize;
        (column.min(self.cols - 1), row.min(self.rows - 1))
    }

    #[inline]
    fn index_of(&self, column: usize, row: usize) -> usize {
        row * self.cols + column
    }

    /// Rebuilds the index from scratch.
    ///
    /// Called once per tick with every entity that can be queried. Incremental updates were
    /// considered and rejected: entities move every tick anyway, so an incremental scheme would
    /// touch nearly every entry while also carrying the bookkeeping to know which.
    pub fn rebuild(&mut self, entities: impl IntoIterator<Item = (Handle, f32, f32)>) {
        self.counts.iter_mut().for_each(|count| *count = 0);
        self.pending.clear();

        // First pass: gather, and count how many land in each cell.
        for (handle, x, y) in entities {
            let (column, row) = self.cell_of(x, y);
            let cell = self.index_of(column, row);
            self.counts[cell] += 1;
            self.pending.push(Entry { handle, x, y });
        }

        // Prefix sum turns counts into the offset each cell starts at.
        let mut running = 0u32;
        for cell in 0..self.counts.len() {
            self.starts[cell] = running;
            running += self.counts[cell];
        }
        let last = self.starts.len() - 1;
        self.starts[last] = running;

        // Second pass: scatter into place, reusing `counts` as a per-cell cursor.
        self.counts.iter_mut().for_each(|count| *count = 0);
        self.entries.clear();
        self.entries.resize(self.pending.len(), Entry::EMPTY);

        for index in 0..self.pending.len() {
            let entry = self.pending[index];
            let (column, row) = self.cell_of(entry.x, entry.y);
            let cell = self.index_of(column, row);
            let at = (self.starts[cell] + self.counts[cell]) as usize;
            self.entries[at] = entry;
            self.counts[cell] += 1;
        }
    }

    /// Every entity within `radius` of a point, appended to `out`.
    ///
    /// `out` is cleared first and is expected to be a buffer the caller reuses, so a query
    /// allocates nothing once it has grown.
    pub fn within(&self, x: f32, y: f32, radius: f32, out: &mut Vec<Handle>) {
        out.clear();
        if radius <= 0.0 || self.entries.is_empty() {
            return;
        }

        let (min_column, min_row) = self.cell_of(x - radius, y - radius);
        let (max_column, max_row) = self.cell_of(x + radius, y + radius);
        let radius_squared = radius * radius;

        for row in min_row..=max_row {
            for column in min_column..=max_column {
                let cell = self.index_of(column, row);
                let from = self.starts[cell] as usize;
                let to = self.starts[cell + 1] as usize;

                for entry in &self.entries[from..to] {
                    let dx = entry.x - x;
                    let dy = entry.y - y;
                    if dx * dx + dy * dy <= radius_squared {
                        out.push(entry.handle);
                    }
                }
            }
        }
    }

    /// Every entity within `radius`, with its squared distance, nearest first.
    ///
    /// Squared rather than actual distance because nothing downstream needs the square root and
    /// skipping it keeps the comparison exact. Sorting costs more than [`Grid::within`], so this is
    /// for the cases that genuinely need order, such as choosing a target or filling a snapshot that may
    /// have to be truncated, where the nearest entities are the ones worth keeping.
    pub fn within_ranked(&self, x: f32, y: f32, radius: f32, out: &mut Vec<(Handle, f32)>) {
        out.clear();
        if radius <= 0.0 || self.entries.is_empty() {
            return;
        }

        let (min_column, min_row) = self.cell_of(x - radius, y - radius);
        let (max_column, max_row) = self.cell_of(x + radius, y + radius);
        let radius_squared = radius * radius;

        for row in min_row..=max_row {
            for column in min_column..=max_column {
                let cell = self.index_of(column, row);
                let from = self.starts[cell] as usize;
                let to = self.starts[cell + 1] as usize;

                for entry in &self.entries[from..to] {
                    let dx = entry.x - x;
                    let dy = entry.y - y;
                    let distance = dx * dx + dy * dy;
                    if distance <= radius_squared {
                        out.push((entry.handle, distance));
                    }
                }
            }
        }

        // The distance travels with the handle, so ordering does not re-derive it per comparison.
        out.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(n: u16) -> Handle {
        Handle(n as u32)
    }

    /// A brute-force answer, to check the grid against.
    fn expected(points: &[(Handle, f32, f32)], x: f32, y: f32, radius: f32) -> Vec<Handle> {
        let mut found: Vec<Handle> = points
            .iter()
            .filter(|(_, px, py)| {
                let (dx, dy) = (px - x, py - y);
                dx * dx + dy * dy <= radius * radius
            })
            .map(|(handle, _, _)| *handle)
            .collect();
        found.sort_unstable();
        found
    }

    #[test]
    fn an_empty_grid_answers_nothing() {
        let grid = Grid::new(64, 64);
        let mut out = Vec::new();
        grid.within(10.0, 10.0, 5.0, &mut out);
        assert!(out.is_empty());
        assert!(grid.is_empty());
    }

    #[test]
    fn a_query_finds_what_is_inside_and_nothing_outside() {
        let mut grid = Grid::new(64, 64);
        let points = [
            (handle(1), 10.0, 10.0),
            (handle(2), 12.0, 10.0),
            (handle(3), 30.0, 30.0),
        ];
        grid.rebuild(points.iter().copied());

        let mut out = Vec::new();
        grid.within(10.0, 10.0, 3.0, &mut out);
        out.sort_unstable();
        assert_eq!(out, vec![handle(1), handle(2)]);

        grid.within(10.0, 10.0, 1.0, &mut out);
        assert_eq!(out, vec![handle(1)]);
    }

    #[test]
    fn the_grid_agrees_with_brute_force_everywhere() {
        // The case a spatial index gets wrong is the boundary, so sweep query centres across cell
        // edges and compare against the obvious implementation.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 10_000) as f32 / 100.0
        };

        let points: Vec<(Handle, f32, f32)> =
            (0..400).map(|n| (handle(n), next(), next())).collect();

        let mut grid = Grid::new(100, 100);
        grid.rebuild(points.iter().copied());

        let mut out = Vec::new();
        for step in 0..60 {
            // Deliberately land on and around cell boundaries.
            let centre = step as f32 * DEFAULT_CELL_SIZE / 2.0;
            for radius in [0.5f32, 1.0, 7.9, 8.0, 8.1, 20.0] {
                grid.within(centre, centre, radius, &mut out);
                out.sort_unstable();
                assert_eq!(
                    out,
                    expected(&points, centre, centre, radius),
                    "at ({centre}, {centre}) radius {radius}"
                );
            }
        }
    }

    #[test]
    fn an_entity_exactly_on_a_cell_boundary_is_found_from_both_sides() {
        let mut grid = Grid::new(64, 64);
        let on_the_line = DEFAULT_CELL_SIZE;
        grid.rebuild([(handle(1), on_the_line, on_the_line)]);

        let mut out = Vec::new();
        grid.within(on_the_line - 0.5, on_the_line - 0.5, 1.0, &mut out);
        assert_eq!(out, vec![handle(1)], "from the cell below");

        grid.within(on_the_line + 0.5, on_the_line + 0.5, 1.0, &mut out);
        assert_eq!(out, vec![handle(1)], "from the cell above");
    }

    #[test]
    fn a_query_returns_each_entity_once() {
        // A radius spanning many cells must not report an entity per cell it overlaps.
        let mut grid = Grid::new(128, 128);
        let points: Vec<(Handle, f32, f32)> = (0..50)
            .map(|n| (handle(n), 64.0 + (n as f32 % 5.0), 64.0 + (n as f32 / 5.0)))
            .collect();
        grid.rebuild(points.iter().copied());

        let mut out = Vec::new();
        grid.within(64.0, 64.0, 40.0, &mut out);

        let mut unique = out.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(out.len(), unique.len(), "an entity was reported twice");
        assert_eq!(out.len(), 50);
    }

    #[test]
    fn entities_outside_the_map_are_still_findable() {
        let mut grid = Grid::new(32, 32);
        grid.rebuild([
            (handle(1), -5.0, -5.0),
            (handle(2), 100.0, 100.0),
            (handle(3), 16.0, 16.0),
        ]);

        let mut out = Vec::new();
        grid.within(-5.0, -5.0, 1.0, &mut out);
        assert_eq!(out, vec![handle(1)], "negative coordinates clamp inward");

        grid.within(100.0, 100.0, 1.0, &mut out);
        assert_eq!(out, vec![handle(2)], "coordinates past the edge clamp too");
    }

    #[test]
    fn rebuilding_replaces_rather_than_accumulates() {
        let mut grid = Grid::new(64, 64);
        grid.rebuild([(handle(1), 10.0, 10.0)]);
        assert_eq!(grid.len(), 1);

        grid.rebuild([(handle(2), 20.0, 20.0), (handle(3), 21.0, 20.0)]);
        assert_eq!(grid.len(), 2);

        let mut out = Vec::new();
        grid.within(10.0, 10.0, 2.0, &mut out);
        assert!(out.is_empty(), "the previous contents should be gone");
    }

    #[test]
    fn ranked_queries_order_by_distance() {
        let mut grid = Grid::new(64, 64);
        grid.rebuild([
            (handle(3), 13.0, 10.0),
            (handle(1), 10.5, 10.0),
            (handle(2), 12.0, 10.0),
        ]);

        let mut out = Vec::new();
        grid.within_ranked(10.0, 10.0, 10.0, &mut out);
        let order: Vec<Handle> = out.iter().map(|(handle, _)| *handle).collect();
        assert_eq!(order, vec![handle(1), handle(2), handle(3)]);

        // Distances come back squared and in step with the handles.
        assert!((out[0].1 - 0.25).abs() < 1e-5, "got {}", out[0].1);
    }

    #[test]
    fn a_warm_grid_stops_allocating() {
        let mut grid = Grid::new(128, 128);
        let points: Vec<(Handle, f32, f32)> = (0..200)
            .map(|n| (handle(n), (n % 100) as f32, (n / 100) as f32))
            .collect();

        grid.rebuild(points.iter().copied());
        let warm = (grid.entries.capacity(), grid.starts.capacity());

        let mut out = Vec::new();
        for _ in 0..50 {
            grid.rebuild(points.iter().copied());
            grid.within(50.0, 1.0, 20.0, &mut out);
        }

        assert_eq!(
            (grid.entries.capacity(), grid.starts.capacity()),
            warm,
            "the index grew after warm-up"
        );
    }

    #[test]
    fn a_zero_or_negative_radius_finds_nothing() {
        let mut grid = Grid::new(64, 64);
        grid.rebuild([(handle(1), 10.0, 10.0)]);

        let mut out = Vec::new();
        grid.within(10.0, 10.0, 0.0, &mut out);
        assert!(out.is_empty());

        grid.within(10.0, 10.0, -1.0, &mut out);
        assert!(out.is_empty());
    }
}
