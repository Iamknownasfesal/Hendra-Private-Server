# The reference itself

Every other page in this directory measures our server against `Server-Side/`. This page measures
`Server-Side/` against the tree as it was imported, because part of what those pages cite was
written by this project.

The baseline is **`94615c4`**, the last commit before this project's own work began. Its
`Server-Side/` tree is byte-identical to the 2020 import (`b726878` → `d8805dd` → `7863f3e`);
`94615c4` itself only touches the top-level README, and `git diff 7863f3e 94615c4 -- Server-Side/`
is empty. Everything dated 2026 is this project's.

Measured against it, `Server-Side/` outside `wServer/logic/db/` has **58 changed files,
+3,211 / −396 lines**, across 22 commits. `wServer/logic/db/` holds a further **61 files** that the
2020 tree does not contain at all.

## Extracting the pristine tree yourself

```
git archive 94615c4 Server-Side/ | tar -x -C <somewhere outside the repo>
```

Do not revert `Server-Side/` in place: the behaviour converter reads `wServer/logic/db/*.cs`, and
the pristine tree has none of them.

## Two facts that limit what "pristine" can mean

**The pristine tree does not compile.** `Server-Side/.gitignore:240` in the 2020 import reads
`wServer/logic/db/*.cs`, so the behaviour scripts were deliberately withheld from the public repo,
while `wServer.csproj` still lists 62 of them as `<Compile Include>`. A clean checkout fails with 62
`CS2001: Source file could not be found`. **There has never been a compilable reference tree in this
repository.** Any runnable "pristine" server is necessarily a hybrid, and this page's is too.

**The pristine tree does not run either.** `XmlDatas/xmls/client/EmbeddedData_RegionsCXML.dat` has
an unclosed `<Region type="0x3c" id="Biome3">` at line 136. `XmlData.LoadXmls` throws
`XmlException: The 'Region' start tag ... does not match the end tag of 'Regions'` before the world
server finishes booting. Commit `614e414`'s one-line `</Region>` is a genuine mechanical repair, not
an invention.

So the honest reconstruction is a **pristine-engine build**: 2020 engine source, plus the minimum
needed to compile and boot. What was added, and nothing else:

| Added to the pristine tree | Why | Category |
| --- | --- | --- |
| `wServer/logic/db/*.cs` (61) | csproj requires them; tree ships none | B |
| `wServer/logic/loot/LootTemplates.cs` | 15 of those 61 call it | A |
| `wServer/logic/behaviors/ScaleHP.cs` | one of them omits `maxAdditional` | A |
| `wServer/logic/BehaviorDb.cs` | `InitMany`, called by the imported scripts | A |
| the `</Region>` repair | otherwise the server cannot boot | repair |
| `wServer.csproj` `<Compile>` list rewritten | the 2020 list names files no fork here has | repair |

`Ported.cs` and `PortedTransitions.cs` are **not** needed — nothing under `logic/db/` references
them. They exist for the `.beh` converter, not for the C# server.

That build runs clean, and `strings wServer.exe` finds no `PositionTimeline`, `VaultState`,
`TickPhases` or `StressSpawn`. The `wServer.exe` serving port 2050 contains all four. **The
reference pair everyone has been measuring against is the contaminated build.**

### Reproducing it

The pristine-engine build lives under this session's scratchpad at `pristine/Server-Side`, built
with `xbuild /p:Configuration=Release Server-Side.sln` against the NuGet `packages/` directory the
earlier round already fetched. It runs from `prun/` on **world 2060, app 8899, Redis db 5** — its
own ports and its own keyspace, so it never touches the pair on 2050/8888 or the accounts in db 0:

```
cd <scratchpad>/prun && mono wServer.exe &   # world  2060
cd <scratchpad>/prun && mono server.exe  &   # app    8899
```

Registering there needs four Redis fields the 2020 server demands and the current one no longer
does — `nameChosen`, `verified`, `alpha` (the alpha gate `HelloHandler.cs:81`, removed by
`fc9d953`), and `rank` for `/spawn`:

```
redis-cli -n 5 hset account.1 nameChosen 1 verified 1 alpha 1 rank 100
```

## Categories

- **A — invention.** Written or modified by this project's sessions. Useless as a specification: a
  citation to it is a citation to us.
- **B — import.** Content taken from a *different* fork in August 2026. Not the 2020 original, but
  not ours either.
- **C — original.** Unchanged 2020 code. Everything under `Server-Side/` not listed below is C.
- **repair.** Minimal mechanical fix without which the tree cannot build or boot.

## A — the 58 changed files

`+add/−del` is against `94615c4`. **New** means the file does not exist in the pristine tree at all.

### Entirely new files (no original to compare against)

| File | Lines | Commit |
| --- | --- | --- |
| `wServer/realm/entities/player/Player.Verify.cs` | +543 | `3deb6b7`, `baf7d2c` |
| `wServer/realm/VaultState.cs` | +552 | `034dd17`, `363eb28`, `c423dea`, `f6715be` |
| `wServer/logic/behaviors/Ported.cs` | +419 | `bbaf029`, `9958a74` |
| `wServer/realm/PositionTimeline.cs` | +163 | `3deb6b7` |
| `wServer/realm/TickPhases.cs` | +119 | `f197d3e`, `9958a74` |
| `wServer/networking/packets/outgoing/VaultUpdate.cs` | +82 | `034dd17`, `363eb28` |
| `wServer/networking/packets/incoming/VaultMove.cs` | +76 | `034dd17`, `363eb28`, `f6715be` |
| `wServer/logic/loot/LootTemplates.cs` | +68 | `bbaf029` |
| `wServer/networking/handlers/VaultMoveHandler.cs` | +50 | `034dd17` |
| `wServer/networking/handlers/VaultBuyHandler.cs` | +42 | `034dd17` |
| `wServer/networking/packets/incoming/VaultBuy.cs` | +30 | `034dd17` |
| `wServer/logic/transitions/PortedTransitions.cs` | +18 | `bbaf029` |

### Modified files, largest first

| File | Δ | Pristine → now | What changed | Commit |
| --- | --- | --- | --- | --- |
| `wServer/realm/entities/Projectile.cs` | +384/−27 | 139 → 496 lines | server-side sweep, hit boxes, deferred-hit accounting | `3deb6b7`, `d354838`, `9958a74` |
| `wServer/realm/worlds/logic/Vault.cs` | +109/−106 | — | vault redesign | `034dd17`, `363eb28`, `614e414`, `baf7d2c` |
| `wServer/realm/RealmManager.cs` | +85/−0 | — | `StressSpawn` measurement hook; `VaultState.Release` | `65125de`, `034dd17` |
| `wServer/wServer.csproj` | +55/−45 | — | the new files above | 4 commits |
| `wServer/realm/worlds/World.cs` | +47/−19 | 679 → 707 | parallel-tick support, vault hooks | `f197d3e`, `9958a74` |
| `wServer/realm/entities/player/Player.Ground.cs` | +47/−45 | — | ground damage cadence | `3deb6b7` |
| `wServer/realm/FLLogicTicker.cs` | +43/−6 | 138 → 175 | `Parallel.ForEach` over worlds, `TickPhases` probes | `f197d3e`, `9958a74` |
| `common/resources/XmlDescriptors.cs` | +33/−2 | — | item stat remap | `c704057` |
| `wServer/realm/entities/player/Player.cs` | +31/−10 | 1137 → 1158 | verification hooks | `3deb6b7`, `d354838`, `9958a74` |
| `common/resources/AppSettings.cs` | +29/−0 | — | vault settings | `c423dea`, `034dd17` |
| `common/log4net.config` | +20/−0 | — | logging for the probes | `f197d3e`, `3deb6b7` |
| `wServer/networking/handlers/MoveHandler.cs` | +16/−7 | — | position timeline recording | `3deb6b7`, `d186199` |
| `wServer/realm/Utils.cs` | +16/−4 | — | helpers for the above | `f197d3e`, `9958a74` |
| `wServer/logic/BehaviorDb.cs` | +15/−0 | — | `InitMany` | `9e08c1c` |
| `XmlDatas/xmls/client/EmbeddedData_PlayersCXML.dat` | +14/−14 | — | **starting equipment** (see below) | `cf4c52b` |
| `wServer/realm/BoostStatManager.cs` | +12/−0 | — | stat remap support | `c704057` |
| `wServer/realm/entities/player/Player.UseItem.cs` | +10/−10 | — | mostly formatting | `9958a74` |
| `wServer/networking/handlers/LaunchRaidHandler.cs` | +8/−8 | — | formatting | `9958a74` |
| `wServer/networking/packets/PacketIds.cs` | +8/−1 | — | vault packet ids | `034dd17` |
| `wServer/networking/handlers/PlayerShootHandler.cs` | +8/−0 | 54 → 62 | server-authoritative shot record | `3deb6b7` |
| `wServer/realm/Stats.cs` | +7/−0 | — | vault stats | `034dd17` |
| `wServer/networking/handlers/EnemyHitHandler.cs` | +6/−0 | — | hit-claim handling | `3deb6b7`, `baf7d2c` |
| `wServer/logic/behaviors/ScaleHP.cs` | +6/−3 | — | default arg + log level | `bbaf029` |
| `wServer/networking/server/CommHandler.cs` | +6/−1 | — | packet length guard | `c23a48a` |
| `wServer/realm/Entity.cs` | +5/−1 | — | vault hook | `034dd17` |
| `wServer/networking/Client.cs` | +5/−1 | — | stat remap | `c704057` |
| `wServer/realm/commands/RankedCommands.cs` | +4/−4 | — | formatting | `9958a74` |
| `server/XmlModels.cs` | +4/−1 | — | **`<CreateTime>`** (see below) | `f377e51` |
| `wServer/realm/Oryx.cs` | +2/−2 | — | formatting | `9958a74` |
| `XmlDatas/xmls/client/EmbeddedData_RegionsCXML.dat` | +1/−0 | — | unclosed tag — **repair** | `614e414` |
| `XmlDatas/worlds/Vault.jm` | +1/−1 | — | vault map | `363eb28` |
| 8 × one-line formatting-only | +1/−1 each | — | `ChangeMusic`, `ChangeMusicOnDeath`, `DropPortalOnDeath`, `Grenade`, `InvisiToss`, `TossObject`, `ISControl`, `AlertNoticeHandler`, `UnrankedCommands`, `Player.Leveling` | `9958a74` |
| `wServer/networking/handlers/HelloHandler.cs` | +0/−6 | — | alpha gate removed | `fc9d953` |
| `wServer/realm/entities/vendors/ClosedVaultChest.cs` | +0/−57 | **deleted** | removed for the vault redesign | `034dd17` |
| `.gitignore`, `server/server.json`, `wServer/wServer.json` | small | — | config / tracking | `1447cd2`, `4c5dbeb` |

## B — the behaviour scripts

All 61 files under `wServer/logic/db/*.cs` are third-party content, tracked in one commit
(`1447cd2`, 2026-08-13) after arriving on disk untracked earlier that month. They are demonstrably
**not** this fork's own scripts: the pristine `wServer.csproj` names 62 db files, the import
supplies 61, and only 33 names are common to both. The 2020 tree expected
`BehaviorDb.LostHalls.cs`, `BehaviorDb.Void.cs`, `BehaviorDb.MarbleColossus.cs`,
`BehaviorDb.WineCellar.cs`, `BehaviorDb.YarrakDragon.cs` and 24 others that the import does not
contain; the import adds `BehaviorDb.Avatar.cs`, `BehaviorDb.Lab.cs`, `BehaviorDb.Sphinx.cs` and 25
more the 2020 csproj never mentions.

They are usable as *content* and are what the Rust server's behaviours were converted from. They are
not evidence about how the game this repository forked behaved.

## Re-tests against the pristine build

Both banked results below were originally measured against the contaminated binary. Re-measured
against the pristine-engine build (world `2060`, app `8899`, Redis db 5), with the same probe.

### Tick quantisation — the banked result holds

| Fixture | Written cooldown | Banked (contaminated) | Pristine build |
| --- | --- | --- | --- |
| `Turret Attack` (`BehaviorDb.Lab.cs:431`) | `coolDown: 1000` | 1337.2 ms | **1335.5 ms mean, 1330.7 ms median** (98 volleys) |
| `Ink Bubble` (`BehaviorDb.OceanTrench.cs:379`) | `coolDown: 100` | 333.8 ms | **332.3 ms median** (185 volleys, 165 in the 330 ms bin) |

**The tick-rate finding is genuine and survives.** It is a property of the 2020 loop, not of this
project's changes to it. The quantisation is to whole `MsPT` units — `MsPT = 1000 / TPS = 166` ms at
the configured `tps: 6` — and the observed periods are integer multiples of it: 100 ms costs two
quanta, 1000 ms costs eight.

The mechanism is in `FLLogicTicker.TickWorlds1`, unchanged from 2020: `RealmTime` is a *class*, and
the 200 ms world-tick block overwrites `t.TickDelta` and `t.ElaspedMsDelta` on the shared instance
before returning, so `TickLoop`'s `loopTime += elapsed − t.ElaspedMsDelta` reconciles against a
value the logic tick never consumed. `TickLogic` therefore receives a repeating `0, 166, 166`
pattern rather than a steady 166.

One caveat worth recording: Mono's `ManualResetEvent.WaitOne(166)` on this host returns after
169.65 ms on average (measured over 30 calls). That ~2 % overshoot is environment-dependent, but it
is second-order — the integer quantisation above is deterministic and accounts for the whole of the
1000 ms → 1330 ms and 100 ms → 332 ms effects.

### `<CreateTime>` in `/char/list` — we should stop sending it

Registered a real account against the pristine app server, created a character through the game
protocol, and read `/char/list` (POST; the reference 404s on GET):

```
<Char id="1">
  ...
  <Dead>false</Dead>
  <HasBackpack>0</HasBackpack>
</Char>
```

**The pristine server does not send `<CreateTime>`.** The field is not missing data — pristine
`XmlModels.cs:389` declares the property and `:419` populates it from `character.CreateTime`. Only
the `XElement` was absent. Commit `f377e51` added it and the built `server.exe` on port 8888 now
emits it, which is how it reached a later brief as "original behaviour to match".

The client does not need it. `SavedCharacter.bornOn()`
(`Client-Side/src/com/company/assembleegameclient/appengine/SavedCharacter.as:191-198`) guards with
`if(!this.charXML_.hasOwnProperty("CreateTime")) return "Unknown";` — absence is a designed-for case,
not a parse failure. Its only caller is `FameContentPopupMediator.as:100`, which shows the string in
the fame popup.

**Verdict: remove it.** The C# is the specification, pristine does not send it, and the client is
explicitly written to cope without it. Our server currently emits it from
`crates/app/src/legacy.rs:812`.

### Starting equipment — the original ships classes unarmed

`EmbeddedData_PlayersCXML.dat` in the pristine tree gives every class
`<Equipment>-1, -1, -1, ...</Equipment>`. Commit `cf4c52b` armed them (`2580, 2646, 2680` for the
first class, and so on). The pristine `/char/list` above confirms it end-to-end: a freshly created
character comes back with 24 × `-1`.

Any claim about what a new character starts with, measured against the running reference, is
measuring this project's change.

## Banked claims that were measured against our own code

The **docs** are largely self-aware — `mechanics/README.md`, `26-verification.md`, `34-persistence.md`,
`14-world-subclasses.md`, `05-projectiles.md`, `30-the-server-loop.md`, `43-the-behaviour-scripts.md`,
`02-behaviours.md` and `03-transitions.md` all flag their subject as this project's own C#. The
problem is in **Rust source comments**, which cite line numbers with no caveat at all.

Highest-stakes first:

1. **`crates/server/src/vault.rs` — the whole module.** 17 line-precise citations to `VaultState.cs`
   (`:6`, `:10`, `:12`, `:36`, `:48`, `:56`, `:61`, `:133`, `:140`, `:222`, `:276`, `:293`, `:316`,
   `:392`, `:463`, `:528`, `:576`), opening with "This is `realm/VaultState.cs`, with the same
   refusals in the same order." `VaultState.cs` is 552 lines of this project's own C#. There is no
   original to be in the same order as. Also `crates/net/src/message.rs:564`, `:577`, `:1298`;
   `crates/store/src/model.rs:1412`; `crates/server/src/session.rs:286`, `:1758`;
   `crates/app/tests/api.rs:2412`; `crates/store/tests/durability.rs:4000`.
   **Resolved** — all 25, plus nine more found while doing it. See
   [the vault section below](#the-vault--settled-against-the-pristine-source).
2. **Four movement / anti-cheat constants justified by `Player.Verify`.**
   `crates/sim/src/world.rs:1692` and `:2680` (`MOVE_GRACE_MS = 3_000`),
   `crates/server/src/world_task.rs:38` and `:1656` (`MOST_ALLOWANCE_MS`, "`Player.Verify.Keep`").
   `Player.Verify.cs` is 543 lines this project wrote; the 2020 tree has only a much smaller
   `Player.AntiCheat.cs`. These numbers are ours quoted back at us.
3. **A timing claim on `Ported.cs:78`** — `crates/sim/src/world.rs:2811` and
   `crates/net/src/message.rs:1411`. `Ported.cs` is +419 lines, entirely this project's.
4. **`docs/audit/10-what-to-fix.md:87` / `PLAN.md:1228`**, the world-wide 3 % tier-1 potion drop,
   already acted on and marked "Fixed". This one **stands**: `World.cs:27` is
   `public Loot WorldLoot = new Loot(` in the pristine tree at the same line number.
5. **`docs/mechanics/25-regions-and-coverage.md:55`, `:65`, `:68`** list `Player.Verify`,
   `PositionTimeline`, `TickPhases` and `LootTemplates` as read reference material with no caveat,
   unlike every other page that touches them.
6. **`docs/audit/03-sim-loop.md:64-67`**, the `0.1 %` → `96 %` chunk-gating measurement. Already
   caveated at `:70`, and the caveat is right: the figures come from `TickPhases` and the
   `HENDRA_STRESS` hook, both this project's, on a binary running a per-bullet sweep the 2020 server
   never ran.

### Line numbers have drifted, mostly without the substance moving

Every line-number citation in `crates/` is indexed against the contaminated tree. For the 20
substantially modified files the numbers no longer locate the pristine code. Spot-checked:

| Citation | Pristine reality |
| --- | --- |
| `Projectile.cs:472` (`ForceHit`) | `ForceHit` is **original**, at pristine `:107-115`. Pristine file is 139 lines; current is 496. Substance holds, number is 3.4× off. |
| `World.cs:147` (`Blocking = 0`) — `docs/audit/06-net.md:40` | Original, at pristine `:132`. Substance holds. |
| `FLLogicTicker.cs:30` (`MsPT = 1000 / TPS`) | Original, at pristine `:28`. |
| `FLLogicTicker.cs:149-156` (200 ms world tick) | Original, at pristine `:112-121`. |
| `MoveHandler.cs:41` (`CheckLabConditions`) | Same line in both. Holds. |
| `World.cs:27` (`WorldLoot`) | Same line in both. Holds. |
| `PlayerShootHandler.cs:46` (`Inventory[0]`) | Drifted; pristine file is 54 lines, current 62. |

So the drift is mostly cosmetic — but it means **no line-number citation into a category-A file can
be trusted without re-checking it against the pristine tree**, and a reader who checks a citation
against the working tree will be confirming it against our own edit.

## The doctrine that closed the loop

`docs/mechanics/README.md:8-11` currently says the target to match is the reference binary "built
from the tree **as it stands** — so where this project has changed the C#, the changed behaviour is
the correct one to match."

That is circular, and it is the mechanism by which `<CreateTime>` became a requirement: a builder
invented the field, committed it into the C#, it was compiled into the running `server.exe`, and a
later brief handed it to the next builder as original behaviour. The rule should be the other way
round — a category-A file is evidence about nothing, and where we need behaviour it does not
specify, we should say we are choosing it rather than say we are matching it.

## The vault — settled against the pristine source

This section used to read "what is still untrustworthy". It has been worked through: every
`VaultState.cs` citation in `crates/` is gone, and so is every citation into the rewritten
`Vault.cs`, the deleted `ClosedVaultChest.cs`'s replacement, `VaultMoveHandler.cs`,
`VaultBuyHandler.cs` and the invented `AppSettings.MaxVaultChests` / `FreeVaultChests`.

**What the 2020 vault was**, from `git show 94615c4:Server-Side/wServer/realm/worlds/logic/Vault.cs`
and `…/realm/entities/vendors/ClosedVaultChest.cs`:

- a private `World` per client (`Vault.cs:23-28`), refused to anyone else — except accounts 1 and 17,
  a hardcoded backdoor (`:47-50`);
- one eight-slot `Container` entity of type `0x0504` per owned chest, on the `TileRegion.Vault` tiles
  nearest the spawn (`:96-107`, slots at `:39`); every leftover tile a buyable `ClosedVaultChest`
  (`:108-113`);
- **80** `Vault` tiles in the 2020 `Vault.jm`, which is the real ceiling — nothing in the C# writes
  one down (`Database.CreateChest` increments without bound, `common/Database.cs:905-911`);
- **one** free chest for a new account (`XmlDatas/data/init.xml:37`), another for **400 fame**
  (`:7`, `ClosedVaultChest.cs:14-16`), answered `"Vault chest purchased!"` (`:47-51`);
- moves were ordinary `InvSwap` between two entities within a tile
  (`networking/handlers/InvSwapHandler.cs:36-137`), with the backpack rule at `:177` and the
  two-way slot audit at `:174-180` — no vault packet exists;
- gifts dealt eight at a time into `GiftChest` entities on the four `Gifting_Chest` tiles
  (`Vault.cs:115-130`).

Re-measured live on the pristine app server (8899, redis db 5): a freshly registered account has
`vaultCount 1`, and `Enumerable.Range(0, acc.VaultCount - 1)` (`server/XmlModels.cs:252`) is real —
`vaultCount` 1 answers `<Vault />` and `vaultCount` 3 answers two `<Chest>` elements. Our
`/char/list` now matches that byte for byte.

**Still flagged as this project's invention, kept but not defended as specification**: the vault
panel itself and its three packets, the version counter, the chest-buy `believed`-count check, the
gift-claim write order, and the lightning the access object throws (`Vault.Crackle`, which the 2020
vault has nothing resembling). Each is now labelled as ours at its definition.
