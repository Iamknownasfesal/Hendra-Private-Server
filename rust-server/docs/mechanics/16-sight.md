# Sight: four modes, chosen per world

Read from `realm/Sight.cs`.

A world's `Blocking` field picks one of four algorithms. This decides which tiles and which entities
a player is sent, so it is a mechanic and not a rendering detail.

```
Radius        = 20 tiles
RayStepSize   = 0.1
AngleStepSize = 2.30 / Radius            // about 6.6 degrees, 54 rays
MaxNumRegions = 2048
```

## Blocking = 0 — unblocked

The precomputed disc: every offset within `x² + y² <= 400`, translated to the player. Walls do not
block anything. This is the default and what the realm uses.

The disc is built once at startup by walking outward in a spiral, which is worth knowing only because
it fixes the **order** tiles are added in — nearest first, roughly — and that order is what the update
packet carries.

## Blocking = 1 — room sight (flood fill)

A breadth-first flood from the player's tile through the eight neighbours, bounded by the radius, and
**stopping at anything that blocks sight**. A blocking tile is added to the visible set but not
expanded through, so you see the wall and not what is behind it.

Each tile carries a `Generation`, and the fill stops past `Radius` generations — so the limit is
**steps taken**, not straight-line distance, and a corridor that doubles back is cut off earlier than
its distance suggests.

## Blocking = 2 — line of sight (ray casting)

54 rays at 6.6-degree steps, each walked in 0.1-tile increments out to the radius. A ray stops at the
first tile whose object blocks sight. Every tile a ray touches is visible, **and so are its eight
neighbours** — a deliberate widening so the rays do not leave gaps between them at range.

## Blocking = 3 — region blocks (the clever one)

Precomputed once per map by `CalcRegionBlocks`, and this is the one worth understanding:

- Every open region of the map is assigned a **distinct prime number**.
- A tile that blocks sight is assigned the **product** of the primes of every region it touches.
- Visibility is then a single test: `tile.SightRegion % playerRegion == 0`.

So a wall between two rooms carries both rooms' primes and is visible from both; a tile in another
room is not divisible by yours and is invisible. The whole per-tile visibility question becomes one
modulo.

`UpdateRegion` maintains it when a sight-blocking object is removed: it takes the minimum of the
neighbouring regions as the new one, divides that prime out of neighbouring walls and multiplies the
new one in, and then — if two regions have just been joined — **walks the entire map** rewriting every
tile of the absorbed region. That last step is why removing a wall is expensive and why the code
bumps every nearby player's `UpdateCount` when it happens.

`checked { }` around the multiplications: the product of primes can overflow, and the original would
rather throw than silently wrap. With 2048 regions available, a tile touching many regions is the
risk.

## Caching

`GetSightCircle` recomputes only when `UpdateCount > 0`. It is bumped when the player moves to a new
tile, and by anything that changes the map — removing a sight-blocking object bumps it for every
player within radius. A spectating player borrows the sight circle of whoever they are watching.

## What this server does differently

We send the whole map when a player arrives and do not maintain a per-player visible set at all, so
none of the four modes exists here. That is a deliberate architectural choice and it has three
consequences worth writing down:

- **`tiles_seen` had to be computed another way** — done, see `PLAN.md` §18.17.
- **Entity visibility is a radius plus a line-of-sight test**, which is closest to mode 2 and is
  applied in every world. A world the original runs in mode 0 (the realm) hides nothing behind walls;
  ours does.
- **Mode 3's region test would be the cheapest thing we could adopt** if we ever want per-world sight
  rules: one modulo per tile, precomputed once per map, and it answers "can this player see this
  tile" without any ray casting.
