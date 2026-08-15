//! The two fights that are built out of `TransferDamageOnDeath`, against the shipped content.
//!
//! The behaviour is easy to read as damage — it is named for damage, and the entities that use it
//! are linked bosses that visibly die together. It deals none. `TransferDamageOnDeath.cs:27-32`
//! finds one entity and merges the damage counters, which moves the loot and experience credit and
//! nothing else. Reading it the other way is not a subtle difference: the Pentaract has ten
//! thousand health and each of its five towers eight thousand, so the second tower to fall would
//! end the fight before it started.
//!
//! Run against the real catalog and the real behaviour files rather than a fixture, because what is
//! being checked is a relationship between numbers the content chose.

use hendra_content::map::{Composition, Map};
use hendra_content::{Catalog, ObjectType, Region, TileType};
use hendra_sim::world::Entity;
use hendra_sim::{Handle, Kind, Terrain, World};

/// Where this server and its client both read their object descriptions from.
const XML: &str = "../../../godot-client/assets/xml";

/// Where the converted enemy behaviours live.
const BEHAVIOURS: &str = "../../content/behaviours";

fn catalog() -> Catalog {
    let (catalog, report) = Catalog::load_dir(std::path::Path::new(XML)).expect("the shipped xml");
    assert!(report.objects > 0, "the shipped objects were read");
    catalog
}

/// An empty field of grass.
fn field(catalog: &Catalog) -> World {
    let squares = (0..32 * 32).map(|_| Composition {
        tile: TileType(0x10),
        object: ObjectType::NONE,
        region: Region::None,
        terrain: hendra_content::Terrain::None,
        config: String::new(),
    });
    let map = Map::from_squares(32, 32, squares).unwrap();
    World::new("Field", Terrain::build(map, catalog), catalog)
}

/// Installs the behaviours from one shipped content file and makes the new bodies visible.
///
/// After the entities are placed rather than before, because this is what gives each of them a
/// mind: an enemy that arrives afterwards has none, and an enemy with no mind has no death
/// behaviours to run.
fn behaving(world: &mut World, catalog: &Catalog, file: &str) {
    let source = std::fs::read_to_string(format!("{BEHAVIOURS}/{file}")).expect("the content file");
    let parsed = hendra_behavior::parse::parse(&source).expect("parses");
    let (programs, diagnostics) = hendra_behavior::compile::compile(&parsed);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    world.set_behaviours(catalog, programs);
    world.reindex();
}

fn place(world: &mut World, catalog: &Catalog, id: &str, x: f32, y: f32) -> Handle {
    let kind = catalog.type_of(id).unwrap_or_else(|| panic!("{id} exists"));
    let health = catalog.object(kind).unwrap().max_hp;

    let mut entity = Entity::fixture(kind, x, y);
    entity.kind = Kind::Enemy;
    entity.max_hp = health;
    entity.hp = health;
    world.spawn(entity).expect("room")
}

/// A player in the corner, out of everything's reach, so that only the transfer moves.
fn a_watcher(world: &mut World, catalog: &Catalog) -> Handle {
    let kind = catalog.type_of("Wizard").expect("a class to play");
    world
        .spawn(Entity::player(kind, 31.0, 31.0, 500))
        .expect("room")
}

/// Kills an entity as a player would have, leaving its damage counter behind.
fn felled_by(world: &mut World, handle: Handle, player: Handle, damage: i32) {
    let entity = world.get_mut(handle).expect("still standing");
    entity.damage_by.push((player, damage));
    entity.last_hurt_by = Some(player);
    entity.dead = true;
}

#[test]
fn the_pentaract_outlives_the_towers_that_fall_before_the_last_one() {
    let catalog = catalog();
    let mut world = field(&catalog);
    let player = a_watcher(&mut world, &catalog);

    let boss = place(&mut world, &catalog, "Pentaract", 10.0, 10.0);
    let towers: Vec<Handle> = (0..5)
        .map(|index| {
            place(
                &mut world,
                &catalog,
                "Pentaract Tower",
                11.0 + index as f32,
                10.0,
            )
        })
        .collect();
    behaving(&mut world, &catalog, "pentaract.beh");

    // The numbers the fight turns on. Two towers' worth of health is more than the boss has.
    assert_eq!(world.get(boss).unwrap().max_hp, 10_000);
    assert_eq!(world.get(towers[0]).unwrap().max_hp, 8_000);

    for tower in &towers[..2] {
        felled_by(&mut world, *tower, player, 8_000);
    }

    world.advance(&catalog, 50);

    let standing = world.get(boss).expect("the Pentaract is still there");
    assert!(!standing.dead, "three towers are still up");
    assert_eq!(standing.hp, 10_000, "no health moved with the credit");

    // And the whole point of the transfer: whoever killed the towers is eligible for the bag.
    assert_eq!(standing.damage_by, vec![(player, 16_000)]);
    assert_eq!(standing.last_hurt_by, Some(player));

    // Each fallen tower turns into a corpse, which is the transfer's other target and which carries
    // the loot table for this fight. An eight-thousand-point hit deletes one the instant it appears.
    let corpses: Vec<Handle> = world
        .iter()
        .filter(|(_, entity)| {
            !entity.dead
                && catalog
                    .object(entity.object_type)
                    .is_some_and(|desc| desc.id == "Pentaract Tower Corpse")
        })
        .map(|(handle, _)| handle)
        .collect();
    assert_eq!(corpses.len(), 2, "one corpse per fallen tower, still there");
    for corpse in corpses {
        assert_eq!(
            world.get(corpse).unwrap().damage_by,
            vec![(player, 8_000)],
            "and each carries the credit for the tower it came from"
        );
    }
}

#[test]
fn the_hermit_gods_drop_survives_the_hermit_god() {
    // Fifty-five thousand health handed to something with a hundred, whose entire job is to carry
    // the loot.
    let catalog = catalog();
    let mut world = field(&catalog);
    let player = a_watcher(&mut world, &catalog);

    let god = place(&mut world, &catalog, "Hermit God", 10.0, 10.0);
    let drop = place(&mut world, &catalog, "Hermit God Drop", 12.0, 10.0);
    behaving(&mut world, &catalog, "hermit.beh");

    assert_eq!(world.get(god).unwrap().max_hp, 55_000);
    assert_eq!(world.get(drop).unwrap().max_hp, 100);

    felled_by(&mut world, god, player, 55_000);
    world.advance(&catalog, 50);

    let drop = world.get(drop).expect("the drop is still there");
    assert!(!drop.dead);
    assert_eq!(drop.hp, 100);
    assert_eq!(
        drop.damage_by,
        vec![(player, 55_000)],
        "and carries the credit for the god"
    );
}
