//! What an enemy leaves behind, against `wServer/logic/loot/Loots.cs`.
//!
//! The loot rules are almost all arithmetic on numbers nobody sees directly — a threshold, a chance
//! divided between a family of items, a count of eight. They are worth pinning here rather than
//! only in a live run because the observable difference between two readings of them is often a bag
//! that appears one time in twenty, which no amount of walking around a dungeon will settle.

use hendra_content::map::{Composition, Map};
use hendra_content::{Catalog, ObjectType, Region, TileType};
use hendra_sim::world::Entity;
use hendra_sim::{Handle, Kind, Terrain, World};

const FIXTURE: &str = r#"<Objects>
    <Ground type="0x10" id="Grass"/>

    <Object type="0x502" id="Slime"><Class>Character</Class><Enemy/>
      <MaxHitPoints>1000</MaxHitPoints></Object>
    <Object type="0x503" id="Troll"><Class>Character</Class><Enemy/><TrollWhiteBag/>
      <MaxHitPoints>1000</MaxHitPoints></Object>
    <Object type="0x600" id="Hero"><Class>Player</Class><Player/></Object>

    <Object type="0x600a" id="Bag Brown"><Class>Container</Class></Object>
    <Object type="0x600b" id="Bag 1"><Class>Container</Class></Object>
    <Object type="0x600c" id="Bag 2"><Class>Container</Class></Object>
    <Object type="0x600d" id="Bag 3"><Class>Container</Class></Object>
    <Object type="0x600e" id="Bag 4"><Class>Container</Class></Object>
    <Object type="0x600f" id="Bag 5"><Class>Container</Class></Object>
    <Object type="0x6010" id="Bag White"><Class>Container</Class></Object>
    <Object type="0x6011" id="Bag White 2"><Class>Container</Class></Object>
    <Object type="0x6012" id="Bag Troll"><Class>Container</Class></Object>
    <Object type="0x6013" id="Bag Boost"><Class>Container</Class></Object>

    <Object type="0x904" id="Rare Blade">
      <Class>Equipment</Class><Item/><SlotType>1</SlotType><BagType>6</BagType>
    </Object>
    <Object type="0x905" id="Plain Blade">
      <Class>Equipment</Class><Item/><SlotType>1</SlotType><BagType>0</BagType>
    </Object>

    <!-- Tier one, one of each slot type in the weapon family and one outside it. -->
    <Object type="0x910" id="Short Sword">
      <Class>Equipment</Class><Item/><SlotType>1</SlotType><Tier>1</Tier>
    </Object>
    <Object type="0x911" id="Dagger">
      <Class>Equipment</Class><Item/><SlotType>2</SlotType><Tier>1</Tier>
    </Object>
    <Object type="0x912" id="Bow">
      <Class>Equipment</Class><Item/><SlotType>3</SlotType><Tier>1</Tier>
    </Object>
    <Object type="0x913" id="Wand">
      <Class>Equipment</Class><Item/><SlotType>8</SlotType><Tier>1</Tier>
    </Object>
    <Object type="0x914" id="Leather Armor">
      <Class>Equipment</Class><Item/><SlotType>6</SlotType><Tier>1</Tier>
    </Object>

    <!-- Tier one potions, which is what the world's own loot table names. -->
    <Object type="0xa22" id="Health Potion">
      <Class>Equipment</Class><Item/><SlotType>10</SlotType><Tier>1</Tier><Potion/>
    </Object>
    <Object type="0xa23" id="Magic Potion">
      <Class>Equipment</Class><Item/><SlotType>10</SlotType><Tier>1</Tier><Potion/>
    </Object>
  </Objects>"#;

const SLIME: ObjectType = ObjectType(0x502);
const HERO: ObjectType = ObjectType(0x600);

/// The nine loot colours, then the red bag a loot-drop boost produces.
const BAGS: [u16; 10] = [
    0x600a, 0x600b, 0x600c, 0x600d, 0x600e, 0x600f, 0x6010, 0x6011, 0x6012, 0x6013,
];

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
    let mut world = World::new("Field", Terrain::build(map, catalog), catalog);
    world.set_bag_types(BAGS.iter().map(|kind| ObjectType(*kind)).collect());
    world
}

fn behaving(world: &mut World, catalog: &Catalog, source: &str) {
    let (programs, diagnostics) =
        hendra_behavior::compile::compile(&hendra_behavior::parse::parse(source).expect("parses"));
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    world.set_behaviours(catalog, programs);
}

/// Kills an enemy with the given loot table and returns every bag it left.
///
/// `damage` is who hit it and for how much, which is all the private half of the table looks at.
fn loot_from(
    catalog: &Catalog,
    table: &str,
    damage: &[(Handle, i32)],
    boosted: &[Handle],
) -> Vec<(ObjectType, Option<Handle>, u16, f32, f32, Vec<ObjectType>)> {
    loot_from_seed(catalog, table, damage, boosted, 1)
}

/// The same, from a named seed, for the tests that measure how often something happens.
fn loot_from_seed(
    catalog: &Catalog,
    table: &str,
    damage: &[(Handle, i32)],
    boosted: &[Handle],
    seed: u32,
) -> Vec<(ObjectType, Option<Handle>, u16, f32, f32, Vec<ObjectType>)> {
    loot_from_lucky(catalog, table, damage, boosted, &[], seed)
}

/// The same, with a luck stat worn by some of the players.
///
/// `luckStatBoost` is `1 + Stats.Boost[10] / 100` (`Loots.cs:117`) and multiplies the account's own
/// loot-drop boost rather than replacing it, so the two have to be exercised together.
fn loot_from_lucky(
    catalog: &Catalog,
    table: &str,
    damage: &[(Handle, i32)],
    boosted: &[Handle],
    luck: &[(Handle, i32)],
    seed: u32,
) -> Vec<(ObjectType, Option<Handle>, u16, f32, f32, Vec<ObjectType>)> {
    let mut world = field(catalog);
    world.reseed(seed.wrapping_mul(2_654_435_761).wrapping_add(1));

    // Players first, so the handles the caller was given line up. They are made by `bystanders`.
    for _ in 0..damage.len() {
        world
            .spawn(Entity::player(HERO, 1.0, 1.0, 100))
            .expect("room for a player");
    }
    for who in boosted {
        world.get_mut(*who).expect("a player").loot_drop = 1.5;
    }
    for (who, amount) in luck {
        let entity = world.get_mut(*who).expect("a player");
        let mut worn = [0i32; hendra_content::STAT_COUNT];
        worn[hendra_content::Stat::Luck.index()] = *amount;
        entity.stats.set_equipment(worn);
    }

    let mut enemy = Entity::fixture(SLIME, 10.0, 10.0);
    enemy.kind = Kind::Enemy;
    enemy.max_hp = 1000;
    enemy.hp = 1000;
    let enemy = world.spawn(enemy).expect("room for an enemy");

    behaving(
        &mut world,
        catalog,
        &format!("enemy \"Slime\" {{ state a {{ }} loot {{ {table} }} }}"),
    );

    {
        let enemy = world.get_mut(enemy).expect("the enemy");
        enemy.damage_by = damage.to_vec();
        enemy.dead = true;
    }
    world.advance(catalog, 50);

    world
        .iter()
        .filter(|(_, entity)| entity.container.is_some() && entity.kind == Kind::Container)
        .map(|(_, entity)| {
            let held: Vec<ObjectType> = entity
                .container
                .as_ref()
                .expect("a container")
                .iter()
                .map(|(_, item)| item)
                .filter(|item| !item.is_none())
                .collect();
            (
                entity.object_type,
                entity.belongs_to,
                entity.size,
                entity.x,
                entity.y,
                held,
            )
        })
        .collect()
}

/// Makes `count` players in a throwaway world only so their handles can be named in a damage list.
///
/// Handles are allocated in spawn order, so the first player spawned in `loot_from` is the first
/// one here too.
fn bystanders(catalog: &Catalog, count: usize) -> Vec<Handle> {
    let mut world = field(catalog);
    (0..count)
        .map(|_| {
            world
                .spawn(Entity::player(HERO, 1.0, 1.0, 100))
                .expect("room for a player")
        })
        .collect()
}

#[test]
fn a_tier_entry_only_drops_something_from_its_family() {
    // `TierLoot`'s weapon family is six slot types, not one (`logic/loot/MobDrops.cs:66`). Reading
    // it as a single slot means `tier(1, weapon)` can only ever drop a sword, and every dagger, bow
    // and wand in the game becomes unobtainable from a tier drop.
    let catalog = catalog();

    let mut seen = std::collections::HashSet::new();
    for seed in 1..40 {
        for bag in loot_from_seed(&catalog, "tier(1, weapon, 1.0)", &[], &[], seed) {
            seen.extend(bag.5);
        }
    }

    // A sword, a dagger, a bow and a wand are all weapons. Armour is not.
    for weapon in [0x910, 0x911, 0x912, 0x913] {
        assert!(
            seen.contains(&ObjectType(weapon)),
            "{weapon:#x} never dropped from a weapon tier"
        );
    }
    assert!(
        !seen.contains(&ObjectType(0x914)),
        "armour dropped from a weapon tier"
    );
}

#[test]
fn a_potion_tier_means_slot_type_ten() {
    // The world's own table is `TierLoot(1, ItemType.Potion, .03)` and `PotionT` is `{10}`. Reading
    // it as slot type zero picks from whatever happens to have no slot type at all, which is where
    // every potion a player ever finds outside a boss comes from.
    let catalog = catalog();

    let mut seen = std::collections::HashSet::new();
    for seed in 1..40 {
        for bag in loot_from_seed(&catalog, "tier(1, potion, 1.0)", &[], &[], seed) {
            seen.extend(bag.5);
        }
    }

    assert_eq!(
        seen,
        [ObjectType(0xa22), ObjectType(0xa23)].into_iter().collect(),
        "only the two tier-one potions"
    );
}

#[test]
fn a_tier_entrys_chance_is_shared_between_the_items_in_it() {
    // `probability / items.Length` per item, each rolled on its own (`MobDrops.cs:100`). So a tier
    // entry at full chance across two items is two coin flips rather than one certainty: it can
    // drop both, one, or neither. Rolling once and then picking would make it always exactly one.
    let catalog = catalog();

    let mut counts = std::collections::HashMap::new();
    for seed in 1..200 {
        let dropped: usize = loot_from_seed(&catalog, "tier(1, potion, 1.0)", &[], &[], seed)
            .iter()
            .map(|bag| bag.5.len())
            .sum();
        *counts.entry(dropped).or_insert(0usize) += 1;
    }

    assert!(
        counts.get(&0).copied().unwrap_or(0) > 0,
        "sometimes neither"
    );
    assert!(counts.get(&2).copied().unwrap_or(0) > 0, "sometimes both");
}

#[test]
fn private_loot_is_earned_by_damage_rather_than_by_a_share_of_health() {
    // The disputed one, and it decides whether soulbound loot drops at all. `GetPlayerData` returns
    // `hitters[player]`, which `HitBy` accumulates in hit points (`DamageCounter.cs:39,55`), and
    // `Loots.cs:122` compares `i.Threshold` to it directly. Every threshold in the content is below
    // one, so a single point of damage earns a player their roll on a thousand-health enemy. Read
    // as a fraction of health instead, a threshold of a half would need five hundred.
    let catalog = catalog();
    let [player] = bystanders(&catalog, 1)[..] else {
        unreachable!()
    };

    let bags = loot_from(
        &catalog,
        r#"threshold(0.5) { item("Plain Blade", 1.0) }"#,
        &[(player, 1)],
        &[],
    );

    assert_eq!(bags.len(), 1, "one bag, and it is theirs");
    assert_eq!(bags[0].1, Some(player), "owned by whoever earned it");
    assert_eq!(bags[0].5, vec![ObjectType(0x905)]);
}

#[test]
fn a_player_who_never_touched_it_earns_nothing() {
    // The other half of the rule: a threshold above zero is private loot, and somebody with no
    // damage recorded against the enemy is not in `GetPlayerData` at all.
    let catalog = catalog();
    let [_watching] = bystanders(&catalog, 1)[..] else {
        unreachable!()
    };

    let bags = loot_from(
        &catalog,
        r#"threshold(0.5) { item("Plain Blade", 1.0) }"#,
        &[],
        &[],
    );

    assert!(bags.is_empty(), "nothing dropped for a spectator");
}

#[test]
fn loot_past_the_eighth_item_starts_a_second_bag() {
    // `ShowBags` fills an eight-slot array and starts a fresh bag on the ninth (`Loots.cs:333-343`).
    // Dropping everything into one bag silently loses the ninth item onward, which on a boss with a
    // long guaranteed table is most of the drop.
    let catalog = catalog();

    let table: String = (0..12)
        .map(|_| "item(\"Plain Blade\", 1.0) ")
        .collect::<String>();
    let bags = loot_from(&catalog, &table, &[], &[]);

    let held: usize = bags.iter().map(|bag| bag.5.len()).sum();
    assert_eq!(held, 12, "every item is somewhere");
    assert_eq!(bags.len(), 2, "in two bags");
    assert!(bags.iter().any(|bag| bag.5.len() == 8), "the first is full");
    assert!(bags.iter().any(|bag| bag.5.len() == 4), "the second is not");
}

#[test]
fn each_bag_takes_the_colour_of_what_is_in_it() {
    // `bagType` is reset to zero after every eighth item (`Loots.cs:340`), so a white item in the
    // first bag does not make the second one white. Reading the colour once over the whole drop
    // would paint a bag of nine potions white because a sword landed in the other one.
    let catalog = catalog();

    let mut table = String::from("item(\"Rare Blade\", 1.0) ");
    table.push_str(&"item(\"Plain Blade\", 1.0) ".repeat(11));
    let bags = loot_from(&catalog, &table, &[], &[]);

    assert_eq!(bags.len(), 2);
    let colours: std::collections::HashSet<ObjectType> = bags.iter().map(|bag| bag.0).collect();
    assert!(
        colours.contains(&ObjectType(BAGS[6])),
        "the bag holding the rare blade is white"
    );
    assert!(
        colours.contains(&ObjectType(BAGS[0])),
        "and the one holding the rest is brown"
    );
}

#[test]
fn a_rarer_bag_is_drawn_larger() {
    // `SetDefaultSize(bagType > 3 ? 120 : 80)`. It is half of how a player reads a pile of bags from
    // across a room, the colour being the other half.
    let catalog = catalog();

    let plain = loot_from(&catalog, r#"item("Plain Blade", 1.0)"#, &[], &[]);
    assert_eq!(plain[0].2, 80);

    let rare = loot_from(&catalog, r#"item("Rare Blade", 1.0)"#, &[], &[]);
    assert_eq!(rare[0].2, 120);
}

#[test]
fn a_loot_boosted_players_own_bag_turns_red() {
    // `boosted && bag < WHITE_BAG` (`Loots.cs:368`), and boosted means a sole owner with the boost
    // running. It is the only feedback a player has that the boost they paid for is doing anything.
    let catalog = catalog();
    let [player] = bystanders(&catalog, 1)[..] else {
        unreachable!()
    };

    let bags = loot_from(
        &catalog,
        r#"threshold(0.1) { item("Plain Blade", 1.0) }"#,
        &[(player, 5)],
        &[player],
    );

    assert_eq!(bags.len(), 1);
    assert_eq!(bags[0].0, ObjectType(BAGS[9]), "the red bag");
}

#[test]
fn a_white_drop_beats_the_boosted_colour() {
    // The same line the other way round: what is in the bag matters more than why it dropped.
    let catalog = catalog();
    let [player] = bystanders(&catalog, 1)[..] else {
        unreachable!()
    };

    let bags = loot_from(
        &catalog,
        r#"threshold(0.1) { item("Rare Blade", 1.0) }"#,
        &[(player, 5)],
        &[player],
    );

    assert_eq!(bags[0].0, ObjectType(BAGS[6]), "still white");
}

#[test]
fn the_shared_bag_is_not_boosted() {
    // `owners.Count() == 1` gates it, and the shared bag has no owners at all.
    let catalog = catalog();
    let [player] = bystanders(&catalog, 1)[..] else {
        unreachable!()
    };

    let bags = loot_from(
        &catalog,
        r#"item("Plain Blade", 1.0)"#,
        &[(player, 5)],
        &[player],
    );

    assert_eq!(bags.len(), 1);
    assert_eq!(bags[0].1, None, "anybody may take it");
    assert_eq!(bags[0].0, ObjectType(BAGS[0]), "and it is brown");
}

#[test]
fn a_required_drop_appears_even_when_the_roll_says_no() {
    // `NumRequired` is a floor: `Loots.cs:139-143` forces out whatever the random pass did not
    // produce. An entry at no chance at all and one required always drops exactly one.
    let catalog = catalog();

    let bags = loot_from(
        &catalog,
        r#"item("Plain Blade", 0.0, num_required: 1)"#,
        &[],
        &[],
    );

    assert_eq!(bags.len(), 1);
    assert_eq!(bags[0].5, vec![ObjectType(0x905)]);
}

#[test]
fn a_required_private_drop_goes_to_one_eligible_player_each() {
    // `Loots.cs:154-168`: the shortfall is handed to eligible players picked at random and removed
    // from the pool as they are picked, so two required go to two different people rather than both
    // to whoever the loop happened to reach first.
    let catalog = catalog();
    let players = bystanders(&catalog, 3);

    let bags = loot_from(
        &catalog,
        r#"threshold(0.1) { item("Plain Blade", 0.0, num_required: 2) }"#,
        &players.iter().map(|who| (*who, 5)).collect::<Vec<_>>(),
        &[],
    );

    assert_eq!(bags.len(), 2, "two bags for two required drops");
    let owners: std::collections::HashSet<Option<Handle>> = bags.iter().map(|bag| bag.1).collect();
    assert_eq!(owners.len(), 2, "and they went to different players");
    assert!(owners.iter().all(|owner| owner.is_some()));
}

#[test]
fn a_bag_lands_beside_the_enemy_rather_than_on_it() {
    // `enemy.X + (Rand.NextDouble() * 2 - 1) * 0.5` on each axis separately (`Loots.cs:379-381`).
    // One roll shared between both axes would put every bag on the same diagonal, which is visible
    // as soon as two drop at once.
    let catalog = catalog();

    let mut offsets = Vec::new();
    for seed in 1..30 {
        for bag in loot_from_seed(&catalog, r#"item("Plain Blade", 1.0)"#, &[], &[], seed) {
            offsets.push((bag.3 - 10.0, bag.4 - 10.0));
        }
    }

    assert!(
        offsets
            .iter()
            .all(|(dx, dy)| dx.abs() <= 0.5 && dy.abs() <= 0.5),
        "within half a tile"
    );
    assert!(
        offsets.iter().any(|(dx, dy)| (dx - dy).abs() > 0.05),
        "and the two axes are rolled apart"
    );
}

#[test]
fn every_enemy_carries_the_worlds_three_per_cent_potion() {
    // `World.WorldLoot`, merged into every enemy's table (`Loots.cs:91-97`). It is where a player's
    // potions come from when they are not farming one boss.
    let catalog = catalog();

    let mut potions = 0;
    let runs = 400u32;
    for seed in 1..=runs {
        for bag in loot_from_seed(&catalog, r#"item("Plain Blade", 0.0)"#, &[], &[], seed) {
            potions += bag
                .5
                .iter()
                .filter(|item| **item == ObjectType(0xa22) || **item == ObjectType(0xa23))
                .count();
        }
    }

    // Three per cent split between two potions, so about twelve in four hundred. Loose bounds:
    // the point is that it happens at all and does not happen every time.
    assert!(
        (1..(runs as usize) / 4).contains(&potions),
        "{potions} potions in {runs} kills"
    );
}

#[test]
fn a_bag_tells_the_client_what_is_in_it() {
    // `Container.ExportStats` writes the eight slots into the entity's stats
    // (`realm/entities/Container.cs:76-90`). Without them the client draws the bag but nothing
    // inside, and a player standing on a white bag sees an empty panel.
    let catalog = catalog();
    let bags = loot_from(&catalog, r#"item("Rare Blade", 1.0)"#, &[], &[]);
    assert_eq!(bags.len(), 1);

    let mut world = field(&catalog);
    let mut bag = Entity::fixture(ObjectType(BAGS[0]), 5.0, 5.0);
    bag.kind = Kind::Container;
    let mut held = hendra_sim::Container::new(hendra_sim::ContainerKind::Bag, 8);
    held.set(0, ObjectType(0x904));
    held.set(3, ObjectType(0x905));
    bag.container = Some(Box::new(held));
    let handle = world.spawn(bag).expect("room for a bag");

    let state = world.get(handle).expect("the bag").state();
    let contents = state.contents.expect("a container says what it holds");
    assert_eq!(contents[0], 0x904);
    assert_eq!(
        contents[1],
        hendra_net::NO_ITEM,
        "an empty slot is not item zero"
    );
    assert_eq!(contents[3], 0x905);
}

#[test]
fn something_that_is_not_a_container_says_nothing() {
    // A field nothing sets costs nothing: the mask bit is never raised and the slots never travel.
    let catalog = catalog();
    let mut world = field(&catalog);
    let player = world
        .spawn(Entity::player(HERO, 5.0, 5.0, 100))
        .expect("room for a player");

    assert_eq!(
        world.get(player).expect("the player").state().contents,
        None
    );
}

/// A dungeon key, which is an ordinary item like any other.
const KEY: &str = r#"<Objects>
    <Object type="0x9f0" id="Lost Halls Key">
      <Class>Equipment</Class><Item/><SlotType>10</SlotType>
    </Object>
  </Objects>"#;

#[test]
fn a_key_falling_into_a_bag_is_not_announced() {
    // `Loots.HandleLoot` builds bags and puts them in the world and says nothing about any of it;
    // every announcement the game makes comes from a behaviour instead -- `AnnounceOnDeath`
    // (`logic/behaviors/AnnounceOnDeath.cs:44`) and `Taunt`.
    //
    // The four keys a locker's doors need never come out of a loot table at all. "Purple Key" and
    // its three siblings are `Character` objects standing in the room
    // (`XmlDatas/xmls/client/EmbeddedData_GhostShipCXML.dat:194-205`), and what announces one is
    // the `Taunt(true, "Purple Key has been found!")` each of them runs when a player walks into
    // it (`logic/db/BehaviorDb.DavyJones.cs:127-142`). So the forty-odd dungeon key *items* --
    // "Lost Halls Key", "Shatters Key" and the rest -- drop in silence, and telling the whole
    // world about one is telling it something the original never says.
    let catalog = Catalog::load_str(&[FIXTURE, KEY]).0;
    let mut world = field(&catalog);

    let mut enemy = Entity::fixture(SLIME, 10.0, 10.0);
    enemy.kind = Kind::Enemy;
    enemy.max_hp = 1000;
    enemy.hp = 1000;
    let enemy = world.spawn(enemy).expect("room for an enemy");

    behaving(
        &mut world,
        &catalog,
        r#"enemy "Slime" { state a { } loot { item("Lost Halls Key", 1.0) } }"#,
    );

    world.get_mut(enemy).expect("the enemy").dead = true;
    world.advance(&catalog, 50);

    let dropped: Vec<ObjectType> = world
        .iter()
        .filter(|(_, entity)| entity.kind == Kind::Container)
        .filter_map(|(_, entity)| entity.container.as_ref())
        .flat_map(|held| held.iter().map(|(_, item)| item))
        .filter(|item| !item.is_none())
        .collect();
    assert_eq!(
        dropped,
        vec![ObjectType(0x9f0)],
        "the key is in a bag, which is the whole of what happens"
    );

    let said: Vec<String> = world
        .take_announcements()
        .into_iter()
        .map(|announcement| announcement.text.to_string())
        .collect();
    assert!(said.is_empty(), "the world said {said:?}");
}

#[test]
fn an_enemy_with_no_table_of_its_own_leaves_nothing() {
    // The whole loot handler hangs off the enemy's own drops. `BehaviorDb.Init` attaches it only
    // `if (defs.Length > 0)` (`BehaviorDb.cs:81-88`), so an enemy declared with no `MobDrops` never
    // reaches `Loots.Handle` and never picks up the world's three per cent potion either.
    //
    // Four hundred and thirty-six of the seven hundred and forty-six converted enemies are declared
    // that way — every wandering slime, every summoned minion, thirty of them roaming the open
    // realm — and merging the world table into all of them turned the realm into a potion farm.
    let catalog = catalog();

    let mut bags = 0;
    for seed in 1..=400 {
        bags += loot_from_seed(&catalog, "", &[], &[], seed).len();
    }

    assert_eq!(bags, 0, "{bags} bags from four hundred kills");

    // The same enemy with a table of its own, however hopeless, is a different enemy: the handler is
    // attached, so the world potion is merged in and does turn up.
    let mut potions = 0;
    for seed in 1..=400 {
        for bag in loot_from_seed(&catalog, r#"item("Plain Blade", 0.0)"#, &[], &[], seed) {
            potions += bag.5.len();
        }
    }
    assert!(
        potions > 0,
        "a declared table should still take the world's"
    );
}

#[test]
fn a_boosted_bag_is_drawn_at_the_size_its_contents_ask_for() {
    // `SetDefaultSize(bagType > 3 ? 120 : 80)` (`Loots.cs:382`) reads the colour the *items* asked
    // for. The boost above it recolours the local `bag` (`Loots.cs:368-371`) and never touches that
    // parameter, so a boosted brown bag is red and still small.
    let catalog = catalog();
    let [player] = bystanders(&catalog, 1)[..] else {
        unreachable!()
    };

    let bags = loot_from(
        &catalog,
        r#"threshold(0.1) { item("Plain Blade", 1.0) }"#,
        &[(player, 5)],
        &[player],
    );

    assert_eq!(bags.len(), 1);
    assert_eq!(bags[0].0, ObjectType(BAGS[9]), "recoloured to red");
    assert_eq!(bags[0].2, 80, "and still the size a brown bag is drawn at");
}

#[test]
fn a_damager_who_has_left_does_not_take_another_players_eligibility() {
    // The required private pass picks from the players who are still here, and has to test each
    // one's own damage. `GetPlayerData` (`DamageCounter.cs:47-58`) drops the departed before
    // `Loots.Handle` ever sees them, so the original's eligible list and its loot list are the same
    // list; building them separately here and then indexing one with the other's positions reads
    // somebody else's damage as soon as anybody has left the world.
    let catalog = catalog();

    let mut world = field(&catalog);
    world.reseed(7);

    let ghost = world
        .spawn(Entity::player(HERO, 1.0, 1.0, 100))
        .expect("room");
    let first = world
        .spawn(Entity::player(HERO, 2.0, 1.0, 100))
        .expect("room");
    let second = world
        .spawn(Entity::player(HERO, 3.0, 1.0, 100))
        .expect("room");

    let mut enemy = Entity::fixture(SLIME, 10.0, 10.0);
    enemy.kind = Kind::Enemy;
    enemy.max_hp = 1000;
    enemy.hp = 1000;
    let enemy = world.spawn(enemy).expect("room");

    behaving(
        &mut world,
        &catalog,
        // No chance at all and two required, so the random pass produces nothing and the whole drop
        // comes out of the required pass. Threshold five, which the departed player is under and
        // both survivors are over.
        r#"enemy "Slime" { state a { } loot { threshold(5) { item("Plain Blade", 0.0, 2) } } }"#,
    );

    // The one who left is still remembered as a damager, which is how the two lists come apart.
    world.despawn(ghost);
    {
        let enemy = world.get_mut(enemy).expect("the enemy");
        enemy.damage_by = vec![(ghost, 0), (first, 10), (second, 10)];
        enemy.dead = true;
    }
    world.advance(&catalog, 50);

    let owners: Vec<Option<Handle>> = world
        .iter()
        .filter(|(_, entity)| entity.kind == Kind::Container)
        .map(|(_, entity)| entity.belongs_to)
        .collect();

    assert_eq!(owners.len(), 2, "one bag each, for the two who stayed");
    assert!(owners.contains(&Some(first)), "the first earned one");
    assert!(owners.contains(&Some(second)), "and so did the second");
}

/// The plain blade, which is what the luck tests watch for.
///
/// They watch for the item rather than for a bag, because an enemy with a table of its own also
/// carries the world's three per cent potion, and a public potion bag alongside the private one
/// would make a bag count say nothing about the roll being measured.
const PLAIN_BLADE: ObjectType = ObjectType(0x905);

fn earned_the_blade(
    bags: &[(ObjectType, Option<Handle>, u16, f32, f32, Vec<ObjectType>)],
    who: Handle,
) -> bool {
    bags.iter()
        .filter(|bag| bag.1 == Some(who))
        .any(|bag| bag.5.contains(&PLAIN_BLADE))
}

#[test]
fn a_loot_boost_makes_earned_loot_likelier() {
    // `Rand.NextDouble() < i.Probabilty * lootDropBoost` (`Loots.cs:123`), where `lootDropBoost` is
    // 1.5 while an account's boost is running. A multiplier rather than a bonus, as the original has
    // it: a boost is worth more on something that already drops often, which is what makes it worth
    // having on a run rather than on one kill.
    //
    // Decisive rather than statistical, because comparing two runs of a random draw can agree by
    // luck and a test that can pass by luck passes with the multiplier taken out as well. Seven
    // tenths boosted by a half is over one, so it is a certainty; unboosted it is not.
    let catalog = catalog();
    let [player] = bystanders(&catalog, 1)[..] else {
        unreachable!()
    };

    let table = r#"threshold(1) { item("Plain Blade", 0.7) }"#;

    for seed in 1..=120 {
        let bags = loot_from_seed(&catalog, table, &[(player, 5)], &[player], seed);
        assert!(
            earned_the_blade(&bags, player),
            "a boosted seven-tenths missed on seed {seed}"
        );
    }

    let missed = (1..=120u32)
        .filter(|seed| {
            !earned_the_blade(
                &loot_from_seed(&catalog, table, &[(player, 5)], &[], *seed),
                player,
            )
        })
        .count();
    assert!(missed > 0, "an unboosted seven-tenths never missed");
}

#[test]
fn the_luck_stat_and_a_loot_boost_multiply_each_other() {
    // `i.Probabilty * lootDropBoost * luckStatBoost` (`Loots.cs:123`): the two are separate factors
    // on the same product, so an account boost and a lucky ring are worth having at once rather
    // than one replacing the other. A half boost and a hundred luck come to three, and a third of a
    // chance tripled is a certainty.
    let catalog = catalog();
    let [player] = bystanders(&catalog, 1)[..] else {
        unreachable!()
    };

    let table = r#"threshold(1) { item("Plain Blade", 0.34) }"#;

    for seed in 1..=120 {
        let bags = loot_from_lucky(
            &catalog,
            table,
            &[(player, 5)],
            &[player],
            &[(player, 100)],
            seed,
        );
        assert!(
            earned_the_blade(&bags, player),
            "a tripled third missed on seed {seed}"
        );
    }

    // The boost alone is not enough, which is what says the luck stat is doing something.
    let missed = (1..=120u32)
        .filter(|seed| {
            !earned_the_blade(
                &loot_from_seed(&catalog, table, &[(player, 5)], &[player], *seed),
                player,
            )
        })
        .count();
    assert!(missed > 0, "a third boosted only by a half never missed");
}

#[test]
fn a_boost_cannot_make_something_drop_that_never_drops() {
    // A multiplier on nothing is nothing, which is what keeps a boost from turning a table entry
    // written at no chance into a certainty.
    let catalog = catalog();
    let [player] = bystanders(&catalog, 1)[..] else {
        unreachable!()
    };

    for seed in 1..=120 {
        let bags = loot_from_lucky(
            &catalog,
            r#"threshold(1) { item("Plain Blade", 0.0) }"#,
            &[(player, 5)],
            &[player],
            &[(player, 900)],
            seed,
        );
        assert!(
            !earned_the_blade(&bags, player),
            "no chance dropped something on seed {seed}"
        );
    }
}

/// Kills a troll — an enemy whose descriptor carries `<TrollWhiteBag/>` — and returns its bags.
fn troll_loot(catalog: &Catalog, table: &str, items: usize) -> Vec<(ObjectType, u16)> {
    let mut world = field(catalog);
    world.reseed(11);

    let mut enemy = Entity::fixture(ObjectType(0x503), 10.0, 10.0);
    enemy.kind = Kind::Enemy;
    enemy.max_hp = 1000;
    enemy.hp = 1000;
    let enemy = world.spawn(enemy).expect("room for an enemy");

    let (programs, diagnostics) = hendra_behavior::compile::compile(
        &hendra_behavior::parse::parse(&format!(
            "enemy \"Troll\" {{ state a {{ }} loot {{ {table} }} }}"
        ))
        .expect("parses"),
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    world.set_behaviours(catalog, programs);

    world.get_mut(enemy).expect("the enemy").dead = true;
    world.advance(catalog, 50);

    let mut bags: Vec<(ObjectType, u16)> = world
        .iter()
        .filter(|(_, entity)| entity.kind == Kind::Container)
        .map(|(_, entity)| {
            let held = entity
                .container
                .as_ref()
                .expect("a container")
                .iter()
                .filter(|(_, item)| !item.is_none())
                .count();
            assert!(held <= 8);
            (entity.object_type, entity.size)
        })
        .collect();
    let _ = items;
    bags.sort_by_key(|bag| bag.0.0);
    bags
}

#[test]
fn a_troll_drops_its_loot_in_a_trolls_bag() {
    // `if (enemy.ObjectDesc.TrollWhiteBag) bagType = 8;` (`Loots.cs:311`), set before the loop that
    // raises the colour to whatever the items ask for. So the flag is a floor on the colour, and a
    // plain blade out of a troll still leaves the troll's bag.
    let catalog = catalog();

    let bags = troll_loot(&catalog, r#"item("Plain Blade", 1.0)"#, 1);
    assert!(
        bags.iter().any(|bag| bag.0 == ObjectType(BAGS[8])),
        "no troll bag among {bags:?}"
    );
}

#[test]
fn a_trolls_bag_is_a_floor_on_the_colour_rather_than_the_colour() {
    // The loop still raises it: `if (i.BagType > bagType) bagType = i.BagType`. Eight is the troll
    // bag and the two whites are six and seven, so nothing an item can ask for beats it here — but
    // the size follows the same number, and a troll bag is one of the large ones.
    let catalog = catalog();

    let bags = troll_loot(&catalog, r#"item("Rare Blade", 1.0)"#, 1);
    let troll = bags
        .iter()
        .find(|bag| bag.0 == ObjectType(BAGS[8]))
        .expect("the troll bag");
    assert_eq!(troll.1, 120, "and it is drawn large");
}

#[test]
fn an_outer_threshold_overrides_the_one_written_inside_it() {
    // `Threshold(outer, Threshold(inner, ...))`. The inner constructor stamps its share onto its
    // children, then the outer calls `Populate` over those same children with its own share as the
    // override, which `MobDrops.Populate` applies whenever it is not negative (`MobDrops.cs:36`) —
    // and a share written in a script never is. So the outer wins.
    //
    // Read the other way round, a boss's rarest drop goes to whoever did the least damage. Pinned
    // with an outer share nobody can meet and an inner share everybody can: nothing should drop.
    let catalog = catalog();
    let [player] = bystanders(&catalog, 1)[..] else {
        unreachable!()
    };

    let bags = loot_from(
        &catalog,
        r#"threshold(9000) { threshold(1) { item("Plain Blade", 1.0) } }"#,
        &[(player, 5)],
        &[],
    );

    assert!(
        !earned_the_blade(&bags, player),
        "the inner share won, so five damage earned a nine-thousand-damage drop"
    );

    // And with the shares the other way about, the same player does earn it.
    let bags = loot_from(
        &catalog,
        r#"threshold(1) { threshold(9000) { item("Plain Blade", 1.0) } }"#,
        &[(player, 5)],
        &[],
    );
    assert!(
        earned_the_blade(&bags, player),
        "the outer share of one should have made this reachable"
    );
}

#[test]
fn an_entry_can_carry_its_own_threshold_without_a_block_around_it() {
    // `ItemLoot(item, probability, numRequired, threshold)` — the fourth argument
    // (`logic/loot/MobDrops.cs:47`). Nothing in the shipped corpus passes it, so this is the rule
    // holding rather than the content exercising it; it was not compiled at all before.
    let catalog = catalog();
    let [player] = bystanders(&catalog, 1)[..] else {
        unreachable!()
    };

    // Out of reach at five damage, so it belongs to nobody and reaches no bag.
    let bags = loot_from(
        &catalog,
        r#"item("Plain Blade", 1.0, 0, 9000)"#,
        &[(player, 5)],
        &[],
    );
    assert!(
        !bags.iter().any(|bag| bag.5.contains(&PLAIN_BLADE)),
        "an unreachable threshold still dropped it"
    );

    // Within reach, and it is theirs rather than everybody's.
    let bags = loot_from(
        &catalog,
        r#"item("Plain Blade", 1.0, 0, 1)"#,
        &[(player, 5)],
        &[],
    );
    assert!(earned_the_blade(&bags, player), "it should be theirs");
}
