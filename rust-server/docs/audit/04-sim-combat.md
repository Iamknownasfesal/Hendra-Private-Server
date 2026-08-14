# `hendra-sim`, combat, stats and progression

Against [page 05](../mechanics/05-projectiles.md), [page 07](../mechanics/07-player.md),
[page 11](../mechanics/11-experience-and-fame.md), [page 18](../mechanics/18-levelling.md),
[page 21](../mechanics/21-abilities.md) and [page 32](../mechanics/32-fame-bonuses.md).

The curves are exact. Experience goal `50 + (level-1)*100`, total-at-level
`50*(level-1) + (level-2)*(level-1)*50`, the five fame milestones at 20/150/400/800/2000, the star
thresholds, `max_hp / 10 * multiplier` per kill, the 10% and 50% level caps, the 25-tile share radius,
and all twenty compounding fame bonuses with Well Equipped and First Born applied last. Kills are
shared flat rather than by damage, which is the original's rule and an easy one to get wrong.

Three things below are not exact, and the first two are large.

## A player's defence does nothing

`Projectiles::resolve_hits` (`crates/sim/src/projectile.rs:413`) reads the target's defence from the
*content descriptor*:

```rust
let defence = catalog
    .object(entity.object_type)
    .map(|desc| desc.defense)
    .unwrap_or(0);
```

For an enemy that is right — an enemy's defence is a property of its type. For a player it is the
`<Defense>` element of the class object, which is the level-one base, and for most classes that is
literally `0`. Levelling, worn armour, defence rings and defence boosts all live in
`Entity::stats`, and `Stats::defence()` exists and is **called from nowhere in the crate.**

The original reads the character's stat, not the descriptor
(`Player.cs:814` → `StatsManager.GetDefenseDamage(dmg, false)` → `this[3]`).

So every point of defence a player has is ignored on every projectile hit. Combined with
[the potion defect](01-content.md), the situation for a player is: their defence bonuses are mostly
dropped at load, the survivors raise the wrong stat, and whatever survives that is never read when
they are hit.

`Rules::damage_after_defence` itself is correct, and the projectile path calls it correctly. Only the
number handed to it is wrong.

## Every minion is worth full experience

`World::award_experience` (`crates/sim/src/world.rs:4243`) passes `awards_experience` as a literal:

```rust
let earned = crate::leveling::experience_for_kill(
    max_hp, multiplier, true, level, was_quest,
);
```

The original suppresses experience in two separate ways, and this server has neither:

```csharp
if (enemy.Spawned) return;                    // DamageCounter.Death:74
float xp = enemy.GivesNoXp ? 0 : 1;           // :79
```

`Spawned` is set by `Spawn` on the children of a parent that was itself spawned, and the whole award
is abandoned — not reduced, abandoned — for anybody. `GivesNoXp` is the `givesNoXp` parameter of
`Spawn`, and its **default is `true`**:

```csharp
public Spawn(string children, int maxChildren = 5, double initialSpawn = 0.5,
             Cooldown coolDown = new Cooldown(), bool givesNoXp = true)
```

There are 364 `Spawn` uses in the content. Unless one explicitly passes `givesNoXp: false`, its
children are worth nothing in the original and are worth `max_hp / 10` each here.

That is a standing farm: any boss with a spawner, or any of the Shatters and Cemetery spawn loops,
becomes a place to level up by standing near it. [Page 02](02-behaviour.md) records that we drop the
`givesNoXp` argument at compile time as well, so even a script that says `false` explicitly is not
being read — but the default is the direction that matters, and the default is "no experience".

Two smaller suppressions are also absent:

| Original | Here |
| --- | --- |
| `playerXp *= .33f` in any world whose display name contains "Theatre" | not applied |
| `playerXp *= 2` while an XP boost is running and the character is under 20 | not applied |

## Wisdom does nothing for abilities

`UseWisMod` (`Player.UseItem.cs:1370`) scales an ability's amount and its range:

```csharp
double totalWisdom = Stats.Base[7] + Stats.Boost[7];
if (totalWisdom < 30) return value;
double n = (value * totalWisdom / 150) + (value * m);
```

Below 30 wisdom it does nothing; above it, a value grows by `wisdom / 150` of itself, so at 75 wisdom
a 100-point heal becomes 150 and at 150 wisdom it doubles. The same call scales the *range* of an
aura.

We parse the flag — `ActivateDesc::flag("useWisMod")` is tested in `desc.rs:666` — and read it
nowhere. `grep -rn "wis_mod" crates` finds nothing outside a test name.

61 activates in the content set `useWisMod="true"`. 23 of those are `GenericActivate`, which
[does nothing at all here](01-content.md), leaving **38 live abilities that ignore wisdom**: every
priest heal nova, every paladin stat aura, every self-buff on a timer. Wisdom currently affects
nothing but MP regeneration, which makes it the one stat with no reason to raise it beyond a
threshold nothing checks.

## A bag can be reached from twice as far

`InvSwapHandler.cs:168` refuses a swap when `Vector2.DistanceSquared(aPos, bPos) > 1` — a reach of
one tile. `BAG_REACH` (`crates/server/src/world_task.rs:444`) is `2.0` tiles.

Minor, and worth matching because loot bags are contested: two tiles is four times the area to grab
from, which changes who gets to a white bag first.

## A comment that will invite the wrong fix

`fame_from_experience` (`crates/sim/src/leveling.rs:56`) matches the original exactly, branch and all:

```rust
if experience < KNEE { experience / 1000 } else { 200 + (experience - KNEE) / 1000 }
```

Both arms compute the same thing — `200 + (e - 200_000)/1000` *is* `e / 1000` — so the knee does
nothing. That is faithful: `Player.Leveling.cs:234` has the identical redundant branch, and it reads
as though a halving was intended and never written.

The **comment above ours says the rate halves past two hundred thousand**, which it does not. Left as
a note here rather than edited, because it is the kind of comment that gets someone to "fix" the code
to match it and quietly change what a lifetime of play is worth. Whoever next touches that function
should correct the comment and not the arithmetic.
