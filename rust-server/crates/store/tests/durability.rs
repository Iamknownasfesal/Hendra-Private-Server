//! Tests against a real Postgres.
//!
//! These need a database, because the properties they check are properties of the database: row
//! locking, transaction isolation, and what two connections racing each other actually do. A mock
//! would test that the mock agrees with itself.
//!
//! Set `HENDRA_TEST_DATABASE` to point at one. Without it the tests skip rather than fail, so a
//! machine with no Postgres can still run the rest of the suite.

use hendra_store::{
    Admin, Currency, Death, DyeSlot, Location, MarketPurchase, Offer, Purchase, Rank, Store,
    StoreError,
};

/// A store with a schema of its own, or `None` when no database is configured.
///
/// Each test gets a distinct schema so they can run in parallel without seeing each other's rows;
/// sharing one would make every test depend on the order the others ran in.
async fn store(schema: &str) -> Option<Store> {
    let url = std::env::var("HENDRA_TEST_DATABASE").ok()?;

    // A fresh schema, used as the search path so the migration lands inside it.
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .ok()?;

    sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .execute(&admin)
        .await
        .ok()?;
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .ok()?;
    admin.close().await;

    let scoped = if url.contains('?') {
        format!("{url}&options=-csearch_path%3D{schema}")
    } else {
        format!("{url}?options=-csearch_path%3D{schema}")
    };

    Store::connect(&scoped).await.ok()
}

/// Two item identities, fixed so a test can name the same item twice.
///
/// Any two distinct UUIDs would do: the store never looks one up, it only moves it about and
/// refuses to let it exist twice.
const WAND: uuid::Uuid = uuid::Uuid::from_u128(0x900);
const ROBE: uuid::Uuid = uuid::Uuid::from_u128(0x901);

/// A class identity, for characters that only need to exist.
const WIZARD: uuid::Uuid = uuid::Uuid::from_u128(0x0300);

#[tokio::test(flavor = "multi_thread")]
async fn an_account_and_character_survive_a_round_trip() {
    let Some(store) = store("t_roundtrip").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    assert_eq!(account.vault_chests, 4, "everyone starts with four chests");

    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();
    assert_eq!(character.hp, 800);
    assert!(character.alive);

    let reloaded = store.character(character.id).await.unwrap();
    assert_eq!(reloaded, character);

    let listed = store.characters(account.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Wizard");
}

#[tokio::test(flavor = "multi_thread")]
async fn account_names_are_unique_regardless_of_case() {
    let Some(store) = store("t_names").await else {
        return;
    };

    store.create_account("Fesal").await.unwrap();

    // Decided by the unique index, not by a prior lookup: a check-then-insert has a window in which
    // someone else inserts the same name.
    assert!(matches!(
        store.create_account("fesal").await,
        Err(StoreError::NameTaken)
    ));
    assert!(matches!(
        store.create_account("FESAL").await,
        Err(StoreError::NameTaken)
    ));

    // And it is findable either way.
    assert_eq!(store.account_by_name("FeSaL").await.unwrap().name, "Fesal");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_item_moves_between_inventory_and_vault() {
    let Some(store) = store("t_move").await else {
        return;
    };

    let account = store.create_account("Mover").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    store
        .set_inventory(character.id, &[(0, WAND)])
        .await
        .unwrap();

    let from = Location::Inventory {
        character_id: character.id,
        slot: 0,
    };
    let to = Location::Vault {
        account_id: account.id,
        slot: 3,
    };

    let outcome = store.move_item(from, to, Some(WAND)).await.unwrap();
    assert_eq!(outcome.source, None, "the inventory slot is now empty");
    assert_eq!(outcome.destination, Some(WAND));

    assert!(
        store
            .character(character.id)
            .await
            .unwrap()
            .inventory
            .is_empty()
    );
    assert_eq!(store.vault(account.id).await.unwrap(), vec![(3, WAND)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_occupied_slots_swap() {
    let Some(store) = store("t_swap").await else {
        return;
    };

    let account = store.create_account("Swapper").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    store
        .set_inventory(character.id, &[(0, WAND)])
        .await
        .unwrap();
    store.set_vault_slot(account.id, 0, ROBE).await.unwrap();

    let inventory = Location::Inventory {
        character_id: character.id,
        slot: 0,
    };
    let vault = Location::Vault {
        account_id: account.id,
        slot: 0,
    };

    store.move_item(inventory, vault, Some(WAND)).await.unwrap();

    assert_eq!(
        store.character(character.id).await.unwrap().inventory,
        vec![(0, ROBE)]
    );
    assert_eq!(store.vault(account.id).await.unwrap(), vec![(0, WAND)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_something_that_is_not_there_is_refused() {
    let Some(store) = store("t_stale").await else {
        return;
    };

    let account = store.create_account("Stale").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    let from = Location::Inventory {
        character_id: character.id,
        slot: 0,
    };
    let to = Location::Vault {
        account_id: account.id,
        slot: 0,
    };

    // The slot is empty, so a request claiming a wand is stale.
    assert!(matches!(
        store.move_item(from, to, Some(WAND)).await,
        Err(StoreError::Refused(_))
    ));
    assert!(store.vault(account.id).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_item_cannot_be_moved_twice_at_once() {
    // The reason this path is transactional. Two requests both read the wand and both try to move
    // it to a different vault slot; without locking, both succeed and there are two wands.
    let Some(store) = store("t_race").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Racer").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    for attempt in 0..25 {
        let slot_a = (attempt * 2) as i16;
        let slot_b = slot_a + 1;

        store
            .set_inventory(character.id, &[(0, WAND)])
            .await
            .unwrap();

        let source = Location::Inventory {
            character_id: character.id,
            slot: 0,
        };

        let one = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .move_item(
                        source,
                        Location::Vault {
                            account_id: account.id,
                            slot: slot_a,
                        },
                        Some(WAND),
                    )
                    .await
            })
        };
        let two = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .move_item(
                        source,
                        Location::Vault {
                            account_id: account.id,
                            slot: slot_b,
                        },
                        Some(WAND),
                    )
                    .await
            })
        };

        let (first, second) = (one.await.unwrap(), two.await.unwrap());
        let winners = [first.is_ok(), second.is_ok()]
            .iter()
            .filter(|ok| **ok)
            .count();

        assert_eq!(
            winners, 1,
            "exactly one of two simultaneous moves must succeed (attempt {attempt})"
        );

        let wands = store
            .vault(account.id)
            .await
            .unwrap()
            .iter()
            .filter(|(_, item)| *item == WAND)
            .count()
            + store
                .character(character.id)
                .await
                .unwrap()
                .inventory
                .iter()
                .filter(|(_, item)| *item == WAND)
                .count();

        assert_eq!(wands, 1, "there must still be exactly one wand");

        // Clear the vault for the next attempt.
        for (slot, _) in store.vault(account.id).await.unwrap() {
            store.clear_vault_slot(account.id, slot).await.unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn opposing_swaps_do_not_deadlock() {
    // Two players swapping with each other in opposite directions at the same moment. Without a
    // fixed lock order each holds one row and waits for the other for ever.
    let Some(store) = store("t_deadlock").await else {
        return;
    };

    let account = store.create_account("Deadlock").await.unwrap();
    let one = store
        .create_character(account.id, WIZARD, "One", 800)
        .await
        .unwrap();
    let two = store
        .create_character(account.id, WIZARD, "Two", 800)
        .await
        .unwrap();

    for _ in 0..20 {
        store.set_inventory(one.id, &[(0, WAND)]).await.unwrap();
        store.set_inventory(two.id, &[(0, ROBE)]).await.unwrap();

        let a = Location::Inventory {
            character_id: one.id,
            slot: 0,
        };
        let b = Location::Inventory {
            character_id: two.id,
            slot: 0,
        };

        let forward = {
            let store = store.clone();
            tokio::spawn(async move { store.move_item(a, b, Some(WAND)).await })
        };
        let backward = {
            let store = store.clone();
            tokio::spawn(async move { store.move_item(b, a, Some(ROBE)).await })
        };

        // The assertion is that these finish at all. A deadlock shows up as a timeout.
        let both = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            futures_join(forward, backward),
        )
        .await;

        assert!(both.is_ok(), "opposing swaps deadlocked");
    }
}

/// Joins two tasks without pulling in a futures crate for one call.
async fn futures_join<T>(
    one: tokio::task::JoinHandle<T>,
    two: tokio::task::JoinHandle<T>,
) -> (T, T) {
    let first = one.await.expect("task should not panic");
    let second = two.await.expect("task should not panic");
    (first, second)
}

#[tokio::test(flavor = "multi_thread")]
async fn giving_an_item_finds_the_first_free_slot() {
    let Some(store) = store("t_give").await else {
        return;
    };

    let account = store.create_account("Giver").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    store
        .set_inventory(character.id, &[(4, WAND), (6, ROBE)])
        .await
        .unwrap();

    assert_eq!(
        store
            .give_item(character.id, WAND, 4, 11)
            .await
            .unwrap()
            .slot,
        5
    );
    assert_eq!(
        store
            .give_item(character.id, WAND, 4, 11)
            .await
            .unwrap()
            .slot,
        7
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_inventory_refuses_an_item() {
    let Some(store) = store("t_full").await else {
        return;
    };

    let account = store.create_account("Full").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    let packed: Vec<(i16, uuid::Uuid)> = (4..=11).map(|slot| (slot, WAND)).collect();
    store.set_inventory(character.id, &packed).await.unwrap();

    assert!(matches!(
        store.give_item(character.id, ROBE, 4, 11).await,
        Err(StoreError::Refused(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn two_pickups_cannot_claim_the_same_slot() {
    // Two bags looted at once with one slot free. Without locking the scan, both would find the
    // same free slot and the second would overwrite the first.
    let Some(store) = store("t_pickup_race").await else {
        return;
    };

    let account = store.create_account("Picker").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    for _ in 0..20 {
        // Everything full except slot 11.
        let packed: Vec<(i16, uuid::Uuid)> = (4..=10).map(|slot| (slot, WAND)).collect();
        store.set_inventory(character.id, &packed).await.unwrap();

        let one = {
            let store = store.clone();
            let id = character.id;
            tokio::spawn(async move { store.give_item(id, ROBE, 4, 11).await })
        };
        let two = {
            let store = store.clone();
            let id = character.id;
            tokio::spawn(async move { store.give_item(id, ROBE, 4, 11).await })
        };

        let (first, second) = (one.await.unwrap(), two.await.unwrap());
        let winners = [first.is_ok(), second.is_ok()]
            .iter()
            .filter(|ok| **ok)
            .count();

        assert_eq!(winners, 1, "one free slot can only take one item");
        assert_eq!(
            store.character(character.id).await.unwrap().inventory.len(),
            8,
            "and the inventory is full, not overfull"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_an_item_that_moved_is_refused() {
    let Some(store) = store("t_take").await else {
        return;
    };

    let account = store.create_account("Taker").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    store
        .set_inventory(character.id, &[(4, WAND)])
        .await
        .unwrap();

    // The condition is what makes two simultaneous drops of the same item resolve to one.
    assert!(matches!(
        store.take_item(character.id, 4, ROBE).await,
        Err(StoreError::Refused(_))
    ));
    store.take_item(character.id, 4, WAND).await.unwrap();
    assert!(matches!(
        store.take_item(character.id, 4, WAND).await,
        Err(StoreError::Refused(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_character_saves_and_dies() {
    let Some(store) = store("t_save").await else {
        return;
    };

    let account = store.create_account("Saver").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    store
        .save_character(character.id, 500, 40, 12, 3400, 25)
        .await
        .unwrap();

    let reloaded = store.character(character.id).await.unwrap();
    assert_eq!(reloaded.hp, 500);
    assert_eq!(reloaded.level, 12);
    assert_eq!(reloaded.fame, 25);

    // Health above the maximum is clamped by the query rather than trusted from the caller.
    store
        .save_character(character.id, 99_999, 40, 12, 3400, 25)
        .await
        .unwrap();
    assert_eq!(store.character(character.id).await.unwrap().hp, 800);

    store.kill_character(character.id).await.unwrap();
    assert!(!store.character(character.id).await.unwrap().alive);
    assert!(
        store.characters(account.id).await.unwrap().is_empty(),
        "the dead do not appear in character select"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn vault_chests_are_bought_up_to_a_limit() {
    let Some(store) = store("t_chests").await else {
        return;
    };

    let account = store.create_account("Buyer").await.unwrap();
    assert_eq!(store.buy_vault_chest(account.id, 6).await.unwrap(), 5);
    assert_eq!(store.buy_vault_chest(account.id, 6).await.unwrap(), 6);

    // The limit is enforced by the update's own WHERE clause, so two simultaneous purchases cannot
    // both see room and both take it.
    assert!(matches!(
        store.buy_vault_chest(account.id, 6).await,
        Err(StoreError::Refused(_))
    ));
    assert_eq!(
        store.account_by_name("Buyer").await.unwrap().vault_chests,
        6
    );
}

// -- trade ------------------------------------------------------------------------------------

/// Two characters on one account, each holding one item.
async fn traders(store: &Store, schema_name: &str) -> (i64, i64) {
    let account = store.create_account(schema_name).await.unwrap();
    let one = store
        .create_character(account.id, WIZARD, "One", 800)
        .await
        .unwrap();
    let two = store
        .create_character(account.id, WIZARD, "Two", 800)
        .await
        .unwrap();

    store.set_inventory(one.id, &[(4, WAND)]).await.unwrap();
    store.set_inventory(two.id, &[(4, ROBE)]).await.unwrap();
    (one.id, two.id)
}

/// Every item both characters hold, sorted.
async fn between(store: &Store, one: i64, two: i64) -> Vec<uuid::Uuid> {
    let mut items: Vec<uuid::Uuid> = store
        .character(one)
        .await
        .unwrap()
        .inventory
        .iter()
        .chain(store.character(two).await.unwrap().inventory.iter())
        .map(|(_, item)| *item)
        .collect();
    items.sort();
    items
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trade_exchanges_both_sides() {
    let Some(store) = store("t_trade").await else {
        return;
    };
    let (one, two) = traders(&store, "Trader").await;

    store
        .trade(
            &Offer::new(one, vec![(4, WAND)]),
            &Offer::new(two, vec![(4, ROBE)]),
            4,
            11,
        )
        .await
        .unwrap();

    let first: Vec<uuid::Uuid> = store
        .character(one)
        .await
        .unwrap()
        .inventory
        .iter()
        .map(|(_, item)| *item)
        .collect();
    let second: Vec<uuid::Uuid> = store
        .character(two)
        .await
        .unwrap()
        .inventory
        .iter()
        .map(|(_, item)| *item)
        .collect();

    assert_eq!(first, vec![ROBE]);
    assert_eq!(second, vec![WAND]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trade_of_an_item_that_moved_does_nothing_at_all() {
    // The failure a sequence of moves would get wrong: one side gives up its item and the other
    // cannot, leaving the asymmetry permanent.
    let Some(store) = store("t_trade_stale").await else {
        return;
    };
    let (one, two) = traders(&store, "Stale Trader").await;

    let before = between(&store, one, two).await;

    // The second player's item is not what the first believes.
    let outcome = store
        .trade(
            &Offer::new(one, vec![(4, WAND)]),
            &Offer::new(two, vec![(4, WAND)]),
            4,
            11,
        )
        .await;

    assert!(matches!(outcome, Err(StoreError::Refused(_))));
    assert_eq!(
        between(&store, one, two).await,
        before,
        "a refused trade must move nothing whatsoever"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trade_into_a_full_inventory_is_refused_before_anything_moves() {
    let Some(store) = store("t_trade_full").await else {
        return;
    };

    let account = store.create_account("Hoarder").await.unwrap();
    let one = store
        .create_character(account.id, WIZARD, "One", 800)
        .await
        .unwrap();
    let two = store
        .create_character(account.id, WIZARD, "Two", 800)
        .await
        .unwrap();

    // One offers nothing and has no room; two offers two items.
    let packed: Vec<(i16, uuid::Uuid)> = (4..=11).map(|slot| (slot, WAND)).collect();
    store.set_inventory(one.id, &packed).await.unwrap();
    store
        .set_inventory(two.id, &[(4, ROBE), (5, ROBE)])
        .await
        .unwrap();

    let before = between(&store, one.id, two.id).await;

    let outcome = store
        .trade(
            &Offer::new(one.id, Vec::new()),
            &Offer::new(two.id, vec![(4, ROBE), (5, ROBE)]),
            4,
            11,
        )
        .await;

    assert!(matches!(outcome, Err(StoreError::Refused(_))));
    assert_eq!(between(&store, one.id, two.id).await, before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_one_sided_trade_is_a_gift() {
    let Some(store) = store("t_gift").await else {
        return;
    };
    let (one, two) = traders(&store, "Giver of Gifts").await;

    store
        .trade(
            &Offer::new(one, vec![(4, WAND)]),
            &Offer::new(two, Vec::new()),
            4,
            11,
        )
        .await
        .unwrap();

    assert!(store.character(one).await.unwrap().inventory.is_empty());
    assert_eq!(store.character(two).await.unwrap().inventory.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_item_cannot_be_traded_to_two_people_at_once() {
    // The dupe this exists to prevent: one item, two simultaneous trades, both reading it as
    // present before either commits.
    let Some(store) = store("t_trade_race").await else {
        return;
    };

    let account = store.create_account("Duper").await.unwrap();
    let seller = store
        .create_character(account.id, WIZARD, "Seller", 800)
        .await
        .unwrap();
    let buyer_one = store
        .create_character(account.id, WIZARD, "One", 800)
        .await
        .unwrap();
    let buyer_two = store
        .create_character(account.id, WIZARD, "Two", 800)
        .await
        .unwrap();

    for _ in 0..20 {
        store.set_inventory(seller.id, &[(4, WAND)]).await.unwrap();
        store.set_inventory(buyer_one.id, &[]).await.unwrap();
        store.set_inventory(buyer_two.id, &[]).await.unwrap();

        let first = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .trade(
                        &Offer::new(seller.id, vec![(4, WAND)]),
                        &Offer::new(buyer_one.id, Vec::new()),
                        4,
                        11,
                    )
                    .await
            })
        };
        let second = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .trade(
                        &Offer::new(seller.id, vec![(4, WAND)]),
                        &Offer::new(buyer_two.id, Vec::new()),
                        4,
                        11,
                    )
                    .await
            })
        };

        let (a, b) = (first.await.unwrap(), second.await.unwrap());
        let winners = [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count();
        assert_eq!(winners, 1, "one item cannot be given to two people");

        let wands: usize = [seller.id, buyer_one.id, buyer_two.id]
            .iter()
            .map(|id| {
                futures_block(store.character(*id))
                    .inventory
                    .iter()
                    .filter(|(_, item)| *item == WAND)
                    .count()
            })
            .sum();

        assert_eq!(wands, 1, "there must still be exactly one wand");
    }
}

/// Awaits inside a synchronous closure by blocking the current thread's runtime handle.
fn futures_block(
    future: impl std::future::Future<Output = hendra_store::Result<hendra_store::Character>>,
) -> hendra_store::Character {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
        .expect("the character should load")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_potion_stack_fills_to_its_limit_and_no_further() {
    let Some(store) = store("t_potions").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    for taken in 0..Store::POTION_LIMIT {
        assert!(
            store.add_potion(character.id, false).await.unwrap(),
            "room for potion {taken}"
        );
    }
    assert!(
        !store.add_potion(character.id, false).await.unwrap(),
        "and no room for one more"
    );

    let held = store.character(character.id).await.unwrap();
    assert_eq!(held.health_potions, Store::POTION_LIMIT);
    assert_eq!(held.magic_potions, 0, "the two stacks are separate");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_potion_cannot_be_taken_from_an_empty_stack() {
    let Some(store) = store("t_potions_empty").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    assert!(!store.take_potion(character.id, false).await.unwrap());

    store.add_potion(character.id, false).await.unwrap();
    assert!(store.take_potion(character.id, false).await.unwrap());
    assert!(!store.take_potion(character.id, false).await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn two_pickups_racing_for_the_last_place_in_a_stack_cannot_both_win() {
    // The ceiling is in the statement rather than checked before it, so both cannot read room for
    // the same last potion.
    let Some(store) = store("t_potions_race").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    for _ in 0..Store::POTION_LIMIT - 1 {
        store.add_potion(character.id, false).await.unwrap();
    }

    let (first, second) = tokio::join!(
        store.add_potion(character.id, false),
        store.add_potion(character.id, false)
    );

    let winners = [first.unwrap(), second.unwrap()]
        .iter()
        .filter(|won| **won)
        .count();
    assert_eq!(winners, 1, "exactly one of two may take the last place");

    let held = store.character(character.id).await.unwrap();
    assert_eq!(held.health_potions, Store::POTION_LIMIT);
}

#[tokio::test(flavor = "multi_thread")]
async fn buying_something_pays_for_it_and_delivers_it() {
    let Some(store) = store("t_buy").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    store.credit(account.id, Currency::Gold, 100).await.unwrap();
    let slot = store
        .buy_item(Purchase {
            account_id: account.id,
            character_id: character.id,
            item: WAND,
            currency: Currency::Gold,
            price: 40,
            first_slot: 4,
            last_slot: 11,
        })
        .await
        .unwrap();

    let held = store.character(character.id).await.unwrap();
    assert!(held.inventory.contains(&(slot, WAND)));
    assert_eq!(store.account(account.id).await.unwrap().gold, 60);
}

#[tokio::test(flavor = "multi_thread")]
async fn buying_what_you_cannot_afford_costs_nothing_and_delivers_nothing() {
    let Some(store) = store("t_buy_poor").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    store.credit(account.id, Currency::Gold, 10).await.unwrap();
    assert!(
        store
            .buy_item(Purchase {
                account_id: account.id,
                character_id: character.id,
                item: WAND,
                currency: Currency::Gold,
                price: 40,
                first_slot: 4,
                last_slot: 11,
            })
            .await
            .is_err()
    );

    assert_eq!(store.account(account.id).await.unwrap().gold, 10);
    assert!(
        store
            .character(character.id)
            .await
            .unwrap()
            .inventory
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_purchase_with_nowhere_to_put_it_leaves_the_money_alone() {
    // Paying and receiving are one transaction, so a failure at either end undoes both. Otherwise
    // a full inventory is a way to lose money and get nothing.
    let Some(store) = store("t_buy_full").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    let full: Vec<(i16, uuid::Uuid)> = (4..=11).map(|slot| (slot, ROBE)).collect();
    store.set_inventory(character.id, &full).await.unwrap();
    store.credit(account.id, Currency::Gold, 100).await.unwrap();

    assert!(
        store
            .buy_item(Purchase {
                account_id: account.id,
                character_id: character.id,
                item: WAND,
                currency: Currency::Gold,
                price: 40,
                first_slot: 4,
                last_slot: 11,
            })
            .await
            .is_err()
    );
    assert_eq!(
        store.account(account.id).await.unwrap().gold,
        100,
        "the money must still be there"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_purchases_racing_for_the_last_coin_cannot_both_win() {
    let Some(store) = store("t_buy_race").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    store.credit(account.id, Currency::Gold, 40).await.unwrap();

    let (first, second) = tokio::join!(
        store.buy_item(Purchase {
            account_id: account.id,
            character_id: character.id,
            item: WAND,
            currency: Currency::Gold,
            price: 40,
            first_slot: 4,
            last_slot: 11,
        }),
        store.buy_item(Purchase {
            account_id: account.id,
            character_id: character.id,
            item: ROBE,
            currency: Currency::Gold,
            price: 40,
            first_slot: 4,
            last_slot: 11,
        })
    );

    let winners = [first.is_ok(), second.is_ok()]
        .iter()
        .filter(|ok| **ok)
        .count();
    assert_eq!(winners, 1, "forty gold buys one of two forty-gold items");

    assert_eq!(store.account(account.id).await.unwrap().gold, 0);
    assert_eq!(
        store.character(character.id).await.unwrap().inventory.len(),
        1,
        "and exactly one item arrived"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_friendship_needs_both_sides_to_ask() {
    let Some(store) = store("t_friends").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let one = store.create_account("Fesal").await.unwrap();
    let two = store.create_account("Someone").await.unwrap();

    assert!(
        !store.befriend(one.id, two.id).await.unwrap(),
        "asking alone is not a friendship"
    );

    let theirs = store.friend_requests(two.id).await.unwrap();
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0].name, "Fesal");

    assert!(
        store.befriend(two.id, one.id).await.unwrap(),
        "and answering makes one"
    );

    for account in [one.id, two.id] {
        let friends = store.friends(account).await.unwrap();
        assert_eq!(friends.len(), 1);
        assert!(friends[0].accepted, "both sides see it accepted");
    }
    assert!(store.friend_requests(two.id).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_friend_removes_it_from_both_lists() {
    // A one-sided removal leaves the other believing in a friendship that is not there.
    let Some(store) = store("t_unfriend").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let one = store.create_account("Fesal").await.unwrap();
    let two = store.create_account("Someone").await.unwrap();

    store.befriend(one.id, two.id).await.unwrap();
    store.befriend(two.id, one.id).await.unwrap();
    store.unfriend(two.id, one.id).await.unwrap();

    assert!(store.friends(one.id).await.unwrap().is_empty());
    assert!(store.friends(two.id).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn asking_twice_is_not_two_friendships() {
    let Some(store) = store("t_friend_twice").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let one = store.create_account("Fesal").await.unwrap();
    let two = store.create_account("Someone").await.unwrap();

    for _ in 0..5 {
        store.befriend(one.id, two.id).await.unwrap();
    }
    store.befriend(two.id, one.id).await.unwrap();

    assert_eq!(store.friends(one.id).await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn nobody_can_befriend_themselves() {
    let Some(store) = store("t_friend_self").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let one = store.create_account("Fesal").await.unwrap();
    assert!(store.befriend(one.id, one.id).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_waits_and_is_read_once() {
    let Some(store) = store("t_messages").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let one = store.create_account("Fesal").await.unwrap();
    let two = store.create_account("Someone").await.unwrap();

    let id = store
        .send_message(one.id, two.id, "are you there")
        .await
        .unwrap();

    let inbox = store.messages(two.id, 10).await.unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].from, "Fesal");
    assert!(!inbox[0].read);

    assert!(store.mark_read(two.id, id).await.unwrap());
    assert!(
        !store.mark_read(two.id, id).await.unwrap(),
        "reading it twice is reading it once"
    );
    assert!(store.messages(two.id, 10).await.unwrap()[0].read);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_can_only_be_read_or_deleted_by_who_it_was_sent_to() {
    let Some(store) = store("t_messages_theirs").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let one = store.create_account("Fesal").await.unwrap();
    let two = store.create_account("Someone").await.unwrap();
    let three = store.create_account("Nosey").await.unwrap();

    let id = store.send_message(one.id, two.id, "private").await.unwrap();

    assert!(!store.mark_read(three.id, id).await.unwrap());
    assert!(!store.delete_message(three.id, id).await.unwrap());
    assert_eq!(store.messages(two.id, 10).await.unwrap().len(), 1);

    assert!(store.delete_message(two.id, id).await.unwrap());
    assert!(store.messages(two.id, 10).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_message_is_refused_and_a_long_one_is_cut() {
    let Some(store) = store("t_messages_bounds").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let one = store.create_account("Fesal").await.unwrap();
    let two = store.create_account("Someone").await.unwrap();

    assert!(store.send_message(one.id, two.id, "   ").await.is_err());
    assert!(store.send_message(one.id, one.id, "hello").await.is_err());

    let long = "a".repeat(hendra_store::social::MAX_MESSAGE_LENGTH * 2);
    store.send_message(one.id, two.id, &long).await.unwrap();

    let inbox = store.messages(two.id, 10).await.unwrap();
    assert_eq!(
        inbox[0].body.chars().count(),
        hendra_store::social::MAX_MESSAGE_LENGTH
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn founding_a_guild_makes_you_its_founder() {
    let Some(store) = store("t_guild").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let one = store.create_account("Fesal").await.unwrap();
    let guild = store.found_guild(one.id, "The Quiet").await.unwrap();

    let members = store.guild_members(guild.id).await.unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].rank, Rank::Founder);
    assert!(members[0].rank.may_rank());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guild_name_is_taken_once_however_it_is_shouted() {
    let Some(store) = store("t_guild_name").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let one = store.create_account("Fesal").await.unwrap();
    let two = store.create_account("Someone").await.unwrap();

    store.found_guild(one.id, "The Quiet").await.unwrap();
    assert!(store.found_guild(two.id, "THE QUIET").await.is_err());
    assert!(store.found_guild(two.id, "the quiet").await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn nobody_is_in_two_guilds() {
    // Leaving one to found another is two decisions, and doing both silently is how someone leaves
    // a guild they meant to keep.
    let Some(store) = store("t_guild_two").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let one = store.create_account("Fesal").await.unwrap();
    let two = store.create_account("Someone").await.unwrap();

    let first = store.found_guild(one.id, "The Quiet").await.unwrap();
    assert!(store.found_guild(one.id, "The Loud").await.is_err());

    let second = store.found_guild(two.id, "The Loud").await.unwrap();
    assert!(store.join_guild(one.id, second.id).await.is_err());

    store.leave_guild(one.id).await.unwrap();
    store.join_guild(one.id, second.id).await.unwrap();
    assert!(store.guild_members(first.id).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn nobody_can_rank_somebody_at_or_above_themselves() {
    // Promoting somebody above yourself is how a guild loses its founder, and demoting somebody
    // who outranks you is how it loses one to a mutiny.
    let Some(store) = store("t_guild_rank").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let founder = store.create_account("Fesal").await.unwrap();
    let officer = store.create_account("Someone").await.unwrap();
    let guild = store.found_guild(founder.id, "The Quiet").await.unwrap();
    store.join_guild(officer.id, guild.id).await.unwrap();

    store
        .set_guild_rank(founder.id, officer.id, Rank::Officer)
        .await
        .unwrap();

    // An officer cannot rank anyone at all.
    assert!(
        store
            .set_guild_rank(officer.id, founder.id, Rank::Initiate)
            .await
            .is_err()
    );

    // And a founder cannot make somebody a founder beside them.
    assert!(
        store
            .set_guild_rank(founder.id, officer.id, Rank::Founder)
            .await
            .is_err()
    );

    let members = store.guild_members(guild.id).await.unwrap();
    assert_eq!(members[0].rank, Rank::Founder);
    assert_eq!(members[0].name, "Fesal");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_an_officer_may_change_the_board() {
    let Some(store) = store("t_guild_board").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let founder = store.create_account("Fesal").await.unwrap();
    let initiate = store.create_account("Someone").await.unwrap();
    let guild = store.found_guild(founder.id, "The Quiet").await.unwrap();
    store.join_guild(initiate.id, guild.id).await.unwrap();

    assert!(
        store
            .set_guild_board(initiate.id, "mine now")
            .await
            .is_err()
    );
    store
        .set_guild_board(founder.id, "be excellent")
        .await
        .unwrap();

    assert_eq!(store.guild(guild.id).await.unwrap().board, "be excellent");
}

#[tokio::test(flavor = "multi_thread")]
async fn ranking_somebody_in_another_guild_is_refused() {
    let Some(store) = store("t_guild_outsider").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let one = store.create_account("Fesal").await.unwrap();
    let two = store.create_account("Someone").await.unwrap();
    store.found_guild(one.id, "The Quiet").await.unwrap();
    store.found_guild(two.id, "The Loud").await.unwrap();

    assert!(
        store
            .set_guild_rank(one.id, two.id, Rank::Initiate)
            .await
            .is_err()
    );
}

/// A seller and a buyer, each with a character and some gold.
async fn market(store: &Store) -> (i64, i64, i64, i64) {
    let seller = store.create_account("Fesal").await.unwrap();
    let buyer = store.create_account("Someone").await.unwrap();

    let seller_character = store
        .create_character(seller.id, WIZARD, "Seller", 800)
        .await
        .unwrap();
    let buyer_character = store
        .create_character(buyer.id, WIZARD, "Buyer", 800)
        .await
        .unwrap();

    store
        .set_inventory(seller_character.id, &[(4, WAND)])
        .await
        .unwrap();
    store.credit(buyer.id, Currency::Gold, 1000).await.unwrap();

    (seller.id, seller_character.id, buyer.id, buyer_character.id)
}

#[tokio::test(flavor = "multi_thread")]
async fn listing_something_takes_it_out_of_the_inventory() {
    // An item that existed in both places is an item that could be sold and kept.
    let Some(store) = store("t_market_list").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let (seller, seller_character, _, _) = market(&store).await;
    store
        .list_item(seller, seller_character, 4, WAND, Currency::Gold, 100)
        .await
        .unwrap();

    assert!(
        store
            .character(seller_character)
            .await
            .unwrap()
            .inventory
            .is_empty(),
        "the wand is in the listing, not the inventory"
    );
    assert_eq!(store.listings(10).await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn buying_moves_the_item_and_the_money() {
    let Some(store) = store("t_market_buy").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let (seller, seller_character, buyer, buyer_character) = market(&store).await;
    let listing = store
        .list_item(seller, seller_character, 4, WAND, Currency::Gold, 100)
        .await
        .unwrap();

    store
        .buy_listing(MarketPurchase {
            buyer_id: buyer,
            character_id: buyer_character,
            listing_id: listing,
            first_slot: 4,
            last_slot: 11,
        })
        .await
        .unwrap();

    let held = store.character(buyer_character).await.unwrap();
    assert!(held.inventory.iter().any(|(_, item)| *item == WAND));
    assert_eq!(store.account(buyer).await.unwrap().gold, 900);

    // The seller is paid the price less the fee.
    let fee = hendra_store::market::fee(100);
    assert_eq!(store.account(seller).await.unwrap().gold, 100 - fee);
    assert!(store.listings(10).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn two_buyers_racing_for_one_listing_cannot_both_win() {
    // The whole point of closing the listing first. This is the one place a dupe would be worth
    // the most, so it is run repeatedly.
    let Some(store) = store("t_market_race").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    for attempt in 0..20 {
        let seller = store
            .create_account(&format!("Seller{attempt}"))
            .await
            .unwrap();
        let one = store
            .create_account(&format!("One{attempt}"))
            .await
            .unwrap();
        let two = store
            .create_account(&format!("Two{attempt}"))
            .await
            .unwrap();

        let stock = store
            .create_character(seller.id, WIZARD, "S", 800)
            .await
            .unwrap();
        let first = store
            .create_character(one.id, WIZARD, "A", 800)
            .await
            .unwrap();
        let second = store
            .create_character(two.id, WIZARD, "B", 800)
            .await
            .unwrap();

        store.set_inventory(stock.id, &[(4, WAND)]).await.unwrap();
        store.credit(one.id, Currency::Gold, 1000).await.unwrap();
        store.credit(two.id, Currency::Gold, 1000).await.unwrap();

        let listing = store
            .list_item(seller.id, stock.id, 4, WAND, Currency::Gold, 100)
            .await
            .unwrap();

        let (a, b) = tokio::join!(
            store.buy_listing(MarketPurchase {
                buyer_id: one.id,
                character_id: first.id,
                listing_id: listing,
                first_slot: 4,
                last_slot: 11,
            }),
            store.buy_listing(MarketPurchase {
                buyer_id: two.id,
                character_id: second.id,
                listing_id: listing,
                first_slot: 4,
                last_slot: 11,
            })
        );

        let winners = [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count();
        assert_eq!(winners, 1, "one listing sells once (attempt {attempt})");

        let wands = store.character(first.id).await.unwrap().inventory.len()
            + store.character(second.id).await.unwrap().inventory.len();
        assert_eq!(wands, 1, "and exactly one wand exists (attempt {attempt})");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_racing_a_sale_resolves_to_one_of_them() {
    let Some(store) = store("t_market_cancel_race").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    for attempt in 0..10 {
        let seller = store
            .create_account(&format!("Seller{attempt}"))
            .await
            .unwrap();
        let buyer = store
            .create_account(&format!("Buyer{attempt}"))
            .await
            .unwrap();
        let stock = store
            .create_character(seller.id, WIZARD, "S", 800)
            .await
            .unwrap();
        let theirs = store
            .create_character(buyer.id, WIZARD, "B", 800)
            .await
            .unwrap();

        store.set_inventory(stock.id, &[(4, WAND)]).await.unwrap();
        store.credit(buyer.id, Currency::Gold, 1000).await.unwrap();

        let listing = store
            .list_item(seller.id, stock.id, 4, WAND, Currency::Gold, 100)
            .await
            .unwrap();

        let (sold, cancelled) = tokio::join!(
            store.buy_listing(MarketPurchase {
                buyer_id: buyer.id,
                character_id: theirs.id,
                listing_id: listing,
                first_slot: 4,
                last_slot: 11,
            }),
            store.cancel_listing(seller.id, listing, stock.id, 4, 11)
        );

        let winners = [sold.is_ok(), cancelled.is_ok()]
            .iter()
            .filter(|ok| **ok)
            .count();
        assert_eq!(
            winners, 1,
            "sold or returned, never both (attempt {attempt})"
        );

        let wands = store.character(stock.id).await.unwrap().inventory.len()
            + store.character(theirs.id).await.unwrap().inventory.len();
        assert_eq!(wands, 1, "and the wand is in exactly one place");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_buyer_who_cannot_pay_leaves_the_listing_open() {
    let Some(store) = store("t_market_poor").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let (seller, seller_character, buyer, buyer_character) = market(&store).await;
    let listing = store
        .list_item(seller, seller_character, 4, WAND, Currency::Gold, 5000)
        .await
        .unwrap();

    assert!(
        store
            .buy_listing(MarketPurchase {
                buyer_id: buyer,
                character_id: buyer_character,
                listing_id: listing,
                first_slot: 4,
                last_slot: 11,
            })
            .await
            .is_err()
    );

    assert_eq!(store.listings(10).await.unwrap().len(), 1, "still for sale");
    assert_eq!(store.account(buyer).await.unwrap().gold, 1000);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_gives_the_item_back_and_only_to_its_seller() {
    let Some(store) = store("t_market_cancel").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let (seller, seller_character, buyer, buyer_character) = market(&store).await;
    let listing = store
        .list_item(seller, seller_character, 4, WAND, Currency::Gold, 100)
        .await
        .unwrap();

    assert!(
        store
            .cancel_listing(buyer, listing, buyer_character, 4, 11)
            .await
            .is_err(),
        "somebody else's listing is not yours to cancel"
    );

    store
        .cancel_listing(seller, listing, seller_character, 4, 11)
        .await
        .unwrap();

    let held = store.character(seller_character).await.unwrap();
    assert!(held.inventory.iter().any(|(_, item)| *item == WAND));
    assert!(store.listings(10).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_moderator_may_mute_and_an_administrator_may_ban() {
    let Some(store) = store("t_moderation").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let player = store.create_account("Fesal").await.unwrap();
    assert_eq!(Admin::from_number(player.admin_rank), Admin::None);
    assert!(!Admin::None.may_mute());
    assert!(!Admin::None.may_ban());

    store
        .set_admin_rank(player.id, Admin::Moderator)
        .await
        .unwrap();
    let moderator = store.account(player.id).await.unwrap();

    // A moderator may silence but not remove: they are different powers and one is reversible in a
    // way the other is not.
    assert!(Admin::from_number(moderator.admin_rank).may_mute());
    assert!(!Admin::from_number(moderator.admin_rank).may_ban());

    store
        .set_admin_rank(player.id, Admin::Administrator)
        .await
        .unwrap();
    let admin = store.account(player.id).await.unwrap();
    assert!(Admin::from_number(admin.admin_rank).may_ban());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mute_expires_on_its_own() {
    // A mute with no end is a ban nobody remembers applying.
    let Some(store) = store("t_mute_expiry").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();

    let past = chrono::Utc::now() - chrono::Duration::minutes(1);
    store.mute(account.id, Some(past)).await.unwrap();

    let held = store.account(account.id).await.unwrap();
    assert!(
        held.muted_until
            .is_some_and(|until| until < chrono::Utc::now()),
        "an expired mute needs nothing to clear it"
    );

    store.mute(account.id, None).await.unwrap();
    assert!(
        store
            .account(account.id)
            .await
            .unwrap()
            .muted_until
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn banning_and_unbanning_are_both_possible() {
    let Some(store) = store("t_ban").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    assert!(!account.banned);

    store.set_banned(account.id, true).await.unwrap();
    assert!(store.account(account.id).await.unwrap().banned);

    store.set_banned(account.id, false).await.unwrap();
    assert!(!store.account(account.id).await.unwrap().banned);
}

#[tokio::test(flavor = "multi_thread")]
async fn news_comes_back_newest_first_and_the_two_kinds_are_kept_apart() {
    let Some(store) = store("t_news").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    store
        .post_news("First", "an old thing", false)
        .await
        .unwrap();
    store
        .post_news("Second", "a newer thing", false)
        .await
        .unwrap();
    store
        .post_news("Inside", "for the game panel", true)
        .await
        .unwrap();

    let front = store.news(false, 10).await.unwrap();
    assert_eq!(front.len(), 2, "the in-game one is not here");
    assert_eq!(front[0].title, "Second", "newest first");

    let inside = store.news(true, 10).await.unwrap();
    assert_eq!(inside.len(), 1);
    assert_eq!(inside[0].title, "Inside");
}

#[tokio::test(flavor = "multi_thread")]
async fn news_needs_a_title() {
    let Some(store) = store("t_news_empty").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    assert!(store.post_news("   ", "a body", false).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn today_can_only_be_claimed_once() {
    // The date is the primary key, so claiming twice is refused by the table rather than by a
    // check that can race with itself.
    let Some(store) = store("t_daily").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();

    let before = store.calendar(account.id).await.unwrap();
    assert_eq!(before.streak, 0);
    assert!(!before.claimed_today);

    assert!(store.claim_today(account.id).await.unwrap());
    assert!(!store.claim_today(account.id).await.unwrap(), "only once");

    let after = store.calendar(account.id).await.unwrap();
    assert_eq!(after.streak, 1);
    assert!(after.claimed_today);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_streak_counts_back_through_consecutive_days_and_stops_at_a_gap() {
    let Some(store) = store("t_daily_streak").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let today = chrono::Utc::now().date_naive();

    // Today, yesterday, the day before, then a gap, then one more.
    for back in [0i64, 1, 2, 5] {
        sqlx::query("INSERT INTO daily_claim (account_id, claimed_on) VALUES ($1, $2)")
            .bind(account.id)
            .bind(today - chrono::Duration::days(back))
            .execute(store.pool())
            .await
            .unwrap();
    }

    let calendar = store.calendar(account.id).await.unwrap();
    assert_eq!(calendar.streak, 3, "the gap ends it");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_streak_survives_not_having_claimed_yet_today() {
    // Somebody who has not claimed yet still has yesterday's run, and telling them it is broken
    // before the day is out would be wrong.
    let Some(store) = store("t_daily_pending").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let today = chrono::Utc::now().date_naive();

    for back in [1i64, 2, 3] {
        sqlx::query("INSERT INTO daily_claim (account_id, claimed_on) VALUES ($1, $2)")
            .bind(account.id)
            .bind(today - chrono::Duration::days(back))
            .execute(store.pool())
            .await
            .unwrap();
    }

    let calendar = store.calendar(account.id).await.unwrap();
    assert_eq!(calendar.streak, 3);
    assert!(!calendar.claimed_today, "but today is still available");
}

#[tokio::test(flavor = "multi_thread")]
async fn strings_are_kept_per_language_and_replaced_rather_than_duplicated() {
    let Some(store) = store("t_strings").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    store.set_string("en", "greeting", "Hello").await.unwrap();
    store.set_string("fr", "greeting", "Bonjour").await.unwrap();
    store.set_string("en", "greeting", "Hi").await.unwrap();

    let english = store.strings("en").await.unwrap();
    assert_eq!(english.len(), 1, "replaced, not duplicated");
    assert_eq!(english[0].1, "Hi");
    assert_eq!(store.strings("fr").await.unwrap()[0].1, "Bonjour");
}

#[tokio::test(flavor = "multi_thread")]
async fn credits_come_from_the_offer_rather_than_from_whoever_is_asking() {
    let Some(store) = store("t_offers").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let (offer,): (i64,) = sqlx::query_as(
        "INSERT INTO credit_offer (name, credits, price_cents) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind("A handful")
    .bind(500)
    .bind(199)
    .fetch_one(store.pool())
    .await
    .unwrap();

    assert_eq!(store.grant_offer(account.id, offer).await.unwrap(), 500);
    assert_eq!(store.account(account.id).await.unwrap().credits, 500);

    assert!(
        store.grant_offer(account.id, 99999).await.is_err(),
        "an offer that does not exist grants nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_quest_advances_to_its_goal_and_finishes_once() {
    let Some(store) = store("t_quests").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    store
        .define_quest("slay", "Slay ten slimes", 10, false)
        .await
        .unwrap();

    assert!(!store.advance_quest(account.id, "slay", 4).await.unwrap());
    assert!(!store.advance_quest(account.id, "slay", 4).await.unwrap());
    assert!(store.advance_quest(account.id, "slay", 4).await.unwrap());

    let quests = store.quests(account.id, None).await.unwrap();
    assert_eq!(quests.len(), 1);
    assert!(quests[0].finished);
    assert_eq!(quests[0].progress, 10, "and never past the goal");
}

#[tokio::test(flavor = "multi_thread")]
async fn weekly_quests_can_be_asked_for_on_their_own() {
    let Some(store) = store("t_quests_weekly").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    store
        .define_quest("always", "A standing task", 1, false)
        .await
        .unwrap();
    store
        .define_quest("thisweek", "A weekly task", 1, true)
        .await
        .unwrap();

    assert_eq!(store.quests(account.id, None).await.unwrap().len(), 2);
    let weekly = store.quests(account.id, Some(true)).await.unwrap();
    assert_eq!(weekly.len(), 1);
    assert_eq!(weekly[0].key, "thisweek");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_skin_is_paid_for_once_and_cannot_be_bought_twice() {
    let Some(store) = store("t_skins").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let skin = uuid::Uuid::from_u128(0x5111);

    sqlx::query("UPDATE account SET credits = 1000 WHERE id = $1")
        .bind(account.id)
        .execute(store.pool())
        .await
        .unwrap();

    store.buy_skin(account.id, skin, 400).await.unwrap();
    assert_eq!(store.account(account.id).await.unwrap().credits, 600);
    assert_eq!(store.owned_skins(account.id).await.unwrap(), vec![skin]);

    // Buying it again is refused, and costs nothing.
    assert!(store.buy_skin(account.id, skin, 400).await.is_err());
    assert_eq!(
        store.account(account.id).await.unwrap().credits,
        600,
        "the second attempt must not have been charged for"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_skin_nobody_can_afford_is_refused_and_costs_nothing() {
    let Some(store) = store("t_skins_poor").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    assert!(
        store
            .buy_skin(account.id, uuid::Uuid::from_u128(1), 400)
            .await
            .is_err()
    );
    assert!(store.owned_skins(account.id).await.unwrap().is_empty());
    assert_eq!(store.account(account.id).await.unwrap().credits, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_picture_is_bounded_and_replaced_rather_than_accumulated() {
    let Some(store) = store("t_pictures").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();

    assert!(store.set_picture(account.id, "png", &[]).await.is_err());

    let huge = vec![0u8; hendra_store::extras::MAX_PICTURE_BYTES + 1];
    assert!(store.set_picture(account.id, "png", &huge).await.is_err());

    store
        .set_picture(account.id, "png", &[1, 2, 3])
        .await
        .unwrap();
    store
        .set_picture(account.id, "jpeg", &[4, 5])
        .await
        .unwrap();

    let (kind, bytes) = store.picture(account.id).await.unwrap().unwrap();
    assert_eq!(kind, "jpeg", "the newer one replaced it");
    assert_eq!(bytes, vec![4, 5]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_age_confirmation_is_recorded_without_the_date() {
    // What the server needs to know is whether somebody said yes. Keeping the date would be
    // keeping something it has no use for.
    let Some(store) = store("t_age").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    assert!(!store.age_verified(account.id).await.unwrap());

    store.set_age_verified(account.id, true).await.unwrap();
    assert!(store.age_verified(account.id).await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dye_lands_in_the_slot_it_belongs_to() {
    let Some(store) = store("t_dye").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    assert_eq!(
        store.set_dye(character.id, 0x0000_0011).await.unwrap(),
        DyeSlot::Cloth
    );
    assert_eq!(
        store.set_dye(character.id, 0x0100_0022).await.unwrap(),
        DyeSlot::Accessory
    );

    let (cloth, accessory): (i32, i32) =
        sqlx::query_as("SELECT dye_cloth, dye_accessory FROM character WHERE id = $1")
            .bind(character.id)
            .fetch_one(store.pool())
            .await
            .unwrap();

    assert_eq!(cloth, 0x0000_0011);
    assert_eq!(accessory, 0x0100_0022, "one did not overwrite the other");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_skin_cannot_be_worn_without_owning_it() {
    let Some(store) = store("t_wear_skin").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    assert!(store.wear_skin(account.id, character.id, 7).await.is_err());

    store
        .grant_skin(account.id, uuid::Uuid::from_u128(7))
        .await
        .unwrap();
    store.wear_skin(account.id, character.id, 7).await.unwrap();

    // And going back to the class's own appearance never needs owning anything.
    store.wear_skin(account.id, character.id, 0).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_backpack_is_granted_once() {
    let Some(store) = store("t_backpack").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, "Wizard", 800)
        .await
        .unwrap();

    assert!(store.grant_backpack(character.id).await.unwrap());
    assert!(
        !store.grant_backpack(character.id).await.unwrap(),
        "a second one is not another row of slots"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn boosts_extend_rather_than_stacking() {
    // Two of the same kind at once is a multiplier nobody wrote down, and a number that grows
    // every time somebody buys another.
    let Some(store) = store("t_boosts").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();

    store
        .add_boost(account.id, "experience", 2.0, 3600)
        .await
        .unwrap();
    store
        .add_boost(account.id, "experience", 2.0, 3600)
        .await
        .unwrap();

    let running = store.boosts(account.id).await.unwrap();
    assert_eq!(running.len(), 1, "one boost, not two");
    assert!((running[0].multiplier - 2.0).abs() < 0.01);

    // A different kind is its own row.
    store
        .add_boost(account.id, "loot_drop", 1.5, 3600)
        .await
        .unwrap();
    assert_eq!(store.boosts(account.id).await.unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_boost_is_not_running() {
    let Some(store) = store("t_boosts_expiry").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    sqlx::query(
        "INSERT INTO account_boost (account_id, kind, multiplier, expires_at)
         VALUES ($1, 'experience', 2.0, now() - interval '1 minute')",
    )
    .bind(account.id)
    .execute(store.pool())
    .await
    .unwrap();

    assert!(store.boosts(account.id).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn pets_are_bounded_and_belong_to_the_account() {
    let Some(store) = store("t_pets").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();

    for n in 0..hendra_store::wardrobe::MAX_PETS {
        store
            .add_pet(account.id, uuid::Uuid::from_u128(n as u128), false)
            .await
            .unwrap();
    }
    assert!(
        store
            .add_pet(account.id, uuid::Uuid::from_u128(999), false)
            .await
            .is_err(),
        "a player with four hundred pets is a player nobody can render"
    );

    let pets = store.pets(account.id).await.unwrap();
    assert_eq!(pets.len() as i64, hendra_store::wardrobe::MAX_PETS);

    assert!(store.set_pet_skin(account.id, pets[0].id, 4).await.unwrap());
    assert_eq!(store.pets(account.id).await.unwrap()[0].skin, 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pet_belonging_to_somebody_else_cannot_be_recoloured() {
    let Some(store) = store("t_pets_theirs").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let one = store.create_account("Fesal").await.unwrap();
    let two = store.create_account("Someone").await.unwrap();

    let pet = store
        .add_pet(one.id, uuid::Uuid::from_u128(1), false)
        .await
        .unwrap();

    assert!(!store.set_pet_skin(two.id, pet, 4).await.unwrap());
    assert_eq!(store.pets(one.id).await.unwrap()[0].skin, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unlocked_portal_is_recorded_once() {
    let Some(store) = store("t_portals").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();

    store
        .unlock_portal(account.id, "The Shatters")
        .await
        .unwrap();
    store
        .unlock_portal(account.id, "The Shatters")
        .await
        .unwrap();

    assert_eq!(
        store.unlocked_portals(account.id).await.unwrap(),
        vec!["The Shatters".to_string()]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn one_discord_id_belongs_to_one_account() {
    // The link exists so something outside the game can say who somebody is. A discord id pointing
    // at two accounts would let one person answer for another.
    let Some(store) = store("t_discord_one").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let first = store.create_account("Fesal").await.unwrap();
    let second = store.create_account("Someone").await.unwrap();

    store.register_discord(first.id, "1234").await.unwrap();
    assert_eq!(
        store.account_of_discord("1234").await.unwrap(),
        Some(first.id)
    );

    // The same id claimed again moves rather than duplicates.
    store.register_discord(second.id, "1234").await.unwrap();
    assert_eq!(
        store.account_of_discord("1234").await.unwrap(),
        Some(second.id),
        "the id should have moved"
    );

    let left = store.account(first.id).await.unwrap();
    assert_eq!(left.discord_id, None, "and left the first account");
}

#[tokio::test(flavor = "multi_thread")]
async fn unlinking_names_the_id_it_is_removing() {
    // Naming the id rather than just the account is what stops a stale request from unlinking
    // whatever happens to be there now.
    let Some(store) = store("t_discord_stale").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    store.register_discord(account.id, "old").await.unwrap();
    store.register_discord(account.id, "new").await.unwrap();

    assert!(
        store.unregister_discord(account.id, "old").await.is_err(),
        "a stale unlink took the current one"
    );
    assert_eq!(
        store.account_of_discord("new").await.unwrap(),
        Some(account.id)
    );

    store.unregister_discord(account.id, "new").await.unwrap();
    assert_eq!(store.account_of_discord("new").await.unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_discord_id_is_not_a_link() {
    let Some(store) = store("t_discord_empty").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    assert!(store.register_discord(account.id, "  ").await.is_err());
    assert_eq!(store.account(account.id).await.unwrap().discord_id, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gift_taken_twice_is_taken_once() {
    // The gift chest holds durable items. Taking one has to remove the row, and without that the
    // same gift is handed out on every visit.
    let Some(store) = store("t_gift_twice").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), "Fesal", 100)
        .await
        .unwrap();

    let item = uuid::Uuid::new_v4();
    let slot = store.add_gift(account.id, item).await.unwrap();

    let from = Location::Gift {
        account_id: account.id,
        slot,
    };
    let to = Location::Inventory {
        character_id: character.id,
        slot: 4,
    };

    store.move_item(from, to, Some(item)).await.unwrap();

    // The second take finds nothing, because the first removed it.
    assert!(store.move_item(from, to, Some(item)).await.is_err());
    assert!(store.gifts(account.id).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_goes_into_a_gift_chest() {
    // A chest that took deposits would be vault space nobody paid for.
    let Some(store) = store("t_gift_oneway").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), "Fesal", 100)
        .await
        .unwrap();

    let item = uuid::Uuid::new_v4();
    store.give_item(character.id, item, 4, 11).await.unwrap();

    let outcome = store
        .move_item(
            Location::Inventory {
                character_id: character.id,
                slot: 4,
            },
            Location::Gift {
                account_id: account.id,
                slot: 0,
            },
            Some(item),
        )
        .await;

    assert!(outcome.is_err(), "the gift chest took a deposit");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_death_is_recorded_and_the_character_is_marked_dead_together() {
    // A character marked dead with no death recorded loses the only account of what happened to it,
    // and a death recorded against a living character is a graveyard entry for somebody playing.
    let Some(store) = store("t_death_record").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), "Fesal", 100)
        .await
        .unwrap();

    assert!(!store.has_died_before(account.id).await.unwrap());

    store
        .record_death(Death {
            account_id: account.id,
            character_id: character.id,
            killed_by: "Slime".to_string(),
            final_fame: 250,
            first_born: true,
            bonuses: Vec::new(),
        })
        .await
        .unwrap();

    assert!(!store.character(character.id).await.unwrap().alive);
    assert!(store.has_died_before(account.id).await.unwrap());

    let graveyard = store.graveyard(account.id, 10).await.unwrap();
    assert_eq!(graveyard.len(), 1);
    assert_eq!(graveyard[0].killed_by, "Slime");
    assert_eq!(graveyard[0].final_fame, 250);
    assert!(graveyard[0].first_born);

    // The fame a character finished with is the account's to keep.
    assert_eq!(store.account(account.id).await.unwrap().fame, 250);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_character_cannot_die_twice() {
    // Two worlds both deciding they killed the same person would put one character in the graveyard
    // twice and pay its fame twice.
    let Some(store) = store("t_death_twice").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), "Fesal", 100)
        .await
        .unwrap();

    let death = Death {
        account_id: account.id,
        character_id: character.id,
        killed_by: "Slime".to_string(),
        final_fame: 100,
        first_born: false,
        bonuses: Vec::new(),
    };

    store.record_death(death.clone()).await.unwrap();
    assert!(store.record_death(death).await.is_err(), "it died twice");

    assert_eq!(store.graveyard(account.id, 10).await.unwrap().len(), 1);
    assert_eq!(store.account(account.id).await.unwrap().fame, 100);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_deaths_racing_on_one_character_resolve_to_one() {
    let Some(store) = store("t_death_race").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), "Fesal", 100)
        .await
        .unwrap();

    let attempts: Vec<_> = (0..8)
        .map(|_| {
            let store = store.clone();
            let death = Death {
                account_id: account.id,
                character_id: character.id,
                killed_by: "Slime".to_string(),
                final_fame: 50,
                first_born: false,
                bonuses: Vec::new(),
            };
            tokio::spawn(async move { store.record_death(death).await })
        })
        .collect();

    let mut recorded = 0;
    for attempt in attempts {
        if attempt.await.unwrap().is_ok() {
            recorded += 1;
        }
    }

    assert_eq!(recorded, 1, "{recorded} of eight attempts were recorded");
    assert_eq!(store.graveyard(account.id, 10).await.unwrap().len(), 1);
    assert_eq!(
        store.account(account.id).await.unwrap().fame,
        50,
        "paid twice"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_death_outlives_the_character_it_happened_to() {
    // A graveyard that emptied itself when somebody tidied up would be no graveyard at all.
    let Some(store) = store("t_death_outlives").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), "Fesal", 100)
        .await
        .unwrap();

    store
        .record_death(Death {
            account_id: account.id,
            character_id: character.id,
            killed_by: "Slime".to_string(),
            final_fame: 10,
            first_born: false,
            bonuses: Vec::new(),
        })
        .await
        .unwrap();

    store
        .delete_character(account.id, character.id)
        .await
        .unwrap();

    let graveyard = store.graveyard(account.id, 10).await.unwrap();
    assert_eq!(graveyard.len(), 1, "the death went with the character");
    assert_eq!(graveyard[0].name, "Fesal");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guild_keeps_what_its_members_finished_with() {
    let Some(store) = store("t_death_guild").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let guild = store.found_guild(account.id, "The Quiet").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), "Fesal", 100)
        .await
        .unwrap();

    store
        .record_death(Death {
            account_id: account.id,
            character_id: character.id,
            killed_by: "Slime".to_string(),
            final_fame: 300,
            first_born: false,
            bonuses: Vec::new(),
        })
        .await
        .unwrap();

    assert_eq!(store.guild(guild.id).await.unwrap().fame, 300);
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_character_did_adds_up_across_sessions() {
    // Added rather than set, so a session that ends without saving loses what it did and not what
    // every earlier session did.
    let Some(store) = store("t_tally_adds").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), "Fesal", 100)
        .await
        .unwrap();

    let session = hendra_store::TallyRow {
        shots: 100,
        shots_that_hit: 40,
        god_kills: 2,
        dungeons_completed: 0b0000_0011,
        ..Default::default()
    };

    store.add_tally(character.id, &session).await.unwrap();
    store.add_tally(character.id, &session).await.unwrap();

    let held = store.tally(character.id).await.unwrap();
    assert_eq!(held.shots, 200);
    assert_eq!(held.shots_that_hit, 80);
    assert_eq!(held.god_kills, 4);

    // Dungeons are a set rather than a count: finishing the same two twice is still two kinds.
    assert_eq!(held.dungeons_completed, 0b0000_0011);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_death_keeps_the_bonuses_that_explain_its_number() {
    let Some(store) = store("t_death_bonuses").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), "Fesal", 100)
        .await
        .unwrap();

    store
        .record_death(Death {
            account_id: account.id,
            character_id: character.id,
            killed_by: "Oryx".to_string(),
            final_fame: 421,
            first_born: true,
            bonuses: vec!["Ancestor: 10".to_string(), "Sniper: 31".to_string()],
        })
        .await
        .unwrap();

    // The best a character has ever finished with, which is what first born is measured against.
    assert_eq!(store.best_final_fame(account.id).await.unwrap(), 421);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_best_death_is_what_first_born_is_measured_against() {
    // Against the graveyard rather than anything living: a character still alive has not finished,
    // and a number that could still go up is not a record.
    let Some(store) = store("t_first_born").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    assert_eq!(store.best_final_fame(account.id).await.unwrap(), 0);

    for fame in [100, 500, 200] {
        let character = store
            .create_character(account.id, uuid::Uuid::nil(), "Fesal", 100)
            .await
            .unwrap();

        store
            .record_death(Death {
                account_id: account.id,
                character_id: character.id,
                killed_by: "Slime".to_string(),
                final_fame: fame,
                first_born: false,
                bonuses: Vec::new(),
            })
            .await
            .unwrap();
    }

    assert_eq!(store.best_final_fame(account.id).await.unwrap(), 500);
}
