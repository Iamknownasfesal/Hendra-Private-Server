# How the content is loaded

Read from `common/resources/XmlData.cs`, `Resources.cs`, `WorldData.cs`, `AppSettings.cs`.

## Two passes over the XML, and the order matters

```
LoadXmls(basePath, "*.xml")   additions this server invented
ZippedXmls = ZipGameXmls()    only the additions, compressed for the client
LoadXmls(basePath, "*.dat")   the xmls the client already embeds
```

The split is by **file extension**, and the comment calls it a hack job: `.xml` means "the client does
not have this, send it", `.dat` means "the client already has it". `ZippedXmls` is snapshotted between
the two passes, which is the only thing making the distinction work.

Both passes recurse the whole directory tree and process `//Object`, `//Ground` and `//EquipmentSet`
in that order per file.

## First wins for the name, last wins for the descriptor

```csharp
if (type2id_obj.ContainsKey(type))
    log.Warn("... same ID ...");
else { type2id_obj[type] = id; type2elem_obj[type] = elem; }
...
items[type] = new Item(type, elem);     // outside the else
```

On a duplicated object type, the **name mapping keeps the first** definition and the **descriptor takes
the last**. So a redefined item answers to the original's name and behaves like the redefinition.
Warned about, not refused.

`DisplayIdToObjectType` is written unconditionally, so display-name collisions are silent — the last
one wins with no warning at all. A `DisplayId` starting with `{` (a client localisation key) is
discarded and the object id is used instead.

An object with no `type` attribute is logged and skipped; an object with no `Class` element is skipped
without a word.

## The class switch decides which table an object lands in

| `Class` | Table |
| --- | --- |
| `Equipment`, `Dye` | `Items` |
| `Skin` | `Skins` |
| `Player` | `Classes` **and** `ObjectDescs` |
| `Portal` | `Portals` **and** `ObjectDescs` |
| `Merchant`, `GuildMerchant` | `Merchants` |
| anything else | `ObjectDescs` |

**Items and merchants are not in `ObjectDescs`.** Anything that looks up an object descriptor by type
therefore fails for an item, which is why `/spawn` filters on `ObjectDescs.ContainsKey`.

`SkinDesc.FromElem` may return null, and that null is skipped — but with `skins.Add`, so a **duplicate
skin type throws** rather than warning like everything else.

## `SlotType2ItemType` is built from whichever Player class is loaded last

```csharp
case "Player":
    slotType2ItemType[pDesc.SlotTypes[0]] = ItemType.Weapon;
    ... [1] Ability, [2] Armor, [3] Ring, [4] Amulet, [5] Belt, [6] Wings, [7] ComingSoon
```

Every player class overwrites the same dictionary. It works because the mapping is keyed by the
*slot type number*, and different classes have different weapon slot types — a wizard's staff slot and
a knight's sword slot are different numbers that both map to `Weapon`. So the dictionary accumulates
across classes rather than being replaced.

But it accumulates **only for slots a class actually has**, and only for the eight positions listed.
The market's `GetItemType` and every "can this go in that slot" question runs through it.

Also note `new PlayerDesc(type, elem)` is constructed **twice** for every player class — once for
`pDesc` and again for `classes[type]` — so the object whose slot types seeded the dictionary is not
the object stored.

## Worlds

Each `.jw` is JSON deserialised into a `ProtoWorld`:

```
name sbName id difficulty background isLimbo restrictTp showDisplays
persist blocking setpiece portals[] maps[] music[] wmap[][]
```

If `maps` is absent, the loader reads the sibling `.jm` (same path, last character changed from `w` to
`m`) and converts it. Otherwise each entry of `maps` is loaded: `.wmap` files raw, everything else
through `Json2Wmap`.

`worlds.Add(world.name, world)` — **two worlds with the same name throw at startup.** The name is also
the key `DynamicWorld` matches class names against, so the name is doing three jobs: file identity,
class binding, and the id used by `/quake` and `AddWorld`.

## Settings, and the defaults that are not in the file

`AppSettings` reads `data/init.xml`, and `GetIntValue` returns **0** for a missing element rather than
failing. Four values have real fallbacks applied afterwards:

```
InventorySize     0 -> 24
MaxVaultChests   <=0 -> 40
FreeVaultChests  <=0 -> 4, then clamped to MaxVaultChests
```

Everything else — `VaultChestCost`, `CharacterSlotCost`, `MaxStackablePotions`, the potion cooldowns —
silently becomes 0 if the element is missing. **A missing `VaultChestCost` makes chests free**, and
nothing says so.

`PotionPurchaseCosts` is read with `//cost` XPath, which is document-rooted rather than relative, so it
would collect `cost` elements from anywhere in the file, not only inside `PotionPurchaseCosts`.

`MaxVaultChests` and `FreeVaultChests` are this project's additions; the rest is original.

## Other resources

- **`FilterList`** is one regex per line of `data/filterList.txt`, compiled at startup with
  `IgnoreCase`. A malformed line throws during load.
- **Textures** are fetched from `realmofthemadgod.appspot.com` (or the testing host) when not present
  locally, then **written to disk** so the fetch happens once. A failed fetch is warned about and the
  texture is skipped. This is a synchronous `WebClient` call at startup: **no network, no server**,
  unless every texture is already cached.
- **Music** is the set of `.mp3` filenames under `web/music`, and that list is what `/music` validates
  against.
- `Resources(path, wServer: true)` loads **only** settings, game data, the filter list, worlds and
  music. The web files, languages, textures, quests and role ranks are the account server's business.
  So the same class is two different loaders depending on one boolean.

## What this server does differently

Our loader is in `crates/content`, reads the same XML, and reports what it could not handle rather
than warning into a log nobody reads. The things worth checking against this:

- **Duplicate object types: first wins for the name, last wins for the descriptor.** If we resolve
  both from one map we will differ on any file that redefines something.
- **Items, merchants and skins are not object descriptors.** Code that treats one table as universal
  behaves differently on each.
- **`SlotType2ItemType` accumulates across every player class**, keyed by slot type number. Building
  it from one class gives a map that is missing most slots.
- **A missing setting is 0, not an error.** Ours should refuse to start rather than silently price
  something at zero.
- The world name is the file key, the class binding and the lookup key all at once. Ours should not
  copy that.
