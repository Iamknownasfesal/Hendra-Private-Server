# How a dungeon is wired

[Page 43](43-the-behaviour-scripts.md) counts what the content uses. This page is about the shapes it
builds out of them. The 61 scripts register **746 entities**, and they are not 746 monsters: a large
share exist only to sequence, gate or reward a fight. A converter that treats every entry as "a
monster with a state machine" will translate them all correctly and still produce a dungeon that
never opens its gates.

Classifying every live `.Init` entry by what its behaviours can do:

| Kind | Entries |
| --- | --- |
| Moves and shoots | 482 |
| **Inert — never moves, never shoots** | **115** |
| Stationary shooter (turrets, obelisks, statues) | 96 |
| Moves but never shoots | 53 |

The 115 inert entries are the wiring. They are switches, timers, controllers, chest spawners, ground
changers and props.

## The orchestrator

An entity with `ConditionalEffect(Invincible)`, no attack of any kind, and a set of states that do
nothing but `Order` other entities. Sixteen entities `Order` without ever shooting:

```
Shatters      shtrs obelisk controller, shtrs obelisk timer, shtrs king timer,
              shtrs Loot Balloon Bridge
HauntedCeme   Area 1 Controller, Area 2 Controller, Area 3 Controller, Area 4 Controller
Draconis      NM Red / Blue / Green / Black Dragon Soul
SnakePit      Snakepit Button, Snakepit Guard Spawner
GarnetJade    Encounter Altar
Lab           Dr Terrible
```

`Order(range, targetEntity, targetState)` is a broadcast: it moves *every* entity of that type within
range into that state. That is why the orchestrator can be a single invisible object in the middle of
an arena.

**Two orchestrators in a cycle is the standard timing loop.** Shatters runs its bridge phase on a
pair: `shtrs obelisk timer` orders `shtrs obelisk controller` into "obeliskshoot", waits 10,000 ms,
orders it into "guardiancheck", waits 7,000 ms, and repeats; the controller in turn orders the four
obelisks. Neither holds the timing alone. `shtrs king timer` is the same shape with one job — after
28,000 ms it orders The Forgotten King into "heheh", which is otherwise an unreachable state.

**A converter must therefore keep `Order` as a world-level broadcast**, not as a message to a
specific instance. The orchestrator does not know which obelisks exist.

### Ordering a state that does not exist freezes the target

`Order` resolves its state name by walking the *target type's* state tree
(`FindState`) and caching the result. A name that matches nothing leaves `_targetState` at `null`,
and the loop below it does not check:

```csharp
foreach (var i in host.GetNearestEntities(_range, _children))
    if (!i.CurrentState.Is(_targetState))
        i.SwitchTo(_targetState);
```

`Is(null)` is false, so every matching entity gets `SwitchTo(null)`. `SwitchTo` assigns
`CurrentState = null`, and `TickState`'s `while (state != null)` loop then runs zero iterations — no
behaviours, no transitions, forever. The entity is still solid and still killable; it simply stops
being a program. Because the failed lookup is retried every tick, the order keeps re-freezing
anything that wanders into range.

`HauntedCeme` does this three times. Its states are named `arena3wave2`, `arena4wave1`,
`arena4wave2`, and three `Order` calls spell them `aren3wave2`, `aren4wave1`, `aren4wave2`:

```csharp
new Order(9999, "Arena South Gate Spawner", "aren3wave2"),
new Order(9999, "Arena East Gate Spawner",  "aren4wave1"),
new Order(9999, "Arena South Gate Spawner", "aren4wave2"),
```

At range 9,999 that is every such spawner in the world.

**This server already diverges here, and correctly.** `World::order_into`
(`crates/sim/src/world.rs`) resolves the state name first and returns without doing anything if the
target has no state by that name. Reproducing the C# would mean reproducing a permanently dead wave
in the Haunted Cemetery, which no player could have come to rely on. Leave it diverged.

## The gate

Three ways the content opens a path, all of them "an entity dies and the ground changes":

| Mechanism | Uses | What it does |
| --- | --- | --- |
| `ChangeGroundOnDeath(from[], to[], radius)` | 14 (13 of them in Shatters) | swaps one tile set for another around the dying entity |
| `RemoveObjectOnDeath(objectId, radius)` | 12 | deletes a placed object — the wall or gate itself |
| `ReplaceTile(from, to, radius)` | 2 | same swap, but as a live behaviour rather than on death |

Shatters' bridges are built from two families of invincible entities sitting on the bridge tiles,
each with an inert "Idle" state and one working state that changes the ground and then `Suicide()`s:

- **`shtrs Bridge Closer`, `Closer2`, `Closer3`, `Closer4`** close a path —
  `ChangeGroundOnDeath({"shtrs Bridge"}, {"shtrs Pure Evil"}, 1)`.
- **`shtrs Spawn Bridge`, `2`, `3`, `5`** open one — `{"shtrs Pure Evil"}` back to `{"shtrs Bridge"}`
  or `{"shtrs Shattered Floor"}`. (`6`, `7` and `8` exist only inside a comment.)

The gating condition lives in the Idle state's transition —
`EntityNotExistsTransition("shtrs Abandoned Switch 3", 500, "Open")` on `Spawn Bridge 2`,
`EntityNotExistsTransition("shtrs Twilight Archmage", 500, "Open")` on `Spawn Bridge 3` — so *pulling
a switch* means *killing the switch entity*, and *beating a boss* opens the next bridge by exactly
the same mechanism.

`ReplaceTile("Dark Cobblestone", "Hot Lava", 0)` with radius **0** appears twice, in
`shtrs king lava1` and `shtrs king lava2`. Radius zero converts exactly the tile the entity stands on,
so the map author places one of these per tile they want to become lava, and the King orders them all
at once.

**A door that opens is a `Decay(0)` state.** There are 18 of them. `Decay`'s parameter is a lifetime
in milliseconds, defaulting to 10,000, so `Decay(0)` means "remove me now": the gate object watches
its switch with `EntityNotExistsTransition` and, when the switch dies, enters a state whose only
behaviour is that. `shtrs Wooden Gate` watching `shtrs Abandoned Switch 1` is the plainest example in
the tree.

## The reward

Loot almost never sits on the boss. The pattern is a **chest spawner** plus a **loot balloon**:

1. An inert entity — `shtrs Chest Spawner 1`, `Spider Egg Sac`, `Encore Huntress Statue` — sits in the
   room, `Invincible`, in "Idle".
2. Its Idle state carries `EntityNotExistsTransition(bossName, range, "Open")`, or
   `EntitiesNotExistsTransition` when several bosses must all be dead.
3. "Open" is `TransformOnDeath(lootHolder)` + `Suicide()`. Twenty-seven entities have exactly this
   pair.
4. The transformed entity — nine of them are named `... Loot Balloon ...` — carries the `Threshold`
   and `TierLoot`/`ItemLoot` list, and usually a short `TimedTransition` so it pops on its own.

The damage credit is carried across by hand. `CopyDamageOnDeath(target)` — 4 uses, three of them the
Shatters bosses — copies the dying boss's damage record onto the balloon so that soulbound loot
reaches the people who did the work. `TransferDamageOnDeath` (3 uses: Hermit God, Pentaract twice)
moves it instead of copying.

**Miss `CopyDamageOnDeath` and every Shatters drop becomes unsoulbound**, because the balloon has no
damage record of its own.

## The phase gate

`EntitiesNotExistsTransition(range, targetState, name1, name2, ...)` — 112 uses — is how a boss waits
for its adds. The Forgotten King's "crystals" state spawns four crystals and holds until all four are
gone. Only the Green Crystal carries `HealGroup(30, "Crystals", healAmount: 1500)`, so the group must
kill it first or nothing else dies; the other three orbit `shtrs Crystal Tracker`, an entity whose
entire program is `Follow(2, 10, 1)` and which exists only to be a moving orbit centre.

That last idiom is worth naming: **an entity used as a coordinate**. It has no combat role at all.
`Orbit(speed, radius, acquireRange, targetName)` needs something to orbit, so the content spawns one.

## The group-size detector

`Ghost King`, `Lich` and `Ent Ancient` share one structure, and it is the closest thing the content
has to difficulty scaling: an opening state that watches its own health for six seconds and branches
through four `HpLessTransition`s in descending order into `HugeMob` / `Mob` / `SmallGroup` / `Solo`.
Faster damage means an earlier threshold trips, which means a harder fight. Nothing counts players.

## The staggered volley, at scale

Every Shatters Bridge Obelisk has a "Shoot" state holding **51 `Shoot` behaviours**, identical except
for `coolDownOffset` stepping 200 ms from 0 to 10,000, all with `coolDown: 10000`. Six obelisks is 306
`Shoot` behaviours describing one ten-second sweep, driven by the controller pair above.

This is the extreme of the idiom described on page 43, and it is the clearest statement of why
`coolDownOffset` cannot be skipped: without it each obelisk fires 51 volleys on the same frame and
then stands silent for ten seconds.

## Where the guard on a repeated order belongs

The C# broadcasts every tick and guards each *target*:

```csharp
if (!i.CurrentState.Is(_targetState))
    i.SwitchTo(_targetState);
```

`Is` walks the parent chain, so a target already in the ordered state *or any descendant of it* is
skipped. The order means "put anyone not already here, here", every tick, for as long as the orderer
stays in its ordering state.

This server used to guard the *sender* instead: `Primitive::Order` emitted at most one order per
1,000 ms and the receiving side re-entered unconditionally. Re-entry is not free — `Mind::enter`
zeroes `in_state_ms` and `deadline_ms` and clears every cooldown slot in the state's ancestry — so a
target under a standing order had its state restarted once a second.

The Shatters bridge phase is where that showed. `shtrs obelisk controller` sits in "obeliskshoot" for
the full ten seconds, continuously ordering the four obelisks into "Shoot", and "Shoot" is the
51-behaviour sweep described above. Restarting it every second meant only the first second of the
sweep ever fired, ten times over.

**The guard now sits where the original puts it.** `World::order_into` skips a target already within
the ordered state — `Mind::is_within`, which walks up from the landing child, because entering a
state lands on its innermost descendant and never on the named state itself. `Primitive::Order` no
longer throttles, so an entity wandering into range is caught on the next tick rather than up to a
second later, and `order_once` still fires once per state entry, which is what `OrderOnce`'s 10 uses
expect.

## What a converter has to get right

- **`Order` is a broadcast by type within a radius**, and orchestrators depend on it reaching
  entities they never spawned.
- **Unreachable-looking states are usually orchestrated.** The King's "heheh" has no transition into
  it from within the King. Pruning states with no inbound transition would delete it.
- **Entity ids are matched case-insensitively**, and an unknown id becomes a `Pirate` rather than an
  error. See [page 43](43-the-behaviour-scripts.md).
- **`TransformOnDeath` + `Suicide` in the same state is a spawn, not a death.** The pair appears 27
  times and is how every chest in the game appears.
- **Damage records must survive a transform** where `CopyDamageOnDeath` or `TransferDamageOnDeath`
  says so.
- **Inert entities still need to tick.** 115 of the 746 never move and never shoot; if a converter or
  the collision index skips entities that cannot fight, every gate in the game stops opening.
