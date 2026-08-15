//! How a boss grows with the room, against `wServer/logic/behaviors/ScaleHP.cs`.
//!
//! # Whose specification this is
//!
//! `ScaleHP.cs` itself is the shipped server's and is pristine. Its *callers* are not: every file in
//! `logic/db` was imported from another fork, and the two calls that exist — `new ScaleHP(50000)` at
//! `BehaviorDb.Oryx.cs:17` and `:96` — cannot compile against the pristine signature, which declares
//! `int maxAdditional` with no default (`ScaleHP.cs:28`). A previous round of this project added
//! `= 0` to the C# to reconcile them, which is editing the specification to fit the content, so no
//! claim here rests on that edit.
//!
//! What is pristine, and what these tests are measured against, is the meaning of nought: the field
//! is documented "leave as 0 for no limit" (`ScaleHP.cs:23`) and both caps are guarded
//! `if (maxAdditional != 0)` (`:78-79`, `:93-96`). So a call that names no cap has none.

use hendra_content::map::{Composition, Map};
use hendra_content::{Catalog, ObjectType, Region, TileType};
use hendra_sim::world::Entity;
use hendra_sim::{Kind, Terrain, World};

const FIXTURE: &str = r#"<Objects>
    <Ground type="0x10" id="Grass"/>
    <Object type="0x502" id="Oryx"><Class>Character</Class><Enemy/>
      <MaxHitPoints>1000</MaxHitPoints></Object>
    <Object type="0x600" id="Hero"><Class>Player</Class><Player/></Object>
  </Objects>"#;

const ORYX: ObjectType = ObjectType(0x502);
const HERO: ObjectType = ObjectType(0x600);

fn catalog() -> Catalog {
    Catalog::load_str(&[FIXTURE]).0
}

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

/// The boss's maximum health after `players` have stood next to it running `table`.
fn scaled_to(catalog: &Catalog, behaviour: &str, players: usize) -> i32 {
    let mut world = field(catalog);

    for index in 0..players {
        world
            .spawn(Entity::player(HERO, 10.0 + index as f32, 10.0, 100))
            .expect("room for a player");
    }

    let mut boss = Entity::fixture(ORYX, 10.0, 10.0);
    boss.kind = Kind::Enemy;
    boss.max_hp = 1000;
    boss.hp = 1000;
    let boss = world.spawn(boss).expect("room for a boss");

    let (programs, diagnostics) = hendra_behavior::compile::compile(
        &hendra_behavior::parse::parse(behaviour).expect("parses"),
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    world.set_behaviours(catalog, programs);
    world.reindex();

    world.advance(catalog, 50);
    world.get(boss).expect("the boss").max_hp
}

#[test]
fn a_boss_that_names_no_cap_scales_without_one() {
    // Nought means "no limit", not "no scaling". Compiling it as a ceiling of nought clamped the
    // growth to nothing, so Oryx — whose only behaviour here is `ScaleHP(50000)` — did not gain a
    // single point of health however many players walked in.
    let catalog = catalog();
    let behaviour = r#"enemy "Oryx" { state a { scale_h_p(50000) } }"#;

    let alone = scaled_to(&catalog, behaviour, 1);
    assert_eq!(alone, 1000 + 50_000, "one player is already worth a share");

    let crowd = scaled_to(&catalog, behaviour, 4);
    assert_eq!(crowd, 1000 + 4 * 50_000, "and four are worth four");
}

#[test]
fn the_first_player_already_counts() {
    // `(plrCount - initialScaleAmount) * amountPerPlayer` where `initialScaleAmount` starts at
    // `scaleAfter` (`ScaleHP.cs:71-74`), and `scaleAfter` defaults to nought and is passed by no
    // call in the corpus. Counting players after the first implements a `scaleAfter` of one that
    // nothing asked for, and cost the boss a whole player's worth of health at every size.
    let catalog = catalog();
    let behaviour = r#"enemy "Oryx" { state a { scale_h_p(100) } }"#;

    assert_eq!(scaled_to(&catalog, behaviour, 1), 1100);
    assert_eq!(scaled_to(&catalog, behaviour, 2), 1200);
    assert_eq!(scaled_to(&catalog, behaviour, 3), 1300);
}

#[test]
fn a_cap_written_out_loud_is_still_a_cap() {
    // The other half of the same line: a non-zero `maxAdditional` takes the branch and bounds the
    // total at `MaximumHP + maxAdditional` (`ScaleHP.cs:93-96`). Nothing in the corpus writes one,
    // so this is the rule holding rather than the content exercising it.
    let catalog = catalog();
    let behaviour = r#"enemy "Oryx" { state a { scale_h_p(100, 250) } }"#;

    assert_eq!(scaled_to(&catalog, behaviour, 2), 1200, "under the cap");
    assert_eq!(scaled_to(&catalog, behaviour, 8), 1250, "and held at it");
}
