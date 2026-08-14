# Abilities: the activate effects

Read from `realm/entities/player/Player.UseItem.cs`.

The gating before an effect runs — distance, magic cost, slot type, consumable, successor — is
covered in `PLAN.md` §18.15–18.16. This page is the effects themselves.

## The wisdom modifier

Many effects scale with wisdom. The formula, from `UseWisMod`:

```
totalWisdom = Base[7] + Boost[7]
if totalWisdom < 30: the value is unchanged
otherwise:
    n = value * totalWisdom / 150 + value        // i.e. value * (1 + wisdom/150)
    truncated to `offset` decimal places, defaulting to one
```

**Wisdom below 30 does nothing at all.** Above it, the multiplier is `1 + wisdom/150`, so 150 wisdom
doubles the effect. The truncation is to one decimal place for amounts and, where `offset` is 0, to
whole numbers.

`UseWisMod` is applied to whichever of amount, duration and range the effect declares `UseWisMod` for
— not to all three automatically.

## Stat boosts from abilities

```
AEStatBoostSelf:
    Stats.Boost.ActivateBoost[idx].Push(amount, NoStack)
    a world timer pops the same amount after DurationMS
```

The push and the pop are matched by **value, not by identity** — `Pop(amount)` removes one entry
equal to that number. Two identical boosts are interchangeable.

`AEStatBoostAura` does the same to every player in range, each with its own timer, and adds one
special case:

```
if NoStack and amount > 0 and idx == 0:
    HP = min(maxHP, HP + amount)
```

A non-stacking **max-health** aura heals for its own amount immediately, so raising the ceiling also
fills it. The comment calls it a hack job; it is what makes those abilities feel like a heal.

The aura's visual is suppressed for non-stacking boosts.

## Shooting abilities

`AEShoot` fires `item.NumProjectiles` at `item.ArcGap` degrees apart, centred on the aim, with damage
from `Stats.GetAttackDamage(min, max, isAbility: true)` — and **`isAbility: true` means the attack
multiplier is 1**, so an ability's damage does not scale with attack. Each shot counts toward
`Shots`.

`AEBulletNova` is fixed at **20 projectiles in a full circle**, originating **at the target point**
rather than at the player, with damage rolled per projectile rather than per volley. It refuses if
the target is beyond `MaxAbilityDist`.

## Area effects

`AEGenericActivate` is the general case:

```
centre = eff.Center == "mouse" ? the aim : the player
targets = eff.Target == "player" ? players : enemies
apply the condition effect to everything in range, skipping anything in Stasis or Invincible
```

**Stasis and Invincible are skipped**, so an ability cannot debuff something that is untouchable.

## Grenades and poison

`AEHealingGrenade` places a `Placeholder` entity at the target for 1500ms, then blasts. The
placeholder exists so the effect has something to be attached to — worth knowing, because a naked
timer would have no object id to name.

`PoisonEnemy` spreads its total damage evenly over the duration:

```
remainingDmg = GetDefenseDamage(enemy, TotalDamage, enemy.Defense)     // defence applied once, up front
perDmg = remainingDmg * 1000 / DurationMS                             // per second
```

So poison's defence reduction is applied **once to the total**, not to each tick.

## Healing

`ActivateHealHp` and `ActivateHealMp` both do nothing at all when the target is already full — no
packet, no notification. Worth matching, because it is the difference between a visible heal and a
silent one.

## What this server does differently

- **The wisdom modifier has no equivalent here.** Confirmed: nothing in the Rust reads wisdom for
  anything but magic regeneration, and **63 activate effects in the content declare `useWisMod`**.
  Every one of them should scale its amount, duration or range by `1 + wisdom/150`, and not at all
  below 30 wisdom. This is a whole stat's worth of effect on every ability in the game.
- **Ability shots fire the wrong projectile.** Confirmed: our `fire_spread` reads
  `entity.weapon` — the player's *equipped weapon* — where `AEShoot` uses `item.Projectiles[0]`, the
  **ability item's own** projectile, with `item.NumProjectiles` and `item.ArcGap`. So an ability that
  should throw its own bullet throws a copy of your weapon's instead: wrong sprite, wrong damage
  range, wrong effects.
- **`isAbility: true` holds the attack multiplier at 1.** Ours happens to match, because
  `fire_spread` applies no multiplier at all — but by accident rather than by decision, and the
  weapon path applies one a few lines away.
- **`AEBulletNova` is fixed at 20 projectiles, originates at the target**, not the player, and rolls
  damage per projectile. Ours takes the count from the effect, fires from the player, and uses the
  weapon's projectile.
- **Non-stacking max-health auras should heal for their own amount.**
- **Abilities should skip targets in Stasis or Invincible.**
- **Poison applies defence once to the total**, then divides.
- A heal that would change nothing should send nothing.
