//! Whether a wall one tile thick actually stops a movement claim, on the game's own maps.
//!
//! Run with:
//!
//!     cargo run --release -p hendra-sim --example wall_sweep
//!     cargo run --release -p hendra-sim --example wall_sweep ../godot-client/assets/xml
//!
//! Every wall in the game is one tile thick, and a movement check that only asks "could you be
//! standing there?" answers yes on either side of one. This finds real one-tile walls in the
//! shipped maps and puts a claim straight across each of them — the claim a client that had its
//! own collision removed would send — then reports where the world actually put the player.
//!
//! Both halves matter. A wall that stops the claim is the check working; a doorway that still lets
//! one through is the check not having walled the dungeon shut to do it.
//!
//! A third count covers the half-tile skin that `FullOccupy` puts around a wall. Two walls that
//! meet only at a corner leave a diagonal gap that is nothing at all if a body is a point: both
//! squares of the diagonal can be stood on, so a claim straight across is a claim between two
//! squares that can each be stood on. The original refuses it, because the body reaches half a tile
//! and that reach overlaps both walls the whole way. A server that only asks whether the ends can
//! be stood on lets players cut every corner in the game.
//!
//! A second pass covers the walls no shipped map contains: the ones a setpiece puts down after the
//! world is already open. Each setpiece is drawn onto an empty field of grass and the same claims
//! are made against it, so a castle whose curtain wall can be walked through shows here rather than
//! in the realm.

use std::path::Path;

use hendra_content::{Catalog, Composition, Map, ObjectType};
use hendra_sim::setpiece::Kind;
use hendra_sim::world::Entity;
use hendra_sim::{Terrain, World};

/// Every setpiece the game can put into a world, realm-scattered and Oryx-raised alike.
const SETPIECES: &[Kind] = &[
    Kind::Building,
    Kind::Graveyard,
    Kind::Grove,
    Kind::LichyTemple,
    Kind::Castle,
    Kind::Tower,
    Kind::TempleA,
    Kind::TempleB,
    Kind::Oasis,
    Kind::Pyre,
    Kind::LavaFissure,
    Kind::LuckyDjinn,
    Kind::LuckyEnt,
    Kind::Crystal,
    Kind::KageKami,
    Kind::SkullShrine,
    Kind::Pentaract,
    Kind::Sphinx,
    Kind::LordOfTheLostLands,
    Kind::Hermit,
    Kind::GhostShip,
    Kind::LordOfSky,
    Kind::Spooky,
];

fn main() {
    let content = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../godot-client/assets/xml".to_string());

    let Ok((catalog, _)) = Catalog::load_dir(Path::new(&content)) else {
        eprintln!("cannot read the content at {content}");
        std::process::exit(1);
    };

    let directory = Path::new("content/maps");
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("hmap"))
        .collect();
    paths.sort();

    println!(
        "{:<30} {:>12} {:>18} {:>10} {:>12} {:>14} {:>16}",
        "map",
        "one-tile walls",
        "claim across one",
        "crossed",
        "doorways",
        "corners cut",
        "wall faces lain on"
    );

    let (mut walls_total, mut crossed_total, mut doors_total, mut doors_open) = (0, 0, 0, 0);
    let (mut corners_total, mut corners_cut) = (0, 0);
    let (mut faces_total, mut faces_flattened) = (0, 0);
    let (mut occupy_total, mut enemy_occupy_total, mut full_occupy_total) = (0, 0, 0);
    let mut spawn_only_total = 0usize;

    for path in paths {
        let Ok(raw) = std::fs::read(&path) else {
            continue;
        };
        let Ok(map) = hendra_content::Map::read(&raw) else {
            continue;
        };

        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let terrain = Terrain::build(map, &catalog);
        let (width, height) = (terrain.width(), terrain.height());
        let mut world = World::new(&name, terrain, &catalog);

        // Somewhere to stand while the claims are made. Moved by hand before each one rather than
        // respawned, because a claim is measured from where the body already is.
        let Some(walker) = world.spawn(Entity::player(ObjectType(0x0300), 1.5, 1.5, 800)) else {
            continue;
        };

        let (occupy, enemy_occupy, full_occupy) = world.terrain().occupancy_counts();
        occupy_total += occupy;
        enemy_occupy_total += enemy_occupy;
        full_occupy_total += full_occupy;

        // Squares a realm enemy may be placed on that a player may not stand on: the gap between
        // `IsPassable(spawning: false)`, which is what `Oryx.Spawn` calls, and `spawning: true`,
        // which is what everything else calls. Every one of them is a tree, cactus or rock a realm
        // is meant to be able to put an enemy in front of.
        for y in 0..height {
            for x in 0..width {
                if world.terrain().passable(x, y) && !world.terrain().walkable(x, y) {
                    spawn_only_total += 1;
                }
            }
        }

        let Counts {
            walls,
            crossed,
            doors,
            open,
            corners,
            cut,
            faces,
            flattened,
        } = sweep(&mut world, &catalog, walker, (1, 1, width - 1, height - 1));

        if walls == 0 && doors == 0 && corners == 0 && faces == 0 {
            continue;
        }

        println!(
            "{:<30} {:>12} {:>18} {:>10} {:>12} {:>14} {:>16}",
            name,
            walls,
            walls,
            if crossed == 0 {
                "none".to_string()
            } else {
                crossed.to_string()
            },
            format!("{open}/{doors} open"),
            format!("{cut}/{corners}"),
            format!("{flattened}/{faces}"),
        );

        walls_total += walls;
        crossed_total += crossed;
        doors_total += doors;
        doors_open += open;
        corners_total += corners;
        corners_cut += cut;
        faces_total += faces;
        faces_flattened += flattened;
    }

    println!();
    println!("{walls_total} one-tile walls, {crossed_total} of them crossed by a claim.");
    println!("{doors_open} of {doors_total} doorways still walked through.");
    println!("{corners_cut} of {corners_total} corner joins cut by a diagonal claim.");
    println!("{faces_flattened} of {faces_total} wall faces a body could lie flat against.");
    println!(
        "squares occupied: {occupy_total} OccupySquare, {enemy_occupy_total} EnemyOccupySquare, \
         {full_occupy_total} FullOccupy."
    );
    println!(
        "{spawn_only_total} squares a realm enemy may be placed on that a player cannot stand on."
    );

    if crossed_total > 0 {
        eprintln!("a claim crossed a wall: the path is not being swept");
        std::process::exit(1);
    }

    if corners_cut > 0 {
        eprintln!("a diagonal claim cut a corner: the half-tile reach is not being tested");
        std::process::exit(1);
    }

    if faces_flattened > 0 {
        eprintln!("a body lay flat against a wall: the half-tile reach is not being tested");
        std::process::exit(1);
    }

    if !setpiece_sweep(&catalog, directory) {
        std::process::exit(1);
    }
}

/// What one sweep of a rectangle found.
#[derive(Default)]
struct Counts {
    walls: usize,
    crossed: usize,
    doors: usize,
    open: usize,
    corners: usize,
    cut: usize,
    faces: usize,
    flattened: usize,
}

/// Finds every one-tile wall, doorway, corner join and wall face in a rectangle and probes each.
///
/// `area` is `(from_x, from_y, to_x, to_y)`, with the far edge excluded, and must stay one square
/// inside the world so every neighbour a probe reads exists.
fn sweep(
    world: &mut World,
    catalog: &Catalog,
    walker: hendra_sim::slab::Handle,
    area: (u32, u32, u32, u32),
) -> Counts {
    let (from_x, from_y, to_x, to_y) = area;
    let mut found = Counts::default();

    for y in from_y..to_y {
        for x in from_x..to_x {
            let blocked = !world.terrain().walkable(x, y);
            let near = world.terrain().walkable(x - 1, y);
            let far = world.terrain().walkable(x + 1, y);

            // A corner join: two full-occupy squares touching only at a corner, with the other
            // diagonal open at both ends. The gap between them is nothing a point could not pass
            // and everything a half-tile reach cannot. Both orientations of the join are looked at,
            // because a wall turning left and a wall turning right are equally common and only one
            // of them is a north-east diagonal.
            if world.terrain().walkable(x, y)
                && world.terrain().walkable(x + 1, y + 1)
                && world.terrain().full_occupy(x + 1, y)
                && world.terrain().full_occupy(x, y + 1)
            {
                found.corners += 1;
                if cut_the_corner(world, catalog, walker, x, y, 1.0, 1.0) {
                    found.cut += 1;
                }
            }

            if world.terrain().walkable(x + 1, y)
                && world.terrain().walkable(x, y + 1)
                && world.terrain().full_occupy(x, y)
                && world.terrain().full_occupy(x + 1, y + 1)
            {
                found.corners += 1;
                if cut_the_corner(world, catalog, walker, x + 1, y, -1.0, 1.0) {
                    found.cut += 1;
                }
            }

            // The half-tile skin itself: somewhere open beside a full-occupy square, and a claim to
            // press flat against its face. A body that is a point can lie against a wall; a body
            // that reaches half a tile stops half a tile short of it.
            if world.terrain().walkable(x, y) && world.terrain().full_occupy(x + 1, y) {
                found.faces += 1;
                if pressed_against(world, catalog, walker, x, y) {
                    found.flattened += 1;
                }
            }

            // A doorway: three squares in a row that can all be walked, with something solid above
            // and below, so it is a hole in a wall rather than open field.
            if !blocked
                && near
                && far
                && !world.terrain().walkable(x, y - 1)
                && !world.terrain().walkable(x, y + 1)
            {
                found.doors += 1;
                if walked_through(world, catalog, walker, x, y) {
                    found.open += 1;
                }
                continue;
            }

            if !blocked || !near || !far {
                continue;
            }

            found.walls += 1;
            if walked_through(world, catalog, walker, x, y) {
                found.crossed += 1;
            }
        }
    }

    found
}

/// Draws each setpiece onto an empty field and puts the same claims against what it built.
///
/// The shipped maps say nothing about this: a setpiece is a drawing program run after the world is
/// open, and its walls reach the collision grids by a different road than a map's do. A wall a
/// castle lays down is a wall, and a claim across it has to be refused for the same reason.
///
/// Returns whether every setpiece held.
fn setpiece_sweep(catalog: &Catalog, maps: &Path) -> bool {
    println!();
    println!("setpieces, each drawn onto an empty field:");
    println!(
        "{:<24} {:>14} {:>10} {:>12} {:>14} {:>16}",
        "setpiece", "one-tile walls", "crossed", "doorways", "corners cut", "wall faces lain on"
    );

    let Some(grass) = catalog.tile_type_of("Grass") else {
        eprintln!("the content has no Grass to draw a setpiece on");
        return false;
    };

    let (mut walls_total, mut crossed_total) = (0, 0);
    let (mut meant_total, mut lost_total) = (0usize, 0usize);
    let mut broken = Vec::new();

    for &kind in SETPIECES {
        // Room for the drawing and a margin of open ground around it, so a wall on the outer edge
        // still has somewhere to be claimed from.
        let side = kind.size() + 8;
        let field = Composition {
            tile: grass,
            ..Composition::default()
        };
        let Ok(map) = Map::from_squares(
            side,
            side,
            std::iter::repeat_n(field, (side as usize) * (side as usize)),
        ) else {
            continue;
        };

        let terrain = Terrain::build(map, catalog);
        let mut world = World::new(format!("{kind:?}"), terrain, catalog);

        let mut dice = hendra_sim::setpiece::Dice::new(0x5eed_beef);
        let drawing = kind.draw(&mut dice);

        // Every square the setpiece means to put something solid on, in world coordinates. Kept
        // before it is placed, so what the drawing asked for can be held against what the world
        // ended up with: a wall that never reached the collision grids is a wall the setpiece drew
        // and the world does not have.
        let mut meant: Vec<(u32, u32)> = Vec::new();

        // A setpiece that is a saved map is stamped rather than painted, which is the road the
        // original takes for `RenderFromProto`.
        if let Some(prefab) = drawing.prefab {
            let Ok(raw) = std::fs::read(maps.join(format!("{prefab}.hmap"))) else {
                println!("{:<24} {:>14}", format!("{kind:?}"), "no map");
                continue;
            };
            let Ok(piece) = Map::read(&raw) else {
                continue;
            };

            let half = kind.size() as f32 / 2.0;
            let at = (4.0 + half, 4.0 + half);
            let left = at.0 as i32 - piece.width() as i32 / 2;
            let top = at.1 as i32 - piece.height() as i32 / 2;

            for (x, y, square) in piece.objects() {
                if catalog
                    .object(square.object)
                    .is_some_and(|desc| desc.occupy_square)
                {
                    meant.push(((left + x as i32) as u32, (top + y as i32) as u32));
                }
            }

            world.stamp(catalog, &piece, at);
        } else {
            for square in &drawing.squares {
                let solid = square
                    .object
                    .and_then(|name| catalog.type_of(name))
                    .and_then(|kind| catalog.object(kind))
                    .is_some_and(|desc| desc.occupy_square);
                if solid {
                    meant.push(((4 + square.x) as u32, (4 + square.y) as u32));
                }
            }

            for name in world.draw(catalog, &drawing, (4, 4)) {
                println!("  {kind:?} wants {name}, which the content does not have");
            }
        }
        world.reindex();

        // A square the setpiece put a wall on that a player may still stand in.
        let lost = meant
            .iter()
            .filter(|&&(x, y)| world.terrain().walkable(x, y))
            .count();
        meant_total += meant.len();
        lost_total += lost;
        if lost > 0 {
            println!(
                "  {kind:?}: {lost} of {} squares it walls are still open ground",
                meant.len()
            );
            broken.push(kind);
        }

        let Some(walker) = world.spawn(Entity::player(ObjectType(0x0300), 1.5, 1.5, 800)) else {
            continue;
        };

        let found = sweep(&mut world, catalog, walker, (1, 1, side - 1, side - 1));

        println!(
            "{:<24} {:>14} {:>10} {:>12} {:>14} {:>16}",
            format!("{kind:?}"),
            found.walls,
            if found.crossed == 0 {
                "none".to_string()
            } else {
                found.crossed.to_string()
            },
            format!("{}/{} open", found.open, found.doors),
            format!("{}/{}", found.cut, found.corners),
            format!("{}/{}", found.flattened, found.faces),
        );

        walls_total += found.walls;
        crossed_total += found.crossed;
        if found.crossed > 0 || found.cut > 0 || found.flattened > 0 {
            broken.push(kind);
        }
    }

    println!();
    println!("{walls_total} one-tile walls placed by setpieces, {crossed_total} of them crossed.");
    println!(
        "{meant_total} squares walled by a setpiece, {lost_total} of them still walkable ground."
    );

    if !broken.is_empty() {
        eprintln!("setpieces whose walls do not stop a claim: {broken:?}");
        return false;
    }

    true
}

/// Puts a player in one open square of a corner join and claims the one diagonally opposite.
///
/// Both squares can be stood on and the claim is a straight diagonal between them, so nothing about
/// where it starts or ends is refusable. Only the half-tile reach against the two walls it passes
/// between can refuse it. `step_x`/`step_y` say which way the far square lies, so both orientations
/// of a join are probed by the same claim.
///
/// Returns whether the player ended up in the far square.
fn cut_the_corner(
    world: &mut World,
    catalog: &Catalog,
    walker: hendra_sim::slab::Handle,
    x: u32,
    y: u32,
    step_x: f32,
    step_y: f32,
) -> bool {
    let (from_x, from_y) = (x as f32 + 0.5, y as f32 + 0.5);
    let (to_x, to_y) = (from_x + step_x, from_y + step_y);

    if let Some(entity) = world.get_mut(walker) {
        entity.x = from_x;
        entity.y = from_y;
    }

    let Some(outcome) = world.resolve_move(walker, catalog, to_x, to_y, 1000) else {
        return false;
    };

    // Arrived in the far square rather than stopped at the join.
    (outcome.x - to_x).abs() < 0.5 && (outcome.y - to_y).abs() < 0.5
}

/// Claims a position pressed flat against the west face of a full-occupy square.
///
/// The claim is a tenth of a tile from the wall, well inside the half-tile the original keeps
/// clear, and it names a square that can be stood on — so only the reach can refuse it.
///
/// Returns whether the player ended up past the half-tile line.
fn pressed_against(
    world: &mut World,
    catalog: &Catalog,
    walker: hendra_sim::slab::Handle,
    x: u32,
    y: u32,
) -> bool {
    let (from_x, from_y) = (x as f32 + 0.5, y as f32 + 0.5);
    let to_x = x as f32 + 0.9;

    if let Some(entity) = world.get_mut(walker) {
        entity.x = from_x;
        entity.y = from_y;
    }

    let Some(outcome) = world.resolve_move(walker, catalog, to_x, from_y, 1000) else {
        return false;
    };

    outcome.x > x as f32 + 0.5
}

/// Puts a player one square to the left of `x, y` and claims the square to its right.
///
/// The claim is two tiles: it starts on ground that can be walked, ends on ground that can be
/// walked, and has the square in the middle in between. A whole second of allowance is handed over
/// so the speed limit has nothing to say about it and the only thing left to refuse it is the
/// ground.
///
/// Returns whether the player ended up past the middle square.
fn walked_through(
    world: &mut World,
    catalog: &Catalog,
    walker: hendra_sim::slab::Handle,
    x: u32,
    y: u32,
) -> bool {
    let (from_x, from_y) = (x as f32 - 0.5, y as f32 + 0.5);
    let (to_x, to_y) = (x as f32 + 1.5, y as f32 + 0.5);

    if let Some(entity) = world.get_mut(walker) {
        entity.x = from_x;
        entity.y = from_y;
    }

    let Some(outcome) = world.resolve_move(walker, catalog, to_x, to_y, 1000) else {
        return false;
    };

    outcome.x > x as f32 + 1.0
}
