# Trading

Read from `realm/entities/player/Player.Trade.cs` and `networking/handlers/AcceptTradeHandler.cs`.

## Requesting

`RequestTrade(name)` refuses for: the test world, an account with no chosen name, already trading, a
rank of 40 or more **on either side**, a guest name, a target that cannot be seen, yourself, and a
target already trading. A target who has you on their **ignore list** is refused silently — no error
at all, which is the point of an ignore list.

The handshake is mutual: each player keeps a `potentialTrader` map of who has asked them and how long
that request has left. A trade starts only when the second player asks the first, so both have
consented.

**Requests time out** — counted down in `CheckTradeTimeout` on every tick, and the *requester* is told
it lapsed.

## The offer

Twelve slots each. `Tradeable` is computed once, when the trade opens:

```
Tradeable = the slot holds something and i >= 4 and not Soulbound
```

So the four **worn** slots can never be traded, and soulbound items never.

## Accepting

```
player.trade = the offer just sent
if the other side's stored offer equals what this packet says it is:
    mark accepted, tell the other side
    if both accepted:
        if one is an administrator and the other is not: cancel outright
        DoTrade
```

The equality check is the anti-swap guard: **you accept a specific offer**, and if the other player
changed theirs in between, the acceptance is simply ignored rather than applied to the new one.

Administrators may only trade with administrators.

## The exchange

```
take every marked item from slots 4..12 of both sides into two lists
for each of the 12 slots of the receiver, for each item still to place:
    place it if the slot is empty and either the slot type is 0 or it matches the item's slot type
execute both inventory transactions together, or fail both
if anything is left unplaced: "An error occured while trading! Some items were lost!"
```

Two things worth copying:

- **Items are placed into matching worn slots when they fit.** The loop starts at slot 0, so an item
  whose slot type matches an empty *equipment* slot is equipped by the trade.
- **Overflow is lost.** The items were already removed from the giver by the transaction; if the
  receiver has no room, the loop simply ends and the message says so. The trade is not rolled back.

Both inventories are written in **one `Inventory.Execute`**, which validates that neither side changed
since the snapshot and applies both or neither.

## What this server does differently

- Ours is a durable transaction in the store with per-side room limits, which is stronger than the
  original: no item can be lost. Worth keeping.
- **We should confirm the `i >= 4` rule** — worn slots are never tradeable — and the soulbound rule,
  both of which we have.
- **The accept-a-specific-offer guard** matters: if the other side's offer changed, the acceptance
  must be ignored. Ours re-confirms the expected items at execution, which achieves the same end.
- **Administrator-to-administrator only** has no equivalent here.
- **The ignore list should silently refuse** a trade request.
- **Trade requests should time out**, and the requester should be told.
