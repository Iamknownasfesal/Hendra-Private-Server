# The ten worlds with their own logic

Read from `realm/worlds/logic/`.

## Realm

```
Init:            load map 1, apply setpieces, create and initialise Oryx
AllowedAccess:   refused while Closed, even to administrators
Tick:            open or close the nexus portal to match Closed
                 every 1800 seconds of world time (a 10-second window), close the realm
                 once Closed and empty: Init() again and set Closed = false
EnemyKilled:     forwarded to Oryx, but only when the enemy was not Spawned
EnterWorld:      players are announced to Oryx
```

Two things worth copying exactly:

- **The realm recycles rather than being deleted.** When it has closed and emptied, the same world
  object reloads its map, re-applies its setpieces and re-creates Oryx, then reopens. Its id, and
  therefore the portal that leads to it, survives.
- **The close is on a modulo of world time**, `secondsElapsed % 1800 < 10`, checked once a tick — not
  a countdown started when the realm opened. Two realms started at different times still close
  together.
- The overseer runs on a **separate task**, one at a time, so a slow population pass does not hold up
  the tick.

## Vault

Entirely reworked by this project: capacity is an integer on the account, the eighty chests are gone,
and one `VaultAccess` object opens a panel. Not a source for original mechanics.

Worth carrying regardless: access is refused unless the account owns the vault, and the whole vault
is sent **on the first tick where the client reports `Ready`**, not in `EnterWorld` — the comment
records that doing it in `EnterWorld` sent the snapshot before the client was listening and drew an
empty vault every time.

## GuildHall

```
AllowedAccess: the player's guild id must match the hall's
Init:          the map is chosen by the guild's level, 0 to 3
GetInstance:   reuse an existing hall for that guild if it has anybody in it;
               if one exists and is empty, delete it and make a new one
```

**An empty hall is destroyed and rebuilt** rather than reused, which is how a guild that has just
been upgraded gets the larger map without anybody having to do anything.

## Davy

A world with four keys, tracked by four booleans:

```
EnterWorld(player): send "showKeyUI", then one notification per key already found
LeaveWorld(entity): if the entity is a Purple/Green/Red/Yellow Key, mark it found and tell everybody
```

The keys are **entities leaving the world**, not items being picked up — so the trigger is the key
object being removed, whatever removed it. A player arriving late is told which keys are already
found, which is the whole point of keeping the flags.

## Nexus, Marketplace, DonorShop, Castle, Candyland, Test

Thin: they place merchants by region, or override nothing at all. `Castle` overrides nothing, which
is why our census records it as "nothing: the original's class overrides nothing either".

## What this server does differently

- **Our realm is deleted and recreated** rather than reset in place. The visible difference is that
  the portal to it changes identity; the original keeps the same world and therefore the same portal.
- **The close is a per-world countdown here**, not a global modulo. Two realms in the original close
  in step; ours close thirty minutes after each opened.
- **An empty guild hall should be destroyed and rebuilt**, so an upgrade takes effect on the next
  visit.
- Davy's key notifications match; ours announces on the key being found and tells arrivals what is
  already found.
