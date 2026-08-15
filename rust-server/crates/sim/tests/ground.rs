//! What a death does to the ground it fell on, against `wServer/logic/behaviors/ChangeGroundOnDeath.cs`.
//!
//! The Shatters is built out of this one behaviour: bridges appear and disappear because something
//! standing on them dies and rewrites the squares underneath. It is worth pinning because all three
//! of its rules are easy to get wrong in the same direction — the area is a square and not a disc,
//! it is placed by its corner and not its centre, and it rewrites only ground that is already one
//! of the kinds it names.

use hendra_content::map::{Composition, Map};
use hendra_content::{Catalog, ObjectType, Region, TileType};
use hendra_sim::world::Entity;
use hendra_sim::{Kind, Terrain, World};

const FIXTURE: &str = r#"<Objects>
    <Ground type="0x10" id="Bridge"/>
    <Ground type="0x11" id="Pure Evil"/>
    <Ground type="0x12" id="Shattered Floor"/>

    <Object type="0x502" id="Closer"><Class>Character</Class><Enemy/>
      <MaxHitPoints>10</MaxHitPoints></Object>
  </Objects>"#;

const CLOSER: ObjectType = ObjectType(0x502);
const BRIDGE: u16 = 0x10;
const EVIL: u16 = 0x11;
const SHATTERED: u16 = 0x12;

fn catalog() -> Catalog {
    Catalog::load_str(&[FIXTURE]).0
}

/// A field of bridge with a stripe of shattered floor down the middle column.
fn field(catalog: &Catalog) -> World {
    let squares = (0..40 * 40).map(|index| Composition {
        tile: TileType(if (index % 40) == 20 {
            SHATTERED
        } else {
            BRIDGE
        }),
        object: ObjectType::NONE,
        region: Region::None,
        terrain: hendra_content::Terrain::None,
        config: String::new(),
    });
    let map = Map::from_squares(40, 40, squares).unwrap();
    World::new("Shatters", Terrain::build(map, catalog), catalog)
}

/// Kills one enemy at `(x, y)` running `table`, and hands back the ground afterwards.
fn ground_after(catalog: &Catalog, source: &str, x: f32, y: f32) -> World {
    let mut world = field(catalog);

    let mut enemy = Entity::fixture(CLOSER, x, y);
    enemy.kind = Kind::Enemy;
    enemy.max_hp = 10;
    enemy.hp = 10;
    let enemy = world.spawn(enemy).expect("room for an enemy");

    let (programs, diagnostics) =
        hendra_behavior::compile::compile(&hendra_behavior::parse::parse(source).expect("parses"));
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    world.set_behaviours(catalog, programs);

    world.get_mut(enemy).expect("the enemy").dead = true;
    world.advance(catalog, 50);
    world
}

fn tile(world: &World, x: u32, y: u32) -> u16 {
    world.terrain().tile_at(x, y).0
}

#[test]
fn only_the_named_ground_is_rewritten() {
    // `if (tile.TileId == dat.IdToTileType[type])` (`ChangeGroundOnDeath.cs:44`). The filter is the
    // whole point of the behaviour: the Shatters' closers turn bridges into pure evil and leave the
    // floor beside them alone. Converting the call without its name lists left nothing to match
    // against, and all fourteen of them did nothing whatever.
    let catalog = catalog();
    let world = ground_after(
        &catalog,
        r#"enemy "Closer" { change_ground_on_death(["Bridge"], ["Pure Evil"], 5)
            state a { } }"#,
        20.0,
        20.0,
    );

    // The middle column was shattered floor, which is not named, so it survives.
    assert_eq!(tile(&world, 20, 20), SHATTERED, "the floor is not a bridge");
    // Its neighbours were bridge, which is.
    assert_eq!(tile(&world, 19, 20), EVIL);
    assert_eq!(tile(&world, 21, 20), EVIL);
}

#[test]
fn the_area_is_a_square_placed_by_its_corner() {
    // `pos = ((int)X - dist / 2, (int)Y - dist / 2)` and then `for x in 0..dist, y in 0..dist`
    // (`ChangeGroundOnDeath.cs:30-36`). Integer division, so a side of five reaches two squares one
    // way and two the other, and the whole square is covered rather than a disc inscribed in it —
    // including its corners, which a disc of the same reach would miss.
    let catalog = catalog();
    let world = ground_after(
        &catalog,
        r#"enemy "Closer" { change_ground_on_death(["Bridge"], ["Pure Evil"], 5)
            state a { } }"#,
        10.0,
        10.0,
    );

    // Corner to corner of the five by five square starting at (8, 8).
    for y in 8..=12 {
        for x in 8..=12 {
            assert_eq!(tile(&world, x, y), EVIL, "inside, at ({x}, {y})");
        }
    }

    // And nothing outside it, on any side.
    for (x, y) in [(7, 10), (13, 10), (10, 7), (10, 13), (7, 7), (13, 13)] {
        assert_eq!(tile(&world, x, y), BRIDGE, "outside, at ({x}, {y})");
    }
}

#[test]
fn a_call_naming_two_kinds_of_ground_changes_both() {
    // `new ChangeGroundOnDeath(new[] { "shtrs Shattered Floor", "shtrs Disaster Floor" }, …)` — four
    // of the fourteen calls name more than one kind, and the C# arrays are what say where one list
    // ends and the next begins. Flattening them into loose arguments, which is how the variadic
    // behaviours spell their name lists, cannot express this.
    let catalog = catalog();
    let world = ground_after(
        &catalog,
        r#"enemy "Closer" {
            change_ground_on_death(["Bridge", "Shattered Floor"], ["Pure Evil"], 5)
            state a { } }"#,
        20.0,
        20.0,
    );

    assert_eq!(tile(&world, 20, 20), EVIL, "the floor went too");
    assert_eq!(tile(&world, 19, 20), EVIL, "and so did the bridge");
}

#[test]
fn naming_no_ground_at_all_rewrites_everything_in_reach() {
    // `groundToChange == null` takes the other branch and rewrites unconditionally
    // (`ChangeGroundOnDeath.cs:48-53`). Nothing in the shipped content does this, but the empty list
    // is how the absence is spelled here and it should mean what the original's null means rather
    // than "match nothing".
    let catalog = catalog();
    let world = ground_after(
        &catalog,
        r#"enemy "Closer" { change_ground_on_death([], ["Pure Evil"], 3) state a { } }"#,
        20.0,
        20.0,
    );

    assert_eq!(tile(&world, 20, 20), EVIL, "the floor, which was not named");
    assert_eq!(tile(&world, 19, 20), EVIL, "and the bridge beside it");
}
