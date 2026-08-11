# Hendra Godot client

A Godot 4.7 / C# client for the server in `../Server-Side`, replacing the Flash/ActionScript client
in `../Client-Side`.

## Getting it running

**1. Extract the assets.** They are generated from the AS3 client and the server's data, and are
deliberately not committed — the repository already contains them once.

```sh
python3 tools/extract_assets.py
```

This reads the Flash `[Embed]` stubs and writes `assets/sheets`, `assets/models`, `assets/xml` and
`assets/manifest.json`. It has no dependencies beyond the standard library, is idempotent, and only
rewrites files whose contents changed so Godot does not needlessly re-import. Re-run it after
changing any art or XML on either side.

**2. Build.**

```sh
dotnet build HendraClient.sln
```

Needs the .NET 8 SDK. Opening the project in Godot 4.7 (the .NET build — plain Godot cannot load C#)
will also build it.

**3. Test.**

```sh
dotnet test tests/Hendra.Tests
```

## Layout

```
tools/extract_assets.py   asset extraction, run before first build
assets/                   generated, gitignored
scenes/                   Godot scenes
src/
  Core/       clock, synced PRNG          -- no engine dependency
  Data/       wire structs, stats, conditions
  Net/        framing, RC4, RSA, packets, session
  Resources/  game XML parsing
  Render/     projection, draw lists, camera
  World/      simulation
  Assets/     sprite sheets and animation
  App/        Godot entry points
tests/        xunit; runs without the engine
```

`Core`, `Data`, `Net`, `Resources` and `World/Movement` have no Godot dependency, which is what lets
the tests run as plain `dotnet test` with no engine and no editor.

## Testing against the server

The tests are parity tests where it matters: the test project compiles `wServer/wRandom.cs`,
`common/NReader.cs` and `common/NWriter.cs` **directly out of the server tree**, so they check
agreement with the code the server actually runs rather than with a second copy of somebody's
reading of it.

Some things no unit test can reach — acknowledgement conservation, the handshake, the continuous
RC4 streams — because they only prove themselves against a live server, and getting them wrong
produces a connection that silently stops rather than an error. `tools/soak` connects with the real
session code and reports how long it survives:

```sh
dotnet run --project tools/soak -- --seconds 300 --char 1
```

The client itself can be driven unattended, which is how the renderer gets checked against a real
world. `--say` sends one line — and slash commands go down the same pipe as chat, so it reaches
anything the server only does on request:

```sh
godot-mono --path . -- --host 127.0.0.1 --guid you@example.com --password pw --char 1 \
    --autofire --say "/spawn 20 Sheep" --screenshot /tmp/shot.png --screenshot-after 12 \
    --quit-after-screenshot
```

`--use-ability` fires the equipped ability on a loop, the same way `--autofire` holds the trigger,
and `--walk` walks in a circle. Two clients, one of them walking, is how remote-entity movement gets
checked — on a server whose monsters have no behaviours, another player is the only thing in the
world that ever changes position.
Repeat `--say` for a script; the lines go out a couple of seconds apart and survive a change of
world, which some of them need — you cannot die in the Nexus, so checking the death screen takes
`--say /realm --say "/killPlayer <name>"`.

Standing up the server locally takes a few steps, none of them obvious:

1. It targets .NET Framework 4.6.1 with old-style project files, so it needs Mono
   (`brew install mono`) plus `nuget.exe` under Mono to restore. Homebrew's Mono no longer ships
   `msbuild`, but the bundled `xbuild` builds it.
2. `wServer.csproj` references 62 files under `logic/db` that are not in the repository — the
   server's own README says behaviours were removed. `BehaviorDb` finds them by reflection, so
   deleting those `<Compile>` entries builds cleanly, just with fewer behaviours.
3. **Monsters will not move.** `wServer/logic/db` is empty and `wServer.csproj` references 62 files
   from it that are not in the repository. That directory is the behaviour database — every
   monster's AI — and the server's own README says it was removed because it is tied to the server
   assets. Nothing in the client can substitute for it: the client moves an entity when a tick says
   the entity moved, and without behaviours the server keeps sending the same position. See
   `tests/Hendra.Tests/EntityMotionTests.cs`, which pins that down from this side.
4. `xmls/client/EmbeddedData_RegionsCXML.dat` is malformed: the `Biome3` region is missing its
   closing tag, and the server throws on startup parsing it.
5. Both `server.json` and `wServer.json` ship bound to a public address, and expect Redis with
   `requirepass alphaversionone`.
6. A fresh account needs `nameChosen` and `alpha` set before the world server will admit it:
   `redis-cli hset account.1 nameChosen 1` and likewise `alpha 1`.
7. The models under `assets/models` are read as raw Wavefront text at runtime rather than through
   Godot's importer, so a packaged export needs `*.obj` in its include filter. Running from source,
   as below, needs nothing.
8. Sound is served, not shipped: the client fetches `/sfx/<name>.mp3` and `/music/<name>.mp3` as
   static files, so `XmlDatas/web/sfx` and `XmlDatas/web/music` have to be present in whatever
   resource folder the app server was pointed at. Both are read into memory at startup, and the
   music alone is 205 MB. The client says how many interface sounds it got — `[audio] 9 of 9` —
   which is the quickest way to tell a silent client from a silent server.

## Notes for anyone reading the code

A few things about the protocol are surprising enough to be worth knowing before you change
anything:

- **RC4 is one continuous stream per direction**, never re-keyed. A single malformed frame
  desynchronises it permanently, and the server discards frames it cannot parse *silently* — so the
  symptom is a connection that simply stops working, with nothing in any log.
- **Acknowledgements are counted exactly.** One `Move` per `NewTick` echoing its tick id, one
  `UpdateAck` per `Update`, one `GotoAck` per `Goto` — including the `Goto`s broadcast when *other*
  players teleport. Sending too many disconnects you; sending too few times out.
- **`Hello.BuildVersion` must be exactly `"Alpha v01"`.** On a mismatch the server sends nothing at
  all and leaves the socket open, so it looks like a hang rather than a rejection.
- **Damage prediction depends on a shared PRNG** seeded from `MapInfo`, stepped in the same order on
  both sides.
- **The camera is an oblique projection, not a perspective one.** The ground plane is not
  foreshortened. See `src/Render/WorldProjection.cs`. One consequence: the models in
  `src/Render/ModelDrawList.cs` are rebuilt every frame, because the projection folds height into
  the ground plane along the camera's up vector — a mesh built once shears the wrong way as soon as
  the camera turns. `--camera-angle` exists to check exactly that.

## Known divergences from the AS3 client

Deliberate, and all of them fixes:

- `MarketResult` and `ServerFull` are decoded. The original never registered handlers and tore down
  the connection on receipt.
- `KeyInfoResponse` is decoded the way the server actually writes it — .NET's `BinaryWriter` string
  format, not the protocol's — which the original misparsed.
- The client's local damage formula floors at 15% while the server's floors at 25%. The server is
  authoritative; the client value is treated as display-only prediction.
- **Camera shake stops.** The original's Jitter effect sets a flag that has no other assignment
  anywhere in the client, so one earthquake left the camera shaking until the session ended.
- **A monster's spray colours are sampled once.** The original meant to cache them — there is a
  dictionary keyed by object type, read on the way in — but nothing ever writes to it, so a monster
  under fire rescanned its own texture several times a second.
- **Trail particles are shed on a fixed interval**, not one per rendered frame. The original's
  effects were three times denser on a fast machine than a slow one.
- **An ability aimed too far is pulled back into range** rather than cast into nothing. The server
  checks the distance *after* taking the magic, so the original's unclamped aim meant a player who
  clicked across a wide screen paid for a cast that never happened, with no message.
- **Other players' shots are drawn but inert.** They exist on the server, where their owner's client
  reports what they hit, so joining in would double the damage reported for them.
- **Remote textures are actually fetched.** The reference client has that path commented out and
  substitutes a placeholder box for every object that uses one; here they are downloaded at startup
  and only fall back to the same box when the server has no artwork under the id.
- **Animated terrain wraps inside its own tile.** The original could slide a texture coordinate
  freely because every tile was its own bitmap with repeat switched on. Here tiles share a sheet, so
  a sliding coordinate walks into whatever sprite sits next door; `shaders/ground.gdshader` wraps it
  explicitly. Fixed and random tile offsets, which need the same wrapping, now work too.
- **Soft-edged sprites keep their soft edges.** The half-alpha test that decides where an outline
  goes was being applied to everything, so shadows and glows were cut off hard at the radius where
  their alpha crossed a half.

## What works

Sign in, create or pick a character, and play: movement with the original's collision rules,
shooting with server-verified timing, abilities, projectiles, damage, three-dimensional scenery,
loot containers, the vault,
merchants, portals between worlds, chat, trading, a nearby-players list, a minimap, the HUD, the
visual effects --
every `ShowEffect` kind, the spray a struck monster throws off, and the camera shake -- sound and
music, the quest arrow, an options panel that remembers itself between sessions, and, when it ends,
the fame tally from `/char/fame`.

## What is not done yet

- The guild and party panels. The packets are all implemented and their results reach the chat log;
  what is missing is the UI to drive them.
- Pets, the market, the quest *log*, and daily rewards. The objective arrow works; what is missing
  is the panel listing what the quest is.
- The charging aura on a Rising Fury enemy is emitted around the enemy rather than sampled over its
  sprite, which is what the original did. Sampling would mean reading the texture back per frame.
- Composite terrain blending, the third of the three blend schemes. Seven ground types use it; they
  fall back to plain artwork rather than being blended wrongly.
- Rebinding keys. The options panel lists what the keys do but cannot change them.

## Out of scope

The in-game map editor, the sprite editor, the tutorial, and the Kongregate/Kabam/Steam account
paths are not ported.
