# The server's content

Everything the running server reads. `hendra-server` and `hendra-app` default every content path
into this directory, so both start with no flags at all.

`Server-Side/` and `Client-Side/` are the specification this project is measured against, and the
parity tests read them to prove we match. Nothing here reads them while it runs. That separation is
the point: our vault can be a different room from the original's without the file that records what
the original shipped being the file that gets edited. `crates/server/src/main.rs` carries the test
that fails if a runtime default points back into either tree.

`worlds/`, `xmls/`, `data/` and `textures/` began as copies of `Server-Side/XmlDatas`; `behaviours/`
and `maps/` are generated. All six are ours to edit. Regenerate the two that are generated with:

```
cargo run --release -p hendra-behavior --example convert      -- content/behaviours
cargo run --release -p hendra-content  --example convert_maps -- content/worlds content/maps
cargo run --release -p hendra-content  --example map_index    -- content/maps > content/maps/INDEX.txt
```

## worlds/

42 world definitions (`.jw`) and the 96 maps they name (`.jm` and `.wmap`). This is what the server
loads a world from, and the legacy map formats are text or near enough, so this is where a room's
layout is changed by hand.

Our own changes to what the original shipped:

- `Vault.jm` — one large chest in the middle of the room instead of eighty around the walls, and no
  gifting chests, because our vault is one panel listing every chest an account owns.
- `WineCellar.jw` — `portals` says `0x242`, the Wine Cellar Portal. The original ships `0x63f4`,
  which no content file defines: `Entity.Resolve` indexes the type table directly
  (`wServer/realm/Entity.cs:589`) and throws on it, so the Wine Cellar Incantation — the only item
  in the game with the `UnlockPortal` effect — can never open the door. Corrected here rather than
  in the specification, which keeps its `0x63f4`.

## xmls/

77 files of object, tile, region and player definitions, as the original keeps them: XML under a
`.dat` extension. Two carry changes of ours, both made before this directory existed and both
copied across with it — starting equipment for every class in `EmbeddedData_PlayersCXML.dat`, and a
closing `</Region>` tag in `EmbeddedData_RegionsCXML.dat` that the original is missing.

## data/ and textures/

What the app server hands back unchanged: `init.xml`, the news, the login calendar, the quest list,
the translation tables, and the artwork the client fetches by name.

## music/

`INDEX.txt` names the 66 tracks `/music` accepts, one per line. The mp3s themselves are 205MB and
are fetched by the client from a web root rather than from this server, so only their names are
kept here. Point `--music` at a directory of mp3s and their stems are read instead.

## behaviours/

58 files, one per C# behaviour database file, holding 748 enemies. Readable text — this is the
source of truth for what an enemy does, and it is meant to be edited by hand once the C# is retired.

Check coverage with `cargo run --release -p hendra-behavior --example gaps`, which reports the
percentage of the content's behaviour and transition uses the runtime implements. Both are at 100%.

Editing these by hand is what `tools/zed-beh` is for: highlighting, an outline, tooltips saying what
each behaviour does with its arguments, and following a name into whichever file defines it. It also
reports arguments written under a name the compiler does not read — there are 475 of those here, and
nothing else says so. The same report without an editor:

```
cargo run --release -p hendra-beh-lsp -- --check content/behaviours
```

## maps/

96 maps in HMAP format, 13.8 million squares, 44% smaller than the legacy `.jm` and `.wmap` in
`worlds/` they came from. Binary, so `INDEX.txt` lists what each one contains.

Two maps exist under both source formats — `snakepit` and `tomb`. The `.jm` versions keep the plain
name because that is what the world files reference; the `.wmap` versions are written alongside as
`<name>.wmap.hmap` rather than silently overwriting them.
