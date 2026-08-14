# What a client is told, and when

Read from `realm/entities/player/Player.Update.cs`.

Our server sends the whole map on arrival and then delta snapshots, so none of this is copied
directly. It is worth recording because it defines **which entities a player is allowed to know
about**, and that is a mechanic: a bag you cannot see is a bag you cannot open.

## The update packet

```
SendUpdate:
    sCircle    = the sight circle for this world's Blocking mode
    tiles      = every tile in the circle whose UpdateCount has moved since we last sent it
    statics    = objects on tiles newly entering the circle
    removed    = entities that have left, plus statics that have left
    added      = entities newly visible
    if anything changed: send Update, then wait for the acknowledgement
```

`FameCounter.TileSent(count)` is called with the number of tiles in this packet — the only feeder of
`TilesUncovered`.

Each player keeps `tiles[width, height]` of bytes, holding the `UpdateCount` last sent for that tile,
so a tile is re-sent only when the map changes it. `TileId == 255` is skipped entirely, which is the
"no tile" value.

## Which entities a player may see

`GetNewEntities`, in order:

1. **Every player in the world**, regardless of distance, subject to `CanBeSeenBy` — which is false
   only for `Hidden`. So players are always known to each other, everywhere in the world.
2. **Decoys within the radius**, from the player collision map.
3. **Everything in the enemy collision map within the radius**, but only if its tile is in the sight
   circle. With one exception:
   - **A container with owners only appears to those owners.** `BagOwners` is checked against the
     account id, and a private bag is invisible to everyone else rather than merely unopenable.
4. **The quest target**, always, wherever it is.
5. **The spectate target**, always.

`GetRemovedEntities` is the mirror, plus anything the client itself reported killing.

Two exemptions from removal are worth noting: **players, the quest target and the spectate target are
never removed** even when they leave the sight circle. That is why a quest arrow keeps pointing at a
boss across a map.

## Statics are kept by a bounding box, not the sight circle

```
StaticBoundingBox = Radius * 2 = 40
```

A static object is dropped only when it is outside a box of that size, and the test is written twice
in ways that disagree — `Math.Abs(40 - ((int)X - i.X)) > 0` in one place and
`40 - ((int)X - i.X) > 0` in the other. The first is true for almost everything, so statics are
effectively never removed by it; the second is a genuine bounding-box test.

Treat the intent as: **statics persist much longer than entities**, on a box twice the sight radius.

## What this server does differently

- We send the whole map, so there is no per-tile update count, no sight circle, and no static
  bookkeeping. `tiles_seen` is computed from movement instead — see `PLAN.md` §18.17.
- **Players are visible to each other at any distance in the same world.** Our snapshot is radius
  based; a player across the room is dropped. Worth matching, because it is what makes `/who` and
  trading feel like they are in the same place.
- **A private bag should be invisible to everyone but its owners**, not merely unopenable. We should
  confirm ours are filtered out of the snapshot rather than filtered at the point of opening.
- **The quest target is always sent**, wherever it is.
- `Hidden` is the only condition that hides a player from another player. `Invisible` hides them from
  *enemies* — see [the entity page](04-entities.md). The two are different effects and are used for
  different things.
