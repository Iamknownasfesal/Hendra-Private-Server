//! One account's vault, as the panel addresses it.
//!
//! # This panel is not how the original does it
//!
//! Cited line numbers below are into the 2020 tree, reachable as
//! `git show 94615c4:Server-Side/<path>`. Read them there and not in the working tree: the working
//! tree's `Vault.cs` was rewritten by this project and its `ClosedVaultChest.cs` deleted.
//!
//! In the original the vault is a private `World` per account (`realm/worlds/logic/Vault.cs:15-50`)
//! furnished with real entities. `InitVault` (`:61-145`) walks the map, collects every
//! `TileRegion.Vault` tile, sorts them by distance from spawn, and stands one eight-slot `Container`
//! of object type `0x0504` on the nearest `VaultCount` of them (`:96-107`); every remaining Vault
//! tile gets a `ClosedVaultChest` you buy by walking into (`:108-113`). Moving an item is then an
//! ordinary `InvSwap` between two entities within one tile of each other
//! (`networking/handlers/InvSwapHandler.cs:36-137`, `:166-169`) — there is no vault packet at all,
//! and each chest saves itself through its own `Inventory.InventoryChanged` hook (`Vault.cs:102`,
//! `:183-191`).
//!
//! **This module is a different design, and the panel, its three messages and the version counter
//! below have no 2020 counterpart.** They are this project's, and are flagged rather than defended:
//! whether to keep them or go back to chest entities in a room is a decision, not a fact recovered
//! from the reference. What *is* taken from the reference is every quantity and every refusal —
//! eight slots to a chest, four hundred fame a chest, the ceiling, the free count, the slot-type
//! audit and the backpack rule — each cited to the pristine line that fixes it.
//!
//! # Where the version lives
//!
//! There is nothing to quote in the original: two clients on one account each get their own `Vault`
//! world (`Vault.cs:23-28`, `AllowedAccess` at `:47-50`) and the chest entities in them are separate
//! objects over the same redis fields, which is a race the original simply has. Here the account
//! lock admits one session at a time and the counter is that session's: a move quoting a version the
//! server has moved past is refused and answered with the truth. Ours, and stricter.
//!
//! # What is persisted
//!
//! One row per occupied slot in `vault_slot`, keyed by account and a flat slot number. The chest
//! numbering the panel draws in is worked out on the way past and stored nowhere: chest `i` owns
//! flat slots `i * 8` through `i * 8 + 7`. The original stores the same shape one chest to a redis
//! field — `DbVaultSingle` writes `vault.<i>` under key `vault.<accountId>`
//! (`common/DbModels.cs:875-891`), and `Vault.cs:98` hands chest `i` index `i`.
//!
//! How many chests an account starts with is the `vault_chests` default in
//! `0032_vault_chest_default.sql`, which is one: `<VaultCount>1</VaultCount>` under `<NewAccounts>`
//! (`XmlDatas/data/init.xml:37`), read into `NewAccounts.VaultCount`
//! (`common/resources/AppSettings.cs:71`, `:84`) and written by `Database.Register`
//! (`common/Database.cs:90`). Confirmed against the pristine app server: a freshly registered
//! account there has `vaultCount 1` in redis.

use hendra_net::message::{ServerMessage, VAULT_SLOT_EMPTY, vault_chest};
use hendra_store::Location;

use crate::session::{Context, EQUIPPED_SLOTS, last_carried_slot, number, read_slot, stacks_as};

/// How many slots one vault chest holds.
///
/// The `Vault Chest` the original injects into its own world's object XML declares
/// `<SlotTypes>0, 0, 0, 0, 0, 0, 0, 0</SlotTypes>` (`realm/worlds/logic/Vault.cs:39`) — eight slots,
/// each of type 0, which accepts anything. `Vault.Tick` names each chest "`n`/8" from the same
/// number (`:155`).
pub const SLOTS_PER_CHEST: i16 = 8;

/// The most chests one account may ever own.
///
/// The original writes no ceiling down. `Database.CreateChest` increments `vaultCount` without
/// bound (`common/Database.cs:905-911`) and `ClosedVaultChest.Buy` never compares it to anything;
/// the ceiling is the *map*, because a chest you cannot see is a chest you cannot use.
/// `InitVault` places one chest per `TileRegion.Vault` tile and stops when the tiles run out
/// (`Vault.cs:96`), and the only thing you can click to buy another is a `ClosedVaultChest` standing
/// on a leftover one (`:108-113`). `XmlDatas/worlds/Vault.jm` in the 2020 tree has **80** tiles in
/// the `Vault` region, so eighty chests is where the original runs out of both floor and things to
/// click.
pub const MAX_CHESTS: i16 = 80;

/// What the next chest costs.
///
/// `<VaultChestCost>400</VaultChestCost>` (`XmlDatas/data/init.xml:7`), which
/// `ClosedVaultChest`'s constructor reads straight into `Price`, with `Currency` set to
/// `CurrencyType.Fame` (`realm/entities/vendors/ClosedVaultChest.cs:14-16`) — whatever the packet's
/// own field name says.
pub const CHEST_COST: i32 = 400;

/// The first flat player slot that only a backpack buys.
///
/// The client lays the character's slots out as one run: worn first, then carried, then the
/// backpack. Sixteen is where the backpack starts, and sixteen is the number the original tests:
/// `ValidateSlotSwap` refuses a swap touching a slot at or past it unless `player.HasBackpack`
/// (`networking/handlers/InvSwapHandler.cs:177`).
const FIRST_BACKPACK_SLOT: i16 = hendra_net::slot::FIRST_BACKPACK_SLOT as i16;

/// What the whole vault is, in the order the panel draws it.
///
/// No counterpart: the original has nothing to snapshot, because the chests *are* the state the
/// client sees and it learns them as ordinary entities entering its sight
/// (`Vault.cs:96-107`, `:147-167` for the "`n`/8" label). This is sent whole rather than as a delta
/// for the reason the panel exists at all — a client told everything cannot drift out of step with
/// the server, and drifting out of step is what duplicates items.
pub async fn snapshot(
    context: &Context,
    account_id: i64,
    version: u32,
) -> ServerMessage<'static> {
    let chests = chest_count(context, account_id).await;

    let mut slots = vec![VAULT_SLOT_EMPTY; chests as usize * SLOTS_PER_CHEST as usize];
    for (slot, item) in context.store.vault(account_id).await.unwrap_or_default() {
        let Some(kind) = number(&context.catalog, item) else {
            continue;
        };
        if let Some(place) = usize::try_from(slot).ok().filter(|at| *at < slots.len()) {
            slots[place] = kind;
        }
    }

    let gifts = gift_list(context, account_id)
        .await
        .into_iter()
        .map(|(_, kind)| kind)
        .collect();

    ServerMessage::VaultUpdate {
        version,
        chest_count: chests.max(0) as u32,
        max_chests: MAX_CHESTS as u32,
        next_chest_price: CHEST_COST.max(0) as u32,
        slots,
        gifts,
    }
}

/// How many chests the account owns, bounded by what it may ever own.
///
/// Read from the store rather than from the session's copy, because a purchase changes it mid-play
/// and a stale count is one that either hides a chest that was paid for or offers slots that do not
/// exist.
async fn chest_count(context: &Context, account_id: i64) -> i16 {
    context
        .store
        .account(account_id)
        .await
        .map(|account| account.vault_chests)
        .unwrap_or(0)
        .clamp(0, MAX_CHESTS)
}

/// The gifts waiting, as the panel sees them: dense, in the order their rows are stored.
///
/// Each carries the row it came from as well as the item type. The panel counts from nought with no
/// holes, and the row it must delete to claim one is whatever slot that gift actually occupies.
///
/// All of them, where the original shows at most thirty-two at a time: it deals `Account.Gifts`
/// eight to a `GiftChest` and there are four `Gifting_Chest` tiles in the 2020 `Vault.jm`, so a
/// thirty-third gift waits for the next visit (`realm/worlds/logic/Vault.cs:115-130`). That ceiling
/// is a property of the floor plan, and there is no floor plan here; a gift held back would be a
/// gift nothing draws.
async fn gift_list(context: &Context, account_id: i64) -> Vec<(i16, u16)> {
    context
        .store
        .gifts(account_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(slot, item)| Some((slot, number(&context.catalog, item)?)))
        .collect()
}

/// Whether a move changed anything, when it was not refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    /// The vault changed, so the version moves on and everybody is told.
    Applied,

    /// Nothing to do — both ends were empty, so no transaction is opened and the version does not
    /// move. The original swaps two nulls and reports success the same way
    /// (`networking/handlers/InvSwapHandler.cs:93-96`, `:111`).
    Nothing,
}

/// Moves an item, either within the vault or between it and the player's own inventory.
///
/// The original's `InvSwapHandler.Handle` (`networking/handlers/InvSwapHandler.cs:36-137`), which
/// is what a vault move is there — the vault chest is a `Container` entity and the packet names it
/// by object id. The refusals kept here are the ones that survive losing the entities: the
/// backpack bound and the two-way slot-type audit (`:174-180`). The ones that do not are about
/// where things are standing — `ValidateEntities` wants both containers within one tile
/// (`:166-169`) and the bag owner to be this account (`:152-160`) — and the panel answers both by
/// construction, since it only ever addresses the caller's own vault.
///
/// A swap always, so nothing is ever removed from one place and added to another as two steps —
/// those two steps are what duplicates an item when the second one fails. The original is a swap
/// for the same reason (`:89-96`), and reverts both halves together if the write fails (`:130`).
#[allow(clippy::too_many_arguments)]
pub async fn try_move(
    context: &Context,
    player: &crate::accounts::Session,
    version: u32,
    quoted: u32,
    from_chest: i16,
    from_slot: i16,
    to_chest: i16,
    to_slot: i16,
) -> Result<Moved, &'static str> {
    // Stale by version: the client computed this move against a vault that has since changed.
    // Refusing and re-sending is the whole of the concurrency story — the loser is told what the
    // vault actually is and can ask again.
    if quoted != version {
        return Err("the vault has changed since you looked");
    }

    // A gift is claimed rather than swapped: there is nothing to send back the other way, and the
    // destination slot has to be empty for that reason.
    if from_chest == vault_chest::GIFTS {
        return claim_gift(context, player, from_slot, to_chest, to_slot).await;
    }

    if to_chest == vault_chest::GIFTS {
        return Err("gifts only come out");
    }

    // A potion stack is a destination and not a container: nothing comes back the other way, so it
    // cannot go through the swap below.
    if to_chest == vault_chest::STACKS {
        return stack(context, player, from_chest, from_slot, to_slot).await;
    }

    let chests = chest_count(context, player.account.id).await;
    let carried = last_carried_slot(&context.store, player.character.id).await;

    let who = Who::of(player);
    let source = resolve(who, chests, carried, from_chest, from_slot);
    let destination = resolve(who, chests, carried, to_chest, to_slot);
    let (Some(source), Some(destination)) = (source, destination) else {
        return Err("there is no such slot");
    };

    if source == destination {
        return Ok(Moved::Nothing);
    }

    let holding = read_slot(&context.store, source).await;
    let displaced = read_slot(&context.store, destination).await;

    // Nothing to do, and worth saying so before opening a transaction for it.
    if holding.is_none() && displaced.is_none() {
        return Ok(Moved::Nothing);
    }

    // Slot types, which only ever say anything on the player's side: a vault chest takes anything,
    // and a ring does not go in a weapon slot.
    if !accepts(context, player, destination, holding) || !accepts(context, player, source, displaced)
    {
        return Err("that does not go there");
    }

    match context
        .store
        .move_item(source, destination, holding)
        .await
    {
        Ok(_) => Ok(Moved::Applied),
        Err(hendra_store::StoreError::Refused(_)) => Err("that item is no longer where you left it"),
        Err(err) => {
            tracing::warn!(%err, "a vault move failed");
            Err("that could not be done")
        }
    }
}

/// Turns one end of a move into the durable slot it names, or refuses it.
///
/// The chest is bounded against how many the account owns and the slot against a chest's eight.
/// The original needs neither bound because there is nothing to bound: the only chests you can
/// address are the ones standing in the room, and `InitVault` stands up exactly `VaultCount` of them
/// (`realm/worlds/logic/Vault.cs:96`). Chest `i` here means the same field it means there — chest
/// `i` is `new DbVaultSingle(account, i)`, which is redis field `vault.<i>`
/// (`Vault.cs:98`, `common/DbModels.cs:875-879`).
///
/// The player's own end is bounded against the slots that character actually has, which is what
/// makes the backpack unreachable without one — `ValidateSlotSwap` at
/// `networking/handlers/InvSwapHandler.cs:177`. Ours has no separate `HasBackpack` test because the
/// last carried slot already answers it.
fn resolve(who: Who, chests: i16, last_carried: i16, chest: i16, slot: i16) -> Option<Location> {
    if chest == vault_chest::PLAYER {
        return player_slot(who, last_carried, slot);
    }

    if chest < 0 || chest >= chests || slot < 0 || slot >= SLOTS_PER_CHEST {
        return None;
    }

    Some(Location::Vault {
        account_id: who.account_id,
        slot: chest * SLOTS_PER_CHEST + slot,
    })
}

/// Whose vault and whose pack a move is about.
///
/// The two ids rather than the session they come from, so the numbering below can be tested without
/// standing up an account and a character to ask it about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Who {
    account_id: i64,
    character_id: i64,
}

impl Who {
    fn of(player: &crate::accounts::Session) -> Who {
        Who {
            account_id: player.account.id,
            character_id: player.character.id,
        }
    }
}

/// Turns a flat player slot, as the panel counts them, into the durable one it means.
///
/// The panel counts worn slots from nought with room for eight of them, carried slots from eight and
/// the backpack from sixteen. The table holds four worn and then everything else, so the carried run
/// moves down by four and the gap the panel leaves is not a slot at all.
///
/// The numbering itself is `hendra_net::slot`, which the client's extension turns the same flat
/// numbers through when a drag happens outside this panel: one description of where a slot is,
/// rather than one here and one there to disagree later.
fn player_slot(who: Who, last_carried: i16, slot: i16) -> Option<Location> {
    let durable = u16::try_from(slot)
        .ok()
        .and_then(hendra_net::slot::flat_to_durable)?;

    // The backpack is only addressable if the character has one, exactly as `ValidateSlotSwap` has
    // it: `slotA < 16 && slotB < 16 || player.HasBackpack`
    // (`networking/handlers/InvSwapHandler.cs:177`). Without this a client reaches eight slots it
    // has not bought. The last carried slot already answers it: a character without a backpack has
    // none past eleven, and the panel's sixteenth slot is the twelfth of the table.
    debug_assert!(slot < FIRST_BACKPACK_SLOT || durable >= 12);
    if durable > last_carried {
        return None;
    }

    Some(Location::Inventory {
        character_id: who.character_id,
        slot: durable,
    })
}

/// Whether a slot takes what is being put into it.
///
/// Only a worn slot ever says no: a vault chest takes anything — its eight slots are all
/// `SlotTypes` 0 (`realm/worlds/logic/Vault.cs:39`) — and so does a carried slot. `ValidateSlotSwap`
/// asks `AuditItem` of both ends for the same reason
/// (`networking/handlers/InvSwapHandler.cs:178-179`), which is why this is called twice.
///
/// The chest also declares `<CanPutSoulboundObjects/>` (`Vault.cs:36`), so a soulbound item stores
/// like any other; the original's separate soulbound guard is about *public* bags and exempts a
/// container whose sole owner is this account (`InvSwapHandler.cs:182-198`), which a vault chest
/// always is (`Vault.cs:100`).
fn accepts(
    context: &Context,
    player: &crate::accounts::Session,
    at: Location,
    item: Option<uuid::Uuid>,
) -> bool {
    let Location::Inventory { slot, .. } = at else {
        return true;
    };
    if slot >= EQUIPPED_SLOTS as i16 {
        return true;
    }

    let kind = item
        .and_then(|item| context.catalog.type_of_uuid(item))
        .unwrap_or(hendra_content::ObjectType::NONE);

    crate::session::worn_slot_accepts(context, player, slot, kind)
}

/// Claims a gift into the player's inventory.
///
/// The original claims a gift by swapping it out of a `GiftChest` entity like any other item and
/// then deleting the account's gift row (`networking/handlers/InvSwapHandler.cs:111-127`,
/// `common/Database.cs` `RemoveGift`); the chest is a live copy of the rows, filled eight at a time
/// from `Account.Gifts` when the world is built (`realm/worlds/logic/Vault.cs:115-130`), and an
/// emptied one is replaced by scenery with a `giftChestEmpty` notification (`:193-209`).
///
/// **Ours orders the two writes the other way round, deliberately.** With no chest entity to hold
/// the item in the meantime, the row *is* the gift, so removing it is what makes the claim real and
/// it goes first; a failure after it puts the gift back. The original's order — item first, row
/// second — is safe only because the item has already left the in-memory chest, and a client that
/// disconnects between the two keeps both. That is a divergence, and this one is not a match to
/// anything.
async fn claim_gift(
    context: &Context,
    player: &crate::accounts::Session,
    index: i16,
    to_chest: i16,
    to_slot: i16,
) -> Result<Moved, &'static str> {
    let gifts = gift_list(context, player.account.id).await;
    let Some((row, _)) = usize::try_from(index)
        .ok()
        .and_then(|at| gifts.get(at))
        .copied()
    else {
        return Err("there is no such gift");
    };

    if to_chest != vault_chest::PLAYER {
        return Err("gifts go into your pack");
    }

    let carried = last_carried_slot(&context.store, player.character.id).await;
    let Some(Location::Inventory { slot, .. }) = player_slot(Who::of(player), carried, to_slot) else {
        return Err("gifts go into your pack");
    };

    let Some(identity) = context
        .store
        .gifts(player.account.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|(at, _)| *at == row)
        .map(|(_, item)| item)
    else {
        return Err("there is no such gift");
    };

    if !accepts(context, player, Location::Inventory { character_id: player.character.id, slot }, Some(identity))
    {
        return Err("that does not go there");
    }

    // Conditional on the gift still being in that row, which is what makes two requests for the
    // same gift resolve to one item rather than two.
    if context
        .store
        .take_gift(player.account.id, row, identity)
        .await
        .is_err()
    {
        return Err("that gift is already claimed");
    }

    // The slot has to be free: there is nothing to send back the other way. `give_item` bounded to
    // one slot places it there or refuses.
    if context
        .store
        .give_item(player.character.id, identity, slot, slot)
        .await
        .is_err()
    {
        // The gift is already off the account, so putting it back is the only honest thing to do —
        // an item that vanished between two writes is the failure this ordering exists to avoid.
        let _ = context.store.add_gift(player.account.id, identity).await;
        return Err("that slot is taken");
    }

    Ok(Moved::Applied)
}

/// Puts a potion straight from the vault into one of the character's stacks.
///
/// The original does this through the same `InvSwap` packet: when the destination is the player and
/// the destination slot is one of `player.Stacks`, `InvSwapHandler` takes the stacking branch and
/// never reaches the swap (`networking/handlers/InvSwapHandler.cs:56-76`). The stacks are the
/// counters beside the health and magic bars, and they are the one destination in this game that is
/// not a slot: an item put there stops being an item and becomes a number. So this cannot be the
/// swap the rest of the vault is built on, and it is the one move here that takes from one place and
/// gives to another as two steps.
///
/// It takes first. If the second step refuses, the potion goes back where it came from; that
/// ordering is the difference between briefly losing sight of a potion and briefly having two of it.
async fn stack(
    context: &Context,
    player: &crate::accounts::Session,
    from_chest: i16,
    from_slot: i16,
    which: i16,
) -> Result<Moved, &'static str> {
    if which != 0 && which != 1 {
        return Err("there is no such stack");
    }
    let magic = which == 1;

    let chests = chest_count(context, player.account.id).await;
    let carried = last_carried_slot(&context.store, player.character.id).await;
    let Some(source) = resolve(Who::of(player), chests, carried, from_chest, from_slot) else {
        return Err("there is no such slot");
    };
    let Location::Vault { slot, .. } = source else {
        // Only the vault sends a potion this way. The pack has its own gesture for it.
        return Err("there is no such slot");
    };

    let Some(identity) = read_slot(&context.store, source).await else {
        return Err("there is nothing there");
    };

    let Some(kind) = context.catalog.type_of_uuid(identity) else {
        return Err("that is not a potion");
    };
    if stacks_as(&context.catalog, kind) != Some(magic) {
        return Err("that is not the right potion");
    }

    if context
        .store
        .take_vault_slot(player.account.id, slot, identity)
        .await
        .is_err()
    {
        return Err("that item is no longer where you left it");
    }

    match context.store.add_potion(player.character.id, magic).await {
        Ok(true) => Ok(Moved::Applied),
        // It said yes a moment ago, or the stack filled up in between. Hand it back rather than let
        // it fall between the two.
        other => {
            let _ = context
                .store
                .set_vault_slot(player.account.id, slot, identity)
                .await;

            if matches!(other, Ok(false)) {
                Err("that stack is full")
            } else {
                Err("that could not be done")
            }
        }
    }
}

/// Buys one more chest.
///
/// `ClosedVaultChest.Buy` (`realm/entities/vendors/ClosedVaultChest.cs:19-53`): validate the
/// customer, then increment `vaultCount` and debit the fame in one redis transaction, then hand the
/// new chest back. `ValidateCustomer` refuses a test map, then an insufficient rank, then
/// insufficient funds (`realm/entities/vendors/SellableObject.cs:94-104`); of those only the purse
/// means anything here, since a `RankReq` is never set on a vault chest and there is no test map to
/// stand in.
///
/// The count the client sent is checked against the count the server has, so a second click
/// arriving while the first is still being paid for buys nothing — the original's own guard against
/// that is that the entity you clicked is removed from the world when it becomes a chest
/// (`realm/worlds/logic/Vault.cs:177-178`). The price and the purse are read here and never sent by
/// the client, as the original reads them off the entity and the account (`ClosedVaultChest.cs:15`,
/// `SellableObject.cs:100`).
pub async fn try_buy(
    context: &Context,
    player: &crate::accounts::Session,
    believed: u32,
) -> Result<(), &'static str> {
    let chests = chest_count(context, player.account.id).await;

    if believed as i64 != chests as i64 {
        return Err("you have already bought that chest");
    }

    if chests >= MAX_CHESTS {
        return Err("you cannot own any more chests");
    }

    // Paid for and created together, in one transaction, as the original's does: if it does not go
    // through, no chest is added and nothing is charged.
    match context
        .store
        .buy_vault_chest(player.account.id, MAX_CHESTS, CHEST_COST)
        .await
    {
        Ok(_) => Ok(()),
        Err(hendra_store::StoreError::Refused(reason)) => Err(reason),
        Err(err) => {
            tracing::warn!(%err, "a vault chest could not be bought");
            Err("that could not be done")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A character without a backpack: four worn and eight carried, slots 0 to 11.
    const NO_BACKPACK: i16 = 11;

    /// A character with one, which buys eight more.
    const BACKPACK: i16 = 19;

    const WHO: Who = Who {
        account_id: 7,
        character_id: 3,
    };

    fn vault_at(slot: i16) -> Option<Location> {
        Some(Location::Vault {
            account_id: WHO.account_id,
            slot,
        })
    }

    fn pack_at(slot: i16) -> Option<Location> {
        Some(Location::Inventory {
            character_id: WHO.character_id,
            slot,
        })
    }

    #[test]
    fn a_chest_and_a_slot_become_one_flat_vault_slot() {
        // Chest `i` owns eight slots starting at `i * 8`, which is the numbering the rows are
        // stored under. The original keeps the same chest number: `new DbVaultSingle(account, i)`
        // for the `i`th chest placed (`realm/worlds/logic/Vault.cs:96-98`), which is redis field
        // `vault.<i>` (`common/DbModels.cs:875-879`).
        assert_eq!(resolve(WHO, 4, NO_BACKPACK, 0, 0), vault_at(0));
        assert_eq!(resolve(WHO, 4, NO_BACKPACK, 0, 7), vault_at(7));
        assert_eq!(resolve(WHO, 4, NO_BACKPACK, 1, 0), vault_at(8));
        assert_eq!(resolve(WHO, 4, NO_BACKPACK, 3, 7), vault_at(31));
    }

    #[test]
    fn a_chest_nobody_paid_for_is_refused() {
        // The bound is how many the account owns, so a client cannot store things in slots that do
        // not exist and nothing draws.
        assert_eq!(resolve(WHO, 4, NO_BACKPACK, 4, 0), None);
        assert_eq!(resolve(WHO, 0, NO_BACKPACK, 0, 0), None);

        // The ceiling is how many chests the original's own map has floor for: `XmlDatas/worlds/
        // Vault.jm` at `94615c4` marks eighty tiles `Vault`, and `InitVault` places one chest per
        // tile until they run out (`realm/worlds/logic/Vault.cs:96`).
        assert_eq!(MAX_CHESTS, 80);
        assert_eq!(
            resolve(WHO, MAX_CHESTS, NO_BACKPACK, MAX_CHESTS - 1, 7),
            vault_at(MAX_CHESTS * SLOTS_PER_CHEST - 1)
        );
        assert_eq!(resolve(WHO, MAX_CHESTS, NO_BACKPACK, MAX_CHESTS, 0), None);
    }

    #[test]
    fn a_slot_past_the_eighth_is_not_in_that_chest() {
        // Otherwise chest nought slot eight would name chest one slot nought, and every chest would
        // be an alias for every chest after it.
        assert_eq!(resolve(WHO, 4, NO_BACKPACK, 0, 8), None);
        assert_eq!(resolve(WHO, 4, NO_BACKPACK, 0, -1), None);
    }

    #[test]
    fn the_panels_flat_player_slots_land_where_the_table_holds_them() {
        // The panel counts worn from nought with room for eight, and carried from eight. The table
        // holds four worn and then everything else, so the carried run moves down by four.
        assert_eq!(player_slot(WHO, NO_BACKPACK, 0), pack_at(0));
        assert_eq!(player_slot(WHO, NO_BACKPACK, 3), pack_at(3));
        assert_eq!(player_slot(WHO, NO_BACKPACK, 8), pack_at(4));
        assert_eq!(player_slot(WHO, NO_BACKPACK, 15), pack_at(11));
    }

    #[test]
    fn the_room_the_panel_leaves_for_worn_slots_we_do_not_have_is_not_a_slot() {
        // Four to seven are drawn by nothing and held by nothing. Letting them through would put an
        // item in the first four carried squares by another name.
        for slot in EQUIPPED_SLOTS as i16..hendra_net::slot::FIRST_CARRIED_SLOT as i16 {
            assert_eq!(player_slot(WHO, NO_BACKPACK, slot), None, "slot {slot}");
        }
        assert_eq!(player_slot(WHO, NO_BACKPACK, -1), None);
    }

    #[test]
    fn the_backpack_is_only_addressable_with_a_backpack() {
        // `ValidateSlotSwap` refuses a swap touching a slot at or past the sixteenth to a character
        // without one (`networking/handlers/InvSwapHandler.cs:177`). Without this a client reaches
        // eight slots it has not bought, and what it puts there is drawn by nothing and can be lost.
        for slot in FIRST_BACKPACK_SLOT..FIRST_BACKPACK_SLOT + 8 {
            assert_eq!(player_slot(WHO, NO_BACKPACK, slot), None, "slot {slot}");
        }

        assert_eq!(player_slot(WHO, BACKPACK, 16), pack_at(12));
        assert_eq!(player_slot(WHO, BACKPACK, 23), pack_at(19));
        assert_eq!(player_slot(WHO, BACKPACK, 24), None);
    }

}
