//! Tests against a real Postgres.
//!
//! These need a database, because the properties they check are properties of the database: row
//! locking, transaction isolation, and what two connections racing each other actually do. A mock
//! would test that the mock agrees with itself.
//!
//! Set `HENDRA_TEST_DATABASE` to point at one. Without it the tests skip rather than fail, so a
//! machine with no Postgres can still run the rest of the suite.

use hendra_store::{Location, Store, StoreError};

/// A store with a schema of its own, or `None` when no database is configured.
///
/// Each test gets a distinct schema so they can run in parallel without seeing each other's rows —
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

const WAND: i32 = 0x900;
const ROBE: i32 = 0x901;

#[tokio::test(flavor = "multi_thread")]
async fn an_account_and_character_survive_a_round_trip() {
    let Some(store) = store("t_roundtrip").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    assert_eq!(account.vault_chests, 4, "everyone starts with four chests");

    let character = store
        .create_character(account.id, 0x0300, "Wizard", 800)
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
        .create_character(account.id, 0x0300, "Wizard", 800)
        .await
        .unwrap();

    store.set_inventory(character.id, &[(0, WAND)]).await.unwrap();

    let from = Location::Inventory {
        character_id: character.id,
        slot: 0,
    };
    let to = Location::Vault {
        account_id: account.id,
        slot: 3,
    };

    let outcome = store.move_item(from, to, WAND).await.unwrap();
    assert_eq!(outcome.source, 0, "the inventory slot is now empty");
    assert_eq!(outcome.destination, WAND);

    assert!(store.character(character.id).await.unwrap().inventory.is_empty());
    assert_eq!(store.vault(account.id).await.unwrap(), vec![(3, WAND)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_occupied_slots_swap() {
    let Some(store) = store("t_swap").await else {
        return;
    };

    let account = store.create_account("Swapper").await.unwrap();
    let character = store
        .create_character(account.id, 0x0300, "Wizard", 800)
        .await
        .unwrap();

    store.set_inventory(character.id, &[(0, WAND)]).await.unwrap();
    store.set_vault_slot(account.id, 0, ROBE).await.unwrap();

    let inventory = Location::Inventory {
        character_id: character.id,
        slot: 0,
    };
    let vault = Location::Vault {
        account_id: account.id,
        slot: 0,
    };

    store.move_item(inventory, vault, WAND).await.unwrap();

    assert_eq!(store.character(character.id).await.unwrap().inventory, vec![(0, ROBE)]);
    assert_eq!(store.vault(account.id).await.unwrap(), vec![(0, WAND)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_something_that_is_not_there_is_refused() {
    let Some(store) = store("t_stale").await else {
        return;
    };

    let account = store.create_account("Stale").await.unwrap();
    let character = store
        .create_character(account.id, 0x0300, "Wizard", 800)
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
        store.move_item(from, to, WAND).await,
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
        .create_character(account.id, 0x0300, "Wizard", 800)
        .await
        .unwrap();

    for attempt in 0..25 {
        let slot_a = (attempt * 2) as i16;
        let slot_b = slot_a + 1;

        store.set_inventory(character.id, &[(0, WAND)]).await.unwrap();

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
                        WAND,
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
                        WAND,
                    )
                    .await
            })
        };

        let (first, second) = (one.await.unwrap(), two.await.unwrap());
        let winners = [first.is_ok(), second.is_ok()].iter().filter(|ok| **ok).count();

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
            store.set_vault_slot(account.id, slot, 0).await.unwrap();
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
        .create_character(account.id, 0x0300, "One", 800)
        .await
        .unwrap();
    let two = store
        .create_character(account.id, 0x0300, "Two", 800)
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
            tokio::spawn(async move { store.move_item(a, b, WAND).await })
        };
        let backward = {
            let store = store.clone();
            tokio::spawn(async move { store.move_item(b, a, ROBE).await })
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
        .create_character(account.id, 0x0300, "Wizard", 800)
        .await
        .unwrap();

    store.set_inventory(character.id, &[(4, WAND), (6, ROBE)]).await.unwrap();

    assert_eq!(store.give_item(character.id, WAND, 4, 11).await.unwrap().slot, 5);
    assert_eq!(store.give_item(character.id, WAND, 4, 11).await.unwrap().slot, 7);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_inventory_refuses_an_item() {
    let Some(store) = store("t_full").await else {
        return;
    };

    let account = store.create_account("Full").await.unwrap();
    let character = store
        .create_character(account.id, 0x0300, "Wizard", 800)
        .await
        .unwrap();

    let packed: Vec<(i16, i32)> = (4..=11).map(|slot| (slot, WAND)).collect();
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
        .create_character(account.id, 0x0300, "Wizard", 800)
        .await
        .unwrap();

    for _ in 0..20 {
        // Everything full except slot 11.
        let packed: Vec<(i16, i32)> = (4..=10).map(|slot| (slot, WAND)).collect();
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
        let winners = [first.is_ok(), second.is_ok()].iter().filter(|ok| **ok).count();

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
        .create_character(account.id, 0x0300, "Wizard", 800)
        .await
        .unwrap();

    store.set_inventory(character.id, &[(4, WAND)]).await.unwrap();

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
        .create_character(account.id, 0x0300, "Wizard", 800)
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
    assert_eq!(store.account_by_name("Buyer").await.unwrap().vault_chests, 6);
}
