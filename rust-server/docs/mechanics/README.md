# Game mechanics, as the original implements them

Written by reading `Server-Side/` file by file, so that the Rust server can be made to match it
exactly. Each page states what the C# does, with its real numbers, and then notes where this server
currently differs.

The goal is one-for-one behaviour. Optimisation is welcome anywhere it cannot be observed from
inside the game: a different data structure, a different loop, a different order of work. A different
*outcome* is not an optimisation.

## What "one-for-one" means here

- Same numbers. A cooldown of 600ms is 600ms, not "about half a second".
- Same order. Where the original ticks A before B and the two can affect each other, so do we.
- Same shape of randomness. Where it picks one of four diagonals, we pick one of four diagonals; a
  smooth heading is a different game even though both look like wandering.
- Bugs in the original are copied only when a player could have come to rely on them, and are named
  as bugs where they are not. Each page says which.

## Status

| Page | Covers | Source files read |
| --- | --- | --- |
| [01-behaviour-engine.md](01-behaviour-engine.md) | How states, behaviours and transitions tick | 5 of 5 |
| [02-behaviours.md](02-behaviours.md) | The behaviour library | 75 of 75 |
| [03-transitions.md](03-transitions.md) | The transition library | 20 of 20 |
| [04-entities.md](04-entities.md) | Ticking, movement, collision, condition effects | `Entity.cs` |
| [05-projectiles.md](05-projectiles.md) | Flight, hit boxes, who decides a hit | `Projectile.cs` |
| [06-loot.md](06-loot.md) | Tables, thresholds, required drops, bags | `Loots.cs`, `World.cs` |
| [07-player.md](07-player.md) | Tick order, damage, death | `Player.cs` |
| [08-stats.md](08-stats.md) | The eleven stats, boosts, and the stat numbering | `StatsManager.cs`, `Stats.cs`, `BoostStatManager.cs`, `XmlDescriptors.cs` |
| [09-worlds.md](09-worlds.md) | The two tick loops, entry and exit, passability | `World.cs` |
| [10-realm.md](10-realm.md) | Population, taunts, events | `Oryx.cs` |
| [11-experience-and-fame.md](11-experience-and-fame.md) | Damage credit, experience, fame counters, boost stacking | `DamageCounter.cs`, `FameCounter.cs`, `ActivateBoost.cs` |
| [12-entity-kinds.md](12-entity-kinds.md) | Enemies, characters, containers, static objects | `Enemy.cs`, `Character.cs`, `Container.cs`, `StaticObject.cs` |
| [13-items-and-shops.md](13-items-and-shops.md) | Potion stacks, merchant reloading, buying | `ItemStacker.cs`, `Merchant.cs`, `SellableObject.cs` |
| [14-world-subclasses.md](14-world-subclasses.md) | The ten worlds with their own logic | `worlds/logic/*.cs` |
| [15-portals-and-decoys.md](15-portals-and-decoys.md) | Portals, player dungeons, decoys | `Portal.cs`, `Decoy.cs` |
| [16-sight.md](16-sight.md) | The four visibility modes | `Sight.cs` |
| [17-collision-index.md](17-collision-index.md) | Chunked index, and which enemies tick | `Collision.cs` |
| [18-levelling.md](18-levelling.md) | Experience curves, levelling, fame, stars | `Player.Leveling.cs` |
| [19-what-a-client-is-told.md](19-what-a-client-is-told.md) | Which entities a player may know about | `Player.Update.cs` |
| [20-maps.md](20-maps.md) | Scenery versus entities, tile ids, regions | `Wmap.cs` |
| [21-abilities.md](21-abilities.md) | Activate effects, the wisdom modifier | `Player.UseItem.cs` |
| [22-trading.md](22-trading.md) | Requests, offers, the exchange | `Player.Trade.cs`, `AcceptTradeHandler.cs` |
| [23-weapon-damage-and-the-rest.md](23-weapon-damage-and-the-rest.md) | Weapon damage, keep-alive, traps, placeholders | `Player.Projectiles.cs`, `BaseStatManager.cs`, `Player.KeepAlive.cs`, `Trap.cs` |
| [24-setpieces.md](24-setpieces.md) | The placement table and how a piece draws | `SetPieces.cs`, `Pentaract.cs` |
| [25-regions-and-coverage.md](25-regions-and-coverage.md) | Region numbering, and what was read | `JsonMap.cs` |
| [26-verification.md](26-verification.md) | What a client is believed about, and the fire-rate check | `Player.Verify.cs`, `Player.AntiCheat.cs`, `PositionTimeline.cs` |
| [27-marketplace.md](27-marketplace.md) | Consignment shelves, merchant rotation, listing fees | `Market.cs`, `Player.Market.cs` |
| [28-chat.md](28-chat.md) | Six channels, emote gating, the filter list | `ChatManager.cs` |
| [29-connecting.md](29-connecting.md) | The queue, reconnect tokens, world resolution, Nexus portals | `ConnectManager.cs`, `ConnectionQueue.cs`, `RealmManager.cs`, `PortalMonitor.cs` |
| [30-the-server-loop.md](30-the-server-loop.md) | The two loops, tick debt, world timers | `FLLogicTicker.cs`, `NetworkTicker.cs`, `WorldTimer.cs` |
| [31-commands.md](31-commands.md) | Registration, ranks, and every command | `Command.cs`, `UnrankedCommands.cs`, `RankedCommands.cs` |
| [32-fame-bonuses.md](32-fame-bonuses.md) | The 25 statistics and the 20 compounding bonuses | `FameStats.cs` |
| [33-content-loading.md](33-content-loading.md) | XML passes, duplicate handling, worlds, settings | `XmlData.cs`, `Resources.cs`, `WorldData.cs`, `AppSettings.cs` |
| [34-persistence.md](34-persistence.md) | The key space, locking, currency, death, the records | `Database.cs`, `DbModels.cs` |
| [35-descriptors.md](35-descriptors.md) | Every default, the third stat numbering, four dead fields | `XmlDescriptors.cs` |
| [36-the-shared-random-stream.md](36-the-shared-random-stream.md) | The lockstep generator, its three draws, and why an extra one desyncs | `wRandom.cs`, `StatsManager.cs` |
| [37-the-wire.md](37-the-wire.md) | Framing, RC4, pooling, keep-alive, disconnect saves | `Client.cs`, `Server.cs`, `CommHandler.cs` |
| [38-handlers.md](38-handlers.md) | Dispatch modes, login, inventory rules, portals, hit claims | `networking/handlers/` |
| [39-fork-economy.md](39-fork-economy.md) | Forging, prestige, Onrane, Sor — this fork's own, and mostly broken | `ForgeItemHandler.cs`, `PrestigeHandler.cs` |

Page 25 lists exactly what was read and states the case for the groups assessed by census rather
than file by file. See "How much of this is actually read" below before trusting any of it.

## The largest defect found

**670 of the 758 item stat bonuses in the content are applied to the wrong stat or to no stat at
all.** The content's `stat="N"` numbers are not `StatsType` values; the original translates them
twice, and this server translates them not at all. 587 bonuses are silently discarded because the
index falls outside an eight-element array; 83 more raise defence instead of magic.

Every ring of attack, every armour's defence bonus, every speed, vitality, wisdom and dexterity
bonus in the game. See [the stats page](08-stats.md).

## How much of this is actually read

The C# server is **547 files**. About **250** were opened and read, including every behaviour, every
transition, every command and every world subclass. The rest was assessed by census,
by signature, or by call site, which is weaker evidence and is how the first two passes of this audit
reached wrong conclusions twice.

The Rust server is **101 files, 62,838 lines**, and **none of it was read end to end.** Every claim
in these pages about what *this* server does was checked by looking up the one function named. That
finds a divergence when you already suspect one and finds nothing when you do not: the stat-numbering
bug was found from the C# side and then confirmed here, and nothing in this audit was ever found by
reading the Rust first.

Where a page says "we should confirm", that is exactly what it means: the C# side is known, the Rust
side is not.

## Bugs in the original

Fixed here rather than reproduced, and named where they are fixed. Four found so far, all of the
same shape: a behaviour writing to its own field instead of to the entity's state, so one entity's
misfortune becomes permanent for every entity running that program.

- `Wander`, `Follow`, `Orbit`, `Protect`, `StayAbove`, `StayCloseToSpawn`, `Charge`: `Paralyzed` sets
  `speed = 0` on the shared behaviour. One paralysed enemy freezes every enemy of that type, forever.
- `HealGroup`: `Stunned` sets the shared cooldown to `999999999`. One stunned healer ends group
  healing for that program, permanently.
- `TossObject`: the same line assigns to a local that is never read, so a stun does nothing at all.
- `Shoot`: `rotateCount` lives on the behaviour, so two of the same enemy rotate their volleys in
  lockstep.
- `Spawn` and `RelativeSpawn`: the child counter is set to `initialSpawn` and then incremented once
  per child, so a spawner believes it has made twice as many as it has.
- `Timed`: the countdown is inside the loop over children, so a wrapper with three children completes
  three times as fast as its period says.
- `OnParentDeathTransition`: both its flags are fields on the shared transition. Only the first
  entity ever subscribes, and when one parent dies every entity of that kind transitions at once.
- `DamageTakenTransition`: `wipeProgress` zeroes a local immediately before that local is recomputed,
  so the argument does nothing.
- `GroundTransform` with `relativeX`/`relativeY` returns before saving the tiles it changed, so that
  variant never restores them even when it should.
- `PlayerTextTransition`: `_transition` and `_player` are fields on the shared transition, so one
  player speaking wakes every enemy of that kind in every world at once.
- `TileDesc.PushY`: the guard reads `dy` off the `Ground` element and the value off `Animate`, so
  every pushing tile in the game pushes horizontally only.
- `ActivateEffect.DurationMS2` and `ObjectId2`: both parsed into the field above them, so neither is
  ever set.
- `MoveTo2`: the `once` latch tests the flag it is about to set, and against exact float equality with
  a target approached by normalised steps, so it never fires. It also uses `Move` rather than
  `ValidateAndMove`, so it walks through walls.
- `ReproduceGroup`: the region filter compares the host against itself rather than against the
  candidate tile, so it is always true and a birth can land anywhere in the region on the map.

## What the reading has found so far

The behaviour library is not one-for-one, and the gap is largest in the most-used behaviour in the
game. `Shoot` has twelve parameters and this server implements five of them. The acquire radius is
dropped for every one of its 4,486 uses, and the cooldown offset that staggers a boss's volleys is
dropped for 2,663 of them.

An earlier pass of this audit compared class names with scripts and reported the behaviour library
complete. It is complete in the sense that every name resolves to something; it is not complete in
the sense that matters.

### One root cause behind most of it

Our compiler reads each argument by a name it guesses and a positional index it guesses, and neither
is checked against the C# constructor. Both fail silently, because a missed argument falls back to a
plausible default:

- **Wrong name.** `HealGroup`'s amount is `healAmount`; we look for `amount`. Every explicit amount
  is missed.
- **Wrong index.** `HealGroup` is `(range, group, coolDown, healAmount)` and `HealEntity` is
  `(range, name, healAmount, coolDown)`. We use the second order for both.
- **Wrong default.** The healers restore to full when no amount is given; ours default to 100.
- **Wrong unit.** `ChangeSize` steps once per 150ms; we step per second, so every resize is 6.67
  times too slow.

None of these produced an error, a warning, or a failing test. The fix is not a series of patches:
it is a check that every parameter of every C# constructor is either consumed or listed as
deliberately ignored, so the next one cannot be silent.

### Structural findings

- `MoveLine` never reports `Completed`, so any `Prioritize` above it latches forever and any
  `Sequence` containing it freezes. That is by design in the original, and the shape of several
  enemies depends on it.
- `Spawn` counts children for the lifetime of the enemy; `Reproduce` counts what is alive. One is a
  budget, the other a population cap, and we treat both as the latter.
- `StayCloseToSpawn` anchors to the position where the *state* was entered; `ReturnToSpawn` anchors
  to the entity's spawn point. We use the spawn point for both.

Pages are added as the reading proceeds. A page is only written from files actually read, never from
a grep of their names: an earlier pass of this audit compared surfaces with scripts and concluded
the behaviour library was complete, and it is not.
