# Levelling and fame goals

Read from `realm/entities/player/Player.Leveling.cs`.

## The curves

```
GetExpGoal(level)  = 50 + (level - 1) * 100          // experience to cross this level
GetLevelExp(level) = 0 for level 1
                   = 50 * (level - 1) + (level - 2) * (level - 1) * 50   // total to have reached it
GetFameGoal(fame)  = 20, 150, 400, 800, 2000, then 0
```

`Experience` is a **lifetime total**, and progress within a level is `Experience - GetLevelExp(Level)`.

## Levelling up

```
if Experience - GetLevelExp(Level) >= ExperienceGoal and Level < 20:
    Level++
    ExperienceGoal = GetExpGoal(Level)
    for each of the class's stats:
        Base[i] += rand(MinIncrease, MaxIncrease + 1)      // inclusive of the maximum
        clamp to MaxValue
    HP = Stats[0]            // filled to the new maximum
    MP = Stats[1]
    if Level == 20: tell everybody in the world
    else: re-send the experience, rebased on the new goal
    questEntity = null       // the quest arrow is dropped and re-chosen
    return true
otherwise:
    CalculateFame()
    return false
```

Four details worth matching exactly:

- **One level per kill.** The check is not a loop, so an enormous experience gain grants one level and
  the rest sits banked as progress toward the next. This is the counterpart to the 10%-of-a-level cap
  in `DamageCounter` — with the cap, one kill can never overflow more than one level anyway.
- **`rand(min, max + 1)` is inclusive of `MaxIncrease`.** An off-by-one here changes every character's
  growth.
- **Health and magic are refilled to the new maximum** on every level, not increased by the delta.
- **Fame is only recalculated when the player does *not* level up.** On a level-up `CalculateFame` is
  skipped entirely, so fame catches up on the next kill.

## Fame

```
newFame = Experience < 200,000 ? Experience / 1000
                              : 200 + (Experience - 200,000) / 1000
```

Both branches are one fame per thousand experience; the second is the same line written twice, so the
"halving past two hundred thousand" that the shape suggests does not actually happen. Worth copying as
written rather than as intended.

The fame goal is taken against **the better of** this character's fame and the best any character of
this class has ever reached:

```
newGoal = GetFameGoal(max(classStats.BestFame, newFame))
```

Crossing into a higher goal announces a **class quest completion** in green and recomputes the star
count. Otherwise, gaining fame announces `+n` in orange.

## Stars

```
per class: BestFame >= 2000 -> 5, >= 800 -> 4, >= 400 -> 3, >= 150 -> 2, >= 20 -> 1
stars = the sum over every class
```

Fourteen classes, so seventy is the maximum. This matches what we implemented.

## EnemyKilled

```
if the enemy was this player's quest target: announce a quest completion nearby
if exp != 0: Experience += exp
FameCounter.Killed(enemy, killer)
return CheckLevelUp()
```

Note the order: **the fame counters are updated before the level check**, and `Killed` is called even
when the experience was zero — which is how a `GivesNoXp` enemy still counts toward kills, assists and
quest completions.

## What this server does differently

- We should confirm **one level per kill**, not a loop.
- We should confirm **`rand(min, max + 1)`** inclusivity, and the clamp to `MaxValue`.
- We should confirm **health and magic are refilled** rather than incremented on level-up.
- **`CalculateFame` is skipped on a level-up.** Ours may recompute both every time; the visible
  difference is one kill's delay in the fame number.
- The **class-quest announcement** on crossing a fame goal, and the `+n` notification otherwise, have
  no equivalent here.
