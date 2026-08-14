# The player: tick order, damage, and death

Read from `realm/entities/player/Player.cs`, with `Player.Effects.cs` and `Player.Leveling.cs`
covered on their own points.

## Arriving in a world

```
Init(world):
  pick a random spawn region from the map, stand at its centre (+0.5, +0.5)
  grant move grace
  tiles = new byte[map.Width, map.Height]      // the fog-of-war record, per player per world
  FameCounter = new FameCounter(this)
  FameGoal = GetFameGoal(bestFameForThisClass)
  ExperienceGoal = GetExpGoal(level)
  Stars = GetStars()
  if world is OceanTrench: OxygenBar = 100
  SetNewbiePeriod()                            // three seconds unseen by enemies
```

The spawn point is **a random region of the map**, not a fixed point, and the fog-of-war array is
allocated per world entry — one byte per tile, which is what makes `tiles_seen` per world rather
than per character.

## The tick order

```
Tick:
  if not KeepAlive(time): return          // connection health first; a dead link stops here
  CheckTradeTimeout(time)
  HandleQuest(time)

  if not Paused:
      HandleRegen(time)
      HandleEffects(time)
      HandleOceanTrenchGround(time)
      TickActivateEffects(time)
      FameCounter.Tick(time)
      ApplyDeferredHits(time)              // this project's addition
      CheckGroundDamage(time)              // this project's addition

  base.Tick(time)                          // the entity tick: state machine, position history, effects
  SendUpdate(time)
  SendNewTick(time)

  if HP <= 0: Death("Unknown", rekt: true)
```

**`Paused` suspends regeneration, effects, boosts and the fame counter**, but not the quest arrow,
not the trade timeout, and not the entity tick beneath it.

Note the last line: a player who reaches zero health by any route not already handled dies as
`"Unknown"` **with `rekt` set**, which — see below — means no death is recorded.

## Regeneration carries a fraction between ticks

```
if HP is full or cannot regen: counter = 0
else:
    counter += GetHPRegen() * dt / 1000
    whole = (int)counter
    HP = min(maxHP, HP + whole); counter -= whole
```

The counter is **reset to zero** whenever health is full or regeneration is suppressed, so a
fractional point in progress is lost rather than banked. Magic works the same way.

## Boost timers

`XPBoostTime` is cleared outright once the player reaches level 20 — an experience boost is worth
nothing at maximum level and is taken away rather than left running.

## Teleporting

Restrictions, all skipped when the caller says to ignore them:

- Not before the cooldown (10 seconds, `SetTPDisabledPeriod`).
- The world must allow teleporting.
- Not while `Paused`, and not **to** a `Paused` player.
- Only to players, never to yourself, and never to an `Invisible` player.

A successful teleport sets the teleport cooldown, **starts the newbie period again**, counts a
teleport for fame, and re-points the quest arrow.

The move itself is a `Goto` to every player in the world, each of whom is made to acknowledge it.

## Damage to a player

```
IsInvulnerable() = Paused or Stasis or Invincible or Invulnerable

HitByProjectile:
  a player's own bullet never hurts a player
  invulnerable -> no hit at all
  dmg = Stats.GetDefenseDamage(projectile.Damage, projectile.ArmorPiercing)
  if not Invulnerable: HP -= dmg
  apply the projectile's condition effects
  broadcast Damage{ effects = Invincible ? none : the projectile's }
  if HP <= 0: Death(named after the shooter)
```

Two subtleties:

- `IsInvulnerable` already returned for `Invulnerable`, so the second check is redundant — but it
  means **`Invulnerable` suppresses the health loss while still applying the effects** if ever
  reached by another path.
- **`Invincible` suppresses the condition effects on the wire** but not their application.

## Death has six escape hatches, in order

```
Death(killer, entity, tile, rekt):
  if already dead or disconnected: return
  if tile != null and tile.Spawned: rekt = true

  if Rekted(rekt)                 -> gravestone marked "got rekt", back to the nexus. No record.
  if NonPermaKillEnemy(entity)    -> killed by a Spawned enemy or a controlled one: same. No record.
  if TestWorld()                  -> died in the test world: gravestone, nexus. No record.
  if Resurrection()               -> an equipped item with Resurrects breaks: nexus. No record.
  if Nexus()                      -> died in the nexus: gravestone, nexus. No record.

  otherwise:
      SaveToCharacter()
      Database.Death(...)          -> the permanent record
      GenerateGravestone()
      AnnounceDeath(killer)
      send Death packet, disconnect after 1s
```

**Five of the six paths end without a death being recorded at all.** The character survives. In
particular:

- **Anything a behaviour spawned cannot permanently kill a player.** `NonPermaKillEnemy` checks
  `entity.Spawned`, and every behaviour-created enemy carries that flag. So a boss's summons can send
  you to the nexus but never take the character.
- **A hazard tile that was itself spawned** likewise only sends you home.
- **Resurrection** searches the four **equipped** slots for an item with `Resurrects`, destroys it,
  announces it, and sends the player home.

The gravestone's object type depends on how many of the eight stats are maxed:

| Maxed | Object | Stands for |
| --- | --- | --- |
| 8 | `0x0735` | 10 minutes |
| 7 | `0x0734` | 10 minutes |
| 6 | `0x072b` | 10 minutes |
| 5 | `0x072a` | 10 minutes |
| 4 | `0x0729` | 10 minutes |
| 3 | `0x0728` | 10 minutes |
| 2 | `0x0727` | 10 minutes |
| 1 | `0x0726` | 10 minutes |
| 0 | `0x0725` | 5 minutes |
| 0, level < 20 | `0x0724` | 1 minute |
| 0, level ≤ 1 | `0x0723` | 30 seconds |

A "rekt" gravestone is named `"{Name} got rekt"` rather than the player's name.

## Death announcements

```
"{name} died to {killer}! ({maxed}/8, {fame} Fame)"
```

- **Six or more stats maxed, or 1,000 fame or more** — announced to every player in **every world**,
  unless the account is an administrator.
- Otherwise, if the player is in a guild **and level 20**, the message goes to the whole guild
  wherever they are, and to everyone else in the same world.
- Otherwise, to everyone in the same world.

## What this server does differently

- **Our death has two of the five escape hatches.** `die()` in `session.rs` handles a safe world (the
  nexus and any personal world) and a resurrection item, in that order and before anything durable
  happens, which matches. Missing:
  - **`NonPermaKillEnemy`** — being killed by a `Spawned` enemy, or one under a player's control,
    sends you home without taking the character. Every enemy a behaviour creates carries that flag,
    so in the original **a boss's summons can never end a character**. This is the largest of the
    three and it changes how dangerous every summoning fight is.
  - **`Rekted`** — the `rekt` flag, set when the killing tile was itself spawned, and set by the
    catch-all at the end of the tick when health reaches zero with no attributed cause. Also spares
    the character, and names the gravestone `"{Name} got rekt"`.
  - **`TestWorld`** — no test world ships here, so this one is genuinely not applicable.
- **`Paused` should suspend regeneration, effects and boosts**; we should check it does.
- The regeneration fraction is **discarded** when health is full, not banked.
- Gravestones: we place one, and should check the eight-way object choice and the three short-lived
  cases for low levels.
- The death announcement's two-tier rule (server-wide for notable deaths, guild-wide at level 20) is
  worth matching; it is most of what makes a death feel like an event.
