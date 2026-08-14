# Experience, damage credit, and the fame counters

Read from `logic/DamageCounter.cs`, `logic/FameCounter.cs`, `realm/ActivateBoost.cs`.

## The damage ledger

```
HitBy(player, projectile, dmg):
    trueDmg = min(dmg, Host.MaximumHP)      // overkill never inflates the ledger
    TotalDamage += trueDmg
    hitters[player] += trueDmg
    LastHitter = player
    player.FameCounter.Hit(...)             // counts a shot that damaged something
```

Each hit is **clamped to the enemy's maximum health** before being added, so a single enormous hit
cannot dominate the damage shares that decide private loot.

`hitters` is a **weak dictionary** keyed by player, so a player who disconnects drops out of the
ledger rather than holding a reference.

`Corpse` and `Parent` chain two counters: an enemy with a corpse defers its death accounting to the
corpse, and `GetPlayerData` walks up to the parent. That is how a multi-phase boss keeps one ledger.

## Experience, on death

```
if enemy.Owner is Realm: Realm.EnemyKilled(enemy, lastHitter)     // BEFORE the spawned check
if enemy.Spawned: return                                          // no experience at all

for each player within 25 tiles:
    skip if Paused
    xp  = GivesNoXp ? 0 : 1
    xp *= MaxHP / 10 * (ExpMultiplier ?? 1)
    cap = ExperienceGoal * 0.1        (0.5 if this enemy is that player's quest target)
    playerXp = min(xp, cap)
    if the world's display name contains "Theatre": playerXp *= 0.33
    if XPBoostTime != 0 and Level < 20: playerXp *= 2
    killer = (lastHitter == player)
    if EnemyKilled(...) levelled them up and they were not the killer: lvlUps++

lastHitter.FameCounter.LevelUpAssist(lvlUps)
```

Points that decide the feel of the game:

- **Experience is not divided.** Every player within 25 tiles receives the full amount, capped
  individually. Eight players killing one enemy generate eight times the experience, not a share
  each.
- **The realm is told about the kill before the `Spawned` check** in `DamageCounter`, but
  `Realm.EnemyKilled` applies the same filter itself (`if (_overseer != null && !enemy.Spawned)`), so
  the outcome is the same: a summoned enemy drives no event. The check simply lives at the other end.
- **`GivesNoXp` zeroes the experience but still counts the kill** — `EnemyKilled` is called with zero,
  so fame counters, quest completion and level-up assists all still fire.
- The **"Theatre" multiplier** is a substring test on the world's display name, worth one third.
- The **experience boost doubles, and only below level 20**, matching the boost being cleared at 20.

## Fame counters, and what feeds each

| Counter | Fed by |
| --- | --- |
| `Shots` | every shot fired |
| `ShotsThatDamage` | every hit that damaged an enemy |
| `MonsterAssists` / `GodAssists` | every enemy killed nearby, split by the `God` flag |
| `MonsterKills` / `GodKills` | the same, but only for the last hitter |
| `CubeKills` / `OryxKills` | last hitter, on enemies flagged `Cube` or `Oryx` |
| `QuestsCompleted` | the dying enemy was that player's quest target |
| `LevelUpAssists` | levels *others* reached from your killing blow |
| `TilesUncovered` | tiles newly sent to that player |
| `Teleports` | each teleport |
| `SpecialAbilityUses` | each ability used |
| `PotionsDrunk` | each potion drunk |
| `MinutesActive` | one per sixty seconds of ticking |
| `*Completed` | ten named dungeons, matched **by world name** |

Two details:

- **Assists are counted for everyone nearby, kills only for the last hitter**, and both come from the
  same call.
- `CompleteDungeon` matches a **hard-coded list of ten world names**: `PirateCave`, `Undead Lair`,
  `Abyss`, `Snake Pit`, `Spider Den`, `Sprite World`, `Tomb`, `OceanTrench`, `Forbidden Jungle`,
  `Manor of the Immortals`. A world not on the list counts for nothing.
- `MinutesActive` ticks only while the player ticks, and the fame counter is inside the `Paused`
  guard, so paused time does not count.

## Boost stacking

`ActivateBoost.GetBoost` is the rule for temporary stat boosts:

```
sort the stack ascending
boost = sum over i of stack[last - i] * 0.5^i     // largest in full, next at a half, then a quarter
boost += base[0]                                  // the largest non-stacking boost
boost += offset
```

So **stacking the same buff is worth less each time**: two rings of eight attack give twelve, not
sixteen. Non-stacking boosts are kept in a separate list sorted descending and **only the largest one
counts**, which is what stops two copies of a no-stack buff from stacking.

`Pop` removes **one entry equal to that amount**, not a specific instance — so two identical boosts
are interchangeable and removing one always leaves the other.

## What this server does differently

- **`MonsterAssists` and `GodAssists` are not modelled.** No fame bonus reads them, so this is
  cosmetic, but they are two of the counters a character sheet shows.
- **The ten dungeon-completion counters** are matched by world name in the original; we hold a bitset
  of dungeon kinds. Worth confirming the ten names line up.
- The **"Theatre" 0.33 multiplier** has no equivalent here.
- We should confirm: the per-hit clamp to maximum health, that experience is not divided among
  players, and that the realm hears about a kill even when the enemy was spawned.
