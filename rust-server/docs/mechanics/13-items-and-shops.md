# Item stacks and shops

Read from `realm/ItemStacker.cs`, `realm/entities/vendors/Merchant.cs`, `SellableObject.cs`,
`PlayerMerchant.cs`.

## Potion stacks

Two stacks, at slots **254** and **255**, exported as `HealthStackCount` and `MagicStackCount`.

```
Put(item):  if Count < MaxCount and item is the stack's item -> Count++, item consumed
            otherwise the item comes back
Pull():     if Count > 0 -> Count--, hand out the item
```

A stack holds exactly one kind of item, decided when it is created, and rejects anything else. The
slot numbers are what `Player.UseItem` keys on: a slot beyond the container's length defaults the
slot type to 10, which is the potion slot type, and that is how drinking from a stack passes the
"item belongs in this slot" test.

## Shops reload on a shared twenty-second clock

```
Tick:
  a = TotalElapsedMs % 20000
  if AwaitingReload or (a - delta <= ReloadOffset and a > ReloadOffset):
      if not AwaitingReload and not Rotate: return
      if any player within 2 tiles:  AwaitingReload = true; return
      if BeingPurchased:             AwaitingReload = true; return
      TimeLeft = -1
      Reload()
```

Every merchant in the game shares one twenty-second cycle, and each has a `ReloadOffset` that decides
where in the cycle it fires — so a row of stalls rotates in sequence rather than all at once.

**A merchant will not reload while a player is standing within two tiles of it**, and will not reload
mid-purchase. In both cases it sets `AwaitingReload` and reloads at the first opportunity, which is
why walking away from a stall changes it.

`Rotate` decides whether a merchant rotates its stock at all; one that does not only ever reloads
after being emptied.

## Buying

```
Buy(player):
  if BeingPurchased: refuse                    // one purchase at a time, server-wide per stall
  BeingPurchased = true
  ValidateCustomer(player):
      test map            -> refuse
      Stars < RankReq     -> refuse
      currency < Price    -> refuse
  PurchaseItem(player)
```

The purchase itself is one transaction: deduct the currency, add the tax to the treasury, and move
the item.

**An item bought with no free inventory slot becomes a gift**, added to the account's gift chest
rather than refused. That is a nicer failure than ours, which refuses the purchase.

```
if success and Count != -1 and --Count <= 0: Reload()
```

`Count == -1` means unlimited stock. Otherwise the stall reloads the moment it is emptied.

## Player merchants

`PlayerMerchant` is a `Merchant` bound to one market listing.

- **`RankReq = 2`** in the constructor, so **two stars are needed to buy any player listing**, always,
  regardless of what the listing says.
- **Administrators below rank 100 are refused outright** — `BuyResult.Admin`, "Admins can't buy player
  merched items."
- The purchase is one transaction: take the buyer's fame, add the tax to the treasury, move the item,
  remove the listing, and **pay the seller `Price - Tax`**. So the tax comes out of the seller's
  proceeds, not the buyer's payment.
- The seller is told, wherever they are: *"Your {item} has sold for {price} fame."* If they are
  online their displayed fame is updated in place.
- On success the stall **reloads immediately** to show the next listing.

Price, seller and item are read into locals **before** the transaction, with a comment saying why:
otherwise a concurrent update could send the seller the wrong price.

## What this server does differently

- **Shops here have no reload clock at all.** Stock is a static table per shop region. The original
  rotates every twenty seconds on a shared cycle with a per-stall offset, refuses to rotate while
  somebody stands within two tiles, and reloads immediately when emptied.
- **A purchase with a full inventory should become a gift**, not a refusal.
- Our potion stacks match: one item kind, a maximum, slots 254 and 255.
- The rank check is against **stars**, which is now correct here.
- **Player listings require two stars to buy**, from `PlayerMerchant`'s constructor rather than from
  the listing. We apply no rank requirement to market purchases.
- **The tax comes out of the seller's proceeds** (`Price - Tax`), not the buyer's payment. We take no
  tax at all, which is consistent with having no treasury, but the seller should still receive less
  than the buyer paid if we ever add one.
- **The seller is told their item sold**, wherever they are.
