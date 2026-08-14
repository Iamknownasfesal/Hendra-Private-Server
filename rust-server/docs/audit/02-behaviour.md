# `hendra-behavior`, against the behaviour pages

9 files, read against [page 01](../mechanics/01-behaviour-engine.md),
[page 02](../mechanics/02-behaviours.md), [page 43](../mechanics/43-the-behaviour-scripts.md) and
[page 44](../mechanics/44-how-a-dungeon-is-wired.md).

The state machine itself is faithful: transitions before behaviours, innermost state outwards, first
match wins, one movement per tick, entry landing on the innermost child. Those are the parts the
`gaps` census can see, and it reports 100%.

The census cannot see arguments. This page is what it misses.

## The census counts names, so it reports a hundred per cent

```
BEHAVIOURS: 0 of 9636 uses unimplemented (100.00% covered)
CONDITIONS: 0 of 2050 uses unimplemented (100.00% covered)
```

Both numbers are honest about what they measure and neither measures what matters. `Shoot` resolves
to `Primitive::Shoot`, so it counts as covered; `Primitive::Shoot` has five fields where the C#
constructor has twelve parameters.

Comparing every C# constructor's parameter list against what `compile.rs` reads — allowing for the
transpiler's renames (`coolDown` → `cooldown`, `projectileIndex` → `projectile`) — **39 of the 118
constructs drop at least one argument.** Most of those drops cost nothing, because the content never
passes the argument. What follows is the ones it does pass, with the count.

| Construct | Argument dropped | Uses that pass it | What it changes |
| --- | --- | --- | --- |
| `Shoot` | `radius` | **4,486** (positional, every use) | the acquire range. Enemies fire at anything within the 20-tile sense radius instead of their own, often 4 to 8. |
| `Shoot` | `coolDownOffset` | **2,663** | the stagger. A rotating pattern becomes one simultaneous volley. |
| `Spawn` | `initialSpawn` | **225** | how many appear on entering the state. |
| `Shoot` | `predictive` | 281 | leading a moving target. |
| `Shoot` | `angleOffset` | 203 | where the spread is centred. |
| `Orbit` | `target` | **137 of 167** | *what is orbited*. See below. |
| `Shoot` | `defaultAngle` | 112 | where to fire with nothing acquired. |
| `Spawn` | `givesNoXp` | 72 | whether the children are worth experience. |
| `Orbit` | `radiusVariance` | 67 | the ring's thickness. |
| `Follow` | `coolDown` | 57 | how often the follow re-aims. |
| `Follow` | `duration` | 53 | how long it follows before stopping. |
| `Orbit` | `speedVariance` | 25 | per-entity speed jitter. |
| `Grenade` | `fixedAngle` | 20 | throwing at a set angle rather than at a target. |
| `Shoot` | `rotateAngle` | 12 | the built-in rotation. |
| `SetAltTexture` | `minValue`..`loop` | 5 | the animating four-argument form. |

`Shoot`'s remaining drop, `shootLowHp`, has no uses. So do `TossObject`'s twelve dropped arguments
except one — `probability` is passed 178 times in the content but almost entirely on `ItemLoot` and
`TierLoot`, never on `TossObject` — and the `region`/`regionRange` pair, which
[page 43](../mechanics/43-the-behaviour-scripts.md) already establishes no script uses.

**The root cause is not the individual drops.** It is that `compile.rs` reads each argument by a name
it guesses and a positional index it guesses, and nothing checks either against the C# constructor.
Both fail silently, because a missed argument falls back to a plausible default. The fix that stops
the next one is a build-time check that every parameter of every C# constructor is either consumed or
named in an explicit ignore list.

## Everything that orbits, orbits the player

`Orbit(speed, radius, acquireRange, target, ...)`. The fourth parameter names an entity, and
`Orbit.cs:39` resolves it:

```csharp
var entity = host.AttackTarget ?? host.GetNearestEntity(acquireRange, target);
```

`GetNearestEntity(dist, objType)` takes `null` to mean "nearest player" and a type to mean "nearest
entity of that type" — the comment in `Utils.cs:116` says so in as many words. So an orbit with a
named target circles that entity.

Ours has no target field at all (`compile.rs:480`), and `run.rs:488` goes straight to
`senses.nearest_player`. **Every orbiting entity in the game circles whoever is closest.**

137 of the 167 uses name a target, and they are the ones that make an encounter readable: the
Forgotten King's four crystals are meant to circle `shtrs Crystal Tracker`, the Royal Guardians to
circle the King. Here they all circle the player instead, which is a different fight — the ring the
player is supposed to move around becomes a ring that follows them.

`AttackTarget` taking precedence is a second, smaller difference: an enemy that has been given a
target orbits it regardless of what the content named.

## A thrown object lands 700 ms early

The C# warning delay is not a parameter. `TossObject.cs` spawns a `WorldTimer(1500, ...)` — a
hardcoded **1,500 ms** between the telegraph and the thing landing.

`compile.rs:545` reads it as an argument:

```rust
warning_ms: number(call, "throw_delay", 9, 800.0).max(0.0) as u32,
```

There is no `throwDelay` in the C# constructor, so the name never matches and the default applies:
**800 ms for all 389 uses**, a little over half the intended telegraph. The comment beside it says
the telegraph is "the difference between a hard fight and an unfair one", which is right, and 800 is
the wrong number.

The positional fallback is worse than unused. Index 9 in the C# parameter list is `maxAngle`, so any
call passing ten or more positional arguments would have an angle read as a delay. Nothing in the
content passes more than seven, so this is latent rather than live — but it is latent in the same way
the rest of the positional guessing is.

## What is right

Worth recording so it is not re-derived:

- The engine's tick order, state entry, and the innermost-child landing all match.
- `Order` now matches, guard included, after [page 44](../mechanics/44-how-a-dungeon-is-wired.md).
- The transpiler strips comments before converting, which is what took the enemy count from 748 to
  the correct 746 and removed the last "bug in the original" the converter reported.
- Loot `probability` is consumed, which is where 170 of the content's 178 uses of that argument are.
- The 33 constructs no script uses need no implementation at all, and five of those were only
  discovered once the census stopped counting commented-out code.
