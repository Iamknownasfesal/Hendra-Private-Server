//! What a setpiece chest holds, against `Loot.GetLoots` in `wServer/logic/loot/Loots.cs:47-68`.
//!
//! The rule that matters here is not how many items a chest holds on average — that was right all
//! along, which is exactly why it hid — but how they are distributed between the *lines* of the
//! chest's table. `Populate` flattens one `TierLoot(6, Weapon, 0.3)` into one `LootDef` per tier-six
//! weapon, each at `0.3 / count`, and `GetLoots` then rolls every one of them independently. So one
//! line can put two or three weapons in a chest, and the count it contributes is binomial rather
//! than a coin flip. Rolling once per line at the full chance gives the same mean and a ceiling of
//! one item per line, which no amount of counting items would ever have shown.

use hendra_content::map::{Composition, Map};
use hendra_content::{Catalog, ObjectType, Region, TileType};
use hendra_sim::setpiece::{Drawing, Placed, Tier};
use hendra_sim::{Kind, Terrain, World};

/// Ten tier-six weapons, so one table line flattens into ten independent rolls.
const WEAPONS: usize = 10;

fn fixture() -> String {
    let mut xml = String::from(
        r#"<Objects>
    <Ground type="0x10" id="Grass"/>
    <Object type="0x700" id="Treasure Chest"><Class>Container</Class></Object>
"#,
    );
    for index in 0..WEAPONS {
        xml.push_str(&format!(
            r#"<Object type="0x{:x}" id="Blade {index}"><Class>Equipment</Class><Item/>
               <SlotType>1</SlotType><Tier>6</Tier></Object>
"#,
            0x900 + index
        ));
    }
    xml.push_str("</Objects>");
    xml
}

/// One line of a chest's table, at nine in ten, spread over the ten weapons.
const TABLE: &[Tier] = &[Tier {
    tier: 6,
    kind: "weapon",
    chance: 0.9,
}];

/// The chance each flattened entry is rolled at, which is what the shape follows from.
const EACH: f64 = 0.9 / WEAPONS as f64;

const CHEST: &[Placed] = &[Placed::Chest {
    x: 4.0,
    y: 4.0,
    loot: TABLE,
    // Eight to nine, so `Rand.Next(min, max)` always yields eight and the count never clips what
    // one line can contribute.
    least: 8,
    most: 9,
}];

fn catalog() -> Catalog {
    Catalog::load_str(&[&fixture()]).0
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

/// How many items one chest ended up holding, over `runs` differently-seeded draws.
fn chest_sizes(catalog: &Catalog, runs: u32) -> Vec<usize> {
    let drawing = Drawing {
        squares: Vec::new(),
        placed: CHEST.to_vec(),
        ..Drawing::default()
    };

    (1..=runs)
        .map(|seed| {
            let mut world = field(catalog);
            world.reseed(seed.wrapping_mul(2_654_435_761).wrapping_add(1));
            world.draw(catalog, &drawing, (2, 2));

            world
                .iter()
                .filter(|(_, entity)| entity.kind == Kind::Container)
                .filter_map(|(_, entity)| entity.container.as_ref())
                .map(|held| held.iter().filter(|(_, item)| !item.is_none()).count())
                .sum()
        })
        .collect()
}

#[test]
fn one_line_of_a_chests_table_can_yield_more_than_one_item() {
    // The whole of the difference. `GetLoots` walks the flattened list, so a line spread over ten
    // items rolls ten times and can hit twice. Rolling the line once and taking one item caps it at
    // one, and a chest whose table is a single line could then never hold two of anything.
    let catalog = catalog();
    let sizes = chest_sizes(&catalog, 3_000);

    let most = sizes.iter().copied().max().unwrap_or(0);
    assert!(
        most >= 3,
        "one line never yielded more than {most} item(s) in three thousand chests"
    );
}

#[test]
fn the_count_one_line_yields_follows_the_binomial_its_flattening_implies() {
    // The shape, stated as the arithmetic it has to match rather than as "more than before".
    // Ten independent entries at `0.9 / 10` each is Binomial(10, 0.09): a mean of nine tenths, and
    // a little over a fifth of chests holding two or more.
    let catalog = catalog();
    let runs = 6_000;
    let sizes = chest_sizes(&catalog, runs);

    let mean = sizes.iter().sum::<usize>() as f64 / runs as f64;
    assert!(
        (mean - 0.9).abs() < 0.07,
        "mean of {mean}, expected about 0.9"
    );

    // `1 - (1-p)^n - n·p·(1-p)^(n-1)`.
    let none = (1.0 - EACH).powi(WEAPONS as i32);
    let one = WEAPONS as f64 * EACH * (1.0 - EACH).powi(WEAPONS as i32 - 1);
    let expected = 1.0 - none - one;

    let two_or_more = sizes.iter().filter(|held| **held >= 2).count() as f64 / runs as f64;
    assert!(
        (two_or_more - expected).abs() < 0.05,
        "{two_or_more} of chests held two or more, expected about {expected}"
    );
}

#[test]
fn the_mean_is_the_same_either_way_which_is_why_this_hid() {
    // Said out loud because it is the reason the defect survived every count anyone took: rolling
    // one line once at nine in ten and rolling ten tenth-chances both yield nine tenths of an item
    // per chest on average. Only the distribution tells them apart.
    let catalog = catalog();
    let runs = 6_000;
    let sizes = chest_sizes(&catalog, runs);

    let mean = sizes.iter().sum::<usize>() as f64 / runs as f64;
    let coin_flip_mean = 0.9;

    assert!(
        (mean - coin_flip_mean).abs() < 0.07,
        "the aggregate should not have moved: {mean} against {coin_flip_mean}"
    );
}
