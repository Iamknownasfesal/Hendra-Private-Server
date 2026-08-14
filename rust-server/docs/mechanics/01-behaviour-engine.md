# The behaviour engine

Read from `logic/Behavior.cs`, `logic/State.cs`, `logic/Transition.cs`, `logic/Cooldown.cs`,
`logic/CycleBehavior.cs`.

## States are a tree, and an entity is in one leaf

A `State` holds child states, behaviours and transitions. `State.Is(other)` walks up parents, so an
entity in a leaf is also "in" every ancestor of that leaf. Behaviours and transitions of every
ancestor run, not only the leaf's.

`SwitchTo` moves the entity to a state; `CommonParent(a, b)` finds where the two branches meet, and
only the states below that point get their entry and exit calls. Moving between two leaves of the
same parent does not re-enter the parent.

## Per-behaviour state lives on the entity, keyed by the behaviour object

`host.StateStorage[behaviour]` holds one object per behaviour instance. Two entities running the
same program each have their own entry. A behaviour that sets its state to `null` has its entry
*removed*, which is how "no state" and "state not yet created" are the same thing.

This matters for one-for-one: a behaviour's storage is created on first tick, not on state entry,
unless it defines `OnStateEntry`.

## Cooldown

```
Cooldown(cooldown, variance)
Normalize()      -> 1000ms when cooldown is 0
Normalize(def)   -> def   when cooldown is 0
Next(rand)       -> cooldown, or cooldown + rand.Next(-variance, variance + 1)
```

`Next` is inclusive of `+variance` and exclusive of nothing on the low side: the range is
`[cooldown - variance, cooldown + variance]`.

## CycleStatus, and why it is the whole engine

A `CycleBehavior` reports one of `NotStarted`, `InProgress`, `Completed` after every tick. Two
containers read it, and nothing else does.

### Prioritize

```
OnStateEntry: index = -1, and every child gets OnStateEntry
Tick:
  if index < 0:                       // selecting
      for i in 0..children:
          children[i].Tick()          // <- the child actually runs
          if status == InProgress { index = i; break }
  else:                               // running
      children[index].Tick()
      if status == Completed or NotStarted { index = -1 }
```

Two consequences that are easy to get wrong:

1. **Selection ticks the children it passes over.** A `Prioritize(StayAbove, Follow, Wander)` that
   ends up on `Wander` has already ticked `StayAbove` and `Follow` this tick, and both of them may
   have acted. It is not "run the first child that wants to act and no others".
2. **Once a child reports `InProgress`, it latches.** Only that child ticks on later ticks, until it
   reports `Completed` or `NotStarted`. The children before it stop running entirely while it holds.

### Sequence

```
Tick:
  children[index].Tick()
  if status == Completed or NotStarted { index = (index + 1) % children.len() }
```

So a child advances the sequence by *finishing*, and a child that is `InProgress` holds the sequence
where it is. A child that never reports anything but `InProgress` freezes the sequence forever.

## Transitions

Every transition of every ancestor state is ticked. The first that returns true switches the entity
and the rest are skipped. A transition may name several target states and pick among them with
`SelectedState`.

Target states are resolved by name once, at load; a name that is not a state is a hard failure at
load time in the original (dictionary lookup), which is why the converter must not emit a name that
does not exist.

## What this server does differently

- **`Prioritize` does not tick the children it passes over**, and does not latch on `InProgress`.
  Ours reads as "first child that wants to act". This changes which enemies shoot while moving.
- **`Sequence`** needs checking against the same rule.
- Our `Mind` holds cooldowns in a flat slot array rather than a per-behaviour map. That is a fair
  optimisation as long as slot assignment is stable, which it is, but it means a behaviour with
  *several* pieces of state has nowhere to put them; the ones below that need more than a cooldown
  are the ones to watch.
