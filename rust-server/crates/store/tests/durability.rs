//! Tests against a real Postgres.
//!
//! These need a database, because the properties they check are properties of the database: row
//! locking, transaction isolation, and what two connections racing each other actually do. A mock
//! would test that the mock agrees with itself.
//!
//! Set `HENDRA_TEST_DATABASE` to point at one. Without it the tests skip rather than fail, so a
//! machine with no Postgres can still run the rest of the suite.

use hendra_store::{
    Admin, Awarded, Currency, Death, DyeSlot, Location, MarketPurchase, Offer, Purchase, Rank,
    Saved, Store, StoreError,
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
    assert_eq!(
        account.vault_chests, 1,
        "a new account owns one chest, as <VaultCount>1</VaultCount> in init.xml says"
    );

    let character = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();
    assert_eq!(character.hp, 800);
    assert!(character.alive);

    let reloaded = store.character(character.id).await.unwrap();
    assert_eq!(reloaded, character);

    let listed = store.characters(account.id).await.unwrap();
    assert_eq!(listed.len(), 1);

    // `Player.cs:423` is `Name = client.Account.Name;`, so this is the account's name and not one
    // the character was given.
    assert_eq!(listed[0].name, "Fesal");
    assert_eq!(character.name, "Fesal");
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
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

/// Four more identities, so a run of drags can tell every item apart.
const TOME: uuid::Uuid = uuid::Uuid::from_u128(0x902);
const RING: uuid::Uuid = uuid::Uuid::from_u128(0x903);
const POTION: uuid::Uuid = uuid::Uuid::from_u128(0x904);
const CLOAK: uuid::Uuid = uuid::Uuid::from_u128(0x905);

/// Every item this account and character hold anywhere, sorted, so two moments can be compared.
///
/// An item that was moved appears once either way; one that was duplicated appears twice and one
/// that was lost does not appear at all. That is the whole property.
async fn everything(store: &Store, account: i64, character: i64) -> Vec<uuid::Uuid> {
    let mut held: Vec<uuid::Uuid> = store
        .character(character)
        .await
        .unwrap()
        .inventory
        .into_iter()
        .map(|(_, item)| item)
        .chain(
            store
                .vault(account)
                .await
                .unwrap()
                .into_iter()
                .map(|(_, item)| item),
        )
        .collect();

    held.sort();
    held
}

/// Carries out a drag the way the game asks for one: two flat slots, and nothing else.
///
/// The flat numbers are what the client counts in and `hendra_net::slot` is what turns them into
/// the slots the table holds, which is the same route a real drag takes. Naming durable slots here
/// would test the database and not the thing that was wrong.
async fn drag(
    store: &Store,
    character: i64,
    from_flat: u16,
    to_flat: u16,
) -> Result<(), StoreError> {
    let (Some(from_slot), Some(to_slot)) = (
        hendra_net::slot::flat_to_durable(from_flat),
        hendra_net::slot::flat_to_durable(to_flat),
    ) else {
        return Err(StoreError::Refused("there is no such slot"));
    };

    let from = Location::Inventory {
        character_id: character,
        slot: from_slot,
    };
    let to = Location::Inventory {
        character_id: character,
        slot: to_slot,
    };

    // What the session reads before it moves anything: the item the client is about to be told it
    // moved. A move naming something else is refused rather than applied.
    let expected = store
        .character(character)
        .await
        .unwrap()
        .inventory
        .into_iter()
        .find(|(at, _)| *at == from_slot)
        .map(|(_, item)| item);

    store.move_item(from, to, expected).await.map(|_| ())
}

/// What is in a flat slot, as the player sees it.
async fn at_flat(store: &Store, character: i64, flat: u16) -> Option<uuid::Uuid> {
    let slot = hendra_net::slot::flat_to_durable(flat)?;
    store
        .character(character)
        .await
        .unwrap()
        .inventory
        .into_iter()
        .find(|(at, _)| *at == slot)
        .map(|(_, item)| item)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drag_moves_the_item_that_was_dragged() {
    // The bug this is here for: the client counts carried slots from eight and the table holds them
    // from four, and a drag that sent the flat number straight through landed four slots along --
    // flat eight moved whatever was in flat twelve. So two items are laid out four apart and the
    // first is dragged; the second must not move.
    let Some(store) = store("t_drag_lands").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Dragger").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();

    // Flat eight is the first square of the pack, flat twelve the fifth.
    store
        .set_inventory(character.id, &[(4, WAND), (8, ROBE)])
        .await
        .unwrap();

    drag(&store, character.id, 8, 9).await.unwrap();

    assert_eq!(
        at_flat(&store, character.id, 9).await,
        Some(WAND),
        "the wand went where it was dragged"
    );
    assert_eq!(
        at_flat(&store, character.id, 12).await,
        Some(ROBE),
        "and the robe four squares along was left alone"
    );
    assert_eq!(at_flat(&store, character.id, 8).await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_run_of_drags_conserves_every_item() {
    // The strongest statement this side of the wire: whatever the sequence of drags, accepted or
    // refused, the items the account holds are exactly the ones it started with. Item loss and item
    // duplication are the two failures worth more than every other bug here put together.
    let Some(store) = store("t_conserved").await else {
        return;
    };

    let account = store.create_account("Keeper").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();

    // Two worn, four carried, one in the vault.
    store
        .set_inventory(
            character.id,
            &[(0, WAND), (1, TOME), (4, ROBE), (5, RING), (7, POTION)],
        )
        .await
        .unwrap();
    store.set_vault_slot(account.id, 0, CLOAK).await.unwrap();

    let before = everything(&store, account.id, character.id).await;
    assert_eq!(before.len(), 6);

    // Ordinary drags: within the pack, out to a worn slot, and back again.
    for (from, to) in [
        (8u16, 10u16),
        (10, 11),
        (11, 0),
        (0, 11),
        (9, 15),
        (15, 9),
        (1, 9),
        (9, 1),
    ] {
        let _ = drag(&store, character.id, from, to).await;
        assert_eq!(
            everything(&store, account.id, character.id).await,
            before,
            "after dragging {from} onto {to}"
        );
    }

    // Refusals. Each of these is a request a client can make and the server must not carry out: a
    // square the layout leaves room for and this game does not have, a number past every slot a
    // character could own, a slot onto itself, and the two stack numbers, which are not slots in
    // the pack at all.
    for (from, to) in [(8u16, 5u16), (5, 8), (8, 24), (8, 8), (8, 254), (255, 8)] {
        assert!(
            drag(&store, character.id, from, to).await.is_err(),
            "dragging {from} onto {to} should be refused"
        );
        assert_eq!(
            everything(&store, account.id, character.id).await,
            before,
            "after a refused drag from {from} to {to}"
        );
    }

    // A drag naming an item that is not there any more, which is what two clients racing looks
    // like from the second one's point of view.
    let source = Location::Inventory {
        character_id: character.id,
        slot: 4,
    };
    let destination = Location::Inventory {
        character_id: character.id,
        slot: 5,
    };
    assert!(matches!(
        store.move_item(source, destination, Some(WAND)).await,
        Err(StoreError::Refused(_))
    ));
    assert_eq!(everything(&store, account.id, character.id).await, before);

    // And to the vault and back, which is the other place an item can be.
    let vault = Location::Vault {
        account_id: account.id,
        slot: 0,
    };
    let carried = at_flat(&store, character.id, 8).await;
    store.move_item(source, vault, carried).await.unwrap();
    assert_eq!(everything(&store, account.id, character.id).await, before);

    store.move_item(source, vault, Some(CLOAK)).await.unwrap();
    assert_eq!(everything(&store, account.id, character.id).await, before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drag_abandoned_half_way_leaves_every_item_where_it_can_be_found() {
    // What a disconnect does to a move in flight: the session's future is dropped, and with it the
    // transaction. The move either happened or it did not, and either way the item exists exactly
    // once -- an item that was taken from one slot and never written to the other is the shape of
    // every lost item report.
    let Some(store) = store("t_abandoned").await else {
        return;
    };

    let account = store.create_account("Vanisher").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();

    store
        .set_inventory(character.id, &[(4, WAND), (5, ROBE)])
        .await
        .unwrap();

    let before = everything(&store, account.id, character.id).await;

    let from = Location::Inventory {
        character_id: character.id,
        slot: 4,
    };
    let to = Location::Inventory {
        character_id: character.id,
        slot: 5,
    };

    // Cut off at every point a move can be cut off at: a nanosecond in, then a microsecond, and so
    // on past the point where the whole thing completes.
    for cut in [1u64, 10, 100, 1_000, 10_000, 100_000, 1_000_000] {
        let expected = store
            .character(character.id)
            .await
            .unwrap()
            .inventory
            .into_iter()
            .find(|(at, _)| *at == 4)
            .map(|(_, item)| item);

        let _ = tokio::time::timeout(
            std::time::Duration::from_nanos(cut),
            store.move_item(from, to, expected),
        )
        .await;

        assert_eq!(
            everything(&store, account.id, character.id).await,
            before,
            "after a move cut off {cut} nanoseconds in"
        );
    }
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();
    let two = store
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
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
async fn a_consumable_with_more_uses_in_it_turns_into_the_next_one() {
    let Some(store) = store("t_succeed").await else {
        return;
    };

    let account = store.create_account("Drinker").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();

    store
        .set_inventory(character.id, &[(4, WAND)])
        .await
        .unwrap();

    store
        .succeed_item(character.id, 4, WAND, ROBE)
        .await
        .unwrap();

    let holding = store.character(character.id).await.unwrap().inventory;
    assert_eq!(
        holding
            .iter()
            .find(|(slot, _)| *slot == 4)
            .map(|(_, item)| *item),
        Some(ROBE),
        "the slot did not become the successor"
    );

    // Conditional on what is there for the same reason a removal is: two simultaneous uses of one
    // elixir should spend one charge rather than two.
    assert!(matches!(
        store.succeed_item(character.id, 4, WAND, ROBE).await,
        Err(StoreError::Refused(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_backpack_is_worth_the_slots_it_promises() {
    // A backpack was granted, stored and read by nobody, so it bought nothing. What it is for is
    // the eight slots past the twelve a character starts with.
    let Some(store) = store("t_backpack").await else {
        return;
    };

    let account = store.create_account("Hoarder").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();

    assert!(!store.has_backpack(character.id).await.unwrap());

    // Fill every slot a character has without one, which is four worn and eight carried.
    let held: Vec<(i16, uuid::Uuid)> = (0..12).map(|slot| (slot, WAND)).collect();
    store.set_inventory(character.id, &held).await.unwrap();

    assert!(
        matches!(
            store.give_item(character.id, ROBE, 4, 11).await,
            Err(StoreError::Refused(_))
        ),
        "a full inventory took another item"
    );

    assert!(store.grant_backpack(character.id).await.unwrap());
    assert!(store.has_backpack(character.id).await.unwrap());

    let placed = store.give_item(character.id, ROBE, 4, 19).await.unwrap();
    assert_eq!(placed.slot, 12, "the first slot a backpack buys");

    // And it is granted once. A second one is refused rather than silently taking the item.
    assert!(!store.grant_backpack(character.id).await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_character_saves_and_dies() {
    let Some(store) = store("t_save").await else {
        return;
    };

    let account = store.create_account("Saver").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();

    let levelled = hendra_store::Saved {
        hp: 500,
        mp: 40,
        max_hp: 800,
        max_mp: 100,
        level: 12,
        experience: 3400,
        fame: 25,
        stats: [800, 100, 30, 20, 15, 25, 35, 45],
    };

    store
        .save_character(character.id, &levelled, None)
        .await
        .unwrap();

    let reloaded = store.character(character.id).await.unwrap();
    assert_eq!(reloaded.hp, 500);
    assert_eq!(reloaded.level, 12);
    assert_eq!(reloaded.fame, 25);
    assert_eq!(
        reloaded.stats,
        levelled.stats.map(Some).to_vec(),
        "the eight base stats persist, every one of them recorded"
    );

    // The maxima are written from the same body, so a character that has levelled into more health
    // does not come back to a row still claiming what it started with.
    let grown = hendra_store::Saved {
        hp: 670,
        max_hp: 670,
        max_mp: 385,
        stats: [670, 385, 40, 25, 20, 30, 40, 50],
        ..levelled
    };
    store
        .save_character(character.id, &grown, None)
        .await
        .unwrap();

    let reloaded = store.character(character.id).await.unwrap();
    assert_eq!(reloaded.hp, 670);
    assert_eq!(reloaded.max_hp, 670);
    assert_eq!(reloaded.max_mp, 385);
    assert_eq!(reloaded.stats, grown.stats.map(Some).to_vec());

    // Health above the maximum is clamped by the query rather than trusted from the caller, and
    // health below one is floored, which is what `Player.cs:371` does with `Math.Max(1, HP)`.
    store
        .save_character(
            character.id,
            &hendra_store::Saved {
                hp: 99_999,
                ..grown
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(store.character(character.id).await.unwrap().hp, 670);

    store
        .save_character(character.id, &hendra_store::Saved { hp: 0, ..grown }, None)
        .await
        .unwrap();
    assert_eq!(store.character(character.id).await.unwrap().hp, 1);

    store.kill_character(character.id).await.unwrap();
    assert!(!store.character(character.id).await.unwrap().alive);
    assert!(
        store.characters(account.id).await.unwrap().is_empty(),
        "the dead do not appear in character select"
    );
}

/// A snapshot taken before a handover, written after it, against a lock that has moved on.
///
/// The delay is injected rather than waited for, so the losing order is the one the test runs
/// every time instead of the one it hopes to catch.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_from_a_session_that_lost_the_lock_cannot_undo_a_newer_one() {
    let Some(store) = store("t_write_order").await else {
        return;
    };

    let account = store.create_account("Overtaken").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();

    let base = Saved {
        hp: 400,
        mp: 40,
        max_hp: 800,
        max_mp: 100,
        level: 12,
        experience: 3400,
        fame: 25,
        stats: [800, 100, 30, 20, 15, 25, 35, 45],
    };

    // The first session, playing, with what it saw at the moment its checkpoint was computed.
    let first = store.acquire_lock(account.id).await.unwrap().unwrap();
    let stale = Saved { hp: 425, ..base };

    // It stops answering for long enough to lose the account, which is the one way another session
    // can take it while this one still believes it is playing.
    sqlx::query(
        "UPDATE account_lock SET expires_at = now() - interval '1 second' WHERE account_id = $1",
    )
    .bind(account.id)
    .execute(store.pool())
    .await
    .unwrap();

    // The second session takes the account and plays on, writing what the character is now.
    let second = store.acquire_lock(account.id).await.unwrap().unwrap();
    let fresh = Saved {
        hp: 700,
        level: 14,
        experience: 9000,
        fame: 60,
        ..base
    };
    assert!(
        store
            .save_character(character.id, &fresh, Some(&second))
            .await
            .unwrap(),
        "the session holding the lock writes"
    );
    assert!(
        store
            .add_tally(
                character.id,
                &hendra_store::TallyRow {
                    shots: 3,
                    ..Default::default()
                },
                Some(&second),
            )
            .await
            .unwrap(),
        "the session holding the lock counts what it did"
    );

    // Only now does the first session get round to its write. It is the older figure, and it
    // arrives last.
    assert!(
        !store
            .save_character(character.id, &stale, Some(&first))
            .await
            .unwrap(),
        "a session that no longer holds the account may not write to it"
    );

    let row = store.character(character.id).await.unwrap();
    assert_eq!(row.hp, 700, "a stale checkpoint undid a newer save");
    assert_eq!(row.level, 14);
    assert_eq!(row.experience, 9000);
    assert_eq!(row.fame, 60);

    // Nor may it add to what the character has done: the counts would land on a row that now
    // belongs to somebody else's session.
    let did = hendra_store::TallyRow {
        shots: 7,
        ..Default::default()
    };
    assert!(
        !store
            .add_tally(character.id, &did, Some(&first))
            .await
            .unwrap(),
        "a session that no longer holds the account may not add to its tally"
    );
    assert_eq!(
        store.tally(character.id).await.unwrap().shots,
        3,
        "only what the session holding the account did is counted"
    );

    // Nor may its abandoned body bury the character. A death is the one write nothing else can
    // undo, so it is held to the lock like any other.
    assert!(
        store
            .record_death(
                Death {
                    account_id: account.id,
                    character_id: character.id,
                    killed_by: "a body nobody was driving".to_string(),
                    final_fame: 60,
                    first_born: false,
                    bonuses: Vec::new(),
                },
                Some(&first),
            )
            .await
            .is_err(),
        "a session that no longer holds the account may not bury its character"
    );
    assert!(
        store.character(character.id).await.unwrap().alive,
        "a character somebody is playing was put in the graveyard"
    );

    // And a session that has released its lock at the end of its own shutdown is in the same
    // position: its final save was its last, and nothing it does afterwards lands.
    store.release_lock(&second).await.unwrap();
    assert!(
        !store
            .save_character(character.id, &stale, Some(&second))
            .await
            .unwrap(),
        "a session that has ended may not write after its final save"
    );
    assert_eq!(store.character(character.id).await.unwrap().hp, 700);
}

/// The same losing order, run as two sessions at once rather than one after the other.
///
/// The delay sits where the original's does not exist at all — between a checkpoint reading the
/// body and the write reaching the database — because that gap is where a handover fits, and a
/// hundred milliseconds of it makes the race happen on every run instead of on an unlucky one.
#[tokio::test(flavor = "multi_thread")]
async fn a_checkpoint_delayed_past_a_handover_loses_to_the_session_that_took_the_account() {
    let Some(store) = store("t_write_race").await else {
        return;
    };

    let account = store.create_account("Handed Over").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();

    let base = Saved {
        hp: 425,
        mp: 40,
        max_hp: 800,
        max_mp: 100,
        level: 12,
        experience: 3400,
        fame: 25,
        stats: [800, 100, 30, 20, 15, 25, 35, 45],
    };

    let leaving = store.acquire_lock(account.id).await.unwrap().unwrap();

    let checkpoint = {
        let store = store.clone();
        tokio::spawn(async move {
            // What the body was when this checkpoint read it, held while the account changes hands.
            let seen = Saved { hp: 288, ..base };
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            store
                .save_character(character.id, &seen, Some(&leaving))
                .await
                .unwrap()
        })
    };

    // The handover happens inside that gap: the leaving session finishes and gives the lock back,
    // and the next one takes it and plays.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    store.release_lock(&leaving).await.unwrap();
    let arriving = store.acquire_lock(account.id).await.unwrap().unwrap();
    let played = Saved {
        hp: 780,
        level: 13,
        experience: 6100,
        ..base
    };
    assert!(
        store
            .save_character(character.id, &played, Some(&arriving))
            .await
            .unwrap()
    );

    assert!(
        !checkpoint.await.unwrap(),
        "a checkpoint from the session that left wrote after the one that replaced it"
    );

    let row = store.character(character.id).await.unwrap();
    assert_eq!(row.hp, 780, "health went backwards after a handover");
    assert_eq!(row.level, 13);
    assert_eq!(row.experience, 6100);
}

/// Giving a character up puts the whole of it back to the starting line.
#[tokio::test(flavor = "multi_thread")]
async fn prestige_returns_a_character_to_what_its_class_starts_with() {
    let Some(store) = store("t_prestige_reset").await else {
        return;
    };

    let account = store.create_account("Given Up").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, 100)
        .await
        .unwrap();

    let grown = Saved {
        hp: 670,
        mp: 385,
        max_hp: 670,
        max_mp: 385,
        level: 20,
        experience: 250_000,
        fame: 3200,
        stats: [670, 385, 75, 25, 50, 75, 60, 80],
    };
    store
        .save_character(character.id, &grown, None)
        .await
        .unwrap();

    // A wizard's starting line.
    let starting = [100, 100, 12, 12, 12, 12, 12, 12];
    assert_eq!(
        store
            .prestige(account.id, character.id, &starting)
            .await
            .unwrap(),
        2,
        "three thousand two hundred fame is two prestige"
    );

    let row = store.character(character.id).await.unwrap();
    assert_eq!(row.level, 1);
    assert_eq!(row.experience, 0);
    assert_eq!(row.fame, 0);
    assert_eq!(
        row.stats,
        starting.map(Some).to_vec(),
        "a level-one character kept a level-twenty body"
    );
    assert_eq!((row.max_hp, row.max_mp), (100, 100));
    assert_eq!((row.hp, row.mp), (100, 100));
}

#[tokio::test(flavor = "multi_thread")]
async fn vault_chests_are_bought_up_to_a_limit() {
    let Some(store) = store("t_chests").await else {
        return;
    };

    let account = store.create_account("Buyer").await.unwrap();
    store
        .credit(account.id, Currency::Fame, 1200)
        .await
        .unwrap();

    assert_eq!(store.buy_vault_chest(account.id, 3, 400).await.unwrap(), 2);
    assert_eq!(store.buy_vault_chest(account.id, 3, 400).await.unwrap(), 3);

    // The limit is enforced by the update's own WHERE clause, so two simultaneous purchases cannot
    // both see room and both take it.
    assert!(matches!(
        store.buy_vault_chest(account.id, 3, 400).await,
        Err(StoreError::Refused(_))
    ));
    assert_eq!(
        store.account_by_name("Buyer").await.unwrap().vault_chests,
        3
    );
}

// -- trade ------------------------------------------------------------------------------------

/// Two characters on one account, each holding one item.
async fn traders(store: &Store, schema_name: &str) -> (i64, i64) {
    let account = store.create_account(schema_name).await.unwrap();
    let one = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();
    let two = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();

    store.set_inventory(one.id, &[(4, WAND)]).await.unwrap();
    store.set_inventory(two.id, &[(4, ROBE)]).await.unwrap();
    (one.id, two.id)
}

#[tokio::test(flavor = "multi_thread")]
async fn each_side_of_a_trade_has_its_own_room() {
    // One trader has a backpack and the other does not, which is the case one range for the whole
    // trade cannot express: the wrong one either loses the extra slots or puts an item where the
    // other player has nowhere to keep it.
    let Some(store) = store("t_trade_room").await else {
        return;
    };
    let (roomy, cramped) = traders(&store, "Rooms").await;

    store.grant_backpack(roomy).await.unwrap();

    // The cramped one is full to the last slot it has, and the roomy one has its backpack empty.
    let full: Vec<(i16, uuid::Uuid)> = (0..12).map(|slot| (slot, ROBE)).collect();
    store.set_inventory(cramped, &full).await.unwrap();
    store.set_inventory(roomy, &[(4, WAND)]).await.unwrap();

    // The cramped side gives up nothing, so it has nowhere to put what it is offered.
    let outcome = store
        .trade(
            &Offer::new(roomy, vec![(4, WAND)], 19),
            &Offer::new(cramped, Vec::new(), 11),
            4,
        )
        .await;

    assert!(
        matches!(outcome, Err(StoreError::Refused(_))),
        "an item was placed in a slot the receiving player does not have"
    );

    // And the same trade with the roomy player receiving goes through, so what was refused above
    // was the room and not the trade.
    store.set_inventory(cramped, &full).await.unwrap();
    store
        .trade(
            &Offer::new(roomy, Vec::new(), 19),
            &Offer::new(cramped, vec![(4, ROBE)], 11),
            4,
        )
        .await
        .unwrap();
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
            &Offer::new(one, vec![(4, WAND)], 11),
            &Offer::new(two, vec![(4, ROBE)], 11),
            4,
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
            &Offer::new(one, vec![(4, WAND)], 11),
            &Offer::new(two, vec![(4, WAND)], 11),
            4,
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
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();
    let two = store
        .create_character(account.id, WIZARD, 800)
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
            &Offer::new(one.id, Vec::new(), 11),
            &Offer::new(two.id, vec![(4, ROBE), (5, ROBE)], 11),
            4,
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
            &Offer::new(one, vec![(4, WAND)], 11),
            &Offer::new(two, Vec::new(), 11),
            4,
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
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();
    let buyer_one = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();
    let buyer_two = store
        .create_character(account.id, WIZARD, 800)
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
                        &Offer::new(seller.id, vec![(4, WAND)], 11),
                        &Offer::new(buyer_one.id, Vec::new(), 11),
                        4,
                    )
                    .await
            })
        };
        let second = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .trade(
                        &Offer::new(seller.id, vec![(4, WAND)], 11),
                        &Offer::new(buyer_two.id, Vec::new(), 11),
                        4,
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(seller.id, WIZARD, 800)
        .await
        .unwrap();
    let buyer_character = store.create_character(buyer.id, WIZARD, 800).await.unwrap();

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

    // The seller is paid the whole price. `PlayerMerchant` pays `Price - Tax`, and a merchant the
    // market created has no `Tax`: it is only ever set from a map property.
    assert_eq!(store.account(seller).await.unwrap().gold, 100);
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
            .create_character(seller.id, WIZARD, 800)
            .await
            .unwrap();
        let first = store.create_character(one.id, WIZARD, 800).await.unwrap();
        let second = store.create_character(two.id, WIZARD, 800).await.unwrap();

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
            .create_character(seller.id, WIZARD, 800)
            .await
            .unwrap();
        let theirs = store.create_character(buyer.id, WIZARD, 800).await.unwrap();

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
            store.cancel_listing(seller.id, listing, 0)
        );

        let winners = [sold.is_ok(), cancelled.is_ok()]
            .iter()
            .filter(|ok| **ok)
            .count();
        assert_eq!(
            winners, 1,
            "sold or returned, never both (attempt {attempt})"
        );

        // A withdrawal puts the wand in the gift chest rather than back in the pack, so the two
        // places it can have ended up are the buyer's inventory and the seller's gifts.
        let wands = store.character(stock.id).await.unwrap().inventory.len()
            + store.character(theirs.id).await.unwrap().inventory.len()
            + store.gifts(seller.id).await.unwrap().len();
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

    let (seller, seller_character, buyer, _) = market(&store).await;
    let listing = store
        .list_item(seller, seller_character, 4, WAND, Currency::Fame, 100)
        .await
        .unwrap();

    assert!(
        store.cancel_listing(buyer, listing, 0).await.is_err(),
        "somebody else's listing is not yours to cancel"
    );

    // `db.AddGift(acc, shopItem.ItemId, trans)`: it goes to the gift chest, not the pack, which is
    // why the original follows a withdrawal with `giftChestOccupied`.
    let taken = store.cancel_listing(seller, listing, 0).await.unwrap();
    assert_eq!(taken.item, WAND);
    assert_eq!(taken.fee, 0, "the last thing listed comes back for nothing");

    assert!(
        store
            .gifts(seller)
            .await
            .unwrap()
            .iter()
            .any(|(_, item)| *item == WAND)
    );
    assert!(
        store
            .character(seller_character)
            .await
            .unwrap()
            .inventory
            .is_empty(),
        "and not back into the pack"
    );
    assert!(store.listings(10).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_back_anything_but_the_last_listing_costs_five_fame() {
    // `if (acc.LastMarketId != shopItemId) t1 = db.UpdateCurrency(acc, -5, CurrencyType.Fame)`.
    let Some(store) = store("t_market_fee").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let (seller, seller_character, _, _) = market(&store).await;
    store
        .set_inventory(seller_character, &[(4, WAND), (5, WAND)])
        .await
        .unwrap();
    store.credit(seller, Currency::Fame, 7).await.unwrap();

    let first = store
        .list_item(seller, seller_character, 4, WAND, Currency::Fame, 100)
        .await
        .unwrap();
    let second = store
        .list_item(seller, seller_character, 5, WAND, Currency::Fame, 100)
        .await
        .unwrap();

    // The newest is the one `LastMarketId` names, so it comes back free.
    assert_eq!(
        store.cancel_listing(seller, second, 0).await.unwrap().fee,
        0
    );
    assert_eq!(store.account(seller).await.unwrap().fame, 7);

    // And the older one costs five, even though nothing is newer than it still for sale.
    assert_eq!(store.cancel_listing(seller, first, 0).await.unwrap().fee, 5);
    assert_eq!(store.account(seller).await.unwrap().fame, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_fee_is_checked_before_the_listing_exists() {
    // The original reads the fame before it looks the listing up, so somebody who cannot pay is
    // told about the fee even when the id names nothing at all.
    let Some(store) = store("t_market_fee_order").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let (seller, _, _, _) = market(&store).await;

    let Err(hendra_store::StoreError::Refused(why)) =
        store.cancel_listing(seller, 999_999, 0).await
    else {
        panic!("a listing that does not exist was withdrawn");
    };
    assert_eq!(
        why,
        "Not enough fame. There is a 5 fame fee to remove that item."
    );

    // With the fee affordable, the same id is answered by the listing itself.
    store.credit(seller, Currency::Fame, 5).await.unwrap();
    let Err(hendra_store::StoreError::Refused(why)) =
        store.cancel_listing(seller, 999_999, 0).await
    else {
        panic!("a listing that does not exist was withdrawn");
    };
    assert!(why.starts_with("Market item does not exist."), "{why}");
    assert_eq!(
        store.account(seller).await.unwrap().fame,
        5,
        "and nothing was charged"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_pack_does_not_refuse_a_purchase() {
    // `Merchant.TransactionItem` asks for a slot and calls `AddGift` when there is none, then lets
    // the purchase through. The buyer paid either way.
    let Some(store) = store("t_market_full").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let (seller, seller_character, buyer, buyer_character) = market(&store).await;
    let listing = store
        .list_item(seller, seller_character, 4, WAND, Currency::Gold, 100)
        .await
        .unwrap();

    // Every carried slot taken.
    let full: Vec<(i16, uuid::Uuid)> = (4..=11).map(|slot| (slot, WIZARD)).collect();
    store.set_inventory(buyer_character, &full).await.unwrap();

    let landed = store
        .buy_listing(MarketPurchase {
            buyer_id: buyer,
            character_id: buyer_character,
            listing_id: listing,
            first_slot: 4,
            last_slot: 11,
        })
        .await
        .unwrap();

    assert!(matches!(landed, hendra_store::Delivered::ToGifts(_)));
    assert!(
        store
            .gifts(buyer)
            .await
            .unwrap()
            .iter()
            .any(|(_, item)| *item == WAND)
    );
    assert_eq!(
        store.account(buyer).await.unwrap().gold,
        900,
        "and it was paid for"
    );
    assert_eq!(store.account(seller).await.unwrap().gold, 100);
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_a_price_somebody_may_ask() {
    // `/market` reads its price with `^(\d+) (\d+)$` and `AddToMarket` refuses only `price < 0`,
    // so zero goes through.
    let Some(store) = store("t_market_free").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let (seller, seller_character, _, _) = market(&store).await;
    let listing = store
        .list_item(seller, seller_character, 4, WAND, Currency::Fame, 0)
        .await
        .unwrap();

    assert_eq!(store.listings(10).await.unwrap()[0].price, 0);
    assert_eq!(store.last_market_id(seller).await.unwrap(), listing);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_moderator_may_both_mute_and_ban() {
    let Some(store) = store("t_moderation").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let player = store.create_account("Fesal").await.unwrap();
    assert_eq!(Admin::from_number(player.admin_rank), Admin::NONE);
    assert!(!Admin::NONE.may_mute());
    assert!(!Admin::NONE.may_ban());

    store
        .set_admin_rank(player.id, Admin::MODERATOR)
        .await
        .unwrap();
    let moderator = store.account(player.id).await.unwrap();

    // One rank carries both powers. `MuteCommand` and `BanAccountCommand` are each declared at
    // permLevel 80 in the original's RankedCommands, so the rank that can silence is the same rank
    // that can remove.
    assert!(Admin::from_number(moderator.admin_rank).may_mute());
    assert!(Admin::from_number(moderator.admin_rank).may_ban());

    store.set_admin_rank(player.id, Admin::OWNER).await.unwrap();
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
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();

    assert!(
        store
            .wear_skin(account.id, character.id, 7, uuid::Uuid::from_u128(7))
            .await
            .is_err()
    );

    store
        .grant_skin(account.id, uuid::Uuid::from_u128(7))
        .await
        .unwrap();
    store
        .wear_skin(account.id, character.id, 7, uuid::Uuid::from_u128(7))
        .await
        .unwrap();

    // And going back to the class's own appearance never needs owning anything.
    store
        .wear_skin(account.id, character.id, 0, uuid::Uuid::nil())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_backpack_is_granted_once() {
    let Some(store) = store("t_backpack").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, 800)
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
        .create_character(account.id, uuid::Uuid::nil(), 100)
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
async fn a_withdrawal_into_a_full_gift_chest_still_succeeds() {
    // `AddGifts` appends to a list with no limit (`common/Database.cs:1324-1333`), so nothing in
    // the original ever refuses a gift. A ceiling of eight here made a market withdrawal fail that
    // the original completes -- and a listing that cannot be taken back is an item nobody can
    // reach, whether or not the row survives.
    let Some(store) = store("t_gift_full").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), 100)
        .await
        .unwrap();

    // Eight gifts: as many as one chest shows.
    for _ in 0..8 {
        store
            .add_gift(account.id, uuid::Uuid::new_v4())
            .await
            .unwrap();
    }
    assert_eq!(store.gifts(account.id).await.unwrap().len(), 8);

    // A listing, then a withdrawal of it, with the chest already at a chest's worth.
    let wand = uuid::Uuid::new_v4();
    let placed = store
        .give_item(character.id, wand, 4, 11)
        .await
        .expect("the seller holds the wand");

    let listing = store
        .list_item(
            account.id,
            character.id,
            placed.slot,
            wand,
            hendra_store::Currency::Fame,
            100,
        )
        .await
        .unwrap();

    let taken = store
        .cancel_listing(account.id, listing, 0)
        .await
        .expect("a full chest does not refuse a withdrawal");

    assert_eq!(taken.item, wand);
    assert_eq!(
        store.gifts(account.id).await.unwrap().len(),
        9,
        "the ninth waits behind the first eight"
    );

    // And the item is in exactly one place: the listing is closed and the pack is empty.
    assert!(
        store
            .character(character.id)
            .await
            .unwrap()
            .inventory
            .iter()
            .all(|(_, held)| *held != wand)
    );
    assert!(
        store
            .listings_of(account.id)
            .await
            .unwrap()
            .iter()
            .all(|open| open.id != listing)
    );
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
        .create_character(account.id, uuid::Uuid::nil(), 100)
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
        .create_character(account.id, uuid::Uuid::nil(), 100)
        .await
        .unwrap();

    assert!(!store.has_died_before(account.id).await.unwrap());

    store
        .record_death(
            Death {
                account_id: account.id,
                character_id: character.id,
                killed_by: "Slime".to_string(),
                final_fame: 250,
                first_born: true,
                bonuses: Vec::new(),
            },
            None,
        )
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
async fn a_death_keeps_the_fame_a_character_earned_apart_from_what_its_death_came_to() {
    // The original writes the post-bonus total to `FinalFame` and leaves `Fame` as the character
    // earned it (`Database.cs:1090`). Overwriting one with the other makes the two identical, and a
    // death screen that subtracts the bonuses from the total then claims a character that earned
    // nothing but bonuses earned all of it by living.
    let Some(store) = store("t_death_base_fame").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), 100)
        .await
        .unwrap();

    store
        .save_character(
            character.id,
            &Saved {
                hp: 100,
                mp: 100,
                max_hp: 100,
                max_mp: 100,
                level: 20,
                experience: 0,
                fame: 100,
                stats: [0; 8],
            },
            None,
        )
        .await
        .unwrap();

    let bonuses = vec![
        Awarded {
            name: "Ancestor".to_string(),
            fame: 30,
        },
        Awarded {
            name: "Thirsty".to_string(),
            fame: 32,
        },
        Awarded {
            name: "First Born".to_string(),
            fame: 16,
        },
    ];

    store
        .record_death(
            Death {
                account_id: account.id,
                character_id: character.id,
                killed_by: "Lava".to_string(),
                final_fame: 178,
                first_born: true,
                bonuses: bonuses.clone(),
            },
            None,
        )
        .await
        .unwrap();

    // What it earned by living is untouched by dying.
    assert_eq!(store.character(character.id).await.unwrap().fame, 100);

    let graveyard = store.graveyard(account.id, 10).await.unwrap();
    assert_eq!(graveyard[0].final_fame, 178);
    assert_eq!(graveyard[0].bonuses, bonuses);

    // Nothing appears and nothing goes missing between the two: the total is the fame the character
    // had plus every bonus it was paid, and that total is what the account is given.
    let awarded: i32 = graveyard[0].bonuses.iter().map(|bonus| bonus.fame).sum();
    assert_eq!(
        store.character(character.id).await.unwrap().fame + awarded,
        graveyard[0].final_fame
    );
    assert_eq!(store.account(account.id).await.unwrap().fame, 178);
}

#[tokio::test(flavor = "multi_thread")]
async fn spending_fame_leaves_the_fame_the_account_has_earned_alone() {
    // `Database.UpdateFame` (`common/Database.cs:812-833`) raises `totalFame` only when the amount
    // is positive and applies every amount to `fame`, so the character list's `<TotalFame>` is what
    // was ever earned and `<Fame>` is what is left. A debit that reached both would make the pair
    // two names for one number, and an account that had spent everything would read as having
    // never earned anything.
    let Some(store) = store("t_total_fame").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), 100)
        .await
        .unwrap();

    store
        .record_death(
            Death {
                account_id: account.id,
                character_id: character.id,
                killed_by: "Lava".to_string(),
                final_fame: 500,
                first_born: false,
                bonuses: Vec::new(),
            },
            None,
        )
        .await
        .unwrap();

    let earned = store.account(account.id).await.unwrap();
    assert_eq!(earned.fame, 500);
    assert_eq!(earned.total_fame, 500, "a death pays both numbers");

    // A vault chest, bought out of the balance.
    store.buy_vault_chest(account.id, 8, 200).await.unwrap();

    let spent = store.account(account.id).await.unwrap();
    assert_eq!(spent.fame, 300, "the balance is what a purchase comes out of");
    assert_eq!(spent.total_fame, 500, "and the lifetime total does not move");

    // Fame from anywhere else lands on both, as the credit half of `UpdateFame` does.
    store
        .credit(account.id, hendra_store::Currency::Fame, 50)
        .await
        .unwrap();

    let credited = store.account(account.id).await.unwrap();
    assert_eq!(credited.fame, 350);
    assert_eq!(credited.total_fame, 550);
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
        .create_character(account.id, uuid::Uuid::nil(), 100)
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

    store.record_death(death.clone(), None).await.unwrap();
    assert!(
        store.record_death(death, None).await.is_err(),
        "it died twice"
    );

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
        .create_character(account.id, uuid::Uuid::nil(), 100)
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
            tokio::spawn(async move { store.record_death(death, None).await })
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
        .create_character(account.id, uuid::Uuid::nil(), 100)
        .await
        .unwrap();

    store
        .record_death(
            Death {
                account_id: account.id,
                character_id: character.id,
                killed_by: "Slime".to_string(),
                final_fame: 10,
                first_born: false,
                bonuses: Vec::new(),
            },
            None,
        )
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
        .create_character(account.id, uuid::Uuid::nil(), 100)
        .await
        .unwrap();

    store
        .record_death(
            Death {
                account_id: account.id,
                character_id: character.id,
                killed_by: "Slime".to_string(),
                final_fame: 300,
                first_born: false,
                bonuses: Vec::new(),
            },
            None,
        )
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
        .create_character(account.id, uuid::Uuid::nil(), 100)
        .await
        .unwrap();

    let session = hendra_store::TallyRow {
        shots: 100,
        shots_that_hit: 40,
        god_kills: 2,
        dungeons_completed: 0b0000_0011,
        ..Default::default()
    };

    store.add_tally(character.id, &session, None).await.unwrap();
    store.add_tally(character.id, &session, None).await.unwrap();

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
        .create_character(account.id, uuid::Uuid::nil(), 100)
        .await
        .unwrap();

    let bonuses = vec![
        Awarded {
            name: "Ancestor".to_string(),
            fame: 10,
        },
        Awarded {
            name: "Sniper".to_string(),
            fame: 31,
        },
    ];

    store
        .record_death(
            Death {
                account_id: account.id,
                character_id: character.id,
                killed_by: "Oryx".to_string(),
                final_fame: 421,
                first_born: true,
                bonuses: bonuses.clone(),
            },
            None,
        )
        .await
        .unwrap();

    // Read back name for name and number for number, and in the order they were paid: a death
    // screen lists them as they were earned, and each is a share of the ones before it.
    let graveyard = store.graveyard(account.id, 10).await.unwrap();
    assert_eq!(graveyard[0].bonuses, bonuses);

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
            .create_character(account.id, uuid::Uuid::nil(), 100)
            .await
            .unwrap();

        store
            .record_death(
                Death {
                    account_id: account.id,
                    character_id: character.id,
                    killed_by: "Slime".to_string(),
                    final_fame: fame,
                    first_born: false,
                    bonuses: Vec::new(),
                },
                None,
            )
            .await
            .unwrap();
    }

    assert_eq!(store.best_final_fame(account.id).await.unwrap(), 500);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_first_two_characters_an_account_made_are_ancestors() {
    // `character.CharId < 2` (`FameStats.cs:122`), counted rather than read: the original numbers
    // characters per account from zero and ours are numbered across the server, so the position is
    // what has to match. Another account's characters must not push mine down the list.
    let Some(store) = store("t_ancestor").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let mine = store.create_account("Fesal").await.unwrap();
    let theirs = store.create_account("Somebody").await.unwrap();

    let mut made = Vec::new();
    for round in 0..3 {
        made.push(
            store
                .create_character(mine.id, uuid::Uuid::nil(), 100)
                .await
                .unwrap(),
        );

        // Interleaved, so the ids of mine are not consecutive.
        store
            .create_character(theirs.id, uuid::Uuid::nil(), 100)
            .await
            .unwrap();

        let _ = round;
    }

    for (position, character) in made.iter().enumerate() {
        assert_eq!(
            store
                .characters_made_before(mine.id, character.id)
                .await
                .unwrap(),
            position as i64,
            "the {position}th character of an account has that many before it"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_potion_cannot_be_drunk_twice() {
    // Taken durably before it heals, or a potion that heals and is still in the stack heals forever.
    let Some(store) = store("t_potion_drink").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Fesal").await.unwrap();
    let character = store
        .create_character(account.id, uuid::Uuid::nil(), 100)
        .await
        .unwrap();

    store.add_potion(character.id, false).await.unwrap();

    let attempts: Vec<_> = (0..8)
        .map(|_| {
            let store = store.clone();
            tokio::spawn(async move { store.take_potion(character.id, false).await })
        })
        .collect();

    let mut drunk = 0;
    for attempt in attempts {
        if attempt.await.unwrap().unwrap() {
            drunk += 1;
        }
    }

    assert_eq!(drunk, 1, "{drunk} of one potion were drunk");
}

/// One account plays in one place at a time.
///
/// The properties are the database's: two sessions racing for the same account must not both be
/// told they have it, and a session that has been taken over must not be able to release the lock
/// its replacement is holding.
#[tokio::test(flavor = "multi_thread")]
async fn one_session_at_a_time_holds_an_account() {
    let Some(store) = store("t_account_lock").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Locked").await.unwrap();

    let first = store.acquire_lock(account.id).await.unwrap();
    assert!(first.is_some(), "nobody held it");

    assert!(
        store.acquire_lock(account.id).await.unwrap().is_none(),
        "a second session is refused while the first holds it"
    );

    let first = first.unwrap();
    assert!(
        store.renew_lock(&first).await.unwrap(),
        "the holder may renew"
    );

    // What whoever was refused is told: a number they can wait out rather than a flat no.
    let left = store.lock_seconds_left(account.id).await.unwrap();
    assert!(
        left > 0 && left <= hendra_store::LOCK_SECONDS as i64,
        "the wait is quoted in seconds, and was {left}"
    );

    // A lock nobody holds is not a lock. Expiring it is what lets an account be played again after
    // the process holding it has gone.
    sqlx::query(
        "UPDATE account_lock SET expires_at = now() - interval '1 second' WHERE account_id = $1",
    )
    .bind(account.id)
    .execute(store.pool())
    .await
    .unwrap();

    let second = store
        .acquire_lock(account.id)
        .await
        .unwrap()
        .expect("an expired lock is taken over");
    assert_ne!(second.token, first.token, "and by a different token");

    assert!(
        !store.renew_lock(&first).await.unwrap(),
        "the session that was taken over no longer holds anything"
    );

    // The important half: releasing is conditional on the token, so a session finishing its
    // shutdown cannot unlock the one that replaced it.
    store.release_lock(&first).await.unwrap();
    assert!(
        store.acquire_lock(account.id).await.unwrap().is_none(),
        "the replacement still holds the account"
    );

    store.release_lock(&second).await.unwrap();
    assert!(
        store.acquire_lock(account.id).await.unwrap().is_some(),
        "and giving it up properly frees it"
    );
}

/// What a character looks like is part of what a character is.
///
/// The original keeps three numbers on the character for this — `Skin`, `Tex1` and `Tex2`
/// (`common/DbModels.cs:684-696`) — and writes all three from the live player in `SaveToCharacter`
/// (`wServer/realm/entities/player/Player.cs:373-375`). Here the two dyes are `dye_cloth` and
/// `dye_accessory`. All three were written and none of them was ever read back, so a skin somebody
/// had bought and put on could not reach a world however many times they logged in.
#[tokio::test(flavor = "multi_thread")]
async fn what_a_character_wears_is_read_back_with_it() {
    let Some(store) = store("t_appearance").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Dressed").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, 800)
        .await
        .unwrap();

    let fresh = store.character(character.id).await.unwrap();
    assert_eq!(fresh.skin, 0, "a new character wears its class's own look");
    assert_eq!((fresh.dye_cloth, fresh.dye_accessory), (0, 0));

    // A skin nobody owns cannot be worn, so it is granted first, exactly as the reskin path does.
    let skin = 0x2b3;
    store
        .grant_skin(account.id, uuid::Uuid::from_u128(skin as u128))
        .await
        .unwrap();
    store
        .wear_skin(
            account.id,
            character.id,
            skin,
            uuid::Uuid::from_u128(skin as u128),
        )
        .await
        .unwrap();

    // The two dye slots are told apart by the dye's own number, as `DyeSlot::of` reads it.
    let cloth = store.set_dye(character.id, 0x0100_0007).await.unwrap();
    let accessory = store.set_dye(character.id, 0x0200_0009).await.unwrap();
    assert_ne!(cloth, accessory, "the two dyes land in different slots");

    let dressed = store.character(character.id).await.unwrap();
    assert_eq!(dressed.skin, skin, "the skin comes back with the character");
    assert!(dressed.dye_cloth != 0 && dressed.dye_accessory != 0);

    // And a checkpoint does not undo any of it. The character write deliberately leaves appearance
    // alone: it carries a snapshot taken at login, and a skin chosen since would be written back to
    // what it was before.
    store
        .save_character(
            character.id,
            &hendra_store::Saved {
                hp: 500,
                mp: 40,
                max_hp: 800,
                max_mp: 100,
                level: 12,
                experience: 3400,
                fame: 25,
                stats: [800, 100, 30, 20, 15, 25, 35, 45],
            },
            None,
        )
        .await
        .unwrap();

    let after = store.character(character.id).await.unwrap();
    assert_eq!(after.skin, skin, "a checkpoint does not undress anybody");
    assert_eq!(
        (after.dye_cloth, after.dye_accessory),
        (dressed.dye_cloth, dressed.dye_accessory)
    );
}

/// A character that predates the stats column is not read as a character that has never levelled.
///
/// `0027` writes what the row still knows — its two maxima — into their slots and leaves the other
/// six null, which is the difference between "not recorded" and "zero".
#[tokio::test(flavor = "multi_thread")]
async fn a_character_from_before_the_stats_column_keeps_its_maxima() {
    let Some(store) = store("t_stats_backfill").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Old").await.unwrap();
    let character = store
        .create_character(account.id, WIZARD, 670)
        .await
        .unwrap();

    // What the migration found: a levelled character with an empty column.
    sqlx::query(
        "UPDATE character SET level = 20, max_hp = 670, max_mp = 385, stats = '{}' WHERE id = $1",
    )
    .bind(character.id)
    .execute(store.pool())
    .await
    .unwrap();

    sqlx::query(
        "UPDATE character
         SET stats = ARRAY[max_hp, max_mp, NULL, NULL, NULL, NULL, NULL, NULL]::integer[]
         WHERE cardinality(stats) = 0",
    )
    .execute(store.pool())
    .await
    .unwrap();

    let backfilled = store.character(character.id).await.unwrap();
    assert_eq!(backfilled.stats.len(), 8);
    assert_eq!(backfilled.stats[0], Some(670), "health is one of the eight");
    assert_eq!(backfilled.stats[1], Some(385), "and so is magic");
    assert!(
        backfilled.stats[2..].iter().all(Option::is_none),
        "the six the row cannot know are not guessed at"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_vault_chest_is_paid_for_and_created_together() {
    let Some(store) = store("t_buychest").await else {
        eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
        return;
    };

    let account = store.create_account("Payer").await.unwrap();
    assert_eq!(account.vault_chests, 1, "a new account owns one chest");

    // Not enough fame, so neither half happens. `ValidateCustomer` reads the purse before `Buy`
    // creates anything (`wServer/realm/entities/vendors/SellableObject.cs:100-101`); here the
    // balance is a condition on the statement, so two purchases arriving together cannot both see
    // the same coin. Four hundred a chest is `<VaultChestCost>400</VaultChestCost>`
    // (`XmlDatas/data/init.xml:7`).
    assert!(matches!(
        store.buy_vault_chest(account.id, 80, 400).await,
        Err(StoreError::Refused(_))
    ));
    let unchanged = store.account(account.id).await.unwrap();
    assert_eq!(unchanged.vault_chests, 1, "a refusal created a chest");
    assert_eq!(unchanged.fame, 0);

    store.credit(account.id, Currency::Fame, 900).await.unwrap();

    assert_eq!(store.buy_vault_chest(account.id, 80, 400).await.unwrap(), 2);
    let paid = store.account(account.id).await.unwrap();
    assert_eq!(paid.vault_chests, 2);
    assert_eq!(paid.fame, 500, "the chest was not charged for");

    // At the ceiling nothing is created, and nothing is charged for the attempt: the two halves
    // are one transaction, so the fame the first statement took is rolled back with it.
    assert!(matches!(
        store.buy_vault_chest(account.id, 2, 400).await,
        Err(StoreError::Refused(_))
    ));
    let refused = store.account(account.id).await.unwrap();
    assert_eq!(refused.vault_chests, 2);
    assert_eq!(refused.fame, 500, "a refused purchase still charged");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_potion_leaves_the_vault_once_however_often_it_is_asked_for() {
    let Some(store) = store("t_takevault").await else {
        return;
    };

    let account = store.create_account("Drinker").await.unwrap();
    store.set_vault_slot(account.id, 3, WAND).await.unwrap();

    // The first take empties the row.
    store.take_vault_slot(account.id, 3, WAND).await.unwrap();
    assert!(store.vault(account.id).await.unwrap().is_empty());

    // The second finds it gone and is told so, rather than reading an empty slot and carrying on --
    // which is how one potion becomes two in a stack.
    assert!(matches!(
        store.take_vault_slot(account.id, 3, WAND).await,
        Err(StoreError::Refused(_))
    ));

    // And a take naming the wrong item leaves the row where it is.
    store.set_vault_slot(account.id, 3, WAND).await.unwrap();
    assert!(matches!(
        store.take_vault_slot(account.id, 3, ROBE).await,
        Err(StoreError::Refused(_))
    ));
    assert_eq!(store.vault(account.id).await.unwrap(), vec![(3, WAND)]);
}
