# The realm: population, taunts, events

Read from `realm/Oryx.cs`.

## The clock

```
Tick: returns immediately unless 10 seconds have passed since the last one
  every other tick  (20s):  HandleAnnouncements()
  every sixth tick  (60s):  EnsurePopulation()
```

So taunts every twenty seconds and a population check every minute.

## Initial population is derived from the map, not configured

```
Init:
  count the tiles of each terrain across the whole map
  for each terrain in RegionMobs:
      maxCount[terrain] = tileCount / density        // density is the table's first field
      spawn until the count is reached
```

The target population of a terrain is **its area divided by a density constant**, so a realm with
more desert carries more desert monsters. Nothing states an absolute number anywhere.

## Choosing what to spawn

`GetRandomObjType` walks the terrain's list accumulating probabilities and takes the first entry
whose running total exceeds a single uniform roll. The list is therefore a cumulative distribution,
and **if the probabilities do not sum to 1 the tail is unreachable** — a roll above the total returns
object type 0, which the caller skips.

## Where a spawn lands

```
do { pick a random tile on the whole map }
while (terrain is wrong || not passable || any player nearby)
```

An **unbounded rejection loop** over the entire map. Nothing places by region or by chunk. Note the
third condition: **nothing ever spawns near a player**, so a realm repopulates out of sight.

A description with a `Spawn` block spawns a **group**:

```
num = clamp(Normal(mean, stdDev), min, max)
```

drawn from a Box-Muller normal, all of them scattered within **±5 tiles** of the chosen point. Every
member inherits the terrain it was spawned for, which is what lets the population counter attribute
them later.

## Keeping the population

```
EnsurePopulation (every 60s):
  recount every enemy by terrain
  for each terrain:
      count > max * 1.5   -> mark for culling, diff = count - max
      count < max * 0.75  -> mark for topping up, diff = max - count
      otherwise           -> leave alone
  cull: walk every enemy; remove those of a marked terrain with no player within 10 tiles
  add:  spawn until the shortfall is met
  recount
```

The hysteresis is **1.5× to cull and 0.75× to top up**, so the population drifts inside a band rather
than being held at a number. Culling only ever removes enemies **with nobody within 10 tiles**, so a
realm never empties around a player.

## Taunts, every twenty seconds

```
pick one critical enemy at random from the table of 21
count how many of that kind are alive in the realm
if none: say nothing
if exactly one and it has a Final line, or it has Final lines and no NumberOfEnemies lines:
    say a random Final line
otherwise:
    say a random NumberOfEnemies line, with {COUNT} replaced
```

So the "my final Lich shall consume your souls" line is picked when one remains, and the counted line
otherwise. The choice of *which* enemy to taunt about is random every time, and an enemy with none
alive is silently skipped — which means a realm with few critical enemies is often quiet.

Taunts stop entirely once the realm is closed.

## Events

Covered in [`NEXT.md`](../../NEXT.md) as the largest missing feature. In short: `OnEnemyKilled` fires
when any enemy the content marks `Quest` dies, picks one of eight live events at random, places it
with the same rejection loop bounded to terrain between Mountains and MidForest, strikes it off the
list if its object has `PerRealmMax = 1`, announces its spawn line, and forces every player's quest
arrow to be recalculated.

## What this server does differently

Our `realm.rs` follows the density model, the 1.5/0.75 hysteresis and the "not near a player" rules,
which is the substance of it. What is missing:

- **Events entirely** — eight of them, plus the `PerRealmMax` rule.
- **Taunts entirely** — the 21 critical enemies and their four kinds of line.
- The group spawn's **normal distribution** and its ±5-tile scatter are worth checking against ours.
- The cull's **10-tile** proximity guard is worth checking; ours uses its own figure.
