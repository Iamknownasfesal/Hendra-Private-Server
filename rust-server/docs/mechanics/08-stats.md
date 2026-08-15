# Stats, boosts, and the stat numbering

Read from `realm/StatsManager.cs`, `realm/BaseStatManager.cs`, `realm/BoostStatManager.cs`,
`realm/Stats.cs`, and `common/resources/XmlDescriptors.cs`.

## There are eleven stats, not eight

```
0  MaxHitPoints      4  Speed        8  DamageMin
1  MaxMagicPoints    5  Dexterity    9  DamageMax
2  Attack            6  Vitality     10 Luck
3  Defense           7  Wisdom
```

`StatsManager.NumStatTypes = 11`, and this server models all eleven. `DamageMin` and `DamageMax` are
the equipped weapon's own bounds, rewritten from slot zero on every recalculation by
`BaseStatManager.SetWeaponDamage`, and they are what a player's shot rolls between; `Luck` multiplies
private loot chances (see [the loot page](06-loot.md)).

A total is always `Base[i] + Boost[i]` — no cap above and no floor below.

## The content's stat numbers are not this enum, and must be translated

The XML writes `<ActivateOnEquip stat="21" amount="4">IncrementStat</ActivateOnEquip>`. That `21` is
**not** a `StatsType`. `XmlDescriptors.XmlStat.ToStatsType` remaps six of them:

| XML | → StatsType | Stat |
| --- | --- | --- |
| 20 | 24 | Attack |
| 21 | 25 | Defense |
| 22 | 26 | Speed |
| 26 | 27 | Vitality |
| 27 | 28 | Wisdom |
| 28 | 29 | Dexterity |
| anything else | unchanged | — |

So `0` stays `MaximumHP` and `3` stays `MaximumMP`. Those two pass through; the other six do not.

**`XmlStat.ToStatsType` is this project's own C#, not the shipped game's.** It wraps two call sites
(`XmlDescriptors.cs:368` and `:679`) that in the shipped 2020 tree read the attribute raw. Read raw,
`stat="26"` is `StatsType 26`, which is **Speed**, not Vitality — so every item stat in the six-entry
band above landed one place off. The reference binary is built from the current source, so the table
above is what the live server does and what to match.

Then `StatsManager.GetStatIndex(StatsType)` turns the `StatsType` into the 0–10 index.

**Two translations, in that order.** Skipping either one silently produces a different stat.

## What this server does

Both translations exist here, composed into one table. `Stat::from_content_number`
(`crates/content/src/player.rs:106`) takes the written number straight to a position in the stat
array, which is why that table is not in order — vitality and wisdom sit before dexterity in
`StatsType` and after it here. It is the single door: `desc.rs:263` puts an equipped item's bonus
through it, and `activate.rs:620` puts an activate's numeric `stat` through it.

A written stat *name* is matched separately, in `stat_index`, because a few files spell it out.
`stat="21"` and `stat="Defense"` both mean defence; `stat="3"` means max magic rather than the
defence its digit would suggest in the name alphabet.

The three stats outside the collision the table untangles — `83` `DamageMin`, `84` `DamageMax` and
`90` `Luck` — pass through both C# translations unchanged and are read as themselves.

Anything outside the table is `None`, and the content is refused at load rather than applied to the
wrong stat. The original indexes an array with the `-1` its lookup returns and throws; no shipped
content reaches that.

## Boosts are recomputed from nothing on every inventory change

```
ReCalculateValues:
  boost[] = 0
  ApplyEquipBonus     // the four worn slots only
  ApplySetBonus
  ApplyActivateBonus
```

- **Only one equipment set can be active.** `ApplySetBonus` `return`s after the first set it finds
  complete, so a player wearing two complete sets gets the first in file order.
- A set that has just been *removed* is detected from the previous inventory and its skin, size and
  condition effects are undone.
- `IncrementBoost` floors **each bonus separately, against the base alone**, rather than flooring the
  sum: a bonus that would take `Base[i] + amount` below one is shortened so it leaves the stat at
  **1** for MaxHitPoints or **0** for anything else. Two consequences, and both are observable. One
  item worth -110 health on a 100-health character leaves 1, not 0. And two items that each empty the
  same stat are each shortened against the untouched base, so together they take it off twice and the
  total lands **below zero** — `this[i]` has no floor under it at all.
- `FixedStat` sets the boost so that the total lands exactly on the given value.

## Stat boosts are visible as condition effects

`ApplyActivateBonus` maps boost index `i` (0–7) to condition effect index `i + 39`: a nonzero boost
applies that effect permanently, and a zero one removes it. That is how the client draws the little
icons for "attack boosted", and it means those eight condition effects are **outputs of the stat
system**, not inputs. Anything that enumerates condition effects has to know they are reserved.

## What this server does differently

- All eleven stats exist here, `DamageMin`, `DamageMax` and `Luck` included.
- `stat_index` returns `None` for a name it does not know rather than falling back to
  `MaxHitPoints`, so a stat the parser cannot honour is refused at load instead of quietly raising
  max health.
- `IncrementBoost`'s per-bonus floor is `Stats::apply_equipment`, and the worn bonuses reach it
  separately rather than summed, which is why `ToWorld::Equipment` carries a list.
- We should check the one-set-only rule and whether we emit the `i + 39` condition effects for active
  boosts.
