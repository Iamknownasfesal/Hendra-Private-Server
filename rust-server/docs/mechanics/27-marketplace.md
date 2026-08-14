# The player market

Read from `realm/Market.cs`, `realm/entities/player/Player.Market.cs`,
`realm/entities/vendors/PlayerMerchant.cs`.

The market is a **player-to-player consignment shop**, not an auction house and not a direct trade.
You hand an item to the server, it goes into a global pool, and a merchant standing on a marked tile
in the Marketplace sells it to whoever walks up. You never meet the buyer.

## Six shelves

Every listed item is sorted into exactly one of six types:

```
Weapon  Ability  Armor  Ring  StatPot  Other
```

`StatPot` is decided first and by behaviour, not slot: an item is a stat potion if it is a potion
**and** any of its activate effects is `IncrementStat`. Everything else goes by `SlotType`, through
`SlotType2ItemType`, and anything whose slot has no mapping falls to `Other`.

## The shelves are ordered, and the order is the price

Each item type keeps a **list sorted by price, ascending**, and within one price by **insert time,
ascending**. Insertion walks the list and stops at the first entry that is more expensive, or the same
price but listed later.

The merchant always sells `shop[0]` — **the cheapest listing of that item type, oldest first among
ties.** That is the entire pricing mechanism: no bidding, no undercutting war beyond being the
cheapest, and listing at the same price as someone else puts you behind them.

The insert returns the index it landed at. **Index 0 means the listing became the new cheapest**, and
that is the only case that touches a live merchant: the merchant already selling that item gets
`TimeLeft = 30000` and a `Reload()`, so the cheaper listing takes over within the reload rather than
after the current one sells out.

## The merchant rotation

Marketplace tiles carry regions, and six of them are the shop slots:

```
Store_9  -> Weapon      Store_12 -> Ring
Store_10 -> Ability     Store_13 -> StatPot
Store_11 -> Armor       Store_14 -> Other
```

Every tile of a shop region becomes a merchant stand. On startup each type's item ids are **shuffled**
and pushed into a FIFO queue; each stand dequeues one id and sells that item until its timer runs
out, then puts the id back on the tail and takes the next.

The stands of one type do not reload in lockstep. Each is given a `ReloadOffset` of `500 * n` where
`n` counts the tiles of that region in map order, so six adjacent weapon stands rotate half a second
apart rather than all at once.

`TimeLeft` is `30000` ms per rotation. Note the commented-out `TimeLeft = Rand.Next(30000, 90000)`:
the original once randomised the initial timer and no longer does, so on a fresh server every stand
of a type starts its first rotation together and only the offsets separate them.

Reload has one more case: **if the item sold out (`itemCount <= 0`) the id is dropped from the type's
set entirely** rather than re-queued, so an item nobody is selling stops occupying a stand. If the
queue is then empty, the stand is removed from the world — the Marketplace visibly empties when there
is nothing to sell.

`_merchants` is keyed by `IntPoint`, and removal recomputes that key from `(int)merchant.X`,
`(int)merchant.Y`. Since merchants are placed at tile centres, the truncation gets back the tile, so
this works — but it is a key derived from a float position rather than kept.

## Listing an item

`AddToMarket(slot, price)` refuses unless: you are in a `Marketplace` world, the market is enabled,
you are not trading, the slot is in range, and the price is at least 0. **Zero is allowed** — a free
listing is legal and sorts to the very front of its shelf.

The item is removed from the inventory in a transaction, and the DB row is created **before** the
inventory transaction is executed. If the inventory transfer then fails, the listing is removed again.
The window between the two is where the item exists in both places.

**The soulbound check is commented out.** In this fork, soulbound items can be listed and therefore
laundered through the market — that is a live hole, not an intentional rule, and our server should not
copy it.

Every listing writes `Account.LastMarketId`, which is what makes the next section's fee free.

## Taking an item back

Removing your own listing costs **5 fame** — unless it is the listing you made most recently
(`LastMarketId`), which is free. That is a mis-click refund, not a general one: list two things and
only the second can be pulled back for nothing.

The item does not go back to the inventory. It is added as a **gift** (`AddGift`), so it arrives in
the gift chest and does not need inventory space at the moment of removal.

`if (acc.Admin && acc.Rank < 100 || ...)` — C# binds `&&` tighter than `||`, so this reads as
"an admin below rank 100, or the market is disabled". An admin of rank 40 cannot remove listings; a
non-admin can. That is almost certainly not what was meant, and it is the kind of thing to write down
rather than reproduce.

## `GetMarketItems` has a dead loop

```csharp
for (var i = items.Length - 1; i >= 0; i--)
    if (items[i].IsLastMarketedItem(Client.Account.LastMarketId))
        break;
return items;
```

The loop body does nothing but `break`, and the full array is returned either way. Dead code in the
original.

## What this server does differently

We have no player market. If one is built, the parts worth carrying are:

- **Six shelves, cheapest-first with oldest-wins ties.** That, plus "the merchant sells `shop[0]`", is
  the whole price mechanism and it is worth keeping exactly.
- **Staggered reload offsets** so a row of stands does not flip together.
- **Sold-out ids leave the rotation** rather than cycling as empty stands.
- **Do not copy** the commented-out soulbound check, and do not copy the admin-rank condition.
- Removal should go to the gift chest, not the inventory, so it cannot fail on a full bag.
