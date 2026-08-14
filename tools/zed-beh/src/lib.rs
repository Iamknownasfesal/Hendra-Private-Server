//! The Zed side of the behaviour language: finding the language server and starting it.
//!
//! Everything an author sees — the tooltips, the outline, following a name into another file — is
//! `beh-lsp` talking to the editor. All this does is say where that program is, which is the one
//! thing a sandboxed extension can answer and the editor cannot guess.

use zed_extension_api::{self as zed, LanguageServerId, Result};

struct BehaviourExtension;

/// Where the repository's own build puts the server, used when nothing else names it.
const BUILT: &str = "rust-server/target/release/beh-lsp";

impl zed::Extension for BehaviourExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        Ok(zed::Command {
            command: binary(id, worktree),
            args: Vec::new(),
            env: worktree.shell_env(),
        })
    }
}

/// The server to run: what the settings name, then what is on the path, then what the repository
/// builds. Nothing here can look at the filesystem — an extension is sandboxed — so the last one is
/// a guess, and a wrong guess is reported by Zed as a server that would not start.
fn binary(id: &LanguageServerId, worktree: &zed::Worktree) -> String {
    if let Ok(settings) = zed::settings::LspSettings::for_worktree(id.as_ref(), worktree) {
        if let Some(binary) = settings.binary {
            if let Some(path) = binary.path {
                return path;
            }
        }
    }

    if let Some(found) = worktree.which("beh-lsp") {
        return found;
    }

    format!("{}/{BUILT}", worktree.root_path())
}

zed::register_extension!(BehaviourExtension);
