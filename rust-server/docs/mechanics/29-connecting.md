# Getting into a world

Read from `realm/ConnectManager.cs`, `realm/ConnectionQueue.cs`, `realm/RealmManager.cs`,
`realm/PortalMonitor.cs`.

## The queue is sorted by rank, and reconnects jump it

`ConnectionQueue` is a `SortedList` with a comparator that never returns 0 for two different
connections, so equal entries are pushed later rather than rejected. The sort key is:

```
rank + (reconnecting ? 101 : 0)      descending
then time added                      ascending
```

`101` is one more than the highest rank, so **any reconnecting player outranks every fresh
connection**, regardless of rank. That is what keeps a player who took a portal from being put behind
the login queue.

Comparing two entries with the same GUID returns 0, which makes `SortedList.Add` throw — and the
`catch` turns that into "Account already in queue". **Duplicate rejection is implemented as an
exception from the comparator**, which works but is why `Add` is wrapped in a bare `try`.

Three paths never queue at all:

- **Reconnecting** connects immediately, before any capacity check.
- **Rank > 0** connects immediately if the player count is under `maxPlayersWithPriority`, and is
  refused outright otherwise. A ranked player is never queued: they get in or they are told the
  server is full.
- Everyone else queues.

`GetPlayerCount()` is `Clients.Count + _recon.Count`, so a player in transit between worlds still
occupies a slot. Without that, a full server would let somebody in through the gap left by every
portal use.

The queue is only drained one entry per tick (`if under cap && queue non-empty, connect one`), so a
server that empties refills at the tick rate rather than all at once.

`ServerFull` is only sent when the position is worse than the free capacity — a player who will be
let in on the next tick is never shown a queue screen.

## Reconnect tokens

```
ReconTTL       90 seconds
ConnectingTTL  15 seconds
```

Taking a portal issues a `ReconInfo(destination world id, key, now + 90s)` keyed by account id. The
returning connection must match **both** the destination and the key byte-for-byte, and the entry is
`TryRemove`d before either check, so a token is single-use even when the check then fails.

Both maps are swept every 5 seconds, not every tick.

`ConnectingTTL` covers the gap between the handshake and the client actually sending `Create` or
`Load`. A client that handshakes and then goes quiet is dropped from `_connecting` after 15 seconds.

## What happens on connect, in order

1. **Admin account override.** An admin with `AccountIdOverride` set is swapped onto the target
   account, and the target records `AccountIdOverrider`. That is how staff inspect an account in
   place.
2. **Reconnect validation**, or, for a fresh connection, the destination is forced to Nexus (the only
   exception being `World.Test`).
3. **`AcquireLock`.** If the account lock is held, every client on that account id **or the same
   Discord id** is disconnected and the lock is tried once more. Failing twice tells the player how
   many seconds are left on the lock.
4. `acc.Reload()` — the account is re-read after the lock is held, not before, so what is loaded is
   what the lock protects.
5. `TryConnect` registers the client and recomputes the server's advertised player count.
6. The target world is resolved. **A deleted or missing world silently becomes Nexus** with an error
   line in chat rather than a disconnect.
7. `IsLimbo` worlds are replaced by `GetInstance(client)` — that is the per-player dungeon split.
8. `AllowedAccess`. A refusal sends the player to Nexus, unless they were already being refused
   *from* Nexus, in which case they are disconnected. **A one-shot world with nobody in it is deleted
   on the way out** (`!Persist && TotalConnects <= 0`), so a refused entry does not leave an orphan.
9. The client's RNG seed is derived and sent.
10. `MapInfo`, then the lock list, then the ignore list.
11. State becomes `Handshaked` and the client enters `_connecting`.

### The seed

```csharp
var seed = (uint)((long)Environment.TickCount * conInfo.GUID.GetHashCode()) % uint.MaxValue;
client.Random = new wRandom(seed);
```

This is the shared random stream both sides step in lockstep, and it is why ground damage cannot
simply be moved to the server — see [the verification page](26-verification.md). Note it is seeded per
*connection*, so every world change re-seeds it.

`% uint.MaxValue` on a value already cast to `uint` is a no-op for all but one value, and
`GUID.GetHashCode()` is not stable across .NET versions. Neither matters for a game seed.

## Portals into the Nexus

`PortalMonitor` owns one portal per world id, placed on a random tile of the Nexus's
`Realm_Portals` region.

`GetRandPosition` picks uniformly among region tiles and **retries until it finds one no portal
occupies** — a plain rejection loop, correct only because it first checks there are more tiles than
portals. When there are not, it returns `(0, 0)` and the portal is placed in the map corner rather
than refused.

The portal's name carries the population, and `Tick` rewrites it:

```csharp
_playerCountRgx = new Regex(@" \((\d+)\)$");
p.Name = _playerCountRgx.Replace(p.Name, $" ({count})");
```

The name is the storage. A world whose display name happens to end in a parenthesised number would
have that eaten. The comparison before assigning means the name field is only marked dirty when the
count actually changed, so a quiet world sends no updates.

Opening a portal announces it **to every player in every world**, phrased as "in Nexus" unless the
listener is already there, and "to this land" if they are in the world it leads to.

`ClosePortal`/`OpenPortal` flip `Usable`, which is separate from `Locked` — `PortalIsOpen` requires
both. Two independent reasons a portal can refuse.

## World ids

Static worlds carry **negative** ids from the world data (`wData.id < 0`), and dynamic ones get
positive ids from an incrementing counter starting at 0. `GetWorld` returns null for id 0, so **world
id 0 is permanently unusable** — the counter is pre-incremented, so nothing ever gets it.

`Marketplace` mode remaps a world onto `World.Nexus` via `actAsNexus`, which is how a marketplace-only
server presents a different world as the hub without changing any client-side id.

`GetRandomGameWorld` picks among non-closed realms by `Environment.TickCount % length` — not random,
just whatever the millisecond count lands on, and falls back to Nexus when every realm is closed.

## What this server does differently

- We have no queue and no capacity cap. If we add one, the two rules that matter are **reconnects
  outrank everything** and **in-transit players still count against capacity**.
- Our world handles are generational, so the "id 0 is unusable" quirk has no counterpart and needs
  none.
- **The population is stored in the portal's name and parsed back out with a regex.** Ours should keep
  the count as a field and format it when sending, which is both cheaper and not fragile.
- The account lock disconnecting by **Discord id as well as account id** is a real rule: one Discord
  account cannot hold two game sessions.
- `AllowedAccess` deleting an empty non-persistent world on refusal is the only thing stopping a
  refused entry from leaking an instance. Worth having wherever we create worlds on demand.
