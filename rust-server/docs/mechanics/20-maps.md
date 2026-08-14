# Map loading: what becomes scenery and what becomes an entity

Read from `realm/terrain/Wmap.cs`.

## The split, exactly

For each tile with an object on it:

```
if ObjType != 0 and (desc is null or not desc.Static or desc.Enemy):
    remember it as an entity to instantiate at this tile
    if desc is null or not (desc.Enemy and desc.Static):
        clear the object off the tile        // it becomes an entity and stops being scenery
```

So:

| Description | Stays on the tile | Becomes an entity |
| --- | --- | --- |
| `Static`, not `Enemy` | yes | no |
| not `Static` | no | yes |
| `Enemy` and `Static` | **yes** | **yes** |
| unknown to the catalog | no | yes |

The third row is the one to get right: **a static enemy is both**. It stays on the tile as scenery
*and* is instantiated as an entity, so the tile keeps blocking and the entity can be shot.

An object whose description the catalog does not have is **still instantiated** — it becomes an
entity and is cleared off the tile — which is how a map referring to content that has been removed
loses its scenery rather than keeping a hole.

## Tile object ids

```
if ObjType != 0 and not (Enemy and Static):
    enCount++; tile.ObjId = idBase + enCount
```

Every remaining scenery tile is given a **world-unique object id**, allocated from a counter shared
across every map loaded by the process. That is what lets a behaviour address a tile object, and what
`ReplaceTile` hands out when it finds a tile with no id.

## Regions

Any tile with a non-zero `Region` is added to a `Regions` dictionary keyed by point. `GetRegionPosition`
uses `Single`, so **a region that appears on more than one tile throws** — regions used for placement
are expected to be unique, and the ones used in bulk (`TossObject`'s region targeting) are read off the
map directly instead.

## Versions

Three map versions, differing only in where elevation is stored: absent in version 0, per-tile-type
in version 1, per-tile in version 2.

## Entity configuration

Each remembered entity carries its tile's `ObjCfg` string, which is the `name`/`size`/`conn` data
covered by the settings census in `PLAN.md` §18.23.

## What this server does differently

Our loader follows the same split — scenery stays on the tile, everything else becomes an entity —
which is what the `map_entities` census measures. Two things to confirm:

- **A static enemy should be both.** If we treat `Enemy && Static` as one or the other, a map's
  shootable scenery is either missing or does not block.
- **An object unknown to the catalog should become an entity**, not be dropped silently.

`InitConnection` runs a second pass over every tile after loading, which is where `conn` is resolved
from the neighbours — presentational, and covered already.
