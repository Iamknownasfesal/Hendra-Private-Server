//! The structures a realm is built with: temples, castles, groves, the lich's floor.
//!
//! Follows `wServer/realm/setpieces/`, where each of these is a small drawing program rather than a
//! saved map. A grove picks a radius and scatters cherry trees around its edge; a building draws
//! four walls, cuts two rooms out of them, then knocks holes in half of it at random. Two groves are
//! not the same grove, which is the point.
//!
//! # Why a drawing rather than a world
//!
//! A setpiece here produces a [`Drawing`]: a list of squares to paint and things to put on them. It
//! does not touch the world. That keeps the drawing pure and testable, keeps every change to the
//! world in one place, and means a setpiece can be checked square by square without building a
//! world to check it in.
//!
//! # Where they go
//!
//! [`SCATTER`] is the original's table: how many of each a realm gets and which ground they may
//! stand on. Positions are drawn at random and rejected where they would overlap something already
//! placed, with a limited number of attempts, so a full realm gets fewer rather than looping.

use hendra_content::Terrain;

/// A simple deterministic generator, so a setpiece can be drawn twice and checked.
///
/// The original uses `System.Random`, which is neither reproducible across runs nor worth
/// reproducing. What matters is that the shapes vary, not which shape a given seed gives.
#[derive(Debug, Clone)]
pub struct Dice(u32);

impl Dice {
    pub fn new(seed: u32) -> Dice {
        Dice(seed | 1)
    }

    /// A value in `0.0..1.0`.
    pub fn roll(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 % 1_000_000) as f32 / 1_000_000.0
    }

    /// A value in `from..to`, as `Random.Next` gives: the top is excluded.
    pub fn range(&mut self, from: i32, to: i32) -> i32 {
        if to <= from {
            return from;
        }
        from + (self.roll() * (to - from) as f32) as i32
    }

    /// Whether a one-in-`n` chance came up.
    fn one_in(&mut self, n: i32) -> bool {
        self.range(0, n) == 0
    }
}

/// One square of a drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct Painted {
    pub x: i32,
    pub y: i32,

    /// The ground it becomes, by content name.
    pub tile: Option<&'static str>,

    /// What stands on it, by content name. `None` with `clear` set empties the square.
    pub object: Option<&'static str>,

    /// A percentage of the object's natural size, or zero for natural.
    pub size: u16,

    /// Whether whatever was standing here goes.
    pub clear: bool,
}

impl Painted {
    fn ground(x: i32, y: i32, tile: &'static str) -> Painted {
        Painted {
            x,
            y,
            tile: Some(tile),
            object: None,
            size: 0,
            clear: true,
        }
    }

    fn standing(x: i32, y: i32, tile: &'static str, object: &'static str) -> Painted {
        Painted {
            x,
            y,
            tile: Some(tile),
            object: Some(object),
            size: 0,
            clear: false,
        }
    }

    fn object_only(x: i32, y: i32, object: &'static str) -> Painted {
        Painted {
            x,
            y,
            tile: None,
            object: Some(object),
            size: 0,
            clear: false,
        }
    }

    fn sized(mut self, size: u16) -> Painted {
        self.size = size;
        self
    }
}

/// One tier of loot a setpiece chest may hold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tier {
    pub tier: u8,
    pub kind: &'static str,
    pub chance: f32,
}

const fn tier(tier: u8, kind: &'static str, chance: f32) -> Tier {
    Tier { tier, kind, chance }
}

/// Something a drawing puts in the world rather than on the ground.
#[derive(Debug, Clone, PartialEq)]
pub enum Placed {
    /// A living thing, by content name.
    Living {
        x: f32,
        y: f32,
        name: &'static str,

        /// A percentage of its natural size, or zero for natural.
        size: u16,
    },

    /// A chest, and what it may hold.
    Chest {
        x: f32,
        y: f32,
        loot: &'static [Tier],

        /// How many items it holds, drawn between the two.
        least: usize,
        most: usize,
    },
}

/// What a setpiece comes to: squares to paint, things to put on them, and sometimes a whole map.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawing {
    pub squares: Vec<Painted>,
    pub placed: Vec<Placed>,

    /// A map to stamp instead of painting, by name without an extension, for the setpieces that
    /// are saved maps rather than drawing programs.
    pub prefab: Option<&'static str>,
}

/// A grid of small codes, which is how every one of these is drawn before it becomes squares.
struct Cells {
    width: usize,
    height: usize,
    cells: Vec<u8>,
}

impl Cells {
    fn new(width: usize, height: usize) -> Cells {
        Cells {
            width,
            height,
            cells: vec![0; width * height],
        }
    }

    fn get(&self, x: usize, y: usize) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.cells[y * self.width + x]
    }

    fn set(&mut self, x: usize, y: usize, value: u8) {
        if x < self.width && y < self.height {
            self.cells[y * self.width + x] = value;
        }
    }

    /// A quarter turn clockwise, which is how the original varies a shape it drew once.
    fn rotate(&self) -> Cells {
        let mut turned = Cells::new(self.height, self.width);
        for y in 0..self.height {
            for x in 0..self.width {
                turned.set(self.height - 1 - y, x, self.get(x, y));
            }
        }
        turned
    }

    fn reflect_across_x(&self) -> Cells {
        let mut flipped = Cells::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                flipped.set(x, self.height - 1 - y, self.get(x, y));
            }
        }
        flipped
    }

    fn reflect_across_y(&self) -> Cells {
        let mut flipped = Cells::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                flipped.set(self.width - 1 - x, y, self.get(x, y));
            }
        }
        flipped
    }

    fn turned(self, quarters: i32, dice: &mut Dice) -> Cells {
        let _ = dice;
        let mut cells = self;
        for _ in 0..quarters {
            cells = cells.rotate();
        }
        cells
    }
}

/// One of the structures a realm is built with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Building,
    Graveyard,
    Grove,
    LichyTemple,
    Castle,
    Tower,
    TempleA,
    TempleB,
    Oasis,
    Pyre,
    LavaFissure,
    LuckyDjinn,
    LuckyEnt,
    Crystal,
    KageKami,
}

impl Kind {
    /// The setpiece a behaviour names, if there is one.
    ///
    /// The names are the class names the original looks up at runtime, so content written for it
    /// works here unchanged.
    pub fn named(name: &str) -> Option<Kind> {
        Some(match name {
            "Building" => Kind::Building,
            "Graveyard" => Kind::Graveyard,
            "Grove" => Kind::Grove,
            "LichyTemple" => Kind::LichyTemple,
            "Castle" => Kind::Castle,
            "Tower" => Kind::Tower,
            "TempleA" => Kind::TempleA,
            "TempleB" => Kind::TempleB,
            "Oasis" => Kind::Oasis,
            "Pyre" => Kind::Pyre,
            "LavaFissure" => Kind::LavaFissure,
            "LuckyDjinn" => Kind::LuckyDjinn,
            "LuckyEnt" => Kind::LuckyEnt,
            "Crystal" => Kind::Crystal,
            "KageKami" => Kind::KageKami,
            _ => return None,
        })
    }

    /// How much room it needs, which is what keeps two of them from being drawn on top of another.
    pub fn size(self) -> u32 {
        match self {
            Kind::Building => 21,
            // Thirty-four in the original, whose graveyard is drawn on a 23x35 grid and so reaches
            // one square past the room it reserves. The size is what keeps two setpieces apart, so
            // it says the truth here.
            Kind::Graveyard => 35,
            Kind::Grove => 25,
            Kind::LichyTemple => 26,
            Kind::Castle => 40,
            Kind::Tower => 27,
            Kind::TempleA | Kind::TempleB => 60,
            Kind::Oasis | Kind::Pyre => 30,
            Kind::LavaFissure => 40,
            Kind::LuckyDjinn | Kind::LuckyEnt | Kind::Crystal => 5,
            Kind::KageKami => 65,
        }
    }

    /// Draws one.
    pub fn draw(self, dice: &mut Dice) -> Drawing {
        match self {
            Kind::Building => building(dice),
            Kind::Graveyard => graveyard(dice),
            Kind::Grove => grove(dice),
            Kind::LichyTemple => lichy_temple(dice),
            Kind::Castle => castle(dice),
            Kind::Tower => tower(dice),
            Kind::TempleA => temple_a(dice),
            Kind::TempleB => temple_b(dice),
            Kind::Oasis => oasis(dice),
            Kind::Pyre => pyre(dice),
            Kind::LavaFissure => lava_fissure(dice),
            Kind::LuckyDjinn => alone("Lucky Djinn"),
            Kind::LuckyEnt => alone("Lucky Ent God"),
            Kind::Crystal => alone("Mysterious Crystal"),
            Kind::KageKami => Drawing {
                prefab: Some("SP_KageKami"),
                ..Drawing::default()
            },
        }
    }
}

/// How many of each a realm gets, and what ground it may stand on.
///
/// The counts are drawn from the range with the top excluded, as the original's `Random.Next` does.
pub const SCATTER: &[(Kind, i32, i32, &[Terrain])] = &[
    (
        Kind::Building,
        80,
        100,
        &[Terrain::LowForest, Terrain::LowPlains, Terrain::MidForest],
    ),
    (
        Kind::Graveyard,
        5,
        10,
        &[Terrain::LowSand, Terrain::LowPlains],
    ),
    (
        Kind::Grove,
        17,
        25,
        &[Terrain::MidForest, Terrain::MidPlains],
    ),
    (
        Kind::LichyTemple,
        4,
        7,
        &[Terrain::MidForest, Terrain::MidPlains],
    ),
    (
        Kind::Castle,
        4,
        7,
        &[Terrain::HighForest, Terrain::HighPlains],
    ),
    (
        Kind::Tower,
        8,
        15,
        &[Terrain::HighForest, Terrain::HighPlains],
    ),
    (
        Kind::TempleA,
        10,
        20,
        &[Terrain::MidForest, Terrain::MidPlains],
    ),
    (
        Kind::TempleB,
        10,
        20,
        &[Terrain::MidForest, Terrain::MidPlains],
    ),
    (Kind::Oasis, 0, 5, &[Terrain::LowSand, Terrain::MidSand]),
    (Kind::Pyre, 0, 5, &[Terrain::MidSand, Terrain::HighSand]),
    (Kind::LavaFissure, 3, 5, &[Terrain::Mountains]),
    (Kind::LuckyDjinn, 1, 1, &[Terrain::Mountains]),
    (Kind::LuckyEnt, 1, 1, &[Terrain::Mountains]),
    (Kind::Crystal, 1, 1, &[Terrain::Mountains]),
    (
        Kind::KageKami,
        2,
        3,
        &[Terrain::HighForest, Terrain::HighPlains],
    ),
];

/// How many places are tried for one setpiece before giving up on it.
///
/// Giving up is the right answer: a realm with nowhere left should get fewer of them, not spin.
pub const PLACEMENT_ATTEMPTS: usize = 50;

/// Where one setpiece is to be drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub kind: Kind,
    pub x: u32,
    pub y: u32,
}

/// Chooses where a realm's setpieces go.
///
/// `ground` answers what terrain a square is. Squares are drawn at random and rejected where the
/// terrain is wrong or where they would overlap something already placed, which is what stops two
/// castles being drawn through each other.
pub fn scatter(
    width: u32,
    height: u32,
    ground: &dyn Fn(u32, u32) -> Terrain,
    dice: &mut Dice,
) -> Vec<Placement> {
    let mut placed: Vec<Placement> = Vec::new();

    for (kind, least, most, terrains) in SCATTER {
        let count = dice.range(*least, *most);

        for _ in 0..count {
            let size = kind.size();

            for _ in 0..PLACEMENT_ATTEMPTS {
                let x = dice.range(0, width as i32) as u32;
                let y = dice.range(0, height as i32) as u32;

                if !terrains.contains(&ground(x, y)) {
                    continue;
                }

                let overlaps = placed.iter().any(|other| {
                    let reach = other.kind.size();
                    !(other.x > x + size
                        || other.x + reach < x
                        || other.y > y + size
                        || other.y + reach < y)
                });
                if overlaps {
                    continue;
                }

                placed.push(Placement { kind: *kind, x, y });
                break;
            }
        }
    }

    placed
}

/// One thing standing on its own, which is all three of the mountain lucks are.
fn alone(name: &'static str) -> Drawing {
    Drawing {
        placed: vec![Placed::Living {
            x: 2.5,
            y: 2.5,
            name,
            size: 0,
        }],
        ..Drawing::default()
    }
}

// -- the drawings -------------------------------------------------------------------------------

const BUILDING_FLOOR: &str = "Brown Lines";
const BUILDING_WALL: &str = "Wooden Wall";

/// Four walls, two rooms cut out of them, then half of it knocked down again.
fn building(dice: &mut Dice) -> Drawing {
    let w = dice.range(19, 22) as usize;
    let h = dice.range(19, 22) as usize;
    let mut cells = Cells::new(w, h);

    for x in 0..w {
        cells.set(x, 0, 1);
        cells.set(x, h - 1, 1);
    }
    for y in 0..h {
        cells.set(0, y, 1);
        cells.set(w - 1, y, 1);
    }

    // A wall across the middle, from one side or the other, leaving a gap at the end it starts from.
    let middle_row = (h / 2).saturating_add_signed(dice.range(-2, 3) as isize);
    let gap = dice.range(2, 4) as usize;
    if dice.one_in(2) {
        for x in gap..w {
            cells.set(x, middle_row, 1);
        }
    } else {
        for x in 0..w.saturating_sub(gap) {
            cells.set(x, middle_row, 1);
        }
    }

    // And one down the middle of whichever half the first wall left.
    let (from, to) = if dice.one_in(2) {
        (0, middle_row)
    } else {
        (middle_row, h)
    };
    let middle_column = (w / 2).saturating_add_signed(dice.range(-2, 3) as isize);
    let gap = dice.range(2, 4) as usize;
    if dice.one_in(2) {
        for y in (from + gap)..to {
            cells.set(middle_column, y, 1);
        }
    } else {
        for y in from..to.saturating_sub(gap) {
            cells.set(middle_column, y, 1);
        }
    }

    for y in 0..h {
        for x in 0..w {
            if cells.get(x, y) == 0 {
                cells.set(x, y, 2);
            }
        }
    }

    // Half of it is then knocked back to nothing, which is what makes a building a ruin.
    for y in 0..h {
        for x in 0..w {
            if dice.one_in(2) {
                cells.set(x, y, 0);
            }
        }
    }

    let turns = dice.range(0, 4);
    let cells = cells.turned(turns, dice);

    let mut squares = Vec::new();
    for y in 0..cells.height {
        for x in 0..cells.width {
            let (x, y) = (x as i32, y as i32);
            match cells.get(x as usize, y as usize) {
                1 => squares.push(Painted::object_only(x, y, BUILDING_WALL)),
                2 => squares.push(Painted::ground(x, y, BUILDING_FLOOR)),
                _ => {}
            }
        }
    }

    Drawing {
        squares,
        ..Drawing::default()
    }
}

const GRAVEYARD_FLOOR: &str = "Grass";
const GRAVEYARD_WALL: &str = "Grey Wall";
const GRAVEYARD_BROKEN_WALL: &str = "Destructible Grey Wall";
const GRAVEYARD_CROSS: &str = "Cross";

const GRAVEYARD_LOOT: &[Tier] = &[
    tier(4, "Weapon", 0.3),
    tier(5, "Weapon", 0.2),
    tier(6, "Weapon", 0.1),
    tier(3, "Armor", 0.3),
    tier(4, "Armor", 0.2),
    tier(5, "Armor", 0.1),
    tier(1, "Ability", 0.3),
    tier(2, "Ability", 0.2),
    tier(3, "Ability", 0.2),
    tier(1, "Ring", 0.25),
    tier(2, "Ring", 0.15),
    tier(1, "Potion", 0.5),
];

/// A walled yard of crosses, with a deathmage and a chest where one cross is missing.
fn graveyard(dice: &mut Dice) -> Drawing {
    let mut cells = Cells::new(23, 35);

    for y in 0..35 {
        for x in 0..23 {
            cells.set(x, y, if dice.one_in(3) { 0 } else { 1 });
        }
    }
    for y in 0..35 {
        cells.set(0, y, 2);
        cells.set(22, y, 2);
    }
    for x in 0..23 {
        cells.set(x, 0, 2);
        cells.set(x, 34, 2);
    }

    // Crosses on a three-square lattice, two thirds of them standing. The gaps are where the boss
    // and the chest go, so a graveyard always has somewhere to put them.
    let mut gaps = Vec::new();
    for y in 0..11 {
        for x in 0..7 {
            if dice.range(0, 3) > 0 {
                cells.set(2 + 3 * x, 2 + 3 * y, 4);
            } else {
                gaps.push((2 + 3 * x, 2 + 3 * y));
            }
        }
    }

    // Walls rot: some become floor, some become the destructible kind.
    for y in 0..35 {
        for x in 0..23 {
            let here = cells.get(x, y);
            if here == 1 || here == 0 || here == 4 {
                continue;
            }
            let roll = dice.roll();
            if roll < 0.1 {
                cells.set(x, y, 1);
            } else if roll < 0.4 {
                cells.set(x, y, here + 1);
            }
        }
    }

    if !gaps.is_empty() {
        let (x, y) = gaps[dice.range(0, gaps.len() as i32) as usize];
        cells.set(x, y, 5);
        cells.set(x + 1, y, 6);
    }

    let turns = dice.range(0, 4);
    let cells = cells.turned(turns, dice);

    let mut squares = Vec::new();
    let mut placed = Vec::new();

    for y in 0..cells.height {
        for x in 0..cells.width {
            let (fx, fy) = (x as i32, y as i32);
            match cells.get(x, y) {
                1 => squares.push(Painted::ground(fx, fy, GRAVEYARD_FLOOR)),
                2 => squares.push(Painted::standing(fx, fy, GRAVEYARD_FLOOR, GRAVEYARD_WALL)),
                3 => {
                    squares.push(Painted::ground(fx, fy, GRAVEYARD_FLOOR));
                    placed.push(Placed::Living {
                        x: fx as f32 + 0.5,
                        y: fy as f32 + 0.5,
                        name: GRAVEYARD_BROKEN_WALL,
                        size: 0,
                    });
                }
                4 => squares.push(Painted::standing(fx, fy, GRAVEYARD_FLOOR, GRAVEYARD_CROSS)),
                5 => placed.push(Placed::Chest {
                    x: fx as f32 + 0.5,
                    y: fy as f32 + 0.5,
                    loot: GRAVEYARD_LOOT,
                    least: 3,
                    most: 8,
                }),
                6 => placed.push(Placed::Living {
                    x: fx as f32,
                    y: fy as f32,
                    name: "Deathmage",
                    size: 0,
                }),
                _ => {}
            }
        }
    }

    Drawing {
        squares,
        placed,
        ..Drawing::default()
    }
}

const GROVE_FLOOR: &str = "Light Grass";
const GROVE_TREE: &str = "Cherry Tree";

/// A circle of light grass with cherry trees around half its edge, and an ancient ent in it.
fn grove(dice: &mut Dice) -> Drawing {
    const SIZE: usize = 25;
    let radius = dice.range(SIZE as i32 - 5, SIZE as i32 + 1) as f32 / 2.0;

    let mut cells = Cells::new(SIZE, SIZE);
    let mut edge = Vec::new();

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - SIZE as f32 / 2.0;
            let dy = y as f32 - SIZE as f32 / 2.0;
            let r = (dx * dx + dy * dy).sqrt();

            if r <= radius {
                cells.set(x, y, 1);
                if radius - r < 1.5 {
                    edge.push((x, y));
                }
            }
        }
    }

    // Half the edge becomes trees, chosen with replacement as the original does, so a grove is
    // ringed rather than fenced.
    let wanted = edge.len() / 2;
    let mut chosen = std::collections::HashSet::new();
    let mut tries = wanted * 8 + 16;
    while chosen.len() < wanted && tries > 0 {
        tries -= 1;
        chosen.insert(edge[dice.range(0, edge.len().max(1) as i32) as usize]);
    }
    for (x, y) in &chosen {
        cells.set(*x, *y, 2);
    }

    let mut squares = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as i32, y as i32);
            match cells.get(x, y) {
                1 => squares.push(Painted::ground(fx, fy, GROVE_FLOOR)),
                2 => {
                    let size = if dice.one_in(2) { 120 } else { 140 };
                    squares.push(Painted::standing(fx, fy, GROVE_FLOOR, GROVE_TREE).sized(size));
                }
                _ => {}
            }
        }
    }

    Drawing {
        squares,
        placed: vec![Placed::Living {
            x: SIZE as f32 / 2.0 + 1.0,
            y: SIZE as f32 / 2.0 + 1.0,
            name: "Ent Ancient",
            size: 140,
        }],
        ..Drawing::default()
    }
}

const LICH_FLOOR: &str = "Blue Floor";
const LICH_WALL: &str = "Blue Wall";
const LICH_BROKEN_WALL: &str = "Destructible Blue Wall";
const LICH_PILLAR: &str = "Blue Pillar";
const LICH_BROKEN_PILLAR: &str = "Broken Blue Pillar";

/// A blue hall with two rows of pillars, and the lich at the centre of it.
fn lichy_temple(dice: &mut Dice) -> Drawing {
    let mut cells = Cells::new(25, 26);

    for x in 2..23 {
        for y in 1..24 {
            cells.set(x, y, if dice.one_in(10) { 0 } else { 1 });
        }
    }

    for y in 1..24 {
        cells.set(2, y, 2);
        cells.set(22, y, 2);
    }
    for x in 2..23 {
        cells.set(x, 23, 2);
    }
    for x in 0..3 {
        for y in 0..3 {
            cells.set(x + 1, y, 2);
            cells.set(x + 21, y, 2);
        }
    }
    for x in 0..5 {
        for y in 0..5 {
            let corner = (x == 0 || x == 4) && (y == 0 || y == 4);
            if corner {
                continue;
            }
            cells.set(x, y + 21, 2);
            cells.set(x + 20, y + 21, 2);
        }
    }

    for y in 0..6 {
        cells.set(9, 4 + 3 * y, 4);
        cells.set(15, 4 + 3 * y, 4);
    }

    for y in 0..26 {
        for x in 0..25 {
            let here = cells.get(x, y);
            if here == 1 || here == 0 {
                continue;
            }
            let roll = dice.roll();
            if roll < 0.1 {
                cells.set(x, y, 1);
            } else if roll < 0.4 {
                cells.set(x, y, here + 1);
            }
        }
    }

    let turns = dice.range(0, 4);
    let cells = cells.turned(turns, dice);

    let mut squares = Vec::new();
    let mut placed = Vec::new();

    for y in 0..cells.height {
        for x in 0..cells.width {
            let (fx, fy) = (x as i32, y as i32);
            match cells.get(x, y) {
                1 => squares.push(Painted::ground(fx, fy, LICH_FLOOR)),
                2 => squares.push(Painted::standing(fx, fy, LICH_FLOOR, LICH_WALL)),
                3 => {
                    squares.push(Painted::ground(fx, fy, LICH_FLOOR));
                    placed.push(Placed::Living {
                        x: fx as f32 + 0.5,
                        y: fy as f32 + 0.5,
                        name: LICH_BROKEN_WALL,
                        size: 0,
                    });
                }
                4 => squares.push(Painted::standing(fx, fy, LICH_FLOOR, LICH_PILLAR)),
                5 => squares.push(Painted::standing(fx, fy, LICH_FLOOR, LICH_BROKEN_PILLAR)),
                _ => {}
            }
        }
    }

    placed.push(Placed::Living {
        x: 13.0,
        y: 13.0,
        name: "Lich",
        size: 0,
    });

    Drawing {
        squares,
        placed,
        ..Drawing::default()
    }
}

const CASTLE_FLOOR: &str = "Rock";
const CASTLE_BRIDGE: &str = "Bridge";
const CASTLE_SHALLOWS: &str = "Shallow Water";
const CASTLE_DEEPS: &str = "Dark Water";
const CASTLE_WALL: &str = "Grey Wall";
const CASTLE_BROKEN_WALL: &str = "Destructible Grey Wall";

const CASTLE_LOOT: &[Tier] = &[
    tier(6, "Weapon", 0.3),
    tier(7, "Weapon", 0.2),
    tier(8, "Weapon", 0.1),
    tier(5, "Armor", 0.3),
    tier(6, "Armor", 0.2),
    tier(7, "Armor", 0.1),
    tier(2, "Ability", 0.3),
    tier(3, "Ability", 0.2),
    tier(4, "Ability", 0.1),
    tier(2, "Ring", 0.25),
    tier(3, "Ring", 0.15),
    tier(1, "Potion", 0.5),
];

/// A moated keep with a bridge in, a cyclops god inside and a chest behind it.
fn castle(dice: &mut Dice) -> Drawing {
    let mut cells = Cells::new(31, 40);

    // Four rounded corners of moat, then the rest of the water between them.
    for y in 0..13 {
        for x in 0..13 {
            let cut = (x == 0 && !(3..=9).contains(&y))
                || (y == 0 && !(3..=9).contains(&x))
                || (x == 12 && !(3..=9).contains(&y))
                || (y == 12 && !(3..=9).contains(&x));
            if cut {
                continue;
            }
            cells.set(x, y, 2);
            cells.set(x + 18, y, 2);
            cells.set(x, y + 27, 2);
            cells.set(x + 18, y + 27, 2);
        }
    }
    for x in 3..28 {
        for y in 3..37 {
            if !(6..=24).contains(&x) || !(6..=33).contains(&y) {
                cells.set(x, y, 2);
            }
        }
    }

    for x in 7..24 {
        for y in 7..33 {
            cells.set(x, y, if dice.one_in(3) { 0 } else { 1 });
        }
    }

    // The keep wall, with a gap in the middle of each side.
    for x in 0..7 {
        for y in 0..7 {
            let cut = (x == 0 && y != 3)
                || (y == 0 && x != 3)
                || (x == 6 && y != 3)
                || (y == 6 && x != 3);
            if cut {
                continue;
            }
            cells.set(x + 3, y + 3, 4);
            cells.set(x + 21, y + 3, 4);
            cells.set(x + 3, y + 30, 4);
            cells.set(x + 21, y + 30, 4);
        }
    }
    for x in 6..25 {
        cells.set(x, 6, 4);
        cells.set(x, 33, 4);
    }
    for y in 6..34 {
        cells.set(6, y, 4);
        cells.set(24, y, 4);
    }

    for x in 13..18 {
        for y in 3..7 {
            cells.set(x, y, 6);
        }
    }

    // Everything decays: water deepens, walls break, and the bridge loses planks.
    for y in 0..40 {
        for x in 0..31 {
            let here = cells.get(x, y);
            if here == 1 || here == 0 {
                continue;
            }
            let roll = dice.roll();

            if here == 6 {
                if roll < 0.4 {
                    cells.set(x, y, 0);
                }
                continue;
            }
            if roll < 0.1 {
                cells.set(x, y, 1);
            } else if roll < 0.4 {
                cells.set(x, y, here + 1);
            }
        }
    }

    cells.set(15, 27, 7);
    cells.set(15, 20, 8);

    let turns = dice.range(0, 4);
    let cells = cells.turned(turns, dice);

    let mut squares = Vec::new();
    let mut placed = Vec::new();

    for y in 0..cells.height {
        for x in 0..cells.width {
            let (fx, fy) = (x as i32, y as i32);
            match cells.get(x, y) {
                1 => squares.push(Painted::ground(fx, fy, CASTLE_FLOOR)),
                2 => squares.push(Painted::ground(fx, fy, CASTLE_SHALLOWS)),
                3 => squares.push(Painted::ground(fx, fy, CASTLE_DEEPS)),
                4 => squares.push(Painted::standing(fx, fy, CASTLE_FLOOR, CASTLE_WALL)),
                5 => {
                    squares.push(Painted::ground(fx, fy, CASTLE_FLOOR));
                    placed.push(Placed::Living {
                        x: fx as f32 + 0.5,
                        y: fy as f32 + 0.5,
                        name: CASTLE_BROKEN_WALL,
                        size: 0,
                    });
                }
                6 => squares.push(Painted {
                    tile: Some(CASTLE_BRIDGE),
                    clear: false,
                    ..Painted::ground(fx, fy, CASTLE_BRIDGE)
                }),
                7 => placed.push(Placed::Chest {
                    x: fx as f32 + 0.5,
                    y: fy as f32 + 0.5,
                    loot: CASTLE_LOOT,
                    least: 5,
                    most: 8,
                }),
                8 => placed.push(Placed::Living {
                    x: fx as f32,
                    y: fy as f32,
                    name: "Cyclops God",
                    size: 0,
                }),
                _ => {}
            }
        }
    }

    Drawing {
        squares,
        placed,
        ..Drawing::default()
    }
}

const TOWER_FLOOR: &str = "Rock";
const TOWER_WALL: &str = "Grey Wall";

/// One quarter of the tower, drawn once and mirrored into the other three.
const TOWER_QUARTER: &[&str] = &[
    "............XX",
    "........XXXXXX",
    "......XXXXXXXX",
    ".....XXXX=====",
    "....XXX=======",
    "...XXX========",
    "..XXX=========",
    "..XX==========",
    ".XXX==========",
    ".XX===========",
    ".XX===========",
    ".XX===========",
    "XXX===========",
    "XXX===========",
];

/// A round tower with a ghost king at its centre.
fn tower(dice: &mut Dice) -> Drawing {
    let mut quarter = Cells::new(14, 14);
    for (y, row) in TOWER_QUARTER.iter().enumerate() {
        for (x, mark) in row.chars().enumerate() {
            quarter.set(
                x,
                y,
                match mark {
                    'X' => 1,
                    '=' => 2,
                    _ => 0,
                },
            );
        }
    }

    let mut cells = Cells::new(27, 27);
    let copy_in = |cells: &mut Cells, from: &Cells, at_x: usize, at_y: usize| {
        for y in 0..14 {
            for x in 0..14 {
                cells.set(at_x + x, at_y + y, from.get(x, y));
            }
        }
    };

    copy_in(&mut cells, &quarter, 0, 0);
    quarter = quarter.reflect_across_y();
    copy_in(&mut cells, &quarter, 13, 0);
    quarter = quarter.reflect_across_x();
    copy_in(&mut cells, &quarter, 13, 13);
    quarter = quarter.reflect_across_y();
    copy_in(&mut cells, &quarter, 0, 13);

    // The way in.
    for y in 1..4 {
        for x in 8..19 {
            cells.set(x, y, 2);
        }
    }
    cells.set(12, 0, 2);
    cells.set(13, 0, 2);
    cells.set(14, 0, 2);

    let turns = dice.range(0, 4);
    let mut cells = cells.turned(turns, dice);
    cells.set(13, 13, 3);

    let mut squares = Vec::new();
    let mut placed = Vec::new();

    for y in 0..cells.height {
        for x in 0..cells.width {
            let (fx, fy) = (x as i32, y as i32);
            match cells.get(x, y) {
                1 => squares.push(Painted::standing(fx, fy, TOWER_FLOOR, TOWER_WALL)),
                2 => squares.push(Painted::ground(fx, fy, TOWER_FLOOR)),
                3 => {
                    squares.push(Painted::ground(fx, fy, TOWER_FLOOR));
                    placed.push(Placed::Living {
                        x: fx as f32,
                        y: fy as f32,
                        name: "Ghost King",
                        size: 0,
                    });
                }
                _ => {}
            }
        }
    }

    Drawing {
        squares,
        placed,
        ..Drawing::default()
    }
}

const TEMPLE_DARK_GRASS: &str = "Dark Grass";
const TEMPLE_FLOOR: &str = "Jungle Temple Floor";
const TEMPLE_BRICKS: &str = "Jungle Temple Bricks";
const TEMPLE_WALLS: &str = "Jungle Temple Walls";
const TEMPLE_COLUMN: &str = "Jungle Temple Column";
const TEMPLE_FLOWERS: &str = "Jungle Ground Flowers";
const TEMPLE_GRASS: &str = "Jungle Grass";
const TEMPLE_TREE: &str = "Jungle Tree Big";

const TEMPLE_LOOT: &[Tier] = &[
    tier(4, "Weapon", 0.3),
    tier(5, "Weapon", 0.2),
    tier(4, "Armor", 0.3),
    tier(5, "Armor", 0.2),
    tier(1, "Ability", 0.25),
    tier(2, "Ability", 0.15),
    tier(2, "Ring", 0.3),
    tier(3, "Ring", 0.2),
    tier(1, "Potion", 0.5),
    tier(1, "Potion", 0.5),
    tier(1, "Potion", 0.5),
];

const TEMPLE_SIZE: usize = 60;

/// The clearing both jungle temples stand in, and the plants around them.
fn temple_ground(dice: &mut Dice) -> Cells {
    let mut ground = Cells::new(TEMPLE_SIZE, TEMPLE_SIZE);
    let half = TEMPLE_SIZE as f32 / 2.0;

    for y in 0..TEMPLE_SIZE {
        for x in 0..TEMPLE_SIZE {
            let out_x = (x as f32 - half).abs() / half + dice.roll() * 0.3;
            let out_y = (y as f32 - half).abs() / half + dice.roll() * 0.3;
            if out_x >= 0.9 || out_y >= 0.9 {
                continue;
            }

            let dx = x as f32 - half;
            let dy = y as f32 - half;
            let away = ((dx * dx + dy * dy) / (half * half)).sqrt();

            // Stone at the middle, grass at the edges, with the boundary ragged rather than round.
            let stone = dice.roll() < (1.0 - away) * (1.0 - away);
            ground.set(x, y, if stone { 2 } else { 1 });
        }
    }

    for y in 0..TEMPLE_SIZE {
        for x in 0..TEMPLE_SIZE {
            if dice.one_in(50) {
                ground.set(x, y, 0);
            }
        }
    }

    ground
}

/// Flowers, grass and the occasional tree, in the band between the clearing's edge and the walls.
fn temple_plants(ground: &Cells, objects: &mut Cells, inner: usize, dice: &mut Dice) {
    for y in 0..TEMPLE_SIZE {
        for x in 0..TEMPLE_SIZE {
            let in_band = (x > 5 && x < inner)
                || (x < TEMPLE_SIZE - 5 && x > TEMPLE_SIZE - inner)
                || (y > 5 && y < inner)
                || (y < TEMPLE_SIZE - 5 && y > TEMPLE_SIZE - inner);

            if !in_band || objects.get(x, y) != 0 || ground.get(x, y) != 1 {
                continue;
            }

            let roll = dice.roll();
            if roll > 0.6 {
                objects.set(x, y, 4);
            } else if roll > 0.35 {
                objects.set(x, y, 5);
            } else if roll > 0.33 {
                objects.set(x, y, 6);
            }
        }
    }
}

/// Turns a temple's two layers into squares.
fn temple_squares(ground: &Cells, objects: &Cells) -> Vec<Painted> {
    let mut squares = Vec::new();

    for y in 0..ground.height {
        for x in 0..ground.width {
            let (fx, fy) = (x as i32, y as i32);

            let tile = match ground.get(x, y) {
                1 => Some(TEMPLE_DARK_GRASS),
                2 => Some(TEMPLE_FLOOR),
                _ => None,
            };

            let object = match objects.get(x, y) {
                1 => Some(TEMPLE_BRICKS),
                2 => Some(TEMPLE_WALLS),
                3 => Some(TEMPLE_COLUMN),
                4 => Some(TEMPLE_FLOWERS),
                5 => Some(TEMPLE_GRASS),
                6 => Some(TEMPLE_TREE),
                _ => None,
            };

            if tile.is_none() && object.is_none() {
                continue;
            }

            squares.push(Painted {
                x: fx,
                y: fy,
                tile,
                object,
                size: 0,
                // The ground clears what stood on it, and then the object layer puts its own back.
                clear: object.is_none() && tile.is_some(),
            });
        }
    }

    squares
}

/// Two walls meeting at a corner, twice, offset from each other: a spiral rather than a room.
fn temple_a(dice: &mut Dice) -> Drawing {
    let ground = temple_ground(dice);
    let mut objects = Cells::new(TEMPLE_SIZE, TEMPLE_SIZE);
    const BASE: usize = 17;

    for x in 0..20 {
        objects.set(BASE + x, BASE, if x == 19 { 2 } else { 1 });
        objects.set(BASE + x, BASE + 1, 2);
    }
    for y in 0..20 {
        objects.set(BASE, BASE + y, if y == 19 { 2 } else { 1 });
        if y != 0 {
            objects.set(BASE + 1, BASE + y, 2);
        }
    }
    for x in 0..19 {
        objects.set(BASE + 8 + x, BASE + 25, 2);
        objects.set(BASE + 8 + x, BASE + 26, if x == 0 { 2 } else { 1 });
    }
    for y in 0..19 {
        if y != 18 {
            objects.set(BASE + 25, BASE + 8 + y, 2);
        }
        objects.set(BASE + 26, BASE + 8 + y, if y == 0 { 2 } else { 1 });
    }

    for (x, y) in [
        (5, 5),
        (21, 5),
        (5, 21),
        (21, 21),
        (9, 9),
        (17, 9),
        (9, 17),
        (17, 17),
    ] {
        objects.set(BASE + x, BASE + y, 3);
    }

    temple_plants(&ground, &mut objects, BASE, dice);

    let turns = dice.range(0, 4);
    let ground = ground.turned(turns, dice);
    let objects = objects.turned(turns, dice);

    let middle = TEMPLE_SIZE as f32 / 2.0;

    Drawing {
        squares: temple_squares(&ground, &objects),
        placed: vec![
            Placed::Chest {
                x: middle,
                y: middle,
                loot: TEMPLE_LOOT,
                least: 3,
                most: 8,
            },
            Placed::Living {
                x: middle,
                y: middle,
                name: "Ghost of Skuld",
                size: 0,
            },
        ],
        ..Drawing::default()
    }
}

/// A double-walled square with a gap in the middle of each side, and a grid of columns inside.
fn temple_b(dice: &mut Dice) -> Drawing {
    let ground = temple_ground(dice);
    let mut objects = Cells::new(TEMPLE_SIZE, TEMPLE_SIZE);
    const BASE: usize = 16;

    for x in 0..23 {
        if x > 9 && x < 13 {
            continue;
        }
        objects.set(BASE + x, BASE, 2);
        objects.set(BASE + x, BASE + 1, 2);
        objects.set(BASE + x, BASE + 21, 2);
        objects.set(BASE + x, BASE + 22, 2);
    }
    for y in 0..23 {
        if y > 9 && y < 13 {
            continue;
        }
        objects.set(BASE, BASE + y, 2);
        objects.set(BASE + 1, BASE + y, 2);
        objects.set(BASE + 21, BASE + y, 2);
        objects.set(BASE + 22, BASE + y, 2);
    }

    // The brick ends where each gap opens out.
    for offset in [7, 8, 9, 13, 14, 15] {
        objects.set(BASE - 1, BASE + offset, 1);
        objects.set(BASE + 23, BASE + offset, 1);
        objects.set(BASE + offset, BASE - 1, 1);
        objects.set(BASE + offset, BASE + 23, 1);
    }

    for y in 0..4 {
        for x in 0..4 {
            objects.set(BASE + 5 + x * 4, BASE + 5 + y * 4, 3);
        }
    }

    temple_plants(&ground, &mut objects, BASE, dice);

    let turns = dice.range(0, 4);
    let ground = ground.turned(turns, dice);
    let objects = objects.turned(turns, dice);

    let middle = BASE as f32 + 11.5;

    Drawing {
        squares: temple_squares(&ground, &objects),
        placed: vec![
            Placed::Chest {
                x: middle,
                y: middle,
                loot: TEMPLE_LOOT,
                least: 3,
                most: 8,
            },
            Placed::Living {
                x: middle,
                y: middle,
                name: "Ghost of Skuld",
                size: 0,
            },
        ],
        ..Drawing::default()
    }
}

const OASIS_FLOOR: &str = "Light Grass";
const OASIS_WATER: &str = "Shallow Water";
const OASIS_TREE: &str = "Palm Tree";

const OASIS_LOOT: &[Tier] = &[
    tier(5, "Weapon", 0.3),
    tier(6, "Weapon", 0.2),
    tier(7, "Weapon", 0.1),
    tier(4, "Armor", 0.3),
    tier(5, "Armor", 0.2),
    tier(6, "Armor", 0.1),
    tier(2, "Ability", 0.3),
    tier(3, "Ability", 0.2),
    tier(1, "Ring", 0.25),
    tier(2, "Ring", 0.15),
    tier(1, "Potion", 0.5),
];

/// A ring of grass around a pool with an island in it, palms on both shores.
fn oasis(dice: &mut Dice) -> Drawing {
    const SIZE: usize = 30;
    let half = SIZE as f32 / 2.0;

    let mut cells = Cells::new(SIZE, SIZE);
    let mut shore = Vec::new();

    for (radius, code) in [(13.0f32, 1u8), (10.0, 2), (3.0, 1)] {
        for y in 0..SIZE {
            for x in 0..SIZE {
                let dx = x as f32 - half;
                let dy = y as f32 - half;
                let r = (dx * dx + dy * dy).sqrt();

                if r <= radius {
                    cells.set(x, y, code);
                    if radius < 13.0 && radius - r < 1.0 {
                        shore.push((x, y));
                    }
                }
            }
        }
    }

    let wanted = shore.len() / 2;
    let mut chosen = std::collections::HashSet::new();
    let mut tries = wanted * 8 + 16;
    while chosen.len() < wanted && tries > 0 {
        tries -= 1;
        chosen.insert(shore[dice.range(0, shore.len().max(1) as i32) as usize]);
    }
    for (x, y) in &chosen {
        cells.set(*x, *y, 3);
    }

    let mut squares = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as i32, y as i32);
            match cells.get(x, y) {
                1 => squares.push(Painted::ground(fx, fy, OASIS_FLOOR)),
                2 => squares.push(Painted::ground(fx, fy, OASIS_WATER)),
                3 => {
                    let size = if dice.one_in(2) { 120 } else { 140 };
                    squares.push(Painted::standing(fx, fy, OASIS_FLOOR, OASIS_TREE).sized(size));
                }
                _ => {}
            }
        }
    }

    Drawing {
        squares,
        placed: vec![
            Placed::Living {
                x: 15.5,
                y: 15.5,
                name: "Oasis Giant",
                size: 0,
            },
            Placed::Chest {
                x: 15.5,
                y: 15.5,
                loot: OASIS_LOOT,
                least: 5,
                most: 8,
            },
        ],
        ..Drawing::default()
    }
}

const PYRE_FLOOR: &str = "Scorch Blend";

const PYRE_LOOT: &[Tier] = &[
    tier(5, "Weapon", 0.3),
    tier(6, "Weapon", 0.2),
    tier(7, "Weapon", 0.1),
    tier(4, "Armor", 0.3),
    tier(5, "Armor", 0.2),
    tier(6, "Armor", 0.1),
    tier(2, "Ability", 0.3),
    tier(3, "Ability", 0.2),
    tier(1, "Ring", 0.25),
    tier(2, "Ring", 0.15),
    tier(1, "Potion", 0.5),
];

/// A burnt circle with a ragged edge, the phoenix lord at the middle of it.
fn pyre(dice: &mut Dice) -> Drawing {
    const SIZE: usize = 30;
    let half = SIZE as f32 / 2.0;
    let mut squares = Vec::new();

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - half;
            let dy = y as f32 - half;

            // The two squares of noise are what keep the burn from being a circle.
            let r = (dx * dx + dy * dy).sqrt() + dice.roll() * 4.0 - 2.0;
            if r <= 10.0 {
                squares.push(Painted::ground(x as i32, y as i32, PYRE_FLOOR));
            }
        }
    }

    Drawing {
        squares,
        placed: vec![
            Placed::Living {
                x: 15.5,
                y: 15.5,
                name: "Phoenix Lord",
                size: 0,
            },
            Placed::Chest {
                x: 15.5,
                y: 15.5,
                loot: PYRE_LOOT,
                least: 5,
                most: 8,
            },
        ],
        ..Drawing::default()
    }
}

const LAVA: &str = "Lava Blend";
const LAVA_ISLAND: &str = "Partial Red Floor";

const LAVA_LOOT: &[Tier] = &[
    tier(7, "Weapon", 0.3),
    tier(8, "Weapon", 0.2),
    tier(9, "Weapon", 0.1),
    tier(6, "Armor", 0.3),
    tier(7, "Armor", 0.2),
    tier(8, "Armor", 0.1),
    tier(2, "Ability", 0.3),
    tier(3, "Ability", 0.2),
    tier(4, "Ability", 0.1),
    tier(2, "Ring", 0.25),
    tier(3, "Ring", 0.15),
    tier(1, "Potion", 0.5),
];

/// A lens of lava across the mountains, with islands in it and the red demon at the centre.
fn lava_fissure(dice: &mut Dice) -> Drawing {
    const SIZE: usize = 40;
    const SCALE: f32 = 5.5;
    let root_two = 2.0f32.sqrt();

    let mut cells = Cells::new(SIZE, SIZE);

    // A diagonal band whose width swells in the middle: two sine curves either side of a line.
    for x in 0..SIZE {
        let t = x as f32 / SIZE as f32 * std::f32::consts::PI;
        let low =
            (t / root_two - 2.0 * t.sin() / (SCALE * root_two)) / (std::f32::consts::PI / root_two);
        let high =
            (t / root_two + t.sin() / (SCALE * root_two)) / (std::f32::consts::PI / root_two);

        let from = (low * SIZE as f32).ceil() as i32;
        let to = (high * SIZE as f32).floor() as i32;

        for y in from.max(0)..to.min(SIZE as i32) {
            cells.set(x, y as usize, 1);
        }
    }

    for y in 0..SIZE {
        for x in 0..SIZE {
            if cells.get(x, y) == 1 && dice.one_in(5) {
                cells.set(x, y, 2);
            }
        }
    }

    let turns = dice.range(0, 4);
    let mut cells = cells.turned(turns, dice);

    // Somewhere to stand at the centre, whatever the rest of it came out as.
    cells.set(20, 20, 2);

    let mut squares = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as i32, y as i32);
            match cells.get(x, y) {
                1 => squares.push(Painted::ground(fx, fy, LAVA)),
                2 => squares.push(Painted::standing(fx, fy, LAVA, LAVA_ISLAND)),
                _ => {}
            }
        }
    }

    Drawing {
        squares,
        placed: vec![
            Placed::Living {
                x: 20.5,
                y: 20.5,
                name: "Red Demon",
                size: 0,
            },
            Placed::Chest {
                x: 20.5,
                y: 20.5,
                loot: LAVA_LOOT,
                least: 5,
                most: 8,
            },
        ],
        ..Drawing::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_kind() -> Vec<Kind> {
        SCATTER.iter().map(|(kind, _, _, _)| *kind).collect()
    }

    #[test]
    fn every_setpiece_draws_something() {
        // A setpiece that draws nothing is a hole in the ground nobody notices.
        for kind in every_kind() {
            let mut dice = Dice::new(0x5eed_1234);
            let drawing = kind.draw(&mut dice);

            assert!(
                !drawing.squares.is_empty()
                    || !drawing.placed.is_empty()
                    || drawing.prefab.is_some(),
                "{kind:?} drew nothing"
            );
        }
    }

    #[test]
    fn nothing_is_drawn_outside_the_room_it_asked_for() {
        // The scatterer keeps them apart by size. A drawing that spills past its own size would
        // overwrite whatever was placed beside it.
        for kind in every_kind() {
            for seed in [1u32, 7, 99, 4242] {
                let mut dice = Dice::new(seed);
                let drawing = kind.draw(&mut dice);
                let size = kind.size() as i32;

                for square in &drawing.squares {
                    assert!(
                        square.x >= 0 && square.y >= 0 && square.x < size && square.y < size,
                        "{kind:?} painted {},{} outside its {size}",
                        square.x,
                        square.y
                    );
                }

                for placed in &drawing.placed {
                    let (x, y) = match placed {
                        Placed::Living { x, y, .. } => (*x, *y),
                        Placed::Chest { x, y, .. } => (*x, *y),
                    };
                    assert!(
                        x >= 0.0 && y >= 0.0 && x <= size as f32 && y <= size as f32,
                        "{kind:?} placed something at {x},{y} outside its {size}"
                    );
                }
            }
        }
    }

    #[test]
    fn two_of_the_same_setpiece_are_not_the_same_setpiece() {
        // Every one of these is a drawing program rather than a saved map, and the reason is that
        // two groves should not be the same grove.
        for kind in every_kind() {
            if matches!(
                kind,
                Kind::LuckyDjinn | Kind::LuckyEnt | Kind::Crystal | Kind::KageKami
            ) {
                continue;
            }

            let first = kind.draw(&mut Dice::new(11));
            let second = kind.draw(&mut Dice::new(9_871));

            assert_ne!(first.squares, second.squares, "{kind:?} came out the same");
        }
    }

    #[test]
    fn every_setpiece_that_should_have_a_boss_has_one() {
        let bossless = [Kind::Building, Kind::KageKami];

        for kind in every_kind() {
            if bossless.contains(&kind) {
                continue;
            }

            let drawing = kind.draw(&mut Dice::new(3));
            assert!(
                drawing
                    .placed
                    .iter()
                    .any(|placed| matches!(placed, Placed::Living { .. })),
                "{kind:?} has nothing living in it"
            );
        }
    }

    #[test]
    fn a_rotation_moves_a_corner_to_the_next_corner() {
        let mut cells = Cells::new(3, 2);
        cells.set(0, 0, 9);

        let turned = cells.rotate();
        assert_eq!(turned.width, 2);
        assert_eq!(turned.height, 3);
        assert_eq!(turned.get(1, 0), 9);
    }

    #[test]
    fn setpieces_do_not_overlap_each_other() {
        // Two castles drawn through each other is one broken castle.
        let mut dice = Dice::new(0x1234_5678);
        let placed = scatter(2048, 2048, &|_, _| Terrain::MidForest, &mut dice);

        assert!(!placed.is_empty(), "nothing was placed at all");

        for (index, one) in placed.iter().enumerate() {
            for other in &placed[index + 1..] {
                let (a, b) = (one.kind.size(), other.kind.size());
                let apart = one.x + a < other.x
                    || other.x + b < one.x
                    || one.y + a < other.y
                    || other.y + b < one.y;
                assert!(apart, "{one:?} and {other:?} overlap");
            }
        }
    }

    #[test]
    fn a_setpiece_only_goes_on_ground_it_belongs_on() {
        let mut dice = Dice::new(77);

        // Half the map is mountain, the other half is forest.
        let placed = scatter(
            512,
            512,
            &|x, _| {
                if x < 256 {
                    Terrain::Mountains
                } else {
                    Terrain::MidForest
                }
            },
            &mut dice,
        );

        for one in &placed {
            let (_, _, _, terrains) = SCATTER
                .iter()
                .find(|(kind, _, _, _)| *kind == one.kind)
                .unwrap();

            let ground = if one.x < 256 {
                Terrain::Mountains
            } else {
                Terrain::MidForest
            };
            assert!(
                terrains.contains(&ground),
                "{:?} was put on {ground:?}",
                one.kind
            );
        }
    }

    #[test]
    fn every_name_a_setpiece_uses_is_in_the_content() {
        // A name the catalog does not have is a hole in the realm somebody walks into.
        let Ok((catalog, _)) = hendra_content::Catalog::load_dir(std::path::Path::new(
            "../../../godot-client/assets/xml",
        )) else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        let mut missing: Vec<String> = Vec::new();

        for kind in every_kind() {
            // Several seeds, because a name only used by one branch of a drawing would otherwise
            // go unchecked.
            for seed in [1u32, 5, 23, 191, 7_777] {
                let drawing = kind.draw(&mut Dice::new(seed));

                for square in &drawing.squares {
                    if let Some(name) = square.tile
                        && catalog.tile_type_of(name).is_none()
                    {
                        missing.push(format!("{kind:?} ground {name}"));
                    }
                    if let Some(name) = square.object
                        && catalog.type_of(name).is_none()
                    {
                        missing.push(format!("{kind:?} object {name}"));
                    }
                }

                for placed in &drawing.placed {
                    match placed {
                        Placed::Living { name, .. } => {
                            if catalog.type_of(name).is_none() {
                                missing.push(format!("{kind:?} living {name}"));
                            }
                        }
                        Placed::Chest { loot, .. } => {
                            for tier in *loot {
                                let any = catalog.items().any(|desc| {
                                    desc.item
                                        .as_ref()
                                        .is_some_and(|item| item.tier == Some(tier.tier as i32))
                                });
                                assert!(any, "{kind:?} wants tier {} of anything", tier.tier);
                            }
                        }
                    }
                }
            }
        }

        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "not in the content: {missing:#?}");
    }

    #[test]
    fn a_realm_with_nowhere_to_put_them_gets_none_rather_than_looping() {
        let mut dice = Dice::new(5);
        let placed = scatter(512, 512, &|_, _| Terrain::BeachTowels, &mut dice);

        assert!(placed.is_empty());
    }
}
