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

To run the real thing you need Redis, then the app server (`:8888`) and the world server (`:2050`),
both with `resourceFolder` pointing at `../Server-Side/XmlDatas`. Note that `wServer.json` currently
binds a public address and needs a local override.

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
  foreshortened. See `src/Render/WorldProjection.cs`.

## Known divergences from the AS3 client

Deliberate, and all of them fixes:

- `MarketResult` and `ServerFull` are decoded. The original never registered handlers and tore down
  the connection on receipt.
- `KeyInfoResponse` is decoded the way the server actually writes it — .NET's `BinaryWriter` string
  format, not the protocol's — which the original misparsed.
- The client's local damage formula floors at 15% while the server's floors at 25%. The server is
  authoritative; the client value is treated as display-only prediction.

## Out of scope

The in-game map editor, the sprite editor, the tutorial, and the Kongregate/Kabam/Steam account
paths are not ported.
