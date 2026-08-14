# `hendra-content`, against the descriptor pages

13 files, read against [page 33](../mechanics/33-content-loading.md),
[page 35](../mechanics/35-descriptors.md), [page 08](../mechanics/08-stats.md) and
[page 20](../mechanics/20-maps.md).

Most of it matches. The condition-effect numbering is identical for all 51 entries, every projectile
default is right (`ArcGap 11.25`, `NumProjectiles 1`, `Frequency 1`, `Magnitude 3`, `Amplitude 0`),
the eight-stat positional order is right, and the region table was already confirmed three ways on
[page 25](../mechanics/25-regions-and-coverage.md).

What follows is where it does not match.

## Every stat potion but one raises the wrong stat

The content writes a stat bonus as a number:

```xml
<Activate stat="20" amount="1">IncrementStat</Activate>
```

That number is in **neither** of the numberings this server knows. The original translates it twice
before it means anything.

**Pass one**, `XmlStat.ToStatsType` in `common/resources/XmlDescriptors.cs:316` — content number to
`StatsType`, the wire numbering:

```csharp
case 20: return 24;  // Attack
case 21: return 25;  // Defense
case 22: return 26;  // Speed
case 26: return 27;  // Vitality
case 27: return 28;  // Wisdom
case 28: return 29;  // Dexterity
default: return xmlStat;
```

**Pass two**, `StatsManager.GetStatIndex(StatsType)` at `wServer/realm/StatsManager.cs:213` —
`StatsType` to the positional index of the eight-element character array. Note that it is not
order-preserving: Vitality and Wisdom come *before* Dexterity in `StatsType` and *after* it in the
array.

Composed, the whole table is eight rows:

| Content `stat=` | Means | Positional index |
| --- | --- | --- |
| 0 | Max HP | 0 |
| 3 | Max MP | 1 |
| 20 | Attack | 2 |
| 21 | Defense | 3 |
| 22 | Speed | 4 |
| 26 | Vitality | **6** |
| 27 | Wisdom | **7** |
| 28 | Dexterity | **5** |

This server translates neither pass, in two separate places, and the two fail differently.

**`crates/content/src/desc.rs:255`** — worn equipment. The raw number goes straight into
`StatBoost.stat`, and `equipment_boosts` (`crates/sim/src/stats.rs:261`) indexes an eight-element
array with it:

```rust
if let Some(slot) = out.get_mut(boost.stat as usize) {
```

Anything from 20 upward is out of bounds and `get_mut` returns `None`, so the bonus is discarded
without a word. `stat="3"` is in bounds and lands on Defense.

**`crates/content/src/activate.rs:455`** — potions and stat-boost abilities. `stat_index` matches the
written number against the *positional* numbering and falls through:

```rust
"mpregen" | "wisdom" | "7" => 7,
_ => 0,
```

So every content number it does not recognise — which is every one above 7 — becomes **stat 0, max
HP**. Not dropped: silently redirected.

Measured against the content this server loads (`godot-client/assets/xml`, 690 `IncrementStat` uses):

| Content `stat=` | Uses | Worn item does | Potion does |
| --- | --- | --- | --- |
| 0 Max HP | 71 | correct | correct |
| 3 Max MP | 75 | raises Defense | raises Defense |
| 20 Attack | 75 | discarded | raises Max HP |
| 21 Defense | 145 | discarded | raises Max HP |
| 22 Speed | 75 | discarded | raises Max HP |
| 26 Vitality | 73 | discarded | raises Max HP |
| 27 Wisdom | 91 | discarded | raises Max HP |
| 28 Dexterity | 85 | discarded | raises Max HP |

**619 of 690 are wrong.** Of the 24 potions and elixirs in the content, 21 raise max HP, one raises
defence, and only the Life line is right:

```
Potion of Attack     stat=20  should raise Attack     raises Max HP
Potion of Defense    stat=21  should raise Defense    raises Max HP
Potion of Dexterity  stat=28  should raise Dexterity  raises Max HP
Potion of Mana       stat=3   should raise Max MP     raises Defense
Potion of Speed      stat=22  should raise Speed      raises Max HP
Potion of Vitality   stat=26  should raise Vitality   raises Max HP
Potion of Wisdom     stat=27  should raise Wisdom     raises Max HP
Potion of Life       stat=0   correct
```

**The fix is one eight-row table applied at parse time**, in `desc.rs` and `activate.rs` both, so
that nothing downstream ever sees a content number. Not a translation at the point of use — the
reason this survived is that two call sites each translated differently and neither knew the other
existed.

An earlier figure on [page 08](../mechanics/08-stats.md) said 670 of 758. The method there counted
differently; the numbers above are from `IncrementStat` uses alone, counting each
`<Activate>` and `<ActivateOnEquip>` once.

## Area damage ignores invulnerability

`GetDefenseDamage` (`wServer/realm/StatsManager.cs:85`) is the original's single damage-reduction
path, and it does five things:

```csharp
if (host.HasConditionEffect(ConditionEffects.Armored)) def *= 2;
if (host.HasConditionEffect(ConditionEffects.ArmorBroken)) def = 0;
float limit = dmg * 0.25f;//0.15f;
...
if (host.HasConditionEffect(ConditionEffects.Curse)) ret = (int)(ret * 1.20);
if (host.HasConditionEffect(ConditionEffects.Invulnerable) ||
    host.HasConditionEffect(ConditionEffects.Invincible)) ret = 0;
```

We have two implementations of it. `Rules::damage_after_defence` (`crates/sim/src/effects.rs:202`) is
correct, floor included — and the commented-out `0.15f` shows the fork raised the floor to **0.25**,
which our constant matches.

The other one does not. `projectile::after_defence` (`crates/sim/src/projectile.rs:257`) is a free
function taking no conditions at all:

```rust
let floor = (raw as f32 * 0.15).round() as i32;
```

Wrong floor, and no Armored, ArmorBroken, Curse, Petrify, Invulnerable or Invincible. It is called
from exactly one place — `World::explode` at `crates/sim/src/world.rs:2249` — which is the path for
**every grenade behaviour (66 uses), every player blast ability, and every vampire blast.**

So a boss that goes invulnerable between phases takes full damage from a spell, and armour is worth
a different amount depending on whether the hit came from a bullet or a blast. Delete the free
function and route `explode` through `Rules`.

## Ground damage is a different mechanic here

The original's is in `wServer/realm/entities/player/Player.Ground.cs` and is precise about it:

| | Original | Ours |
| --- | --- | --- |
| Cadence | one roll per **500 ms** burn | prorated every tick |
| Amount | `NextIntRange(min, max)`, one draw | `(min + max) / 2`, no draw |
| Who burns | **players only** | every living entity, enemies included |
| Requires | either damage element to be present | `max_damage > 0` — near enough, and now explicit |
| Skipped when | `Paused` or `Invincible` | never |
| Skipped when | an object on the tile has `ProtectFromGroundDamage` | never |
| Defence | bypassed — `HP -= dmg` | bypassed, same |

`apply_hazards` (`crates/sim/src/world.rs:2547`) filters on `is_alive_kind()`, so enemies standing in
lava take damage they never took in the original. Several dungeons place enemies on hazard tiles
deliberately.

The 500 ms cadence is not cosmetic. The comment on `GroundDamagePeriodMs` says why: the client rolls
the same burn from the same shared random stream, and *both sides must not roll*, because a stream
stepped once here and twice there desynchronises every later shot. See
[page 36](../mechanics/36-the-shared-random-stream.md).

We parse `ObjectDesc::protect_from_ground_damage` and never read it.

`TileDesc.Damaging` turned out **not** to be a written flag: the original derives it from the
presence of `MinDamage` or `MaxDamage`, and no content file contains a `<Damaging/>` element. An
earlier draft of this page had it as a separate element the content sets, which would have made
lava stop hurting. It is derived here now too, and the distinction costs nothing.

## Ocean Trench has no oxygen

`HandleOceanTrenchGround` is the whole of that dungeon's pressure: on a **100 ms** clock, a player
not standing within one tile of object type `0x0731` loses 2 oxygen, and at zero oxygen loses 10 HP a
tick; standing near one restores 8 up to a cap of 100. Hidden players are exempt.

Nothing in this server mentions oxygen. The dungeon loads and is survivable anywhere in it.

## Projectile damage can roll its maximum

Both of the original's damage rolls are half-open. `Shoot.cs:181` uses .NET's
`Random.Next(min, max)`, exclusive of `max`; `wRandom.NextIntRange` is `min + Gen() % (max - min)`,
also exclusive. So a 55–90 projectile rolls 55 to 89.

`ProjectileDesc::roll_damage` (`crates/content/src/desc.rs:152`) is inclusive:

```rust
self.min_damage + (roll * (span + 1.0)) as i32
```

The `+ 1.0` is what makes the top value reachable. Removing it matches the original, for every
projectile in the game.

## `GenericActivate` is not generic

26 items use it, and we treat it as an unknown id and tell the player "nothing happens"
(`crates/server/src/session.rs:2169`).

It is not an unknown id. `AEGenericActivate`
(`wServer/realm/entities/player/Player.UseItem.cs:1183`) is a fully specified area effect: apply
`condEffect` for `duration` over `range`, centred on the player or on the mouse depending on
`center`, to players or to enemies depending on `target`, with `useWisMod` scaling both the duration
and the range. Entities in `Stasis` or `Invincible` are skipped.

Every argument it needs is already parsed into `ActivateDesc.args`. The work is to give it a variant
rather than to discover what it does.

## Fields we do not parse, and need not

Checked because they are in the C# `ObjectDesc` and not in ours. None of them costs anything:

| Field | Why it does not matter |
| --- | --- |
| `NoMiniMap`, `ShowName`, `DontFaceAttacks` | never read by the C# server; client rendering |
| `Tags`, `Restricted` | parsed by the original and read nowhere |
| `TrollWhiteBag` | read by `Loots.ShowBags`, but **no object in the content sets it** |
| `TileDesc.PushX` / `PushY` | never read by the C# server; pushing is client-side |

That last one corrects [page 35](../mechanics/35-descriptors.md), which records the `PushY` parse bug
— reading `elem.Attribute("dy")` as the guard and `anim.Attribute("dy")` as the value — as though it
mattered. It is a bug in a value the server never uses.

`TileDesc.Damaging` is the one flag in this group that does matter, and it is in the ground-damage
section above.
