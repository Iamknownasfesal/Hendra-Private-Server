# The spatial index

Read from `realm/Collision.cs`.

Two of these exist per world: `PlayersCollision` and `EnemiesCollision`.

## Shape

```
CHUNK_SIZE = 16 tiles
chunks[cW, cH] where cW = ceil(width / 16)
```

Each chunk holds an **intrusive doubly-linked list** of entities. The entity owns its node, so
insertion and removal are pointer work with no allocation and no search. Moving between chunks
unlinks and relinks; moving within a chunk does nothing at all — the chunk id is packed into one int
(`x | y << 8 | type << 16`) and compared before any work is done.

## HitTest is by chunk, not by distance

```
HitTest(x, y, radius):
    for every chunk overlapping the bounding box of (x ± radius, y ± radius):
        yield every entity in that chunk
```

**It does not filter by distance.** Everything in every overlapping chunk is returned, and the caller
is expected to test the distance itself. Every caller in `Utils.cs` does exactly that
(`if (d < dist) yield return i`), which is why the radius appears twice in those functions.

This matters for parity: a search with radius 1 still returns everything in a 16-tile chunk, so the
*order* entities come back in is chunk order, not distance order. `GetNearestEntity` walks them all
and keeps the minimum, so the answer is stable — but anything that takes the **first** match rather
than the nearest gets a chunk-ordered answer.

## GetActiveChunks: which enemies tick at all

```
ACTIVE_RADIUS = 3 chunks
GetActiveChunks(playersMap):
    for every chunk of the players map that has anybody in it:
        collect every entity of this map within ±3 chunks
```

**Three chunks is 48 tiles.** So the enemy tick covers a 97×97-tile square around each occupied
chunk. That is more than twice the 20-tile sight radius, which is deliberate: an enemy needs to be
running before a player can see it, or it would visibly start moving the moment it came into view.

The result is a `HashSet`, so an enemy near two players ticks once.

This is the container-level half of the freeze rule; `Entity.Tick`'s `AnyPlayerNearby()` is the
per-entity half, and that one uses the **20-tile** sight radius. So an enemy between 20 and 48 tiles
away is visited by the loop and then declines to tick.

## What this server does differently

Our `Grid` is a uniform spatial hash queried by radius, with the distance test inside. That is a fair
optimisation of `HitTest` — it does strictly less work and returns the same set.

What is missing is the **two-tier activity rule**:

- We have no equivalent of `GetActiveChunks`, so every enemy is visited.
- We have no equivalent of the per-entity `AnyPlayerNearby()` check, so every enemy visited also
  ticks.

Adopting it is worth doing for behaviour as much as for cost: a realm holds thousands of enemies and
the original runs almost none of them.

One detail to preserve if we do: the two radii are different on purpose — 48 tiles to be *considered*,
20 tiles to actually *think*.
