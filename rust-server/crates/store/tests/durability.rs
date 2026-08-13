//! Tests against a real Postgres.
//!
//! These need a database, because the properties they check are properties of the database: row
//! locking, transaction isolation, and what two connections racing each other actually do. A mock
//! would test that the mock agrees with itself.
//!
//! Set `HENDRA_TEST_DATABASE` to point at one. Without it the tests skip rather than fail, so a
//! machine with no Postgres can still run the rest of the suite.

use hendra_store::{Currency, Location, Offer, Purchase, Store, StoreError};

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
