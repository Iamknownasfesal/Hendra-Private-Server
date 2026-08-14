# Loot

Read from `logic/loot/Loots.cs` and the loot table at the top of `realm/worlds/World.cs`.

## Every world has its own loot table, rolled on every death

```csharp
public Loot WorldLoot = new Loot(new TierLoot(1, ItemType.Potion, .03));
```

Every enemy in every world rolls this **in addition to its own table**: a 3% chance of a tier-1
potion. It is a property of the world, so a world subclass can change it.

World drops are merged into the enemy's list with an override rule: any world drop **replaces** an
identical entry in the enemy's own table, except object types `0xa22` and `0xa23`, which are exempt.
The comment gives the reason — so a world can suppress a drop the enemy would otherwise give, such as
Oryx's arena key not dropping inside the arena.

This server has no world-level loot table at all.

## A spawned enemy drops nothing

```
if (enemy.Spawned) return;
```

Anything marked `Spawned` — everything a behaviour created, since `Spawn`, `TossObject`, `Reproduce`
and the rest all propagate the flag — drops no loot whatsoever. That is the whole anti-farming rule,
and it is one line.

## Public loot and private loot

Each entry has a `Threshold`, which is a share of the enemy's total damage.

- **Threshold ≤ 0**: public. Rolled once, and the bag belongs to nobody.
- **Threshold > 0**: private, per player, and only for players whose damage share reached it.

For a private roll the probability is scaled:

```
chance = probability × lootDropBoost × luckStatBoost
lootDropBoost = 1.5 when the player's loot-drop boost is running, else 1
luckStatBoost = 1 + Boost[10] / 100          // the luck stat
```

**Boost index 10 is the luck stat**, and this is the one place it is read. This server models eight
stats and has no tenth, so `luckStatBoost` is always 1 here.

## `NumRequired` guarantees a drop

Every entry carries a required count. After the random rolls, anything still owing is added outright:
public entries to the shared bag, private entries handed to randomly chosen eligible players, one
each, and never to a player who already got that item from a random roll.

This is what makes a boss's signature drop guaranteed rather than merely likely.

## Bags

Loot is packed **eight items to a bag**; a ninth starts a second bag.

The bag's colour is the **highest `BagType` of the items in it**:

```
0 brown, 1 pink, 2 purple, 3 egg basket, 4 cyan, 5 potion, 6 white, 7 white2, 8 troll white
```

An enemy marked `TrollWhiteBag` starts every bag at 8. A player with a loot-drop boost running gets a
**red bag** instead, unless the contents earned a white bag, which wins.

Bags last **60 seconds**, are dropped within half a tile of the corpse in a random direction, and are
drawn at size 120 for bag types above 3 and 80 otherwise.

`BagOwners` is a list of **account** ids, so a private bag is opened by the account rather than the
character.

## The notification list

A hard-coded list of about a hundred item names — the good ones — that, when dropped, announce
themselves to everybody in the world along with the finder's damage share. The list contains
duplicates and is worth treating as content rather than as code.

## The three kinds of loot entry

From `logic/loot/MobDrops.cs`.

### ItemLoot

```
ItemLoot(item, probability = 1, numRequired = 0, threshold = 0)
```

One named item. A name the content does not have is **warned about and skipped**, not fatal.

### TierLoot — the probability is split across the tier

```
TierLoot(tier, type, probability = 1, numRequired = 0, threshold = 0)
```

Every item of that tier and type becomes its own entry with

```
probability / items.Length
```

So `TierLoot(2, Weapon, 0.3)` is **0.3 spread across every tier-2 weapon**, not 0.3 for each. The
chance of getting *something* is roughly 0.3; the chance of any particular sword is 0.3 divided by
how many tier-2 weapons the content has. Adding a new tier-2 weapon makes every other one rarer.

The item type is decided by **slot type**, not by a category field:

```
Weapon  : 1, 2, 3, 8, 17, 24
Ability : 4, 5, 11, 12, 13, 15, 16, 18, 19, 20, 21, 22, 23, 25
Armor   : 6, 7, 14
Ring    : 9
Potion  : 10
```

### Threshold — a wrapper, not an entry

```
Threshold(threshold, children...)
```

It re-populates its children with the threshold overridden, leaving probability and required count
alone (`-1` means "keep the child's"). So a threshold is applied to a whole group at once and is not
a drop of its own.

## What this server does differently

- **No world loot table.** The 3% tier-1 potion on every enemy in the game is missing, as is the
  override rule that lets a world suppress a drop.
- **The luck stat is not modelled**, so `luckStatBoost` is always 1. This is a known and deliberate
  gap, recorded in `PLAN.md`: the original reads boost index 10 and this server has eight stats.
- We should confirm: eight items per bag, the highest bag type winning, the red bag rule and its
  white-bag exception, the 60-second lifetime, and that `NumRequired` is honoured after the rolls.
- We should confirm that spawned enemies drop nothing.
