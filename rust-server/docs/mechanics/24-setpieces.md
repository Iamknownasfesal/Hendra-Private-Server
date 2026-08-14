# Setpieces

Read from `realm/setpieces/SetPieces.cs` and the individual pieces.

## The placement table

```
piece            count      terrains
Building         80..100    LowForest, LowPlains, MidForest
Graveyard         5..10     LowSand, LowPlains
Grove            17..25     MidForest, MidPlains
LichyTemple       4..7      MidForest, MidPlains
Castle            4..7      HighForest, HighPlains
Tower             8..15     HighForest, HighPlains
TempleA          10..20     MidForest, MidPlains
TempleB          10..20     MidForest, MidPlains
Oasis             0..5      LowSand, MidSand
Pyre              0..5      MidSand, HighSand
LavaFissure       3..5      Mountains
LuckyDjinn        1..1      Mountains
LuckyEnt          1..1      Mountains
Crystal           1..1      Mountains
KageKami          2..3      HighForest, HighPlains
```

`rand.Next(min, max)` is **exclusive of the maximum**, so "80..100" is 80 to 99, and the three
`1..1` entries produce **exactly one attempt each** — `rand.Next(1, 1)` is 1.

Note `Oasis` and `Pyre` start at 0: a realm can legitimately have none.

## Placement

```
for each piece kind, for each of its count:
    up to 50 attempts:
        pick a random tile anywhere on the map
        reject if the terrain is not one of the piece's
        reject if the piece's bounding square intersects any square already placed
    give up after 50 attempts and skip this one
```

Two things follow:

- **Setpieces never overlap each other**, and the test is a square bounding box of the piece's own
  `Size`, kept for the life of the placement pass.
- **A piece that cannot find a spot in fifty tries is silently skipped**, so a realm's actual count is
  at most the rolled count and often less. There is no retry and no report.

The bounding boxes are **not** kept between calls, so re-applying setpieces to a recycled realm starts
with an empty set — which is correct, because the map is reloaded first.

## What a piece renders

Each piece writes an integer grid and interprets it: `1` is floor, `2`/`3` are floor plus an entity,
and so on per piece. `Pentaract` is representative:

```
Size = 41
five circles at 72-degree intervals, radius 15 from the centre
each circle is a 7x7 stamp with a pillar (0x0d5e) at its centre
the Pentaract itself at the middle of the 41x41
```

Writing a tile means setting `TileId`, clearing `ObjType`, and bumping `UpdateCount` so the change is
sent. Entities are placed at tile centres (`+0.5`).

Note `Pentaract` loops `x` and `y` to **40** while its grid is 41 wide — the last row and column are
never drawn. Harmless here because the outermost ring is empty, and worth knowing before assuming the
grids are square-complete.

Three helpers exist for the pieces that need them — `rotateCW`, `reflectVert`, `reflectHori` — so a
piece can be drawn once and placed in four orientations.

## RenderFromProto

A setpiece can also be **a saved map**: `RenderFromProto` loads a world's `.wmap`, picking at random
when the proto lists several, and projects it onto the world at a point. That is how the larger
event pieces are built without hand-written grids.

## What this server does differently

Our fifteen realm setpieces match this table by name. To confirm:

- **The counts are `rand(min, max)` exclusive of the maximum**, and the three `1..1` entries mean
  exactly one attempt.
- **Fifty attempts then skip**, with no retry — so the placed count is a ceiling, not a target.
- **Non-overlap is a square bounding box** of the piece's `Size`, shared across all kinds in one pass.
- Entities land on tile centres.

The remaining 23 pieces in the directory are the **event** pieces, listed in `NEXT.md` as part of the
realm-events gap; they are placed by `Oryx.SpawnEvent`, not by this table.
