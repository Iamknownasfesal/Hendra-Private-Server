# Entities: ticking, movement, collision, condition effects

Read from `realm/Entity.cs`.

## An enemy with nobody near it does not think

```
Tick:
  if this is a Projectile or has no world: return
  if CurrentState != null
     and not Stasis
     and not TickStateManually
     and (AnyPlayerNearby() or ConditionEffects != 0):
         TickState(time)
  if player: posHistory[++posIdx] = position
  ProcessConditionEffects(time)
```

Three things fall out of this and all three are mechanics, not optimisations:

1. **A state machine only runs while a player is within sight radius**, unless the entity carries a
   condition effect. An enemy alone in a room is frozen: its cooldowns do not advance, its spawners
   do not spawn, its timers do not count down. Walking away and coming back finds it exactly as it
   was.
2. **An entity holding any condition effect ticks regardless**, which is how a poisoned enemy keeps
   burning down in an empty room and how a `Decay` timer still expires.
3. **`Stasis` freezes the state machine entirely**, separately from the pause on movement.

The position history is 256 entries, written every tick, indexed by a wrapping byte. Only players
keep one. `TryGetHistory(ticks)` reads back up to 255 ticks and is what `Shoot`'s prediction uses.

## Switching state always descends to the first leaf

```
SwitchTo(state):
  CurrentState = state
  while CurrentState.States.Count > 0:
      CurrentState = CurrentState.States[0]
  commonRoot = CommonParent(oldState, CurrentState)
  stateEntry = true
```

Naming a parent state enters its **first child, recursively**. A transition to a state that has
children never lands on that state itself.

## The tick order within a state

```
if stateEntry:
    for s from CurrentState up to commonRoot:
        every behaviour of s gets OnStateEntry

transited = false
for state from CurrentState up to the root:
    if not transited:
        for each transition of state:
            if it fires: transited = true; break
    for each behaviour of state:
        tick it

if transited:
    for s from originalState up to commonRoot:
        every behaviour of s gets OnStateExit
```

Two consequences worth writing down:

- **Behaviours still run on the tick a transition fires**, for the state that fired it and for every
  ancestor. The state change takes effect from the *next* tick.
- **Only one transition fires per tick across the whole chain.** Once one fires, ancestors'
  transitions are skipped — but their behaviours are not.

## Movement: sub-stepping, then sliding

`ValidateAndMove(x, y)` is what every movement behaviour calls.

```
if Paralyzed or Petrify: do not move at all
if |dx| < 0.4 and |dy| < 0.4: one CalcNewLocation and done
otherwise: walk the segment in steps of 0.4 / max(|dx|, |dy|) of the way, calling
           CalcNewLocation at each step
```

The 0.4-tile sub-step is what stops a fast entity tunnelling through a wall. **`Petrify` blocks
movement as completely as `Paralyzed`** — worth noting, since it is otherwise a damage modifier.

`CalcNewLocation` slides along walls on a **half-tile grid**. It asks whether the move crosses a half
tile in x or in y (`(int)(X / .5f) != (int)(x / .5f)`), and if the destination is blocked it clamps
the crossing axis back to the half-tile boundary minus `0.01`, trying the axis with more room first.
The result is that an entity walking into a wall at an angle slides along it rather than stopping.

`RegionUnblocked` is not a single-tile test. For a position inside a tile it also tests the
neighbouring tiles on whichever side of the half-tile the position sits, so an entity is blocked by a
`FullOccupy` object it is merely *next to*, not only one it is standing on.

Two different notions of blocking:
- `TileOccupied`: the tile is off the map, its ground is `NoWalk`, or its object is
  `EnemyOccupySquare`.
- `TileFullOccupied`: the tile is off the map, or its object is `FullOccupy`.

Off the map counts as blocked in both.

## Condition effects are 51 slots of milliseconds

```
_effects[51]           // per effect: remaining ms, 0 for absent, -1 for permanent
_tickingEffects        // set when anything is applied; cleared when nothing is counting down
```

Each tick, every slot above zero is decremented; a slot reaching zero clears its bit; a slot holding
`-1` keeps its bit forever. The whole bitmask is rebuilt from the array each tick rather than being
edited in place.

`ApplyConditionEffect` with a duration of `0` is how an effect is **removed** — the bit is not set,
and the slot is zeroed.

**Immunities refuse the effect outright** rather than letting it land and be ignored: Stunned,
Stasis, Paralyzed, ArmorBroken, Curse, Petrify, Dazed and Slowed each have a matching `*Immune`.
Nothing else does, so there is no immunity to Sick, Bleeding, Quiet or Weak.

## Speech reaches transitions directly

```
OnChatTextReceived(player, text):
  for state from CurrentState up to the root:
      every PlayerTextTransition of that state hears it
```

Speech is delivered to the transition, not stored on the entity — which is why the transition holds
its own matched flag and why the delivery has to happen before the tick that reads it.

## Entity kinds

`Entity.Resolve` maps the content's `Class` to a runtime type. Worth noting for parity:

- `GameObject`, `CharacterChanger`, `MoneyChanger`, `NameChanger` are all plain static objects with
  health, hit-testable.
- `GuildRegister`, `GuildChronicle`, `GuildBoard` are static objects with no health and no
  hit-testing.
- `Wall` and `DoubleWall` are one type; `ConnectedWall` and `CaveWall` are the connected kind.
- `ClosedVaultChestGold`, `ClosedGiftChest`, `VaultChest` and `Merchant` are all `WorldMerchant`.
- `VaultAccess` and `ClosedVaultChest` are scenery in this fork, because vault capacity became an
  integer on the account. That is a change this project made, not the original.
- Anything unrecognised becomes a bare `Entity` and is logged.

## Projectiles are numbered per owner, 256 at a time

`CreateProjectile` assigns `projectileId++` as a **byte**, so ids wrap at 256, and creating a
projectile in a slot that is still occupied **destroys the old one**. A fast enough shooter can
therefore erase its own bullets in flight.

`HitByProjectile` defaults to "only enemies and players are hit".

## What this server does differently

- **We tick every enemy every tick.** The original freezes any enemy with no player nearby and no
  condition effect. This is a large performance difference and a real mechanical one: our cooldowns
  and spawners keep running in empty rooms.
- **`Petrify` does not stop our movement**, only `Paralyzed` does.
- **Our movement does not sub-step.** We resolve one step per tick against the terrain, so a fast
  entity can cross a thin wall.
- **Our collision is per tile, not per half tile.** The original blocks on `FullOccupy` neighbours
  depending on which half of a tile the position sits in.
- Condition effects: we hold a bitmask plus a list of timed effects, which is equivalent, but we
  should confirm that applying a duration of zero removes an effect.
