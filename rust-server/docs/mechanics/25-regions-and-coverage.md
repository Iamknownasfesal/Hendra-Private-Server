# Regions, and what remains unread

## Tile regions match, and an earlier version of this page said they did not

There are **four** `TileRegion` enums in the C# tree:

| File | Namespace | Compiled |
| --- | --- | --- |
| `wServer/realm/terrain/Wmap.cs` | `wServer.realm.terrain` | yes — this is the live one |
| `wServer/realm/terrain/TerrainTile.cs` | `terrain` | yes |
| `common/terrain/TerrainTile.cs` | `terrain` | yes |
| `wServer/realm/JsonMap.cs` | `wServer.realm` | **no** |

The first three are **identical**, 57 entries:

```
None=0 Spawn=1 Realm_Portals=2 Store_1..Store_6=3..8 Vault=9 Loot=10 Defender=11
Hallway=12 Enemy=13 Hallway_1..3=14..16 Store_7..Store_9=17..19 Gifting_Chest=20
Store_10..Store_24=21..35 Item_Spawn_Point=36 Store_25..Store_40=37..52 Biome1..4=53..56
```

That is exactly our `crates/content/src/region.rs`. **The numbering agrees, and it agrees to the last
entry.**

The fourth, in `realm/JsonMap.cs`, has 20 entries and puts `Store_7..Store_9` before `Vault`. That
file `using db;` — a namespace that does not exist anywhere in this tree — and **is not listed in
`wServer.csproj`**, so it is never compiled. An earlier pass of this audit read that file, found the
disagreement, and wrote it up as a divergence. It is not one.

Two lessons worth keeping: **the same type name can exist four times in one solution**, and *compiled*
is a question the project file answers, not the directory listing.

The market's shop regions (`Store_9` -> Weapon through `Store_14` -> Other, see
[the marketplace page](27-marketplace.md)) and Nexus's portal placements (`Store_37`, `Store_39`) are
resolved against the live enum, so those ordinals are correct as written.

Decoding the `.hmap` files this server ships against our numbering produces a sensible distribution
(1,065 spawn tiles, 3 vault tiles, stores spread across the shop maps), which is independent
confirmation of the same conclusion.

`TossObject` and `Reproduce` both take a `TileRegion` parameter, and **no behaviour and no setpiece in
the C# database names a region at all**, so nothing depends on it either way.

---

## What has been read

Every file below was opened and read, not grepped.

| Area | Files |
| --- | --- |
| Behaviour engine | `Behavior`, `State`, `Transition`, `Cooldown`, `CycleBehavior` |
| Behaviours | all 75 in `logic/behaviors/` |
| Transitions | all 20 in `logic/transitions/` |
| Loot | `Loots`, `MobDrops`, `LootTemplates` |
| Damage and fame | `DamageCounter`, `FameCounter`, `ActivateBoost` |
| Entities | `Entity`, `Character`, `Enemy`, `Container`, `StaticObject`, `Portal`, `Decoy`, `Trap`, `Sign`, `GiftChest`, `OneWayContainer`, `Placeholder`, `GuildHallPortal` |
| Player | `Player`, `Player.UseItem`, `Player.Update`, `Player.Effects`, `Player.Leveling`, `Player.Chat`, `Player.Ground`, `Player.Trade`, `Player.KeepAlive`, `Player.Projectiles` |
| Stats | `StatsManager`, `BaseStatManager`, `BoostStatManager`, `Stats`, `XmlDescriptors` (the stat translation) |
| World | `World`, `Oryx`, `Realm`, `Vault`, `GuildHall`, `Davy`, `Sight`, `Collision`, `Inventory`, `ItemStacker`, `JsonMap`, `Wmap` |
| Shops | `Merchant`, `SellableObject`, `PlayerMerchant` |
| Setpieces | `SetPieces` and the placement table; `Pentaract` as a representative piece |
| Projectiles | `Projectile` |
| Handlers | `AcceptTradeHandler` |
| Verification | `Player.Verify`, `Player.AntiCheat`, `PositionTimeline` |
| Market | `Market`, `Player.Market` |
| Chat and connection | `ChatManager`, `ConnectManager`, `ConnectionQueue`, `RealmManager`, `PortalMonitor`, `ISControl`, `DbEvents`, `DbServerManager`, `Player.Networking` |
| Loops | `FLLogicTicker`, `NetworkTicker`, `LogicTicker`, `WorldTicker`, `TickPhases`, `SpatialStorage` |
| Commands | `Command`, all of `UnrankedCommands` and `RankedCommands` |
| World subclasses | all of `worlds/logic/`, plus `DynamicWorld` and `DungeonTemplates` |
| `common/` | `Database`, `DbModels`, `FameStats`, `XmlData`, `XmlDescriptors`, `Resources`, `AppSettings`, `WorldData`, `WeeklyQuest`, `Utils`, `Json2Wmap`, `TerrainTile` |

## What has not been read, and why that is defensible

**The 103 packet definitions and 51 handlers.** These are wire format and dispatch, not mechanics.
Coverage is established by the `handlers` census, which I verified this session against the real
directory listing: all 51 are accounted for, including the five hit-claim packets that are
deliberately answered "decided by the world". The mechanics *behind* the handlers are in the files
above.

**The 48 files of the `server/` HTTP tree.** Coverage is established by the `endpoints` census, which
I verified against the routes the C# registers: 37 real routes, all covered, with `/account/rp` an
alias of `resetPassword`.

**7 of the 29 files of `common/`.** The remaining ones are `NReader`/`NWriter` (byte-level
serialisation), `WeakDictionary`, `TimedLock`, `ISManager`/`ISDataTypes`/`InterServerChannel` (the
Redis pub/sub bus), `ConfigModels`, `DbStatus`, `PrivateMessages`, `ChangePassword`, `Ranks`,
`WorldMapExporter` and `Interfaces`. None carries game mechanics.

**37 of the 38 setpieces.** The placement table and one representative piece were read. Each of the
others is the same shape: an integer grid, a floor tile, and entities at marked cells. Fifteen are
already implemented and match the table exactly; the other 23 are the event pieces, and they are
blocked on the realm-events feature rather than on knowing what they draw.

If any of those four groups turns out to matter, the census that covers it is the thing to distrust
first — one of them was under-reporting for the life of the project before this audit.

## Dead code in the C# tree, so nobody ports it

| File | Why it is dead |
| --- | --- |
| `realm/JsonMap.cs` | not in `wServer.csproj`; `using db;` names a namespace that does not exist |
| `realm/LogicTicker.cs` | compiled, constructed by nothing; superseded by `FLLogicTicker` |
| `realm/SpatialStorage.cs` | compiled, constructed by nothing; the live index is `Collision.cs` |
| `worlds/DungeonTemplates.cs` | entirely inside a comment |
| `Database.UpdateCurrency(acc, type, trans)` | `throw new NotImplementedException()` |
| `ActivateEffect.DurationMS2`, `ObjectId2` | never assigned; see [the descriptors page](35-descriptors.md) |
