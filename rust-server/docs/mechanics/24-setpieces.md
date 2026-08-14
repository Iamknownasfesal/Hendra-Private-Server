# Setpieces

**All 38 read from the directory**, plus `SetPieces.cs`, `ISetPiece.cs` and `Noise.cs`.

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

## The three shapes a piece takes

Having read all 38, every one is one of three things.

### A one-liner that places a single entity

Seventeen of them. `Size` is 5 (or 32 for the two large ones), and the whole body is:

```csharp
var e = Entity.Resolve(world.Manager, "<name>");
e.Move(pos.X + 2.5f, pos.Y + 2.5f);
world.EnterWorld(e);
```

`Crystal`, `CubeGod`, `LuckyEnt`, `LuckyDjinn`, `LordoftheLostLands`, `TheKid`, `Boshy`, `Sanic`,
`Megaman`, `Spooky` (which resolves `"LH Sentry"`), `Bedlam` (`"BedlamGod"`), `FanaticofChaos`.

**`FanaticofChaos` declares `Size = 32` and still places at `+2.5`**, so it reserves a 32-tile square
and puts its boss in the top-left corner of it.

### A saved map projected onto the world

Six use `RenderFromProto`: `Avatar` (32), `GhostShip` (40), `ZombieHorde` (5, world `"Horde"`),
`KageKami` (65), `Hermit` (32), `LordOfSky` (5), `RockDragon` (32).

`ZombieHorde` and `LordOfSky` declare `Size = 5` for maps that are plainly larger, so their
non-overlap boxes are far too small and they can be drawn over.

### A generated grid

The remaining fifteen build an `int[,]`, optionally rotate it, and interpret each value as a tile, an
object, a container or an entity.

## What the generated pieces actually draw

| Piece | Size | Boss | Chest tiers |
| --- | --- | --- | --- |
| Building | 21 | none | none |
| Graveyard | 34 | Deathmage | weapon 4-6, armor 3-5, ability 1-3, ring 1-2, potion 1 |
| Grove | 25 | Ent Ancient | none |
| LichyTemple | 26 | Lich | none |
| Castle | 40 | Cyclops God | weapon 6-8, armor 5-7, ability 2-4, ring 2-3, potion 1 |
| Tower | 27 | Ghost King (`0x0928`) | none |
| TempleA / TempleB | 60 | `0x0dc2` | weapon 4-5, armor 4-5, ability 1-2, ring 2-3, potion 1 x3 |
| Oasis | 30 | Oasis Giant | weapon 5-7, armor 4-6, ability 2-3, ring 1-2, potion 1 |
| Pyre | 30 | Phoenix Lord | same as Oasis |
| LavaFissure | 40 | Red Demon | weapon 7-9, armor 6-8, ability 2-4, ring 2-3, potion 1 |
| ArchMage | 11 | Archmage | weapon 9-11, armor 10-12, ability 3-5, ring 3-4, potion 2 |
| SkullShrine | 33 | Skull Shrine | none |
| Sphinx | 81 | Grand Sphinx | none |
| Pentaract | 41 | Pentaract | none |

Every chest is object type **`0x0501`** and is filled with `chest.GetLoots(manager, min, 8)` —
`min` is 5 for the higher pieces and 3 for Graveyard and the Temples. The chest is placed **on the
same tile as the boss** in Oasis, Pyre, LavaFissure and TempleA.

`ArchMage`'s chest is by far the richest in the game outside a dungeon: tier 9-11 weapons and 10-12
armour, out of an 11x11 piece with a lava moat, and it is **not in the placement table** — it is an
event piece.

### The techniques

- **Rotation.** Castle, Graveyard, LichyTemple, TempleA, TempleB, Tower, LavaFissure, Building and
  ArchMage each pick `rand.Next(0, 4)` quarter-turns. Tower also builds itself from one quarter
  reflected twice, so it is symmetric by construction.
- **"Corruption".** Castle, Graveyard, LichyTemple and Building sweep the finished grid and randomly
  degrade tiles — `p < 0.1` back to floor, `p < 0.4` to the next value up (wall becomes destructible
  wall, water becomes deep water). Building's is blunter: **half of every tile it drew is erased**
  (`rand.Next() % 2 == 0`), which is what makes each one a different ruin.
- **Simplex noise.** Only `SkullShrine` uses `Noise` — a 3D simplex generator seeded from
  `Environment.TickCount` — to punch holes in the shrine below a threshold of 0.2.
- **`ConnectionComputer`.** Only `Sphinx` uses it, to give each `Tomb Wall` a `size:`-style config
  string describing which neighbours are also walls, so the client draws connected masonry.
- **Trees on the border.** Grove and Oasis collect the tiles within 1 to 1.5 of the boundary radius
  and fill **half** of them with trees, chosen with a `HashSet` and a rejection loop — which spins
  until it happens to hit enough distinct points.
- **Parametric curves.** `LavaFissure` draws its crack from a sine-warped diagonal
  (`t/√2 ± sin(t)/(5.5√2)`), not from a grid.

## Bugs and oddities in the pieces

- **`ArchMage`** writes `t[x, y] = t[x, y] = 1;` twice on consecutive lines — four assignments where
  one was meant. Harmless.
- **`Castle`** never emits value `5` (destructible wall) from its own generation; only the corruption
  pass can produce it, by incrementing a `4`.
- **`Graveyard`** places the boss at `t[pt.X + 1, pt.Y]` without checking the bound, so a cross chosen
  on the right edge writes outside the array. The cross grid stops at `x = 2 + 3*6 = 20` in a
  23-wide array, so it never actually happens.
- **`Pentaract`** loops to 40 in a 41-wide grid, leaving the last row and column undrawn.
- Several pieces place the boss with `Move(pos.X + x, pos.Y + y)` — **no `+0.5`** — so it sits on a
  tile corner rather than a centre. Castle, Graveyard, Tower and TempleA/B all do this; Oasis, Pyre,
  LavaFissure, Sphinx and the one-liners use centres.
- **Every piece constructs its own `new Random()` as an instance field**, seeded from the clock at
  construction. Since the placement table constructs each piece **once**, at static initialisation,
  all fifteen share the same startup instant — and each piece's own sequence then advances normally.

## What this server does differently

Our fifteen realm setpieces match this table by name. To confirm:

- **The counts are `rand(min, max)` exclusive of the maximum**, and the three `1..1` entries mean
  exactly one attempt.
- **Fifty attempts then skip**, with no retry — so the placed count is a ceiling, not a target.
- **Non-overlap is a square bounding box** of the piece's `Size`, shared across all kinds in one pass.
- Entities land on tile centres.

The remaining 23 pieces in the directory are the **event** pieces, placed by `Oryx.SpawnEvent` rather
than by this table. Now that all of them have been read, they are not blocked on knowing what they
draw: **seventeen are a single `Entity.Resolve` and a `Move`**, six project a saved map, and only
`Pentaract`, `Sphinx`, `SkullShrine` and `ArchMage` generate anything. The work left in them is the
events feature, not the pieces.

Two things to carry that this page previously did not say:

- **`FanaticofChaos`, `ZombieHorde` and `LordOfSky` declare a `Size` that does not match what they
  draw**, so their non-overlap boxes are wrong. Ours should take the size from the piece rather than
  from a hand-written constant, or at least assert the two agree.
- **Bosses are placed on tile corners in five pieces and tile centres in the rest.** That is a
  half-tile difference in where a boss stands, and it is visible.
