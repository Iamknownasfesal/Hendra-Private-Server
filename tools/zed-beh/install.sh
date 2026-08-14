#!/usr/bin/env bash
#
# Prepares the extension for this machine and says how to load it.
#
# Two things here cannot be committed. The language server is a binary, so it is built; and Zed
# fetches a grammar by cloning a git repository at a revision, so the grammar in this directory is
# copied into one under `.build` and `extension.toml` is written pointing at it. Both are per
# machine, which is why the file in git is `extension.toml.in`.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
build="$here/.build"
grammar="$build/tree-sitter-beh"

say() { printf '\033[1m%s\033[0m\n' "$*"; }

# -- the language server ------------------------------------------------------------------------

say "Building the language server…"
cargo build --release --manifest-path "$root/rust-server/Cargo.toml" -p hendra-beh-lsp

server="$root/rust-server/target/release/beh-lsp"
[ -x "$server" ] || { echo "the build produced no $server" >&2; exit 1; }

# -- the grammar --------------------------------------------------------------------------------

# `parser.c` is generated and committed, so the tree-sitter CLI is only needed by whoever changes
# the grammar itself.
if [ "$here/grammar/grammar.js" -nt "$here/grammar/src/parser.c" ]; then
    if command -v tree-sitter >/dev/null 2>&1; then
        say "Regenerating the parser from grammar.js…"
        (cd "$here/grammar" && tree-sitter generate)
    else
        echo "warning: grammar.js is newer than src/parser.c, and the tree-sitter CLI is not" >&2
        echo "         installed to regenerate it (npm install -g tree-sitter-cli)." >&2
    fi
fi

say "Preparing the grammar repository…"
mkdir -p "$grammar"
rsync -a --delete --exclude .git "$here/grammar/" "$grammar/" 2>/dev/null ||
    { rm -rf "${grammar:?}/src" "${grammar:?}/grammar.js"; cp -R "$here/grammar/." "$grammar/"; }

if [ ! -d "$grammar/.git" ]; then
    git -C "$grammar" init --quiet --initial-branch=main
fi
# Zed fetches the revision by its hash, which a local repository will only serve when told to.
git -C "$grammar" config uploadpack.allowAnySHA1InWant true
git -C "$grammar" config user.email "extension@hendra.local"
git -C "$grammar" config user.name "Behaviour extension"
git -C "$grammar" add -A
git -C "$grammar" diff --quiet --cached ||
    git -C "$grammar" commit --quiet -m "The behaviour grammar, as Zed wants it: a repository"

rev="$(git -C "$grammar" rev-parse HEAD)"

# -- the manifest -------------------------------------------------------------------------------

sed -e "s|@GRAMMAR@|file://$grammar|" -e "s|@REV@|$rev|" \
    "$here/extension.toml.in" > "$here/extension.toml"

say "Ready."
cat <<INSTRUCTIONS

  The server:   $server
  The extension: $here

  In Zed, open the command palette and run

      zed: install dev extension

  then choose

      $here

  Zed builds the grammar and the extension itself, which takes a minute the first time. Open any
  .beh file afterwards. Run this script again after changing the grammar or the server; for the
  server alone, "zed: reload extensions" is enough to pick the new binary up.

INSTRUCTIONS
