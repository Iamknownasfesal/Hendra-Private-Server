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

`StatsManager.NumStatTypes = 11`. This server models the first eight. `DamageMin` and `DamageMax`
are flat additions to weapon damage; `Luck` multiplies private loot chances (see
[the loot page](06-loot.md)).

A total is always `Base[i] + Boost[i]`.

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

Then `StatsManager.GetStatIndex(StatsType)` turns the `StatsType` into the 0–10 index.

**Two translations, in that order.** Skipping either one silently produces a different stat.

## What this server does, and what it costs

Neither translation exists here. Two separate paths read the raw XML number:

```rust
// desc.rs — how an equipped item's bonuses are lifted out
stat: a.int("stat")? as u8            // the raw XML number, unmapped

// stats.rs — how they are applied
if let Some(slot) = out.get_mut(boost.stat as usize) { *slot += boost.amount; }
```

```rust
// activate.rs — the other path, used by sets and activations
"attack" | "2" => 2, "defense" | "3" => 3, ...
_ => 0,                               // anything unrecognised becomes MaxHitPoints
```

Counting every `stat="N"` in the shipped content:

| XML stat | Uses | Should raise | Equipment path does | Activate path does |
| --- | ---: | --- | --- | --- |
| 0 | 88 | MaxHitPoints | MaxHitPoints ✓ | MaxHitPoints ✓ |
| 3 | 83 | **MaxMagicPoints** | **Defense** | **Defense** |
| 20 | 87 | **Attack** | **nothing** | **MaxHitPoints** |
| 21 | 153 | **Defense** | **nothing** | **MaxHitPoints** |
| 22 | 84 | **Speed** | **nothing** | **MaxHitPoints** |
| 26 | 75 | **Vitality** | **nothing** | **MaxHitPoints** |
| 27 | 92 | **Wisdom** | **nothing** | **MaxHitPoints** |
| 28 | 96 | **Dexterity** | **nothing** | **MaxHitPoints** |

**670 of the 758 item stat bonuses in the game are wrong, and 587 of them do nothing at all.**
`out.get_mut(21)` on an `[i32; 8]` returns `None`, so the bonus is dropped without a word.

Every ring of attack, every armour's defence, every bonus to speed, vitality, wisdom or dexterity in
the entire content: silently discarded. The 83 uses of stat 3 raise defence instead of magic.

Nothing detected this because both failure modes are silent by construction — an out-of-range index
is skipped and an unrecognised name falls back to zero.

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
- `IncrementBoost` clamps: a negative boost may not take a stat below **1** for MaxHitPoints or below
  **0** for anything else.
- `FixedStat` sets the boost so that the total lands exactly on the given value.

## Stat boosts are visible as condition effects

`ApplyActivateBonus` maps boost index `i` (0–7) to condition effect index `i + 39`: a nonzero boost
applies that effect permanently, and a zero one removes it. That is how the client draws the little
icons for "attack boosted", and it means those eight condition effects are **outputs of the stat
system**, not inputs. Anything that enumerates condition effects has to know they are reserved.

## What this server does differently

- **The two stat translations are missing**, which is the defect above and by some distance the
  largest found in this audit.
- **Three stats are missing**: `DamageMin`, `DamageMax` and `Luck`.
- The **`_ => 0` fallback** in `stat_index` should be an error, not `MaxHitPoints`. A stat name the
  parser does not know is content the server cannot honour, and it should say so at load.
- We should check the one-set-only rule, the clamps in `IncrementBoost`, and whether we emit the
  `i + 39` condition effects for active boosts.
