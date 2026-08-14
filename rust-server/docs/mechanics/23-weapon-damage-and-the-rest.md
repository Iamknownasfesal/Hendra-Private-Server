# Weapon damage, keep-alive, and the small entities

Read from `Player.Projectiles.cs`, `BaseStatManager.cs`, `Player.KeepAlive.cs`, `Trap.cs`,
`Placeholder.cs`, `OneWayContainer.cs`.

## Weapon damage comes from stats 8 and 9, not from the projectile

```csharp
// BaseStatManager, recomputed on every inventory change
_base[8] = weapon.Projectiles[0].MinDamage;
_base[9] = weapon.Projectiles[0].MaxDamage;

// Player.Projectiles
var dmg = Stats.GetAttackDamage(Stats[8], Stats[9]);
```

So a player's shot is `rand(Base[8] + Boost[8], Base[9] + Boost[9]) * attackMultiplier`, where the
base pair is copied from the equipped weapon's first projectile.

This is why `DamageMin` and `DamageMax` are stats: **an item can add flat damage** through
`Boost[8]`/`Boost[9]`, on top of whatever weapon is held.

Our shot damage reads the weapon's projectile and applies the attack multiplier, which is the same
answer for the base case. The difference is only that a `DamageMin`/`DamageMax` **boost** would do
nothing here — and the shipped content grants none, so it is a structural gap rather than a live one.
Worth knowing before any content adds one.

## Keep-alive

```
PingPeriod  = 3000ms
DcThreshold = 12000ms
```

A ping every three seconds; no pong for twelve seconds disconnects. Three separate acknowledgement
queues — shoot, update and goto — each with its own deadline, and **overrunning any of them
disconnects**, which is how the original stops a client from ignoring the packets that would slow it
down.

Every ping also does two things beyond timing:

- **Renews the account lock**, disconnecting if it cannot. This is the single-session guarantee, and
  it is renewed rather than held.
- **Saves the character**, unless in the test world. So progress is written every three seconds, not
  only on leaving.

Clock mapping is kept as running averages:

```
TimeMap = mean(serverTime - clientTime)     // client clock to server clock
Latency = mean((serverTime - pingSerial) / 2)
C2STime(t) = t + TimeMap
```

Both are means over the whole session, so they drift toward the truth and never jump.

## Trap

```
lifetime 10 seconds, radius as given
every 500ms: draw the trap ring at radius/2
after 20 draws (10s): explode
each tick: if any enemy is within radius/2, explode
explode: blast at the full radius, damage every enemy in it, apply the effect, leave the world
```

The **trigger radius is half the blast radius** — you have to step well inside for it to go off, and
then it catches things further out than that.

## Placeholder

An invisible zero-size static object with a lifetime, object type `0x070f`. It exists so a delayed
effect has something with an id to be attached to — grenades, healing grenades, anything that lands
later.

## OneWayContainer

A `Container` subclass with **no behaviour of its own at all**. The one-way rule lives in
`ItemUtils.AuditItem`:

```csharp
if (container is OneWayContainer && item != null) return false;
```

So the container refuses anything being put **in**, and taking out is ordinary. That single line is
the whole mechanic, and it is why a merchant's stall cannot be used as storage.

## What this server does differently

- **Weapon damage matches** for the base case; the boost path is unused by current content.
- **We have no acknowledgement deadlines.** The original disconnects a client that fails to
  acknowledge a shoot, update or goto within its deadline. Ours has a strike counter for refused
  moves, which covers a different thing.
- **The account lock is renewed on every ping** and the character is saved every three seconds. Ours
  saves at other moments; worth confirming a crash cannot lose more than a few seconds.
- **Trap's trigger radius is half its blast radius.** Worth checking ours.
- The one-way rule — refuse anything put in, allow anything taken out — should be a property of the
  container kind, which it is here.
