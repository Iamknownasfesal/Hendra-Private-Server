# The behaviour language in Zed

`content/behaviours/` is 20,000 lines of enemy AI across sixty-one files, and the hard part of
editing it has never been the syntax. It is knowing what `predictive: 0.45` does, which file holds
the thing this boss spawns, and whether the argument you just wrote is one the compiler reads. This
extension answers those three questions in the editor.

## What it gives you

**Tooltips.** Hover a behaviour and get what it does, every argument it reads, what each one means,
its units and its default, and how often the content uses it. Hover an argument for the same thing
about that argument alone.

**Following names.** A quoted name is a link: go-to-definition on `spawn(children: "Malphas Missile")`
opens whichever of the sixty-one files defines it. Go-to-definition on a transition's target jumps to
the state. Find-references on an enemy lists every behaviour that spawns, orders, protects or
transforms into it. `order(children: "Guard", target_state: "attack")` resolves the state against the
guard, not against the enemy giving the order.

**Hovering a state** shows the state itself — its behaviours, its ways out, and the text — so a
transition to something two hundred lines away can be read without going there.

**An outline** of enemies and their nested states, in the outline panel and the breadcrumbs, and
`workspace/symbol` over every enemy in the content, so any of the 748 can be opened by name.

**Diagnostics**, which are the reason this exists:

- what the parser rejects, where it rejects it;
- what the compiler will not recognise, with a suggestion;
- **arguments written under a name nothing reads.** `on timed(time: 500)` compiles, runs, and waits
  one second, because the compiler reads that argument as `after`. There are 475 of these in the
  content today. Warnings are the ones where a value was thrown away; hints are the ones where the
  runtime simply has no such feature, which the author cannot fix.

**Completion** of behaviours, transitions and loot entries in the right places, of an argument's
name inside a call, of state names after `->`, of every enemy name inside quotes, and of the
condition effects.

## Installing it

```
tools/zed-beh/install.sh
```

then, in Zed, run `zed: install dev extension` from the command palette and choose
`tools/zed-beh`. Zed compiles the grammar and the extension itself, which takes a minute the first
time.

The script builds the language server into `rust-server/target/release/beh-lsp` and writes the
`extension.toml` that points Zed at the grammar. Both are per machine, which is why the file in git
is `extension.toml.in` and running the script is a step rather than a convenience.

The extension looks for the server in the settings first, then on the path, then in the
repository's own build. To run it from somewhere else:

```json
{ "lsp": { "beh-lsp": { "binary": { "path": "/somewhere/beh-lsp" } } } }
```

Re-run `install.sh` after changing the grammar or the server. For the server alone, `zed: reload
extensions` is enough.

## The same findings without an editor

```
cargo run --release -p hendra-beh-lsp -- --check rust-server/content/behaviours
```

prints every diagnostic as `file:line:column: severity: message` and exits non-zero if anything
failed to parse.

## What is here

```
grammar/            the tree-sitter grammar, with its generated parser committed
languages/beh/      how Zed colours, folds, indents and outlines a file
src/lib.rs          the extension itself: where to find the language server
extension.toml.in   the manifest install.sh completes
```

The server is `rust-server/crates/beh-lsp`, and it reads files with the compiler's own parser, so
what the editor says a file means is what the game will do with it.

Changing `grammar/grammar.js` means regenerating `grammar/src/parser.c` with the tree-sitter CLI
(`npm install -g tree-sitter-cli`); `install.sh` does it when it can and says so when it cannot.
