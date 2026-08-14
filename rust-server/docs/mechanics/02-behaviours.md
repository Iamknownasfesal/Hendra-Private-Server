# The behaviour library

One section per class in `logic/behaviors/`, written from the file. Ordered by how often the
behaviour database uses it.

---

## Shoot — 4,486 uses

`behaviors/Shoot.cs`.

```
Shoot(radius, count = 1, shootAngle = null, projectileIndex = 0, fixedAngle = null,
      rotateAngle = null, angleOffset = 0, defaultAngle = null, predictive = 0,
      coolDownOffset = 0, coolDown = 1000ms, shootLowHp = false)
```

Construction:
- `shootAngle` is `0` when `count == 1`, otherwise `shootAngle ?? 360/count`, in degrees, converted
  to radians. So a volley with no explicit angle is spread evenly around a full circle.
- Every angle argument is degrees in, radians held.
- `coolDown.Normalize()` makes a zero cooldown 1000ms.

`OnStateEntry` sets the stored cooldown to `coolDownOffset`, which is how two shoot behaviours in one
state are made to alternate.

Each tick:
1. If the stored cooldown is above zero, subtract the elapsed time, report **InProgress**, done.
2. Otherwise, if **Stunned**, return *without firing and without touching the cooldown*. The
   behaviour retries next tick and reports `NotStarted` for this one.
3. **Dazed** halves the count, rounded **up**: `ceil(count / 2)`.
4. Choose a target: `host.AttackTarget` if one is set, else the lowest-health entity within `radius`
   when `shootLowHp`, else the nearest entity within `radius`. Both searches are the player search,
   which skips players an enemy cannot see.
5. If there is a target, or a `defaultAngle`, or a `fixedAngle`, fire. Otherwise fire nothing.
6. **Either way**, set the cooldown to `coolDown.Next(rand)` and report **Completed**.

Point 6 is the one to get right: holding fire still spends the cooldown. An enemy that cannot see
anybody is not storing up a shot.

Angle:
```
a = fixedAngle
  | predicted(target)         when predictive != 0 and predictive > rand.NextDouble()
  | atan2(target - host)
  | defaultAngle
  | 0
a += angleOffset + rotateAngle * rotateCount     // rotateCount++ on every volley fired
startAngle = a - shootAngle * (count - 1) / 2
```

`rotateCount` lives on the behaviour, not on the entity, so every enemy sharing the program shares
the rotation counter. That is a bug in the original and it is visible: two of the same enemy in one
room rotate together. Copy it only if we decide to; otherwise per-entity is the sane reading.

Damage is rolled **once per volley**, `rand.Next(minDamage, maxDamage)`, and every projectile in the
volley carries that same number. Not once per projectile.

Prediction, when it fires: take the target's position one tick ago, extrapolate four ticks ahead of
the current position, aim there. With no history, aim straight at them.

### What this server does differently, measured against the 4,486 uses

Our `Primitive::Shoot` carries five of the twelve parameters: `count`, `spread`, `fixed_angle`,
`cooldown_ms`, `projectile`. Everything below is emitted by the converter into
`content/behaviours/` and then dropped by the compiler.

| Missing | Uses affected | What a player sees |
| --- | --- | --- |
| `radius` (the acquire range) | **all 4,486** | Enemies fire at anything within the 20-tile sense radius instead of their own range, which is often 4 to 8. They snipe across the screen. |
| `coolDownOffset` | **2,663** | Volleys meant to be staggered fire together. Three shoots offset 500/1000/1500ms are one triple-shot every 3s instead of a rhythm. This is what a boss's pattern *is*. |
| `predictive` | 281 | Nothing leads a moving target. |
| `angleOffset` | 203 | Volleys point where the unrotated aim points. |
| `defaultAngle` | 112 | An enemy with nobody in range holds fire; it should shoot along the default angle. |
| `rotateAngle` | 12 | Rotating volleys do not rotate. |
| Stunned | — | Stunned enemies keep shooting. |
| Dazed | — | Dazed enemies fire a full volley instead of `ceil(count/2)`. |
| Cooldown when holding fire | — | Ours returns early and keeps the cooldown at zero, so it fires the instant a target appears. The original has already spent it. |

The radius is dropped for a specific reason worth recording: the converter emits it as the first
*positional* argument, `shoot(10, count: 5, ...)`, and our compiler reads only named arguments and a
handful of positional indices, none of which is index 0. The comment above that code says the radius
"is a radius that nothing reads", which is not true of `Shoot.cs:145`.

Three of our positional fallbacks are also misaligned with the C# constructor: `shoot_angle` reads
index 9 where the parameter is at 2, `cooldown` reads 8 where it is at 10, and `projectile` reads 7
where it is at 3. Harmless today because the converter names those three, and a trap the day it
does not.

## ConditionalEffect — 626 uses

`behaviors/ConditionalEffect.cs`.

```
ConditionalEffect(effect, perm = false, duration = -1)
```

Applies the effect on **state entry** with the given duration, where `-1` means no expiry. On
**state exit** it applies the same effect with duration `0`, which is how the original removes one —
unless `perm` is set, in which case it stays.

Nothing happens on tick. The whole behaviour is entry and exit, so an enemy that never changes state
keeps the effect forever, and one that does loses it the moment it leaves.

**`perm` means "survives the state", not "lasts while the state does".** With `perm = false`, the
default, the effect is applied on entry and taken off on exit. With `perm = true` it is applied on
entry and **left in place** when the state ends.

The parameter order is `(effect, perm, duration)`. This server reads index 1 as the *duration* and
index 2 as a *range*, so:

| Content | Uses | Original | Ours |
| --- | --- | --- | --- |
| `conditional_effect(invulnerable)` | 324 | on entry, off on exit | same, by renewal |
| `conditional_effect(invulnerable, false)` | 13 | on entry, off on exit | same, by renewal |
| `conditional_effect(invincible, true)` | **48** | on entry, **stays after** | taken off on exit |

So forty-eight uses that are meant to make an effect outlive its state lose it instead. Every one of
them is `invincible` or `invulnerable` on a boss phase.

Our approach of renewing the effect every tick rather than applying it on entry and removing it on
exit is a fair substitute for the default case, and it cannot express the `perm` case at all. It also
depends on the entity ticking, which the original does not: see
[the entity page](04-entities.md) on enemies with nobody nearby.

## Taunt — 311 uses

`behaviors/Taunt.cs`.

```
Taunt(text...)                                    probability 1, cooldown 0, broadcast false
Taunt(probability, text...)
Taunt(broadcast, text...)
Taunt(cooldown, text...)
...and the combinations
```

- **Cooldown 0 means once per state entry.** The stored state is cleared on entry; on the first tick
  it is set, and every later tick returns immediately.
- Otherwise: count down, and on expiry re-arm with `cooldown.Next()` and roll `probability`. Failing
  the roll still re-arms, so a 0.5 probability is half the lines, not half the rate.
- The line is chosen at random from the list.
- `{PLAYER}` is replaced with the name of the nearest player within **10** tiles, and if there is
  nobody, **the taunt is not said at all**.
- `{HP}` is replaced with the host's current health.
- `broadcast` sends to the whole world; otherwise to players within **15** tiles.
- The bubble lasts 3 seconds and the speaker is named `#DisplayId`.

## Wander — 550 uses

`behaviors/Wander.cs`.

```
Wander(speed)
```

Direction is redrawn only when the previous leg is spent:
```
direction = normalize(Vector2(rand even ? -1 : 1, rand even ? -1 : 1))   // one of four diagonals
remainingDistance = 0.6 tiles
status = Completed on the tick a new leg starts, InProgress otherwise
```
Every tick it moves `host.GetSpeed(speed) * dt` along the current direction and subtracts that from
the remaining distance.

So the original wanders in 0.6-tile diagonal legs, always at 45 degrees. **This server drifts a
continuous heading instead**, which is a different motion and a different game to dodge.

`Paralyzed` sets `speed = 0` — on the shared behaviour object, permanently, for every entity running
that program. A real bug; do not copy it. Ours should read paralysis per entity.

## Follow — 374 uses

`behaviors/Follow.cs`.

```
Follow(speed, acquireRange = 10, range = 6, duration = 0, coolDown = duration == 0 ? 0 : 1000)
```

A three-state machine per entity: `DontKnowWhere`, `Acquired`, `Resting`.

- **DontKnowWhere**: with a target in `acquireRange` and no remaining time, become `Acquired` and
  fall through to it immediately; the remaining time is set to `duration` when duration is used.
  Otherwise count the remaining time down.
- **Acquired**: lose the target and go back to `DontKnowWhere`. With `duration > 0` and the time
  spent, go back to `DontKnowWhere`, set the remaining time to `coolDown.Next()`, report
  **Completed**. Otherwise, if further than `range`, report **InProgress** and step toward them;
  if within `range`, report **Completed** and become `Resting`.
- **Resting**: report **Completed** every tick. Re-acquire only once the target is further than
  `range + 1`, which is the hysteresis that stops an enemy stuttering on the boundary.

The step is jittered before it is normalised:
```
vect = target - host
vect.x -= rand.Next(-2, 2) / 2      // one of -1, -0.5, 0, 0.5
vect.y -= rand.Next(-2, 2) / 2
normalize, then move speed * dt
```

**This server has the approach and neither the resting state, the `+1` hysteresis, the duration and
cooldown cycle, nor the jitter.**

## Orbit — 167 uses

`behaviors/Orbit.cs`.

```
Orbit(speed, radius, acquireRange = 10, target = null,
      speedVariance = speed * 0.1, radiusVariance = speed * 0.1, orbitClockwise = false)
```

Note `radiusVariance` defaults to a share of **speed**, not of radius. That is what the file says.

`OnStateEntry` draws the entity's own orbit once:
```
speed  = speed  + speedVariance  * (rand * 2 - 1)
radius = radius + radiusVariance * (rand * 2 - 1)
direction = orbitClockwise == null ? (coin flip: 1 or -1) : (orbitClockwise ? 1 : -1)
```

Each tick, with a target (its `AttackTarget`, else the nearest entity of `target`'s kind in
`acquireRange`):
```
angle = atan2(host - entity)                       // note: host relative to entity
angle += direction * GetSpeed(speed) / radius * dt // angular speed
aim = entity + (cos angle, sin angle) * radius
step toward aim by GetSpeed(speed) * dt
status = InProgress
```
Standing exactly on the target adds a random offset in `[-1, 1]` to both axes before the atan2, so
the angle is defined.

With no target it reports **NotStarted** and does not move.

## TossObject — 389 uses

`behaviors/TossObject.cs`.

```
TossObject(child, range = 5, angle = null, coolDown = 1000ms, coolDownOffset = 0,
           tossInvis = false, probability = 1, group = null,
           minAngle = null, maxAngle = null, minRange = null, maxRange = null,
           densityRange = null, maxDensity = null, region = None, regionRange = 10)
```

Naming a `group` instead of a `child` makes the toss pick one member of the group at random, per
throw.

Each tick, once the cooldown is spent:
1. Roll `probability`. Failing re-arms the cooldown and does nothing.
2. Find the nearest player within `range`. With no player and no fixed `angle`, do nothing **and do
   not re-arm** — it retries every tick.
3. With `densityRange` and `maxDensity`, count the kind (or the whole group) nearby and give up if
   the crowd is already that big. Failing this re-arms.
4. Pick the landing spot: `minRange`/`maxRange` randomise the distance, `minAngle`/`maxAngle`
   randomise the angle, a fixed `angle` overrides both. With no angle at all, the spot is the
   player's current position.
5. `region` overrides everything: the spot is a random tile of that region within `regionRange`,
   measured as a **square** (`|dx| <= r && |dy| <= r`), not a circle.
6. Unless `tossInvis`, everyone nearby sees a throw effect in amber (`0xffffbf00`).
7. **1500ms later** the child appears — if the landing square is passable. If it is not, nothing
   spawns and nothing is refunded.

A child inherits the host's terrain, and if the host was itself spawned, the child is marked spawned
and given permanent `Invisible`.

**Bug in the original, to fix rather than copy:** the stun check assigns to a local it never uses
(`var coolDown = _coolDown; if (Stunned) coolDown = 999999999;` and then `_coolDown.Next()` is what
is stored). Stunned enemies toss exactly as fast as unstunned ones. The comment in the file says
"same thing as in grenade.cs". Here a stun should stop the toss.

## Spawn — 364 uses

`behaviors/Spawn.cs`.

```
Spawn(children, maxChildren = 5, initialSpawn = 0.5, coolDown = 0, givesNoXp = true)
```

`initialSpawn` is a **fraction of maxChildren**, floored: the default makes half of them at once.

On state entry it spawns that many at the host's exact position. Afterwards, one child per cooldown
while the count is below `maxChildren`.

Two things to match carefully:

- **The count never goes down.** It is a lifetime cap, not a population cap: killing the children
  does not let the spawner make more. This server counts what is actually standing nearby instead,
  which turns a fixed budget into an endless one. That is a real difference in difficulty.
- **Bug in the original, to fix:** entry sets the count to `initialSpawn` *and* increments it once
  per child spawned, so it ends at twice `initialSpawn`. A spawner with `maxChildren: 5` makes two
  immediately, believes it has four, and only ever makes one more.

`givesNoXp` defaults to **true**, so spawned children are worth nothing unless the content says
otherwise. A child also inherits the host's terrain, and the host's own `GivesNoXp` when its own is
false.

## Order — 253 uses

`behaviors/Order.cs`.

```
Order(range, children, targetState)
```

Every entity of kind `children` within `range` that is not already in `targetState` is switched to
it. The state is looked up in *that child's own program*, not in the host's, and resolved once on
first tick and cached on the behaviour.

No cooldown, no probability: it runs every tick the host is in the state.

## Protect — 121 uses

`behaviors/Protect.cs`.

```
Protect(speed, protectee, acquireRange = 10, protectionRange = 2, reprotectRange = 1)
```

The same three-state shape as `Follow`, and the two range names read backwards from what you would
guess: it closes until it is within **`reprotectRange`** (1 by default), and it sets off again only
once the protectee is further than **`protectionRange`** (2). So the guard sits within one tile and
tolerates two before moving.

Reports `InProgress` while closing and `Completed` once in place, which is what lets a `Prioritize`
above it fall through to shooting while the guard is settled.

## Reproduce — 90 uses

`behaviors/Reproduce.cs`.

```
Reproduce(children = null, densityRadius = 10, densityMax = 5, coolDown = 60000ms,
          region = None, regionRange = 10)
```

Default cooldown is a **minute**. With no `children` it makes copies of itself.

Unlike `Spawn`, the density check is a **live count** of that kind within `densityRadius`, so this
one is a true population cap and killing the offspring does let it breed again.

Offspring always `GivesNoXp = true`. Region targeting works as in `TossObject`, and an impassable
target re-arms the cooldown and spawns nothing.

## Grenade — 66 uses

`behaviors/Grenade.cs`.

```
Grenade(radius, damage, range = 5, fixedAngle = null, coolDown = 1000ms,
        effect = 0, effectDuration = 0, color = 0xffff0000)
```

- **Stunned returns without firing and without spending the cooldown**, unlike `Shoot`, which spends
  it. The two behaviours genuinely differ here.
- Target is `AttackTarget`, else the nearest **player** within `range`.
- With a `fixedAngle` the landing spot is `range` tiles along that angle; otherwise it is the
  target's position at the moment of throwing.
- The throw effect is broadcast immediately, in red by default.
- **1500ms later** the blast lands: everything within `radius` takes `damage`, and takes `effect`
  for `effectDuration` unless it is `Invincible` or in `Stasis`.
- If there is no target and no fixed angle, nothing is thrown but **the cooldown is still spent**.

## HealGroup — 23 uses, and the worst mismatch found so far

`behaviors/HealGroup.cs`.

```
HealGroup(range, group, coolDown = 1000ms, healAmount = null)
```

**With no `healAmount`, it heals to full.** `newHp = entity.ObjectDesc.MaxHP`, and only when an
amount is given is it `min(HP + amount, MaxHP)`. Every enemy of the group within `range` is healed,
and each heal shows a potion effect on the target, a trail from the healer, and a green `+n`.

Our compiler gets three things wrong here at once:

```rust
amount:      number(call, "amount", 2, 100.0)      // C# index 2 is coolDown; the name is heal_amount
cooldown_ms: number(call, "cooldown", 3, 1000.0)   // C# index 3 is healAmount
```

1. The argument is called **`heal_amount`** in the converted content and our compiler looks for
   `amount`. So every explicit amount is missed.
2. The positional fallbacks are **swapped**: index 2 is the cooldown in the C#, index 3 the amount.
   `heal_group(10, "OrcKings", 300)` means *heal to full every 300ms*; we read it as *heal 300 every
   1000ms*.
3. The missing-amount default is `100`, where the original's is **heal to full**.

Eleven of the sixteen distinct uses give no amount at all, so eleven bosses that should restore their
guards completely restore a hundred points instead. `heal_group(15, "Heros", cooldown: 200)` is the
Lich fully healing its heroes five times a second.

**Bug in the original, to fix:** the stun check writes `coolDown = 999999999` to the behaviour's own
field, shared by every entity running that program. One stunned healer disables that program's group
healing permanently, for everyone.

## Charge — used in the long tail

`behaviors/Charge.cs`.

```
Charge(speed = 4, range = 10, coolDown = 2000ms)
```

Picks the nearest player within `range`, locks the direction, and runs in a straight line for
exactly as long as it would take to cover the original distance: `RemainingTime = d / speed * 1000`.
It does not track. When the time is up it clears the direction, arms the cooldown and reports
`Completed`; the next charge begins after it.

A target standing on exactly the same X *and* Y is skipped (`player.X != host.X && player.Y != host.Y`).

## StayAbove — 113 uses

`behaviors/StayAbove.cs`.

```
StayAbove(speed, altitude)
```

If the tile under the host has a **nonzero** elevation below `altitude`, walk toward the centre of
the map: `(map.Width / 2, map.Height / 2)`. Elevation zero means the map has no elevation there and
is exempt. Reports `InProgress` while climbing, `Completed` when high enough.

## StayCloseToSpawn — 113 uses

`behaviors/StayCloseToSpawn.cs`.

```
StayCloseToSpawn(speed, range = 5)
```

"Spawn" here is the position **when the state was entered**, not where the entity was created. The
file says so: *assume spawn = state entry position*. An enemy that changes state re-anchors where it
happens to be standing.

This server anchors to the entity's spawn point instead, which for a boss that walks across a room
before entering its fighting state is a different leash entirely.

## The healing family — HealSelf, HealGroup, HealEntity, HealPlayer

Four behaviours, one shared rule and three different parameter orders. This is where our compiler
comes apart.

**The shared rule: no amount means heal to full.**
```
newHp = target.ObjectDesc.MaxHP
if amount given: newHp = min(HP + amount, MaxHP)
```
All three enemy healers do this. A heal shows a potion effect on the target, a white trail from the
healer, and a green `+n`, and does nothing at all when the target is already full.

**The orders, from the four constructors:**

| Behaviour | 0 | 1 | 2 | 3 |
| --- | --- | --- | --- | --- |
| `HealSelf` | coolDown | amount | | |
| `HealGroup` | range | group | **coolDown** | **healAmount** |
| `HealEntity` | range | name | **healAmount** | **coolDown** |

`HealGroup` and `HealEntity` are **the same shape with positions 2 and 3 swapped**. Our compiler
uses the `HealEntity` order for both, so `HealEntity` is right and `HealGroup` reads the cooldown as
an amount and the amount as a cooldown.

`HealSelf` is swapped too: ours reads `amount` at index 0 and `cooldown` at index 1, where the C#
has cooldown first.

In practice the converter names most arguments, which hides all of this — until it does not.
`heal_group(10, "OrcKings", 300)` and `heal_entity(20, "md Janus the Doorwarden", 2000, ...)` are
both positional, and one of the two is read backwards.

**And the default is wrong everywhere.** Ours is `100`; the original's is *heal to full*. Every
`heal_self()` with no amount — and the content has several — should restore the enemy completely.

## SetAltTexture — 255 uses

`behaviors/SetAltTexture.cs`.

```
SetAltTexture(minValue, maxValue = -1, cooldown = 0, loop = false)
```

With `maxValue == -1` it sets the texture once on state entry and never ticks again. Otherwise it
steps `min → min+1 → … → max` on each cooldown, and with `loop` it wraps back to `min`; without
`loop` it stops at `max` and stays there.

State entry sets the texture to `min` immediately if it is not already there.

## Flash — 162 uses

`behaviors/Flash.cs`.

```
Flash(color, flashPeriod, flashRepeats)
```

Entirely a state-entry broadcast: one `ShowEffect` of type `Flashing` carrying the period and repeat
count in the two position fields. Nothing ticks. Purely visual, but it is 162 uses of a packet, so a
client expecting it will notice its absence.

## Suicide — 138 uses

`behaviors/Suicide.cs`. Kills the host on the first tick, as a *death*, so everything hanging off
dying runs: loot, death effects, experience. `Decay` is the version that does not count as a death.

## Decay — long tail

`behaviors/Decay.cs`.

```
Decay(time = 10000)
```

After `time` the host **leaves the world** rather than dying: no loot, no death effects, no
experience. This is the difference between a summoned minion expiring and one being killed.

## ReturnToSpawn

`behaviors/ReturnToSpawn.cs`.

```
ReturnToSpawn(speed, returnWithinRadius = 1)
```

Walks back to the enemy's real `SpawnPoint` — unlike `StayCloseToSpawn`, which anchors to where the
state was entered. `InProgress` while returning, `Completed` once within the radius.

**Bug in the original, to fix:** paralysis assigns to a field named `speed` that nothing reads; the
behaviour uses `_speed`. Paralysed enemies return home at full speed.

## TransformOnDeath — 68 uses

`behaviors/TransformOnDeath.cs`.

```
TransformOnDeath(target, min = 1, max = 1, probability = 1)
```

Registered on the **parent state's** death event, and fires only if the entity died while in that
state or one of its descendants. Rolls `probability`, then spawns `Random.Next(min, max + 1)` copies
of `target` at the corpse.

**If `target` resolves to a `Portal`, it does nothing at all** — an explicit early return in the
file. Transforming into a portal is disabled, and `DropPortalOnDeath` is the behaviour that does it.

## Swirl

`behaviors/Swirl.cs`.

```
Swirl(speed = 1, radius = 8, acquireRange = 10, targeted = true)
```

Untargeted, it circles the point where the state was entered. Targeted, it finds a player and solves
for the circle of the given radius passing through both host and player:

```
l = distance(host, player)
h = midpoint(host, player)
c = sqrt(|radius² - l²| / 4)
centre = h + c * perpendicular(host - player) / l
```

It then orbits that centre for one full period, `1000 * radius / speed * 2π` milliseconds. While
unacquired it moves at **a fifth** of its speed. It gives up and re-acquires early if it has been
going more than 200ms and a player is within 2 tiles.

## ChangeSize — a factor of 6.67 out

`behaviors/ChangeSize.cs`.

```
ChangeSize(rate, target)
```

The rate is applied **once every 150ms**, as a whole step:

```
every 150ms:  size += rate, clamped so it never overshoots target
```

So `rate` is size units per 150ms, which is `rate * 6.67` per second.

This server applies `rate * elapsed / 1000` — treating the rate as units per second. Every resize in
the game therefore takes **6.67 times too long**. `ChangeSize(-15, 25)`, which the content uses for a
boss that shrinks between phases, should take about half a second and takes three and a half.

## MoveTo

`behaviors/MoveTo.cs`. Walks to a fixed map coordinate, reporting `Completed` on the tick it arrives
and snapping exactly onto the point rather than overshooting.

## MoveLine

`behaviors/MoveLine.cs`.

```
MoveLine(speed, direction = 0)     // direction in degrees
```

Walks forever along a fixed compass direction and **never reports `Completed`** — a comment in the
file asks whether that is right. The consequence is structural: a `Prioritize` holding a `MoveLine`
latches onto it permanently and never reaches the children below, and a `Sequence` holding one
freezes at that step. Anything built on `MoveLine` is built on that.

## BackAndForth

`behaviors/BackAndForth.cs`.

```
BackAndForth(speed, distance = 5)
```

Paces along the **X axis only**, `distance` tiles out and `distance` back, reporting `Completed` at
each end of the run.

## StayBack — 113 uses

`behaviors/StayBack.cs`.

```
StayBack(speed, distance = 8, entity = null)
```

Moves directly away from the nearest player within `distance` — or from the nearest entity of a
**named kind**, which this server does not support at all.

It reports `Completed` once per second and `InProgress` in between, on an internal 1000ms cycle that
runs regardless of whether it is actually retreating. That cadence is what lets a `Prioritize` above
it drop through to the next child once a second.

## The wrapper behaviours

Four of these wrap other behaviours, and each has a rule worth stating exactly.

### Timed

`behaviors/Timed.cs`.

```
Timed(period, behaviors...)
```

Every child runs **every tick**, unconditionally. The period does not gate them; it only decides
when the wrapper reports `Completed`.

**Bug in the original, to fix:** the countdown is inside the loop over children, so the period drains
once per child. `Timed(1000, a, b, c)` completes every 333ms, not every 1000ms. When it does
complete, any child that is a `Prioritize` has its selection reset to `-1`, which is how a pattern
restarts from its first branch.

This server's `Every { period_ms, children }` gates the children on the period, which is the natural
reading and not what the file does.

### Duration

`behaviors/Duration.cs`.

```
Duration(child, duration)
```

Ticks the child for the first `duration` milliseconds after state entry, then stops ticking it
forever. There is no reset short of re-entering the state.

### WhileEntityWithin / WhileEntityNotWithin

`behaviors/WhileEntityWithin.cs`.

```
WhileEntityWithin(child, entityName, range)
```

Ticks the child only while an entity of that **name** is within range. The child is entered once on
state entry regardless.

### WhileWatched

`behaviors/WhileWatched.cs`. Ticks the child only while some player within the sight radius actually
has the host in its own client-side entity set — that is, only while somebody is really looking at
it. Entry and exit are gated the same way, so a host nobody can see never enters its child at all.

This has no equivalent here and cannot have one as written: it reads the *client's* view set. The
nearest honest equivalent is "a player is within sight radius and has line of sight".

## Transform

`behaviors/Transform.cs`. Replaces the host with another entity at the same spot: the new one enters
the world and the host leaves it, so nothing dies and nothing drops. **Refuses to transform into a
`Portal`**, exactly as `TransformOnDeath` does.

## ScaleHP

`behaviors/ScaleHP.cs`.

```
ScaleHP(amountPerPlayer, maxAdditional = 0, healAfterMax = false, dist = 0, scaleAfter = 0)
```

Once per second, count players by **name**, each counted once and never uncounted, optionally only
those within `dist`. For every player past `scaleAfter`, raise maximum health by `amountPerPlayer`.

The current health is scaled to keep the same **percentage**, not the same absolute value: a boss at
half health that gains 1,200 maximum keeps half. `maxAdditional` caps the total gain, and once
capped, `healAfterMax` decides whether further arrivals still heal it.

Counting by name and never uncounting means a boss stays scaled after everyone leaves.

## SpawnGroup

`behaviors/SpawnGroup.cs`. As `Spawn`, but picking a random member of a group per spawn and
scattering them by `Random.NextDouble() * radius` on each axis — a positive-only offset, so children
land down and to the right of the host rather than around it.

It does **not** have `Spawn`'s double-counting bug on entry. It does count an impassable spot against
the budget on tick, so a spawner in a corner exhausts itself without spawning anything.

## HealPlayer

`behaviors/HealPlayer.cs`.

```
HealPlayer(range, coolDown = 1000ms, healAmount = 100)
```

Unlike the enemy healers, this one **defaults to 100** rather than to full, and it heals players
within range, skipping anybody who is `Sick`, and skipping everybody except the host's `AttackTarget`
when it has one.

## A note on `Ported.cs`

This file is not part of the original server. It was written by this project to make the imported
dungeon scripts compile, and it holds nine classes: `ConditionEffectBehavior`, `EnemyAOE`,
`JumpToRandomOffset`, `ScaleHP2`, `TossObject2`, `EntityCountGreaterThan`, `If`, `OpenGate` and the
`ICondition` interface they use.

Every one of them is instantiated either nowhere or only inside commented-out script blocks, so none
of it reaches the game. It should be read as scaffolding, not as mechanics to match.

## The ground behaviours

### GroundTransform

`behaviors/GroundTransform.cs`.

```
GroundTransform(tileId, radius = 0, relativeX = null, relativeY = null, persist = false)
```

Two things this server gets wrong:

1. **The area is a square, not a circle.** `for i in hx-radius..=hx+radius, j in hy-radius..=hy+radius`.
   Our `reshape_ground` tests `dx² + dy² > radius²` and lays a disc. A radius-5 transform should cover
   121 tiles and covers 81 here.
2. **The original puts the ground back on state exit** unless `persist`. It remembers every tile it
   changed, with that tile's old type and spawned flag, and restores them all when the state ends.
   Ours changes the ground permanently, so a boss phase that lays lava leaves it there forever.

With `relativeX`/`relativeY` it changes a single tile at that offset instead of an area — and in that
branch it returns before saving the tile list, so that variant never restores even without `persist`.

### RemoveTileObject

`behaviors/RemoveTileObject.cs`. Blanks every tile object of a named type within `range`, measured as
a **square** (`|dx| > range || |dy| > range` skips). If the object blocked sight, the visibility index
is rebuilt and every player within sight radius has their sight counter bumped.

## The death-effect family

All of these hang off the parent state's `Death` event through `Resolve(State parent)`, so they fire
only when the entity dies **while in that state or one of its descendants**.

### DropPortalOnDeath

```
DropPortalOnDeath(target, probability = 1, timeout = null, XAdjustment = 0, YAdjustment = 0)
```

Skipped entirely when the host was `Spawned`. The timeout is subtle: `null` means take the portal's
own timeout from the content, `0` means never close, and any other value overrides the content's, in
**seconds**. A portal with a timeout gets a world timer that removes it.

### TransferDamageOnDeath

```
TransferDamageOnDeath(target, radius = 50)
```

Hands the whole damage ledger to the nearest entity of `target` within radius, so whoever earned the
loot on the dying phase still earns it on the next one. Without this, a boss with phases pays out
only for the last phase.

### AnnounceOnDeath

```
AnnounceOnDeath(msg)
```

Server-wide announcement, skipped when the host was `Spawned` or the world is the test world.
`{COUNT}` becomes the number of non-administrator players in the world and `{PL_LIST}` their names,
comma-separated.

### RealmPortalDrop

No arguments. On death, drops a **Realm Portal** — at the position of the nearest object of type
`0x5e4b` within 100 tiles if there is one, otherwise at the corpse. On state *entry* it also places a
"Realm Portal Opener" if no `0x5e4b` is already nearby. Skipped when the host was `Spawned`.

## The rest, briefly

- **`ConditionEffectRegion(effects, range = 2, duration = -1)`** — every tick, applies all the named
  effects to players within a **square** of `range`, using a strict `<`. Does nothing while the host
  is `Paused`.
- **`RelativeSpawn(children, x, y, maxChildren = 5, initialSpawn = 0.5, coolDown)`** — as `Spawn`,
  but always at a fixed offset from the host, centred on the tile (`+0.5`). Carries the same
  double-counting bug on entry.
- **`ReproduceChildren(maxChildren, initialSpawn, coolDown, children...)`** — keeps a list of the
  actual child entities and prunes those with `HP < 0`, so it is a live population cap. It never
  notices a child that left the world without dying, which stays counted forever.
- **`SpawnGroup`** — see above.
- **`MultiplyLootValue(multiplier)`** — multiplies the host's loot value once, on the first tick
  after state entry, and remembers it has.
- **`RemoveEntity(dist, children)`** — kills every named entity nearby as a real death, but marks each
  `Spawned` first, which suppresses their portal drops. Loops until a pass kills nothing.
- **`TeleporttoTarget(range)`** — jumps onto its `AttackTarget` when further than `range`. Only ever
  uses `AttackTarget`, never searches.
- **`OrderOnce(range, children, targetState)`** — as `Order`, but on state entry only.
- **`InvisiToss(child, range, angle, coolDown, coolDownOffset)`** — a toss with **no throw effect and
  no flight time**: the child appears immediately at the fixed angle. Note it dereferences
  `angle.Value` unconditionally, so a use without an angle throws.
- **`SetNoXP()`** — sets `GivesNoXp` every tick while in the state.
- **`HealPlayerMP`** — the magic counterpart of `HealPlayer`.

### ChangeGroundOnDeath

```
ChangeGroundOnDeath(groundToChange, changeTo, dist)
```

Note the geometry, which is unlike every other area behaviour: the square runs from
`host - dist/2` and is **`dist` tiles on a side**, so `dist` is a diameter here and a radius
everywhere else. A null `groundToChange` means every tile; otherwise only tiles of the listed types
are replaced. The replacement is drawn at random from `changeTo` **per tile per candidate type**.

It does not bound the loop to the map, so a death near an edge indexes outside it.

### CopyDamageOnDeath

```
CopyDamageOnDeath(child, dist = 50)
```

*Copies* the damage ledger onto the nearest `child`, where `TransferDamageOnDeath` *moves* it. Two
behaviours, nearly the same name, and the difference decides whether a phase's damage counts twice.

### RemoveObjectOnDeath

Identical to `RemoveTileObject` in every respect, including the square range and the sight rebuild,
but on death rather than on state entry.

### OrderOnDeath

```
OrderOnDeath(range, target, state, probability = 1)
```

`Order`, once, on death, behind a probability roll, and only if the host died in the state that owns
it.

## ReplaceTile

`behaviors/ReplaceTile.cs`.

```
ReplaceTile(objName, replacedObjName, range)
```

Despite the name, both arguments are **tile types**, not objects. Square range again. A replaced tile
that has no object id is given one from the world's id counter, which is how a tile that never
mattered becomes addressable.

## ApplySetpiece

`behaviors/ApplySetpiece.cs`. Instantiates `wServer.realm.setpieces.<name>` by reflection and renders
it with the host's tile as the **top-left corner**, not the centre.

Four of the names used in the content resolve to no class, so the original throws where it stands.
Ours records them and carries on; see the note in `PLAN.md` §18.

---

## The last ten, now read

### BringEnemy(name, range)

On state entry, **teleports** every enemy of that name within range onto the host's own tile. Not a
pull over time — an instant move. Used to gather a boss's guards.

### ChangeMusic(file) and ChangeMusicOnDeath(file)

Sets the world's track and tells every player, **staggered by 100ms each** so a full room does not
receive one packet per player in the same tick. Skipped entirely when the world is already playing
that track. `ChangeMusicOnDeath` is the same body on the parent state's death event.

### DestroyOnDeath(target)

On death, removes every entity of that name within **250 tiles** — effectively the whole map. Removal
is `LeaveWorld`, not a death, so the targets drop nothing and give nothing.

### HealPlayerMP(range, coolDown = 1000ms, healAmount = 100)

The magic counterpart of `HealPlayer`, and it confirms the asymmetry: **the player healers default to
100 where the enemy healers heal to full**. Skips anybody who is `Quiet`, and skips everybody except
the host's `AttackTarget` when it has one.

### KillPlayer(killMessage, coolDown = 1000ms, rekt = true, killAll = false)

Instant death to the host's `AttackTarget`, or to **every player in the world** with `killAll` —
excluding only those who are `Hidden`.

**`rekt` defaults to `true`.** That routes the kill through `Rekted` in `Player.Death`, which means a
gravestone, a trip to the nexus, and **no death recorded**: the character survives. So the game's
instakill mechanic does not, by default, end characters. That is a substantial thing to know before
implementing it, and it pairs with the escape hatches on [the player page](07-player.md).

A kill message is spoken to players within **15** tiles, as a taunt.

### MoveTo2(X, Y, speed = 2, once = false, isMapPosition = false, instant = false)

`MoveTo` with three additions: coordinates are **relative to the host** unless `isMapPosition`,
`instant` teleports on state entry instead of walking, and `once` is meant to stop it repeating.

**Bug in the original, to fix:** the `once` latch reads `if (host.X == X && host.Y == Y && once)` and
then sets `once = true` — it tests the flag it is about to set, and tests exact float equality against
a target it approaches by normalised steps. The latch effectively never fires. It also uses
`host.Move` rather than `ValidateAndMove`, so **it walks through walls**.

### MutePlayer(durationMin = 0)

Mutes the host's `AttackTarget` by **invoking the `/mute` command** with the player's name and a
duration in minutes. A behaviour reaching into the command system is worth noting: the mute is the
real, durable one, not an in-world effect.

### RemoveConditionEffect(effect)

The class inside is named `RemoveConditionalEffect`. Applies the effect with `DurationMS = 0` on state
entry, which is how the original removes one — the same idiom `ConditionalEffect` uses on exit.

### ReproduceGroup(group, densityRadius = 10, densityMax = 5, coolDown = 60000ms, region, regionRange)

`Reproduce` over a group: the density count is of the **whole group**, and each birth picks a random
member. Same minute-long default cooldown, same region targeting, same `GivesNoXp = true`.

**Bug in the original, to fix:** the region filter compares `Math.Abs(sx - host.X)` where every other
copy of this code compares `Math.Abs(sx - p.X)` — it measures the host against itself, so the filter
is always true and the spawn can land on **any** tile of the region anywhere on the map.
