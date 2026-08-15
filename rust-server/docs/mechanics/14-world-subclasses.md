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

The shipped 2020 design differs in kind, not degree. Read it at
`git show 94615c4:Server-Side/wServer/realm/worlds/logic/Vault.cs` and
`…/realm/entities/vendors/ClosedVaultChest.cs`, not in the working tree. `Vault.cs` built **one
8-slot `Container` entity per owned chest** (`:96-107`), each backed by a `DbVaultSingle` over redis
field `vault.<i>` and placed on the `TileRegion.Vault` tiles nearest the spawn, sorted by distance;
the map's supply of those tiles was the real capacity limit — **80** in the 2020 `Vault.jm` — every
leftover tile got a `ClosedVaultChest` you bought by walking into (`:108-113`), and gifts were dealt
eight at a time into `GiftChest` entities on `Gifting_Chest` tiles (`:115-136`). A move was an
ordinary `InvSwap` between two entities within a tile of each other
(`networking/handlers/InvSwapHandler.cs:36-137`). A new account owned **one** chest
(`XmlDatas/data/init.xml:37`) and another cost 400 fame (`:7`, `ClosedVaultChest.cs:14-16`).

**Our Rust implements the reworked design, not the 2020 one.** That is a choice this project made,
not a fact recovered from the reference — the reworked `VaultState.cs` and the current `Vault.cs` are
this project's own writing, so measuring against the binary built from them measures us. The
quantities above are the part that *is* evidence, and `crates/server/src/vault.rs` now cites them at
their pristine lines; the panel, its three packets and the version counter are flagged there as ours.

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

## Nexus

`Init` walks every world the manager already has and gives it a portal:

- **every `Realm`** gets a default portal on a random `Realm_Portals` tile;
- **`ClothBazaar`** gets object `0x167` at `Store_39`;
- **`Marketplace`** gets object `0x190` at `Store_37`, and only when the market is enabled.

Worlds with a **non-negative id are skipped** — that is, only the static hub worlds are linked, and
dynamic instances are not. A named portal whose region is missing from the map is skipped silently
(`if (pos == null) continue`), so a Nexus map without a `Store_37` tile simply has no marketplace
portal and says nothing about it.

The names carry `(0)` from the start because `PortalMonitor.Tick` finds the count by regex on the
name and needs something to replace.

## Castle

Overrides `GetSpawnPoints`, and it is the only world that scales its spawn spread to the crowd:

```
< 20 players entering  ->  1 spawn tile
< 40                   ->  2
< 60                   ->  3
otherwise              ->  every spawn tile
```

`Take(n)` on `Map.Regions` takes the **first n in dictionary order**, not n chosen or n spread apart,
so which tiles those are is whatever the map load happened to produce.

`PlayersEntering` is a constructor argument defaulting to 100, and there is a second constructor that
sets it to **0** — used when the Castle arrives by `/quake`, so that everybody quaked in lands on one
tile together. The reflection-based `DynamicWorld.TryGetWorld` calls `(ProtoWorld, Client)`, which is
the zero one, so **a Castle created the ordinary way always uses a single spawn tile.** The 100
default is only reachable from a call site that passes it explicitly.

## Candyland

Two enemies in the map are singled out by object type: the candy spawners (`0x5e31`) and the boss
spawner (`0x5e43`). Both are set `TickStateManually = true` in `Init`, which takes them out of the
ordinary behaviour tick, and the world then ticks their state itself from `Tick` — the **200 ms slow
tick**, not the fast one.

So Candyland's spawners run at a fifth of the rate every other enemy's behaviour runs at. That is the
whole mechanism: no counters, no timers, just a slower clock for the two things that spawn.

The guard `_candyBossSpawner == null` returns early for the **whole method**, so a map missing the boss
spawner also stops the candy spawners ticking.

## Marketplace and DonorShop

Identical: `Init` calls `Manager.Market.InitMarketplace(this)`. **Both** register as the marketplace,
and the second one to initialise wins — `InitMarketplace` assigns `_marketplace` outright — so on a
server running both, the merchants are placed in whichever loaded last. `RealmManager` adds
`DonorShop` before `Marketplace` in the `Marketplace` server mode, so the Marketplace wins there.

## Test

`Init` is overridden to do **nothing at all**, so a Test world starts with no map. The map arrives
from the client: `ConnectManager` writes the submitted JSON to disk, calls `LoadJson`, and the world
converts it with `Json2Wmap` and runs `InitShops`. Requires rank 50.

`LoadJson` guards the conversion with `JsonLoaded` but calls `InitShops()` **outside** the guard, so a
second load re-runs shop placement on an unchanged map.

## How a world class is chosen

`DynamicWorld` reflects over every `World` subclass at startup and matches **`type.Name` against
`proto.name`** — the class name and the world's name in the data must be the same string. There is no
registration list and no error when nothing matches; `TryGetWorld` just leaves `world` null and
`RealmManager` falls back to a plain `World`. Renaming a class silently demotes that world to having
no logic.

`DungeonTemplates.cs` is entirely commented out.

## What this server does differently

- **Our realm is deleted and recreated** rather than reset in place. The visible difference is that
  the portal to it changes identity; the original keeps the same world and therefore the same portal.
- **The close is a per-world countdown here**, not a global modulo. Two realms in the original close
  in step; ours close thirty minutes after each opened.
- **An empty guild hall should be destroyed and rebuilt**, so an upgrade takes effect on the next
  visit.
- Davy's key notifications match; ours announces on the key being found and tells arrivals what is
  already found.
- **Castle's spawn spreading was recorded as "overrides nothing" by an earlier census.** It overrides
  `GetSpawnPoints`, and the practical behaviour is one spawn tile because of which constructor the
  reflection picks. Worth having, and worth having deliberately rather than by accident.
- **Candyland's spawners tick on the 200 ms world tick, not the behaviour tick.** If we tick them with
  everything else they spawn five times as fast.
- Matching a world class to a world by **class name** is the kind of implicit link that fails
  silently. Ours should name the world explicitly in the data.
