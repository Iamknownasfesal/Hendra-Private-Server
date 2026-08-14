# What a client may ask for

Read from `networking/handlers/` — 50 handlers. This page covers the ones that carry mechanics; the
rest are thin passes into code covered elsewhere.

## Two dispatch modes, and the choice is per handler

```csharp
client.Manager.Logic.AddPendingAction(t => Handle(...));   // runs on the logic tick
Handle(...);                                                // runs on the network thread
```

Handlers that touch world state should queue; handlers that only touch the client's own containers do
not. The split as written:

| Queued onto the tick | Run inline on the network thread |
| --- | --- |
| `Move`, `Teleport`, `UseItem`, `GroundDamage`, `PlayerHit`, `EnemyHit`, `PlayerText`, `Pong` | `PlayerShoot`, `InvSwap`, `InvDrop`, `UsePortal`, `Escape` |

Several of the inline ones have their queued version **commented out immediately above**, so the
inline form is a deliberate later change. `PlayerShoot` running inline is the significant one: it
creates a projectile and calls `EnterWorld` from a thread that is not the tick.

## Five handlers do nothing

```
OtherHit      empty
SquareHit     empty
ShootAck      body commented out
AoeAck        "TODO: implement something"
SetCondition  "TODO: implement something"
```

`OtherHit` and `SquareHit` are the client reporting that a bullet hit another player or a wall — both
ignored, so a bullet that hits somebody else is never removed by their claim. `SetCondition` being
empty is why a client cannot give itself an effect.

Three acks do something: `UpdateAck` and `GotoAck` feed the player's tick accounting, and `Pong`
feeds latency.

## Logging in

`Hello` checks, in order: the build version matches; the credentials verify; `NameChosen`; not
banned; the IP is not banned; the server is not admin-only; and `Rank >= serverInfo.minRank`.

Two things worth noting.

**An unknown account is registered on the spot**, as a guest, with the submitted GUID and password.
There is no separate registration step in the game protocol.

**A build-version mismatch returns without saying anything** — the `SendFailure` is commented out. The
client is left connected with no answer and no disconnect, which reads to a player as the server
being down.

On success the IP is logged against the account (`LogAccountByIp`, which is what makes an IP ban reach
alts), the account is stamped with the IP, and the connection is queued onto the tick as a `ConInfo`.

`Create` and `Load` both require `State == Handshaked`, build the player, `EnterWorld`, send
`CreateSuccess` at **high priority**, and move to `Ready`. `Load` refuses a character marked `Dead`.
Both construct `new Player(client, false)` in a `Test` world — the flag that stops it saving.

## Choosing a name

```
letters only, 3 to 10 characters, first letter upper-cased, not a guest name
5,000 fame if a name has already been chosen, free the first time
```

The uniqueness check and the rename happen under the global `nameLock`, taken with the same
**unbounded spin** as `/rename`: `while ((lockToken = AcquireLock(key)) == null) {}`. The rename
itself is a second spin: `while (!RenameIGN(...)) {}`.

The fame is deducted **before** the rename is attempted, and the rename spin cannot fail out, so the
two cannot separate — but nothing releases the fame if the process dies between them.

## Inventory

### Swapping

`ValidateEntities` is the interesting half:

- both ends must be `IContainer`;
- neither may be a **different** player;
- a `Container` with a non-empty `BagOwners` must include this account;
- a `OneWayContainer` may only pair with the player themselves;
- **`DistanceSquared(a, b) <= 1`** — the two containers must be within one tile of each other.

Then, if the destination is the player and the destination slot is a **potion stack**, the swap
becomes a stack `Put` instead, and the source slot is cleared. Anything taken out of a `GiftChest`
also has to be removed from the account's gift list, and the comment says why: *a stackable item that
ends up in a gift chest becomes infinite if not removed.*

Slot audit is `(slotA < 16 && slotB < 16 || player.HasBackpack)` plus both containers accepting the
incoming item's slot type.

The two writes go through **one `Inventory.Execute(transA, transB)`**, and the gift-chest database
write is a second transaction after it. If the database write fails, the inventory change is reverted
— and a failed revert logs `"{player} has an extra {item}"`, which is the duplication this design is
arranged to avoid and cannot fully prevent.

### Soulbound items and the rank check that is always true

```csharp
!item.Soulbound && !player.Client.Account.Admin && player.Client.Account.Rank >= 0
```

`Rank >= 0` is true for every account. So the condition reduces to *not soulbound and not an admin*,
and an item that fails it is **not refused** — it is put into a fresh soulbound bag
(`0x0503`, 60 seconds, `BagOwners = [this account]`) at the player's feet.

Consequence: **an admin cannot put anything into a public container.** Every item they try to move
into one bounces into a private bag instead. The comment says "(includes any swaped item from
admins)", so it is known.

### Dropping

```csharp
if (item.Soulbound || player.Client.Account.Admin || player.Client.Account.Rank >= 0)
    soulbound bag
else
    normal bag
```

Same always-true clause, in the *first* position this time. **Every dropped item goes into a soulbound
bag.** The normal bag type `0x0500` is unreachable, and dropping something for a friend does not work
in this fork.

Both handlers place the bag at `player.position ± 0.5` on each axis with `System.Random`, at size 75,
with a 60-second life.

Other rules: dropping is refused outright in the world **named** `"Nexus"`, refused from another
player's inventory, and refused for a potion stack slot. Note `player.Owner.Name` is read before
`player.Owner == null` is checked, so the null guard below it is unreachable.

## Portals

```csharp
_realmPortals = { 0x0704, 0x070e, 0x071c, 0x703, 0x070d, 0x0d40 }
```

A portal with **no world instance** whose type is in that list picks a random open realm. Everything
else needs an instance, or creates one.

Entering a `Realm` counts as **completing the dungeon you were in** — the fame counter is credited on
the way *out*, not on the boss dying — unless the portal's id contains `"Cowardice"`. So leaving by a
cowardice portal forfeits the completion, which is the whole point of that portal.

World creation for a dynamic portal is a background task guarded by `CreateWorldLock`, with the
player subscribed to `WorldInstanceSet`, so ten players clicking at once create one world and all
reconnect to it.

Taking a portal removes the player from that world's `Invites` and adds them to `Invited` — the
one-shot invite of [the commands page](31-commands.md).

## Movement

`NewPosition` of **`-1` means "did not move"**, not a coordinate. Two comments record what happened
when that was forgotten: indexing the map with -1 threw, which skipped `MoveReceived`, which skipped
the tick accounting the connection depends on, so the player never finished arriving.

The Lab's water tiles are handled here rather than in any world class:

```
green water (0xa9, 0x82)  ->  apply Hexed + Stunned + Speedy
blue water  (0xa7, 0x83)  ->  clear  Hexed + Stunned + Speedy
```

Both require `tile.ObjId == 0` — an object standing on the tile suppresses it.

## Shooting and hit claims

`PlayerShoot` refuses in four ways, and **every refusal calls `DropNextRandom()`**: an unknown item
type, the ability slot (handled by `UseItem` instead), a failed `ValidatePlayerShoot`, or a failed
`ValidateShotGeometry`. That is the shared random stream of
[page 36](36-the-shared-random-stream.md) — the client drew a damage roll for the shot it thinks it
fired, so the server must burn one to stay in phase.

The projectile description used is always `item.Projectiles[0]`, with the comment "assume only one".

`EnemyHit` refuses without acting if the client **is lagging** or the player is `Hidden`, then looks
the bullet up by id in the player's own 256-entry ring, then runs `ValidateEnemyHit`. `Killed` on the
packet queues the entity onto `ClientKilledEntity` rather than killing it.

`PlayerHit` looks the bullet up on the named owner, falling back to a **linear scan of every
projectile in the world** matching owner id and bullet id — `SingleOrDefault`, so two matches throw.

## What this server does differently

We decide all of this ourselves and have no hit-claim packets at all, so most of the above is
background. What is worth carrying:

- **Containers must be within one tile to swap between.** That is the only positional check on
  inventory movement, and without it a player reaches any bag on the map.
- **`BagOwners`, and `OneWayContainer` only pairing with the player.**
- **Taking an item out of a gift chest must remove the gift**, or a stackable becomes infinite.
- **Completing a dungeon is credited on entering a realm**, not on the boss dying, and a cowardice
  portal forfeits it.
- **Two containers, one transaction.** Ours does this in `moves.rs`.
- The Lab water tiles are a real mechanic living in a movement handler; ours needs somewhere for it.

And not to carry: `Rank >= 0`, twice, which makes normal drop bags unreachable and stops admins using
public containers at all.
