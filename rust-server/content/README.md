# Converted content

Generated, not hand-written. Regenerate with:

```
cargo run --release -p hendra-behavior --example convert     -- content/behaviours
cargo run --release -p hendra-content  --example convert_maps -- ../Server-Side/XmlDatas/worlds content/maps
cargo run --release -p hendra-content  --example map_index    -- content/maps > content/maps/INDEX.txt
```

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

96 maps in HMAP format, 13.8 million squares, 44% smaller than the legacy `.jm` and `.wmap` they
came from. Binary, so `INDEX.txt` lists what each one contains.

Two maps exist under both source formats — `snakepit` and `tomb`. The `.jm` versions keep the plain
name because that is what the world files reference; the `.wmap` versions are written alongside as
`<name>.wmap.hmap` rather than silently overwriting them.
