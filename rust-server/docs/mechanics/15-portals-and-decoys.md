# Portals, decoys and traps

Read from `realm/entities/Portal.cs`, `Decoy.cs`.

## Portal

A portal is a **dying static object**: its "health" is a lifetime in milliseconds, so a portal with a
timeout counts itself down and leaves the world.

```
ValidatePortal:  a portal type the content does not have falls back to 0x0703,
                 the Portal of Cowardice, with a warning
Locked:          read from the content
Usable:          a stat on the wire, so the client can grey it out
HitByProjectile: always false
```

`CreateWorld` finds the first world definition whose `portals` list contains this object type:

- **A negative id is a fixed world** — fetched from the manager, shared by everybody.
- **A positive id is a new instance** — created through `DynamicWorld.TryGetWorld`, which maps the
  world's name to a subclass, or a plain `World` if none matches.

A portal opened by a player marks its world as a **player dungeon**, records the opener's name, and
gives it empty invite lists. That is the whole of the private-dungeon mechanic: `Opener`, `Invites`
and `Invited` on the world.

The world is created **once**, behind a lock, and everyone who walks in afterwards is handed the same
instance through `WorldInstanceSet`.

## Decoy

A `Decoy` is a static object that pretends to be a player. It is dying, so its health is its
lifetime.

```
direction: the player's own heading, taken from their position one tick ago;
           a random direction if there is no history or they were standing still
Tick:      while HP > duration - 2000, keep moving at the given speed
           when HP < 250 and it has not yet: broadcast a red AreaBlast once
```

So a decoy **runs for the first two seconds of its life and then stands still**, and puffs when it
has a quarter of a second left. It carries the player's own textures, so it looks like them.

It implements `IPlayer` with `Damage` doing nothing and `IsVisibleToEnemy` always true — which is the
point: enemies target it, and it cannot be hurt.

It is inserted into `PlayersCollision`, not `EnemiesCollision`, which is what makes enemy searches
find it.

## What this server does differently

- **A decoy here does not move.** The original takes the player's heading from their position one
  tick ago, runs for two seconds, then stops. Ours places a stationary one.
- **The quarter-second warning blast** is missing.
- **A decoy must be found by enemy target searches**, which means it belongs in the player collision
  set. Worth confirming ours is.
- **An unknown portal type should fall back to the Portal of Cowardice with a warning**, not be
  dropped.
- Player dungeons — `Opener`, `Invites`, `Invited` — have no equivalent here at all. Worth recording
  as a gap: a portal opened by a player creates a world only they and their invitees may enter.
