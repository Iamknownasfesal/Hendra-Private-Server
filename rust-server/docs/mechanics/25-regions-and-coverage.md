# Regions, and what remains unread

## Tile regions are numbered differently in the two servers

The C# `TileRegion` (in `realm/JsonMap.cs`) runs:

```
None=0 Spawn=1 Realm_Portals=2 Store_1..Store_9=3..11 Vault=12 Loot=13 Defender=14
Hallway=15 Hallway_1..3=16..18 Enemy=19
```

Ours (`crates/content/src/region.rs`) runs:

```
None=0 Spawn=1 RealmPortals=2 Store1..Store6=3..8 Vault=9 Loot=10 Defender=11
Hallway=12 Enemy=13 Hallway1..3=14..16 Store7..Store9=17..19 GiftingChest=20 Store10..=21..
```

**The two agree up to 8 and disagree from 9 onward.** Ours also has entries the C# server has no
name for — `GiftingChest`, `ItemSpawnPoint`, `Store10` through `Store21` — so it was derived from a
later fork's numbering, which is the same numbering the `.hmap` files this server ships were written
with. Decoding those maps produces a sensible distribution (1,065 spawn tiles, 3 vault tiles, stores
spread across the shop maps), so the pipeline is self-consistent.

The risk this leaves is narrow and worth writing down: **anything that names a region by the C#
ordinal would be wrong here.** `TossObject` and `Reproduce` both take a `TileRegion` parameter, so
a behaviour using region-targeted spawning would be affected.

Checked: **no behaviour and no setpiece in the C# database names a region at all**, so nothing is
currently affected. The parameter exists and is never used. If content ever starts using it, the
ordinal must be translated the same way the stat numbers are — see
[the stats page](08-stats.md), where exactly this class of mistake is live.

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

## What has not been read, and why that is defensible

**The 103 packet definitions and 51 handlers.** These are wire format and dispatch, not mechanics.
Coverage is established by the `handlers` census, which I verified this session against the real
directory listing: all 51 are accounted for, including the five hit-claim packets that are
deliberately answered "decided by the world". The mechanics *behind* the handlers are in the files
above.

**The 48 files of the `server/` HTTP tree.** Coverage is established by the `endpoints` census, which
I verified against the routes the C# registers: 37 real routes, all covered, with `/account/rp` an
alias of `resetPassword`.

**The 23 files of `common/`.** `Database.cs` was compared method by method earlier in the audit;
`XmlDescriptors.cs` was read for the stat translation, which is where its mechanics live. The rest is
serialisation.

**37 of the 38 setpieces.** The placement table and one representative piece were read. Each of the
others is the same shape: an integer grid, a floor tile, and entities at marked cells. Fifteen are
already implemented and match the table exactly; the other 23 are the event pieces, and they are
blocked on the realm-events feature rather than on knowing what they draw.

If any of those four groups turns out to matter, the census that covers it is the thing to distrust
first — one of them was under-reporting for the life of the project before this audit.
