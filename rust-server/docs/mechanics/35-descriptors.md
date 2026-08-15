# The descriptors, and every default in them

Read from `common/resources/XmlDescriptors.cs`.

This is where the content's XML becomes numbers. Almost every field has a default, and the defaults
are the specification for anything the content does not say.

## There is a third stat numbering

[The stats page](08-stats.md) covers two: the XML's `stat="N"` and the wire's `StatsType`. There is a
third, and it is the one a character's stat array is in.

```csharp
0 MaxHitPoints  1 MaxMagicPoints  2 Attack   3 Defense
4 Speed         5 Dexterity       6 HpRegen  7 MpRegen
```

**`HpRegen` is Vitality and `MpRegen` is Wisdom.** The class XML calls them by the old names, the
array index is 6 and 7, and everything downstream — `Stats.Base[6]`, `/max`, `/lefttomax`, level-up
increases — is in this order. So the same eight stats are numbered three different ways in three
different places, and only this one is positional.

`Stat` reads `<MaxHitPoints max="...">value</MaxHitPoints>` for the starting and maximum values, and
finds its per-level increase by scanning `<LevelIncrease>` elements for one whose **text** matches the
stat name.

`XmlStat.ToStatsType` is the translation between the first two, documented in place:

```
20 -> 24 Attack     21 -> 25 Defense    22 -> 26 Speed
26 -> 27 Vitality   27 -> 28 Wisdom     28 -> 29 Dexterity
everything else passes through
```

It is applied in exactly two places: `ActivateEffect.Stats` and `Item.StatsBoost`. Anywhere else that
reads a `stat` attribute is unconverted. Both of those reads are original and were raw until this
project wrapped them: `ToStatsType` is its own addition, and in the shipped 2020 server `stat="26"`
went through as `StatsType 26`, which is Speed. See [page 08](08-stats.md).

## Defaults

### Projectiles

```
LifetimeMS   0        Speed        required
Size         0        Damage       <Damage> sets both min and max, else both required
Amplitude    0        Frequency    1        Magnitude   3
```

Seven boolean flags are presence-only: `MultiHit`, `PassesCover`, `ArmorPiercing`, `ParticleTrail`,
`Wavy`, `Parametric`, `Boomerang`.

`Frequency = 1` and `Magnitude = 3` are the ones to get right: a wavy or parametric bullet with no
explicit values still curves, and it curves by those numbers.

### Items

```
RateOfFire      1        ArcGap  ArcGap1  ArcGap2   11.25
NumProjectiles  1        NumProjectiles1  NumProjectiles2   1
Tier            -1       BagType  MpCost  MpEndCost  FameBonus  Doses  FeedPower   0
Cooldown        0        Texture1  Texture2   0 (parsed as hex)
```

`Tier` is `-1` both when absent **and when it fails to parse**, so `Tier="UT"` is untiered rather than
an error.

`DisplayName` falls back to the object id when `DisplayId` is absent **or starts with `{`** — the brace
form is a client localisation key and is never shown server-side.

### Objects

```
Size present     -> MinSize = MaxSize = it, SizeStep = 0
Size absent      -> MinSize 100, MaxSize 100, SizeStep 0
DungeonName      -> falls back to DisplayId
MaxHP, Defense   -> 0
Level, PerRealmMax, ExpMultiplier -> null, which is distinct from 0
```

`ExpMultiplier` carries its formula in a comment: **experience gained = level total / 10 * multiplier**.

The eight immunities (`ArmorBreakImmune`, `CurseImmune`, `DazedImmune`, `ParalyzeImmune`,
`PetrifyImmune`, `SlowedImmune`, `StasisImmune`, `StunImmune`) are presence-only elements on the
descriptor, separate from the condition-effect flags of the same names.

### Portals and skins

```
PortalDesc.Timeout   30 seconds
SkinDesc.Cost        1000
SkinDesc.Size        100
```

`SkinDesc.FromElem` returns **null** when there is no `PlayerClassType`, and the loader skips nulls —
so a skin element without a class is silently not a skin.

## Condition effects: 51 of them, and a typo

`ConditionEffects` is a `[Flags] ulong` and `ConditionEffectIndex` is the same list as ordinals 0-50.
They are kept in step by hand.

They do not quite agree: the flags enum spells bit 11 **`StunImmume`** and the index enum spells it
`StunImmune`. Both are in use. Anything parsing an effect by name from XML goes through
`ConditionEffectIndex`, so content says `StunImmune`; anything testing a mask uses the misspelling.

`ConditionEffect(XElement)` parses the name with **spaces stripped**, so `"Armor Broken"` in content
resolves to `ArmorBroken`. Duration is in **seconds in the XML** and milliseconds in the object.

## Four fields that are never assigned

- **`ActivateEffect.DurationMS2`** — `duration2` is parsed into `DurationMS`, overwriting the value
  `duration` just set. Anything wanting two durations gets the second one twice.
- **`ActivateEffect.ObjectId2`** — `objectId2` is likewise parsed into `ObjectId`. Same shape, same
  line-copy origin.
- **`TileDesc.PushY`** — the guard reads `elem.Attribute("dy")` (the `Ground` element) while the value
  reads `anim.Attribute("dy")` (the `Animate` child). Ground elements do not carry `dy`, so **every
  pushing tile in the game pushes only horizontally.** `PushX` is guarded correctly and works.
- `Item.DisplayId` when it is a localisation key: kept, but `DisplayName` ignores it.

The first two are copy-paste; the third changes how conveyor tiles behave and is worth fixing rather
than reproducing.

## Equipment sets

`EquipmentSetDesc` holds `ActivateOnEquipAll` effects and a list of `Setpiece(type, slot, itemtype)`
entries — the slot and item type that count towards the set. The skin type granted by a set is found
by scanning the set's activate effects for the first with a non-zero `skinType`, and that is what
`SkinTypeToEquipSetType` is keyed on.

`Tag` deduplicates by removing then re-adding, so a repeated child keeps the **last** value.

## What this server does differently

Our `crates/content` loader must match every default above, because content that omits a field is
relying on it. In particular:

- **`ArcGap = 11.25`** and **`NumProjectiles = 1`** decide the shape of every volley in the game.
- **`Frequency = 1`, `Magnitude = 3`, `Amplitude = 0`** decide the shape of every curving bullet.
- **`MinSize = MaxSize = 100`** and `Size` collapsing the range.
- **`Timeout = 30`** on portals.
- **`Tier = -1` on a parse failure**, not an error.
- **Stat array order is HP, MP, Att, Def, Spd, Dex, Vit, Wis**, with Vitality and Wisdom named
  `HpRegen` and `MpRegen` in the XML. That is a third numbering and it is the positional one.
- Condition-effect names are parsed with spaces removed.
- **Fix `PushY`.** Reproducing it would mean pushing tiles that only work east-west, and no content
  author intended that.
