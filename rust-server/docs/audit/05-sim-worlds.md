# `hendra-sim`, worlds, loot and the realm

Against [page 06](../mechanics/06-loot.md), [page 10](../mechanics/10-realm.md),
[page 14](../mechanics/14-world-subclasses.md), [page 15](../mechanics/15-portals-and-decoys.md),
[page 16](../mechanics/16-sight.md) and [page 24](../mechanics/24-setpieces.md).

The realm is faithful. Population is counted per terrain from the map's own squares, held in a band
rather than at a number — topped up below `0.75` of target, thinned above `1.5`, left alone between —
and neither spawning nor thinning happens within ten tiles of a player. Those are `Oryx.cs`'s numbers
exactly, and the ten-tile clearance is the detail that stops an enemy vanishing out of a fight.

Loot has the same two-tier shape as the original: entries with no threshold go in a bag anyone can
open, entries with one are rolled per player against their share of the enemy's starting health, and
a loot-drop boost multiplies the probability. That is `Loots.cs`'s structure.

Two things are missing from it.

## Every enemy in the game should have a 3% chance to drop a potion

`World.cs:27`, on the base class, so every world in the game inherits it:

```csharp
public Loot WorldLoot = new Loot(
        new TierLoot(1, ItemType.Potion, .03)
    );
```

`Loots.HandleDrops` merges this into every enemy's own table
(`possibleDrops.AddRange(possibleWorldDrops)`) before rolling. Nothing else in the tree assigns
`WorldLoot`, so no world overrides it: **every enemy that dies anywhere has a 3% chance of a tier-1
potion**, on top of whatever its own table says.

There is no equivalent here. `World::drop_loot` rolls the enemy's own table and nothing else, so the
only potions that drop are the ones a specific enemy names.

That single line is the game's baseline potion supply, and it is the reason a player can drink their
way up without farming a specific boss. Its absence changes the economy rather than one encounter.

The merge also has an override rule worth copying with it: a world drop is removed from the enemy's
own table first, so an enemy naming the same item does not get two rolls — except for object types
`0xa22` and `0xa23`, which are exempt from the removal. The comment says why: it is how Oryx's arena
key is stopped from dropping in the arena itself.

## Required drops are counted and ours are not

Each `LootDef` carries `numRequired`. After the random rolls, `Loots.cs` forces out however many
copies did not drop: into the public bag for unthresholded entries, and to randomly chosen eligible
players for thresholded ones, one each, never twice to the same player.

We have no equivalent — an entry either rolls or does not.

This one is small on this content. `numRequired` is passed **three times** in all 61 behaviour
scripts, so three drops in the game are guaranteed and here are merely likely. Worth recording so it
is not mistaken for a subsystem, and worth fixing only alongside something else.

## The luck stat is a stat we do not have

The private-loot roll is:

```csharp
Rand.NextDouble() < i.Probabilty * lootDropBoost * luckStatBoost
```

where `luckStatBoost = 1 + player.Stats.Boost[10] / 100.0`. Index 10 is `LuckBoost`, which
`StatsManager.GetStatIndex(string)` knows by name and which is **not one of the eight positional
stats** — the array runs 0 to 7, and 8, 9 and 10 are `DamageMin`, `DamageMax` and `LuckBoost`,
reachable only through the string form.

We have `loot_drop` as a per-player multiplier, which covers `lootDropBoost`. There is no luck.
Whether that matters depends on whether any content grants `LuckBoost`, which is worth measuring
before implementing anything: it may be another `TrollWhiteBag` — a mechanism the original reads and
no content ever sets.

## What matches

- Realm population: per-terrain targets from square counts, the 0.75/1.5 band, the ten-tile
  clearance for both spawning and thinning, group spread.
- Loot: public versus threshold, share measured against starting health rather than current, the
  loot-drop boost.
- Setpieces: all 38, confirmed against [page 24](../mechanics/24-setpieces.md) when that page was
  rewritten from the pieces themselves.
- The `Threshold` share is measured against `base_max_hp` rather than `max_hp`, which is right and
  is the sort of thing that is wrong everywhere it is not deliberate: a dead enemy's current health
  is zero, and a share of zero is a threshold everyone meets.
