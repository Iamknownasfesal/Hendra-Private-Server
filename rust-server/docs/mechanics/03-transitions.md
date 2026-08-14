# Transitions

**All 20 read from the file.**

A transition is ticked for the state the entity is in and for every ancestor of that state. The
first that returns true wins and the rest are skipped, so order matters: innermost state first, and
within a state, declaration order.

`SelectedState` picks among several named targets. Most transitions name one.

---

## TimedTransition — 1,383 uses

```
TimedTransition(time, targetState, randomized = false)
```

Counts down from `time`, or from `Random.Next(time)` on the first tick when `randomized`. On firing
it **re-arms to the full `time`** rather than clearing, so a transition that fires and is re-entered
starts a fresh full countdown.

## HpLessTransition — 248 uses

```
HpLessTransition(threshold, targetState)
```

```
HP / MaximumHP < threshold
```

A **fraction**, strictly less than, measured against the enemy's *current* maximum — so an enemy
whose maximum was raised by `ScaleHP` needs proportionally more damage to trip it.

Our name for this is `hp_below`, which says what it does.

## HpBoundaryTransition

```
HpBoundaryTransition(thresholds[], targetStates[])
HpBoundaryTransition(threshold, targetState)
```

A ladder. Each threshold fires **once**, in order, and selects the target state at the same index.
When the list is exhausted it never fires again.

Measured against `host.ObjectDesc.MaxHP` — the **content's** maximum, not the scaled one. That is the
opposite of `HpLessTransition`, and it means a scaled boss crosses its phase boundaries at the same
absolute health it always would.

## PlayerWithinTransition — 135 uses

```
PlayerWithinTransition(dist, targetState, seeInvis = false)
```

Nearest player within `dist`, honouring `IsVisibleToEnemy` unless `seeInvis`. Ten uses in the content
set `seeInvis`.

## NoPlayerWithinTransition

```
NoPlayerWithinTransition(dist, targetState)
```

The same search with **no `seeInvis` option**, so it always respects invisibility. An enemy alone
with an invisible player therefore believes itself alone.

## PlayerTextTransition

```
PlayerTextTransition(targetState, regex, dist = null, setAttackTarget = false, ignoreCase = true)
```

The target state is the **first** argument here and the last in most other transitions, which is what
made this one invisible to our converter for the life of the project.

Two halves. `OnChatReceived` runs a **regular expression** — not a word match — over the text as it is
said, and remembers whether it matched and who said it. `TickCore` then fires while that memory holds,
the speaker is still in the world, and the distance is satisfied.

Details that matter:

- **The distance is measured at tick time, against the speaker's current position**, not where they
  stood when they spoke. Walking away after speaking prevents the transition.
- `dist` is optional; with none given, any distance works.
- `AttackTarget` is **set on every tick the memory holds**, and set to `null` when `setAttackTarget`
  is false — so this transition clears an attack target that something else set.
- **The memory is never cleared by firing.** It is cleared only by the next non-matching thing
  anybody says. So the transition keeps firing every tick until someone says something else.

**Bug in the original, to fix:** `_transition` and `_player` are fields on the **transition**, shared
by every entity running that program. One player saying "Red" transitions *every* enemy of that kind
in *every* world at once. For Draconis, where three souls share one program per colour, that is the
difference between waking one and waking all of them everywhere.

## EntitiesNotExistsTransition — 112 uses

```
EntitiesNotExistsTransition(dist, targetState, targets...)
```

True when **every** named target is absent within `dist`. With no targets given it never fires at
all — the constructor returns early and leaves the list null.

Target state is the second argument, after the distance.

## EntityNotExistsTransition — 120 uses

```
EntityNotExistsTransition(target, dist, targetState, checkAttackTarget = false)
```

Normally: true when no entity of `target` is within `dist`.

With `checkAttackTarget`, it ignores the target and distance entirely and instead asks whether the
host's `AttackTarget` has **gone from the world** — and clears the stale reference before firing.
That is the counterpart to `PlayerTextTransition` setting one: the pair is how a boss latches onto a
player and lets go when they leave.

## EntityExistsTransition

```
EntityExistsTransition(target, dist, targetState)
```

True while an entity of that **type** is within `dist`. Note the asymmetry with the pair below:
this one resolves the name to an object type, while `EntityWithinTransition` matches **by name**
through `GetNearestEntityByName`. Two ways of naming the same thing, in two transitions that read
almost identically.

## GroupNotExistTransition

```
GroupNotExistTransition(dist, targetState, group)
```

True when no member of the group is within `dist`. An empty or whitespace group name returns false
rather than true — so a typo makes the transition never fire, which is the safe direction.

## EntityWithinTransition

```
EntityWithinTransition(dist, entity, targetState)
```

True while an entity of that **name** is within `dist`.

## NoEntityWithinTransition and AnyEntityWithinTransition

```
NoEntityWithinTransition(dist, targetState)     ->  !anyEnemyNearby && !anyPlayerNearby
AnyEntityWithinTransition(dist, targetState)    ->   anyEnemyNearby ||  anyPlayerNearby
```

Neither names a kind. **Both count enemies and players together**, so "any entity" includes the
host's own allies, and "no entity" requires the area to be empty of both.

## EntityHpLessTransition

```
EntityHpLessTransition(dist, entity, threshold, targetState)
```

`HpLessTransition` applied to a *named other* entity — a guard that reacts when the thing it guards
is hurt. Measured against **`ObjectDesc.MaxHP`**, the content's maximum, like `HpBoundaryTransition`
and unlike `HpLessTransition`.

## DamageTakenTransition

```
DamageTakenTransition(damage, targetState, wipeProgress = false)
```

Sums the whole damage ledger — every player's contribution — and fires once the total reaches
`damage`. It is cumulative for the life of the enemy, not per state.

`wipeProgress` does nothing: it zeroes a local immediately before that local is recomputed from
scratch. Dead argument in the original.

## NotMovingTransition

```
NotMovingTransition(targetState, delay = 250)
```

Samples the position, waits `delay`, and compares. **Equal positions fire**; different positions
re-sample and wait again. So it is not "has not moved for `delay`" but "was in the same place at two
samples `delay` apart" — an entity that leaves and returns within the window counts as still.

Movement is measured, not intended, so an enemy walking into a wall counts as not moving.

## GroundTransition

```
GroundTransition(ground, targetState)
```

True while the tile under the host is of the named type. The tile type is resolved on first tick and
cached on the transition, which is safe because it is the same for every entity.

## TimedRandomTransition

```
TimedRandomTransition(time, randomizedTime = false, states...)
```

The target state is the **first** argument and the states are variadic at the end — another one our
converter had to be told about.

The first wait is `Random.Next(time)` when `randomizedTime`, and `time` exactly afterwards. On firing
it picks a target uniformly at random from the list.

## OnParentDeathTransition

Fires when the entity that spawned this one dies.

**Bug in the original, to fix:** `parentDead` and `init` are fields on the *transition*, shared by
every entity running that program. So the first entity to tick is the only one that ever subscribes,
and when any one parent dies, every entity of that kind in every world transitions at once. Per
entity is the only sane reading.

---

## What this server does differently

- `HpBoundaryTransition` measures against the content maximum and `HpLessTransition` against the
  live maximum. We should check we have not collapsed the two.
- `DamageTakenTransition` is cumulative across states; a per-state reading would fire far more often.
- The two transitions whose target state comes first — `PlayerTextTransition` and
  `TimedRandomTransition` — are handled, along with `EntitiesNotExistsTransition`, whose target is
  second. That list is in `transpile.rs` and is now covered by a test.
- **`PlayerTextTransition` matches a regular expression**, not a whole word. Ours matches whole words,
  which is right for every use in the content (all plain colour words) and wrong the day one is not.
- **`EntityNotExistsTransition(checkAttackTarget: true)`** is a different test entirely — "has my
  attack target left the world" — and clears the reference. We have no attack-target concept at all.
- **`NoEntityWithin` and `AnyEntityWithin` count players and enemies together.**
- **`PortedTransitions.cs`** holds one class, `EntityNotExistTransition`, a subclass of
  `EntityNotExistsTransition` differing by one letter. It is this project's own file, added so the
  imported scripts compile.
