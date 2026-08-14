# Fame bonuses on death

Read from `common/FameStats.cs`.

When a character dies, its accumulated base fame is multiplied up by a list of bonuses. This is the
whole of the end-of-life fame calculation, and it is one of the few parts of the game where the exact
order of operations is visible to players.

## The 25 tracked statistics

Serialised as `(byte id, int32 value)` pairs, ids 0 to 24, in this order:

```
0  Shots                   9  GodAssists              18 SpriteWorldsCompleted
1  ShotsThatDamage        10  CubeKills               19 LevelUpAssists
2  SpecialAbilityUses     11  OryxKills               20 MinutesActive
3  TilesUncovered         12  QuestsCompleted         21 TombsCompleted
4  Teleports              13  PirateCavesCompleted    22 TrenchesCompleted
5  PotionsDrunk           14  UndeadLairsCompleted    23 JunglesCompleted
6  MonsterKills           15  AbyssOfDemonsCompleted  24 ManorsCompleted
7  MonsterAssists         16  SnakePitsCompleted
8  GodKills               17  SpiderDensCompleted
```

`Read` is tolerant — an unknown id falls through the switch and the loop continues, which **desyncs
the stream** because it does not consume the int that followed. `Write` always writes all 25, so this
only bites on data written by a different version.

## The bonuses, in the order they are applied

Order matters because **each bonus is computed on the running total including the ones before it.**
The list is compounding, not additive against the base.

| Bonus | Condition | Award |
| --- | --- | --- |
| Ancestor | `CharId < 2` — the account's first or second character | `10% + 20` |
| Pacifist | no shot ever damaged an enemy | 25% |
| Thirsty | no potion drunk from the inventory | 25% |
| Mundane | level 20 and no ability use | 25% |
| Boots on the Ground | never teleported | 25% |
| Tunnel Rat | all ten dungeon types completed at least once | 10% |
| Enemy of the Gods | level 20, god kills > 10% of kills | 10% |
| Slayer of the Gods | level 20, god kills > 50% of kills | 10% |
| Oryx Slayer | killing blow on Oryx at least once | 10% |
| Accurate | level 20, accuracy > 25% | 10% |
| Sharpshooter | level 20, accuracy > 50% | 10% |
| Sniper | level 20, accuracy > 75% | 10% |
| Explorer | more than 1,000,000 tiles uncovered | 5% |
| Cartographer | more than 4,000,000 tiles uncovered | 5% |
| Team Player | more than 100 party level-ups | 10% |
| Leader of Men | more than 1,000 party level-ups | 10% |
| Doer of Deeds | more than 1,000 quests completed | 10% |
| Friend of the Cubes | level 20 and no cube killed | 10% |
| **Well Equipped** | sum of `FameBonus` on the four equipped items | that sum, as a percent |
| **First Born** | this character's total beats every previous character's `BestFame` | 10% |

The tiered bonuses **stack rather than replace**: a 75%-accuracy character earns Accurate,
Sharpshooter *and* Sniper, for 30% compounded, not 10%. Same for Explorer + Cartographer and Team
Player + Leader of Men.

Every award is `(int)` truncated, so a bonus worth 0.9 fame is worth nothing.

## Three ways of computing it, and they disagree

```csharp
CalculateTotal(...)   f += i.Item4(character.Fame + f);   // condition tested with baseFame
GetBonuses(...)       f += i.Item4(character.Fame + f);   // condition tested with Fame + f
```

`CalculateTotal` passes `character.Fame` as the condition's third argument while `GetBonuses` passes
`character.Fame + f`. **No condition in the list reads that argument**, so today the two agree — but
the display path and the awarding path are computing the same thing two different ways, and a bonus
that ever used the argument would show one number and pay another.

`GetBonuses` also does not add First Born to `f` before yielding it, which is correct only because it
is last.

## Well Equipped and First Born

**Well Equipped** sums `FameBonus` across inventory slots 0 to 3 — the four equipment slots — skipping
`0xffff` (empty). The sum is treated as a percentage, so two items at 5% each give 10%, applied to the
running total. Items in the backpack do not count.

**First Born** compares `character.Fame + f` against the maximum `BestFame` across **all classes** the
account has ever played, and an account with no recorded class stats (`bestFames.Length <= 0`) gets it
automatically. So a brand new account's first death always earns both Ancestor and First Born.

Note the comparison is against the *bonused* total but `BestFame` is what gets stored, so the bar
rises to include bonuses — First Born gets harder to earn over an account's life at exactly the rate
the player improves.

## What this server does differently

We compute fame at death in `leveling.rs` and do not have this bonus list. When it is added:

- **The compounding order is the specification**, not a detail. Applying all bonuses against the base
  fame and summing gives a visibly smaller number.
- **The tiers stack.** Three accuracy bonuses, not one.
- Truncate each award individually, in the listed order — rounding, or summing the percentages first,
  produces different totals.
- Well Equipped reads only the four equipment slots.
- First Born's bar is the best *bonused* fame of any previous character, and an account with no
  history always qualifies.
