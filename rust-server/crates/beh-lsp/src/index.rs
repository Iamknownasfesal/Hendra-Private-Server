//! Every behaviour file at once.
//!
//! An enemy names the things it spawns, orders and transforms into, and those live in whichever of
//! the sixty-one files their author found convenient. Following a name is therefore a question
//! about the whole directory rather than about the open file, so the whole directory is held here
//! and kept current as files are edited.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::source::{EnemyBlock, Kind, Source, outline};

pub struct File {
    pub uri: String,
    pub path: PathBuf,
    pub source: Source,
    pub enemies: Vec<EnemyBlock>,
}

impl File {
    fn new(uri: String, path: PathBuf, text: String) -> Self {
        let source = Source::new(text);
        let enemies = outline(&source);
        Self {
            uri,
            path,
            source,
            enemies,
        }
    }

    /// What the file is called, for saying where something was found.
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.uri.clone())
    }

    /// The enemy whose braces contain an offset.
    pub fn enemy_at(&self, offset: usize) -> Option<&EnemyBlock> {
        self.enemies
            .iter()
            .find(|enemy| enemy.span.contains(&offset))
    }
}

#[derive(Default)]
pub struct Workspace {
    files: BTreeMap<String, File>,
}

impl Workspace {
    /// Reads every `.beh` file under a directory.
    ///
    /// Build directories are skipped rather than walked: `target` alone holds more files than the
    /// rest of the repository together, and none of them are content.
    pub fn scan(&mut self, root: &Path) {
        let mut pending = vec![root.to_path_buf()];

        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();

                if path.is_dir() {
                    let skip = name.starts_with('.')
                        || matches!(name.as_ref(), "target" | "node_modules" | "obj" | "bin");
                    if !skip {
                        pending.push(path);
                    }
                    continue;
                }

                if path.extension().is_some_and(|suffix| suffix == "beh")
                    && let Ok(text) = std::fs::read_to_string(&path)
                {
                    let uri = uri_of(&path);
                    self.files
                        .insert(uri.clone(), File::new(uri, path, text));
                }
            }
        }
    }

    /// Replaces what a file holds, as an editor buffer changes.
    pub fn set(&mut self, uri: &str, text: String) {
        let path = path_of(uri).unwrap_or_else(|| PathBuf::from(uri));
        self.files
            .insert(uri.to_string(), File::new(uri.to_string(), path, text));
    }

    pub fn get(&self, uri: &str) -> Option<&File> {
        self.files.get(uri)
    }

    pub fn uris(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// Where an enemy is defined, wherever that is.
    pub fn enemy(&self, name: &str) -> Option<(&File, &EnemyBlock)> {
        self.files.values().find_map(|file| definition(file, name))
    }

    /// The same, but starting from the file being read.
    ///
    /// The repository holds two copies of the content — the converted one and the one beside the
    /// C# it came from — so a name can be defined twice. Following it should stay in the copy the
    /// author is editing rather than land in the other one by alphabetical accident.
    pub fn enemy_near<'a>(
        &'a self,
        near: &'a File,
        name: &str,
    ) -> Option<(&'a File, &'a EnemyBlock)> {
        if let Some(found) = definition(near, name) {
            return Some(found);
        }

        let directory = near.path.parent();
        self.files
            .values()
            .filter(|file| file.path.parent() == directory)
            .find_map(|file| definition(file, name))
            .or_else(|| self.enemy(name))
    }

    pub fn enemies(&self) -> impl Iterator<Item = (&File, &EnemyBlock)> {
        self.files
            .values()
            .flat_map(|file| file.enemies.iter().map(move |enemy| (file, enemy)))
    }

    /// Every quoted use of a name, including the enemy header that defines it.
    pub fn mentions(&self, name: &str) -> Vec<(&File, Range<usize>)> {
        self.files
            .values()
            .flat_map(|file| {
                file.source
                    .tokens
                    .iter()
                    .filter(move |token| token.kind == Kind::Text && token.value == name)
                    .map(move |token| (file, token.span.clone()))
            })
            .collect()
    }

    /// How many times a behaviour or transition is called across the content, which is the most
    /// useful thing to know about one that is unfamiliar.
    pub fn calls(&self, name: &str) -> usize {
        self.files
            .values()
            .map(|file| {
                file.source
                    .tokens
                    .iter()
                    .enumerate()
                    .filter(|(at, token)| {
                        token.kind == Kind::Word
                            && token.value == name
                            && file.source.kind(at + 1) == Some(Kind::OpenParen)
                    })
                    .count()
            })
            .sum()
    }
}

fn definition<'a>(file: &'a File, name: &str) -> Option<(&'a File, &'a EnemyBlock)> {
    file.enemies
        .iter()
        .find(|enemy| enemy.name == name)
        .map(|enemy| (file, enemy))
}

/// A `file://` URI for a path.
pub fn uri_of(path: &Path) -> String {
    let mut out = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'/' | b'-' | b'.' | b'_' | b'~' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// The path a `file://` URI names.
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let raw = rest.as_bytes();
    let mut at = 0;

    while at < raw.len() {
        if raw[at] == b'%' && at + 2 < raw.len() {
            let hex = std::str::from_utf8(&raw[at + 1..at + 3]).ok()?;
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                bytes.push(byte);
                at += 3;
                continue;
            }
        }
        bytes.push(raw[at]);
        at += 1;
    }

    Some(PathBuf::from(String::from_utf8(bytes).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> Workspace {
        let mut workspace = Workspace::default();
        workspace.set(
            "file:///a.beh",
            r#"enemy "Guard" { state idle { shoot(radius: 5) } }"#.into(),
        );
        workspace.set(
            "file:///b.beh",
            r#"enemy "Boss" { state fight { spawn(children: "Guard") shoot(radius: 8) } }"#.into(),
        );
        workspace
    }

    #[test]
    fn an_enemy_is_found_in_whichever_file_holds_it() {
        let workspace = workspace();
        let (file, enemy) = workspace.enemy("Guard").expect("defined in a.beh");
        assert_eq!(file.uri, "file:///a.beh");
        assert_eq!(enemy.states[0].name, "idle");
        assert!(workspace.enemy("Nobody").is_none());
    }

    #[test]
    fn a_name_defined_twice_resolves_in_the_file_being_read() {
        let mut workspace = workspace();
        // The repository holds a second copy of the content beside the C# it was converted from.
        workspace.set(
            "file:///Server-Side/a.beh",
            r#"enemy "Guard" { state old { } }"#.into(),
        );

        let reading = workspace.get("file:///b.beh").expect("open");
        let (file, _) = workspace.enemy_near(reading, "Guard").expect("found");
        assert_eq!(file.uri, "file:///a.beh");

        // Alphabetical order would have found the other copy first.
        assert_eq!(workspace.enemy("Guard").map(|(file, _)| file.uri.as_str()), Some("file:///Server-Side/a.beh"));
    }

    #[test]
    fn mentions_include_the_definition_and_the_uses() {
        let workspace = workspace();
        assert_eq!(workspace.mentions("Guard").len(), 2);
    }

    #[test]
    fn calls_are_counted_across_files() {
        let workspace = workspace();
        assert_eq!(workspace.calls("shoot"), 2);
        assert_eq!(workspace.calls("spawn"), 1);
    }

    #[test]
    fn a_uri_survives_a_round_trip() {
        let path = Path::new("/tmp/a dir/x.beh");
        assert_eq!(uri_of(path), "file:///tmp/a%20dir/x.beh");
        assert_eq!(path_of(&uri_of(path)).as_deref(), Some(path));
    }
}
