# Enemies, characters, containers and static objects

Read from `realm/entities/Enemy.cs`, `Character.cs`, `Container.cs`, `StaticObject.cs`.

## Character: the base for anything with health

Construction reads the description and does three things:

```
size:  SizeStep != 0 ? MinSize + rand(0..(MaxSize-MinSize)/SizeStep) * SizeStep : MinSize
immunities: eight flags on the description become permanent condition effects
HP = MaximumHP = ObjectDesc.MaxHP
```

**Size is randomised per entity at construction**, in steps, which is why a field of the same monster
is not a field of identical monsters.

The eight immunities — `ArmorBreakImmune`, `CurseImmune`, `DazedImmune`, `ParalyzeImmune`,
`PetrifyImmune`, `SlowedImmune`, `StasisImmune`, `StunImmune` — are applied as **permanent condition
effects**, not read from the description at the point of use. So they show on the wire like any other
effect and are refused by `ApplyCondition` on arrival.

A player exports `HP` but **not** `MaximumHP`; every other character exports both.

## Enemy

```
stat = (ObjectDesc.MaxHP == 0)
```

An enemy whose description gives it no health is a **"stat" enemy**: it takes no damage from anything,
and does not bleed. That is how decorative and invulnerable enemies are expressed — by omission,
not by a flag.

### Damage

```
if stat: 0
if Invincible: 0
if Paused or Stasis: 0                    // the whole damage block is skipped
def = ObjectDesc.Defense, or 0 if armour-piercing / noDef
dmg = GetDefenseDamage(this, dmg, def)
if not Invulnerable: HP -= dmg
apply the projectile's effects
broadcast Damage
counter.HitBy(...)                        // credited even when Invulnerable
if HP < 0: Death()
```

Three things worth copying exactly:

- **`Paused` and `Stasis` make an enemy immune**, not merely inert.
- **`Invulnerable` still records the damage in the ledger.** The health does not move but the player
  still earns the loot share. `Invincible` returns before any of it.
- **Death is `HP < 0`, strictly.** An enemy sitting on exactly zero health is **alive**. Ours uses
  `hp <= 0`, so we kill things one hit earlier in the exact-zero case.

### Bleeding

Enemies bleed at **28 health per second**, accumulated as a fraction on the entity, and **without the
"never below one" floor that players get**. An enemy can bleed to death; a player cannot.

### Spawn point

`SpawnPoint` is captured on the **first tick**, not at construction:

```
if (pos == null) pos = current position
```

So `ReturnToSpawn` returns to where the enemy stood on its first tick, which for anything placed and
then moved by its own first behaviour is not where it was created.

### `SetDamageCounter` is empty

The method body is `{ }`. `CopyDamageOnDeath` calls nothing but this, so **that behaviour does
nothing at all** in the original. `TransferDamageOnDeath` uses `TransferData` and does work.

## StaticObject

```
Vulnerable  = a MaxHitPoints element exists
Dying       = health drains with time
Hittestable = whether projectiles test against it
```

- **`Dying` subtracts elapsed milliseconds from health every tick**, so "health" for a dying object is
  a lifetime in milliseconds. That is how a loot bag with a 60,000 lifetime works.
- An invulnerable static object exports `HP = int.MaxValue`.
- When one dies it **clears itself off the map tile** at `(X - 0.5, Y - 0.5)` if that tile still holds
  its own object type, and bumps the tile's update count.
- Only **players'** projectiles damage them.

## Container

- **Eight slots.** `SlotTypes` and any starting `Equipment` come from the object's own XML.
- **A container with nothing in it leaves the world on its next tick** — except object type `0x504`,
  the vault chest, which is exempt.
- Containers are never hit by projectiles.
- `OwnerAccountId` is exported as the single owner when there is exactly one, else `-1`.

## What this server does differently

- **`HP < 0` versus `hp <= 0`.** Confirmed: every death test here is `hp <= 0`, in five places. We
  kill things one hit earlier whenever a hit lands exactly on zero.
- **`Stasis` does not make an enemy untouchable here.** Confirmed:
  `untouchable = Invincible || paused`, with no `Stasis`. In the original both `Paused` and `Stasis`
  skip the whole damage block, so a stasised enemy takes nothing.
- **Size is not randomised.** Confirmed: `SizeStep` is parsed into `desc.rs` and read by nobody, so
  every instance of a monster is the same size where the original varies them in steps.
- **Enemy bleeding has no floor in the original**; check ours does not apply the player's floor to
  enemies.
- **`Invulnerable` should still credit damage** to the ledger.
- **`Paused`/`Stasis` should make an enemy immune to damage**, not just inert.
- **A dying static object's health is a millisecond countdown**; ours uses `expires_in_ms`, which is
  equivalent, but the object also clears its map tile on death and we should confirm we do.
- An empty container should remove itself; the vault chest is the one exemption.
