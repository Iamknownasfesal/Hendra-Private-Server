# Worlds: two tick loops, entry and exit, passability

Read from `realm/worlds/World.cs`.

## There are two loops, on different threads and at different rates

```
Tick(time)        // the "network" ticker
    timers
    every player
    every projectile

TickLogic(time)   // the "logic" ticker, under the delete lock
    enemies, but only those in chunks near players
    decoys
```

Players and projectiles advance on one loop; enemies advance on the other. They are separate tickers
(`NetworkTicker` and `FLLogicTicker`) with their own periods, so **an enemy and a player do not step
at the same rate**, and a projectile is walked on the player loop rather than the enemy one.

`TickLogic` is where the enemy-proximity rule from [the entity page](04-entities.md) is actually
enforced at the container level:

```
foreach (var i in EnemiesCollision.GetActiveChunks(PlayersCollision))
    i.Tick(time);
```

Only chunks of the collision map that contain, or neighbour, a player are walked at all. The
per-entity `AnyPlayerNearby()` check inside `Entity.Tick` is the second half of the same rule. So an
enemy far from every player is not merely skipped — its chunk is never visited.

This server ticks one loop and every enemy in it.

## A world deletes itself when it empties

```
if (!Persist && _elapsedTime > 60000 && Players.Count <= 0) Delete();
```

Sixty seconds of world time must have passed **in total**, not sixty seconds of being empty: a world
younger than a minute is never deleted even with nobody in it. `Persist` exempts a world entirely.

## Timers are queued and folded in at the top of the tick

`AddTimer` enqueues; the tick drains the queue onto the list before running it, and walks the list
**backwards** so removals do not disturb it. A timer that throws is logged and removed rather than
retried.

## Entering and leaving

`EnterWorld` assigns an id from a global counter and inserts into one of two collision maps:

- **Players** and **decoys** go into `PlayersCollision`.
- **Enemies** and every other static object go into `EnemiesCollision`.
- Projectiles go into neither; they are keyed by `(ownerId, bulletId)`.
- An enemy whose description is marked `Quest` is also added to a `Quests` index, which is what the
  quest arrow searches.

`LeaveWorld` mirrors it, and does two extra things worth copying:

- **Leaving cancels a trade** the player was in.
- **Removing an object that blocked sight** rebuilds the visibility region and bumps the sight
  counter of every player within sight radius, so the newly opened line is sent out.

## Passability has a "spawning" mode

```
IsPassable(x, y, spawning):
    off the map              -> impassable
    ground NoWalk            -> impassable
    object FullOccupy        -> impassable
    object EnemyOccupySquare -> impassable
    object OccupySquare      -> impassable only when spawning
```

`OccupySquare` blocks *placement* but not *movement*. Every behaviour that spawns something passes
`spawning: true`, which is why a spawner will not drop a child onto a square a player could walk
across.

## Broadcasts are by sight radius

`BroadcastPacketNearby` uses `DistSqr < Player.RadiusSqr` — 20 tiles, the same radius as sight. There
is a variant that takes a position rather than an entity, used by anything that happens away from its
owner, such as a grenade landing.

## Speech reaches every enemy in the world

```
ChatReceived(player, text):
    every enemy       gets OnChatTextReceived
    every static object gets OnChatTextReceived
```

**Not filtered by distance at all.** The distance check lives in `PlayerTextTransition` itself. This
matters for parity: our `World::heard` records the speaker's position and each listener measures its
own distance, which is equivalent, but the original also gives speech to **static objects**, and we
only give it to entities with a mind.

## Quake: moving a whole world somewhere else

```
QuakeToWorld(newWorld):
    close this world unless it persists
    broadcast an Earthquake effect
    after 8s: reconnect everybody to newWorld -- except Paused players, who go to the nexus
    after 20s (non-persistent worlds only): disconnect anyone still here
```

## What this server does differently

- **One tick loop, not two.** Players, enemies and projectiles all advance together here. The
  original runs enemies on a separate ticker under a lock, and only for chunks near a player.
- **We tick every enemy.** The original visits only active chunks, then checks proximity per entity.
- **Speech does not reach static objects here**, only entities with a behaviour.
- We should check: the sixty-second minimum before an empty world is deleted, trade cancellation on
  leaving, the sight-region rebuild when a sight-blocking object is removed, and the `spawning` mode
  of passability.
