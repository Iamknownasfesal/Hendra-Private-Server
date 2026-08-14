# The behaviour scripts, as content

The 61 files of `logic/db/` — 24,392 lines. This is the game's *content*, written in C# as a
constructor-expression DSL, and it is what our converter consumes.

**How this page was read.** **42 of the 61 files were read line by line.** Counted, not estimated:

```
Misc BeachBum BeerGod ForbiddenJungle SkullShrine Oasis RedDemon Beachzone Pentaract
CubeGod Forest Phoenix Sphinx Tutorial Cyclops Hermit
Manor LotLL DeadwaterDocks Jungle Shore PirateCave
Woodland Abyss Deathmage SpiderDen Golems
CaveTT RockDragon EntAncient IceCave Crystal
Oryx SpriteWorld DavyJones Lich Janus
OryxChicken Belladonna CrawlingDepths GhostKing Lowland
```

**The 19 not read line by line** are the largest dungeon files:

```
Shatters Avatar HauntedCeme Draconis Midland Highland Encore Tomb Catacombs
Lab OryxCastle UndeadLair SnakePit Mountain Candyland Puppet OceanTrench
GarnetJade GhostShip
```

Those 19 were surveyed by extracting **every constructor call and every named argument in all 61
files**, which is the census below.

That census is complete and is what establishes the negative claims (the 28 unused constructs). The
line-by-line reading is what the notes at the end come from, and **those notes do not cover the 19
unread files** — a specific oddity inside one of them would not have been found.

## What the content actually uses

| Behaviour | Uses | | Behaviour | Uses |
| --- | --- | --- | --- | --- |
| `Shoot` | **4,498** | | `ReturnToSpawn` | 33 |
| `ConditionalEffect` | 641 | | `MoveTo2` | 29 |
| `Wander` | 555 | | `Charge` | 28 |
| `Prioritize` | 459 | | `MoveTo` | 23 |
| `TossObject` | 389 | | `Timed` | 22 |
| `Follow` | 376 | | `HealGroup` | 18 |
| `Spawn` | 366 | | `ChangeGroundOnDeath` | 17 |
| `Taunt` | 312 | | `RemoveEntity` | 16 |
| `SetAltTexture` | 261 | | `HealSelf` | 16 |
| `Order` | 255 | | `Transform` | 14 |
| `Orbit` | 167 | | `RealmPortalDrop` | 13 |
| `Flash` | 162 | | `RemoveObjectOnDeath` | 12 |
| `Suicide` | 141 | | `Swirl` / `OrderOnce` / `BackAndForth` | 10 each |
| `Protect` | 121 | | `HealEntity` | 8 |
| `StayCloseToSpawn` | 113 | | `RemoveConditionalEffect` | 7 |
| `StayAbove` | 113 | | `SpawnGroup` / `SetNoXP` / `Sequence` / `GroundTransform` | 5 each |
| `Reproduce` | 90 | | `RemoveTileObject` / `CopyDamageOnDeath` / `ApplySetpiece` | 4 each |
| `TransformOnDeath` | 69 | | `TransferDamageOnDeath` / `SpawnOnDeath` / `Random` / `MoveLine` | 3 each |
| `Grenade` | 69 | | `ScaleHP` / `ReplaceTile` / `OrderOnDeath` / `Buzz` | 2 each |
| `ChangeSize` | 63 | | `ReproduceChildren` / `OpenGate` / `OnDeathBehavior` / `If` / `HealPlayer` | 1 each |
| `StayBack` | 50 | | | |
| `Decay` | 49 | | | |
| `InvisiToss` | 46 | | | |
| `DropPortalOnDeath` | 40 | | | |

| Transition | Uses |
| --- | --- |
| `TimedTransition` | **1,370** |
| `HpLessTransition` | 248 |
| `PlayerWithinTransition` | 136 |
| `EntityNotExistsTransition` | 123 |
| `EntitiesNotExistsTransition` | 112 |
| `NoPlayerWithinTransition` | 21 |
| `EntityExistsTransition` | 20 |
| `TimedRandomTransition` | 17 |
| `PlayerTextTransition` | 8 |
| `DamageTakenTransition` | 8 |
| `NotMovingTransition` | 1 |
| `EntityNotExistTransition` | 1 (the one-letter subclass from `PortedTransitions.cs`) |

## Twenty-eight constructs the content never uses

**Nine transitions with zero uses:**

```
AnyEntityWithinTransition   EntitiesNotExistTransition   EntityHpLessTransition
EntityWithinTransition      GroundTransition             GroupNotExistTransition
HpBoundaryTransition        NoEntityWithinTransition     OnParentDeathTransition
```

**Nineteen behaviours with zero uses:**

```
AnnounceOnDeath   BringEnemy         ChangeMusic          ChangeMusicOnDeath
ConditionEffectRegion   DestroyOnDeath   Duration          HealPlayerMP
KillPlayer        MultiplyLootValue  MutePlayer           Ported
RelativeSpawn     RemoveConditionEffect   ReproduceGroup   TeleporttoTarget
WhileEntityNotWithin    WhileEntityWithin       WhileWatched
```

This changes the priority of several defects recorded elsewhere:

- **`OnParentDeathTransition`'s shared-field bug affects nothing** — no script uses it.
- **`ReproduceGroup`'s region-filter bug affects nothing** — no script uses it.
- **`RelativeSpawn`'s double-counted children affects nothing**; the identical bug in `Spawn` affects
  **366** uses.
- **`HpBoundaryTransition` measuring against the content maximum rather than the scaled one** is a
  distinction with no consequence, because no script uses it.
- **`GroundTransition` and the `TileRegion` parameter** are both unused, which is the second
  independent confirmation of [the regions page](25-regions-and-coverage.md).

By contrast, **`MoveTo2` has 29 uses** despite its `once` latch never firing and its walking through
walls, and **`Sequence` has 5** — so `MoveLine`'s never-completing behaviour matters in exactly those
places.

## The argument census, and what it says about `Shoot`

| Argument | Uses | | Argument | Uses |
| --- | --- | --- | --- | --- |
| `coolDown` | **3,611** | | `speed` | 151 |
| `coolDownOffset` | **2,693** | | `defaultAngle` | 112 |
| `projectileIndex` | 2,516 | | `protectionRange` | 100 |
| `count` | 2,037 | | `givesNoXp` | 72 |
| `fixedAngle` | 1,814 | | `reprotectRange` | 69 |
| `shootAngle` | 1,501 | | `radiusVariance` | 67 |
| `range` | 495 | | `duration` | 53 |
| `maxChildren` | 295 | | `densityMax` | 44 |
| `predictive` | 291 | | `speedVariance` | 25 |
| `angle` | 246 | | `rotateAngle` | 12 |
| `initialSpawn` | 227 | | `seeInvis` | 10 |
| `angleOffset` | 203 | | `orbitClockwise` | 3 |
| `radius` | 191 | | `healAmount` | 3 |
| `acquireRange` | 181 | | | |

**`coolDownOffset` is used 2,693 times.** It is the second most common argument in the entire content
after `coolDown` itself, and the way most bosses build a rotating or staggered pattern: a dozen
`Shoot` behaviours with the same enormous `coolDown` and offsets 200 ms apart, so each fires once in
sequence and then never again. `Golden Oryx Effigy`'s "Attack2" is 34 `Shoot`s with
`coolDown: 10000000` and offsets from 0 to 7000.

Dropping `coolDownOffset` does not make those bosses slightly wrong — it makes them **fire everything
at once**, once, and then stand still.

`radius` as `Shoot`'s acquire range appears 191 times named and 4,300 more times positionally as the
first argument.

## Loot templates

Eight shared templates, all in `LootTemplates.cs`:

```
Sor3Perc  9    Sor2Perc  7    StatPots  3    Sor5Perc  3
Sor4Perc  3    SorRare   1    Sor1Perc  1    RaidTokens 1
```

These are this fork's Sor-fragment economy, and they are the reason `LootTemplates` exists at all;
the original inlined every drop.

## Patterns worth knowing before writing a converter

**States are deeply nested and inherit behaviours.** A boss's root state carries `ScaleHP`,
`DropPortalOnDeath` and a global `Taunt`; a mid-level state carries the movement; leaf states carry
the shooting. Every level's behaviours tick, and every level's transitions are evaluated.

**Rotation is done with states, not with a rotate parameter.** `rotateAngle` has 12 uses. Everything
else builds rotation as a chain of states with the same `Shoot` at different `fixedAngle` values and a
`TimedTransition` between them — `Crystal Prisoner`'s eight `Quadforce` states, 15 degrees apart,
200 ms each.

**Invulnerability windows are states.** `ConditionalEffect(Invulnerable)` plus a pair of states
flipping every few seconds is how nearly every boss controls when it can be damaged. 641 uses.

**A huge `coolDown` means "once".** `100000`, `320000`, `10000000`, `90000001` all appear. They are
not timings; they are the idiom for a behaviour that should fire on entering a state and never again.

**`TossObject` with an explicit `angle` is how formations are placed.** Eight `TossObject`s at 45
degree intervals is the standard ring. 389 uses, 246 with an explicit angle.

**Two scripts are near-duplicates by copy-paste.** `Oryx the Mad God 2` and
`Oryx the Mad God 2OA` are identical except for the loot table; `Deathmage`'s four skeleton types
share one state machine written out four times. A converter that deduplicates will produce fewer
programs than there are entries, and that is correct.

## Notes from the files read line by line

- **`Golden Oryx Effigy`** uses `SetAltTexture(minValue, maxValue, cooldown, loop)` — the animating
  four-argument form — alongside the single-value form. Both are in use.
- **`Rock Dragon`** builds a ten-segment body where each `Body Segment` protects the one in front of
  it and falls back to the head when that one dies, with `EntitiesNotExistsTransition` as the
  fallback. `Body Segment G` protects `Body Segment E` while its transition watches `F` — a
  copy-paste slip in the content, and the kind of thing a converter must reproduce rather than fix.
- **`Skull Shrine`**'s comment `// add prediction after fixing it...` next to `predictive: 1` says the
  original's own authors did not trust their prediction code.
- **`Arachna the Spider Queen`** has `Shoot(3000, ...)` — an acquire radius of 3,000 tiles, which is
  the whole map. Deliberate: the web spokes must fire regardless of where anyone is.
- **`Mysterious Crystal`** is a damage-per-second check dressed as a fight: it heals itself through
  `HealGroup(1, "Crystals")` every 5 seconds and only breaks if a group out-damages that. The 60
  second `TimedTransition` to "Fail" is the failure branch.
- **`Beach zone`'s Masked Party God** is twelve states of `Taunt` + `SetAltTexture` + a 500 ms
  transition, and nothing else. It never fights.
- **`Misc.cs`** carries a note that Black Cat and Snowman lack their upstream `PetFollow` because this
  server has no pet entity.
- **Janus's eight keys are the largest use of `MoveTo2`** — 16 of its 29 uses, all `once: true`,
  moving a key six tiles toward or away from the boss. Since [the latch never fires](README.md), every
  one of those keys moves continuously instead of once, and walks through the wall while doing it.
  This is the place the `MoveTo2` bug actually shows.
- **`Belladonna`** is the only script that names a `const float` and does arithmetic on it
  (`fixedAngle_RingAttack2 * 3`), so a converter parsing literal arguments must fold constants.
- **`CrawlingDepths`** has a large commented-out first draft of its boss that uses `If` and
  `EntityCountGreaterThan` — the only two appearances of either construct in the tree, and both are
  inside the comment.
- **`Ghost King`** and **`Lich`** and **`Ent Ancient`** all share one structure: a starting state that
  measures how fast its health drops over six seconds and branches to `HugeMob` / `Mob` /
  `SmallGroup` / `Solo`. That is the original's group-size detector, and it is built entirely from
  four `HpLessTransition`s in descending order.

## What this server does differently

Our converter is measured by the `gaps` census, which reports every construct it could not translate.
This page is what that census should be compared against:

- **The 28 unused constructs need no implementation at all**, and four documented "bugs in the
  original" turn out to affect nothing.
- **`coolDownOffset` is the single highest-value missing argument.** 2,693 uses, and dropping it
  collapses a staggered boss pattern into one simultaneous volley.
- **`Shoot`'s acquire radius** is the other one: 4,498 uses, all positional.
- A converter should **not** normalise a huge `coolDown` — it is load-bearing idiom.
- Deep state nesting with behaviour inheritance is the norm, not the exception; a flat state machine
  cannot represent this content.
