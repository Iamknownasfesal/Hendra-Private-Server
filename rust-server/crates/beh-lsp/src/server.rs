//! The editor's questions, answered.
//!
//! One struct holding the whole content directory, and a method per request. Everything positional
//! comes from `source`, everything about meaning comes from `docs`, and everything about where a
//! name lives comes from `index`; this file is the join between them.

use std::ops::Range;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::cursor::{self, CallSite, What};
use crate::docs;
use crate::index::{File, Workspace, path_of};
use crate::source::{EnemyBlock, Kind, Source, StateBlock};

pub struct Server {
    pub workspace: Workspace,
    pub root: Option<PathBuf>,
}

/// A diagnostic before it is turned into JSON.
struct Finding {
    span: Range<usize>,
    severity: u8,
    message: String,
}

const ERROR: u8 = 1;
const WARNING: u8 = 2;
const HINT: u8 = 4;

impl Server {
    pub fn new() -> Self {
        Self {
            workspace: Workspace::default(),
            root: None,
        }
    }

    // -- lifecycle ------------------------------------------------------------------------------

    pub fn initialize(&mut self, params: &Value) -> Value {
        self.root = root_of(params);
        if let Some(root) = self.root.clone() {
            self.workspace.scan(&root);
        }

        json!({
            "capabilities": {
                // Whole documents: a behaviour file is a few hundred lines, and reparsing one is
                // faster than working out which part of it changed.
                "textDocumentSync": 1,
                "hoverProvider": true,
                "definitionProvider": true,
                "referencesProvider": true,
                "documentSymbolProvider": true,
                "workspaceSymbolProvider": true,
                "completionProvider": {
                    "triggerCharacters": ["(", ",", " ", "\"", ">", ":"],
                },
            },
            "serverInfo": { "name": "beh-lsp", "version": env!("CARGO_PKG_VERSION") },
        })
    }

    pub fn open(&mut self, uri: &str, text: String) {
        self.workspace.set(uri, text);
    }

    pub fn close(&mut self, _uri: &str) {}

    // -- diagnostics ----------------------------------------------------------------------------

    /// Everything worth saying about a file: what will not parse, what the compiler will not
    /// understand, and what is written in a way that quietly does nothing.
    pub fn diagnostics(&self, uri: &str) -> Value {
        let Some(file) = self.workspace.get(uri) else {
            return json!([]);
        };
        let source = &file.source;
        let mut findings = Vec::new();

        match hendra_behavior::parse(&source.text) {
            Ok(parsed) => {
                let (_, complaints) = hendra_behavior::compile(&parsed);
                for complaint in complaints {
                    let start = source.from_span(complaint.at);
                    findings.push(Finding {
                        span: word_at(source, start),
                        severity: WARNING,
                        message: complaint.message,
                    });
                }
            }
            Err(error) => findings.push(parse_finding(source, &error)),
        }

        findings.extend(self.arguments(file));

        let items: Vec<Value> = findings
            .into_iter()
            .map(|finding| {
                json!({
                    "range": range_of(source, finding.span),
                    "severity": finding.severity,
                    "source": "beh",
                    "message": finding.message,
                })
            })
            .collect();

        json!(items)
    }

    /// Arguments written with a name nothing reads.
    ///
    /// The compiler takes each argument by the name it expects, falling back to the position the
    /// C# constructor had. An argument written under any other name is not an error anywhere: it
    /// parses, it compiles, and the value is thrown away for a default. This is the only place
    /// that says so.
    fn arguments(&self, file: &File) -> Vec<Finding> {
        let source = &file.source;
        let mut findings = Vec::new();

        for index in 0..source.tokens.len() {
            if source.kind(index) != Some(Kind::Word) {
                continue;
            }
            let Some(open) = source.after(index).filter(|at| source.kind(*at) == Some(Kind::OpenParen))
            else {
                continue;
            };
            // Only a name in call position: `radius: 5` is an argument, not a call.
            if source
                .before(index)
                .is_some_and(|at| source.kind(at) == Some(Kind::Colon))
            {
                continue;
            }

            let site = CallSite {
                name: source.tokens[index].value.clone(),
                kind: cursor::at(source, source.tokens[index].span.start)
                    .and_then(|what| match what {
                        What::Call(site) => Some(site.kind),
                        _ => None,
                    })
                    .unwrap_or(docs::Kind::Behaviour),
                span: source.tokens[index].span.clone(),
            };
            let Some(entry) = site.entry() else {
                continue;
            };

            for (name, span) in named_arguments(source, open) {
                if entry.param(&name).is_some() {
                    continue;
                }
                // A value that would have been read under another name is a mistake in the file;
                // one the runtime has no feature for is not something its author can fix.
                let mut severity = WARNING;
                let message = match entry.ignored(&name) {
                    Some(ignored) => {
                        if !ignored.misread {
                            severity = HINT;
                        }
                        format!("`{}` does not read `{name}`: {}.", entry.name, ignored.why)
                    }
                    None => {
                        let hint = docs::nearest_param(entry, &name)
                            .map(|near| format!(" (did you mean `{near}`?)"))
                            .unwrap_or_default();
                        format!(
                            "`{}` has no argument called `{name}`, so the value is ignored{hint}.",
                            entry.name
                        )
                    }
                };
                findings.push(Finding {
                    span,
                    severity,
                    message,
                });
            }
        }

        findings
    }

    // -- hover ----------------------------------------------------------------------------------

    pub fn hover(&self, uri: &str, line: u32, character: u32) -> Value {
        let Some(file) = self.workspace.get(uri) else {
            return Value::Null;
        };
        let source = &file.source;
        let offset = source.offset(line, character);
        let Some(what) = cursor::at(source, offset) else {
            return Value::Null;
        };

        let text = match &what {
            What::Keyword(word) => keyword_help(word).to_string(),
            What::Call(site) => self.call_help(site),
            What::Argument { call, name, .. } => self.argument_help(call, name),
            What::Enemy { name, .. } => self.enemy_help(file, name),
            What::State { name, .. } => self
                .state_help(file, offset, name)
                .unwrap_or_else(|| format!("`{name}` is not a state of this enemy.")),
            What::Value {
                call,
                argument,
                text,
                quoted,
                ..
            } => self.value_help(file, offset, call.as_ref(), argument.as_deref(), text, *quoted),
        };

        if text.is_empty() {
            return Value::Null;
        }

        let range = what.span().map(|span| range_of(source, span));
        json!({
            "contents": { "kind": "markdown", "value": text },
            "range": range,
        })
    }

    fn call_help(&self, site: &CallSite) -> String {
        let Some(entry) = site.entry() else {
            let table = match site.kind {
                docs::Kind::Condition => docs::CONDITIONS,
                docs::Kind::Loot => docs::LOOT,
                _ => docs::BEHAVIOURS,
            };
            let hint = docs::nearest(table, &site.name)
                .map(|near| format!(" Did you mean `{near}`?"))
                .unwrap_or_default();
            return format!(
                "`{}` is not a {} this runtime knows, so it will do nothing.{hint}",
                site.name,
                site.kind.label()
            );
        };

        let uses = self.workspace.calls(entry.name);
        let mut out = format!("```beh\n{}\n```\n", entry.signature());
        out.push_str(&format!("**{}**", entry.kind.label()));
        if uses > 0 {
            out.push_str(&format!(" · written {} in the content", times(uses)));
        }
        if !entry.aliases.is_empty() {
            let aliases: Vec<String> = entry
                .aliases
                .iter()
                .map(|alias| format!("`{alias}`"))
                .collect();
            out.push_str(&format!(" · also spelled {}", aliases.join(", ")));
        }
        out.push_str("\n\n");
        out.push_str(entry.summary);

        if let Some(note) = entry.note {
            out.push_str(&format!("\n\n⚠️ {note}"));
        }

        if !entry.params.is_empty() {
            out.push_str("\n\n**Arguments**\n\n");
            for param in entry.params {
                out.push_str(&format!(
                    "- **{}** — {}, default `{}`. {}\n",
                    param.name, param.value, param.default, param.doc
                ));
            }
        }

        out
    }

    fn argument_help(&self, call: &CallSite, name: &str) -> String {
        let Some(entry) = call.entry() else {
            return format!("`{name}`, an argument of `{}`.", call.name);
        };

        if let Some(param) = entry.param(name) {
            let position = match param.position {
                Some(index) => format!(", or written {} without a name", ordinal(index)),
                None => String::new(),
            };
            return format!(
                "```beh\n{}(… {name} …)\n```\n{}, default `{}`{position}\n\n{}",
                entry.name, param.value, param.default, param.doc
            );
        }

        if let Some(ignored) = entry.ignored(name) {
            return format!(
                "⚠️ **`{}` does not read `{name}`.**\n\n{}.\n\nThe value written here is thrown \
                 away, and the behaviour runs as if it were not there.",
                entry.name, ignored.why
            );
        }

        let hint = docs::nearest_param(entry, name)
            .map(|near| format!(" Did you mean `{near}`?"))
            .unwrap_or_default();
        format!(
            "⚠️ `{}` has no argument called `{name}`, so the value is ignored.{hint}",
            entry.name
        )
    }

    /// An enemy by name: where it is written, and what it is made of.
    fn enemy_help(&self, from: &File, name: &str) -> String {
        let Some((file, enemy)) = self.workspace.enemy_near(from, name) else {
            return format!(
                "**{name}**\n\nNo behaviour file defines this. It may be a plain object from the \
                 XML content, which needs no behaviour of its own."
            );
        };

        let mut out = format!("**{name}** — enemy in `{}`\n\n", file.name());
        let states = enemy.all_states();
        if states.is_empty() {
            out.push_str("No states: everything it does, it does all the time.\n");
        } else {
            out.push_str(&format!("{}\n\n", counted(states.len(), "state")));
            out.push_str("```\n");
            for state in &enemy.states {
                write_state_tree(&mut out, state, 0);
            }
            out.push_str("```\n");
        }

        if !enemy.has_loot {
            out.push_str("\nNo loot table: it drops nothing of its own.\n");
        }

        let mentions = self.workspace.mentions(name).len().saturating_sub(1);
        if mentions > 0 {
            out.push_str(&format!(
                "\nNamed by {} elsewhere.",
                counted(mentions, "other behaviour")
            ));
        }
        out
    }

    /// A state of the enemy the cursor is in.
    fn state_help(&self, file: &File, offset: usize, name: &str) -> Option<String> {
        let enemy = file.enemy_at(offset)?;
        let state = enemy.state(name)?;
        Some(state_summary(&file.source, enemy, state))
    }

    /// A value: usually a name that belongs to another file.
    fn value_help(
        &self,
        file: &File,
        offset: usize,
        call: Option<&CallSite>,
        argument: Option<&str>,
        text: &str,
        quoted: bool,
    ) -> String {
        // A state of whoever is being ordered, which is another enemy entirely.
        if argument == Some("target_state")
            && let Some(call) = call
            && let Some(help) = self.ordered_state_help(file, offset, call, text)
        {
            return help;
        }

        if quoted {
            return self.enemy_help(file, text);
        }

        if let Some(index) = docs::effect(text) {
            return format!(
                "**{text}** — condition effect {index}.\n\nApplied with `conditional_effect` and \
                 taken off with `remove_conditional_effect`."
            );
        }

        if let Some(call) = call
            && let Some(entry) = call.entry()
            && let Some(name) = argument
            && let Some(param) = entry.param(name)
        {
            return format!("`{}` — {}. {}", param.name, param.value, param.doc);
        }

        String::new()
    }

    /// `order(children: "Guard", target_state: "attack")` names a state of the guard.
    fn ordered_state_help(
        &self,
        file: &File,
        offset: usize,
        call: &CallSite,
        state: &str,
    ) -> Option<String> {
        let (target_file, enemy) = self.ordered_enemy(file, offset, call)?;
        let found = enemy.state(state)?;
        Some(format!(
            "{}\n\nIn **{}**, from `{}`.",
            state_summary(&target_file.source, enemy, found),
            enemy.name,
            target_file.name()
        ))
    }

    /// The enemy an `order` is aimed at, taken from the `children` written beside it.
    fn ordered_enemy<'a>(
        &'a self,
        file: &'a File,
        offset: usize,
        call: &CallSite,
    ) -> Option<(&'a File, &'a EnemyBlock)> {
        if !matches!(call.name.as_str(), "order" | "order_once" | "order_on_death") {
            return None;
        }
        let source = &file.source;
        let index = source.token_at(offset)?;
        let open = source.after(source.token_at(call.span.start)?)?;

        let children = named_arguments(source, open)
            .into_iter()
            .find(|(name, _)| name == "children")
            .and_then(|(_, span)| {
                source
                    .tokens
                    .iter()
                    .find(|token| token.span.start > span.end && token.kind == Kind::Text)
                    .map(|token| token.value.clone())
            });

        let _ = index;
        self.workspace.enemy_near(file, &children?)
    }

    // -- going places ---------------------------------------------------------------------------

    pub fn definition(&self, uri: &str, line: u32, character: u32) -> Value {
        let Some(file) = self.workspace.get(uri) else {
            return Value::Null;
        };
        let source = &file.source;
        let offset = source.offset(line, character);
        let Some(what) = cursor::at(source, offset) else {
            return Value::Null;
        };

        match what {
            // A name in quotes is nearly always another enemy.
            What::Value {
                text,
                quoted: true,
                call,
                argument,
                ..
            } => {
                if argument.as_deref() == Some("target_state")
                    && let Some(call) = call.as_ref()
                    && let Some((target, enemy)) = self.ordered_enemy(file, offset, call)
                    && let Some(state) = enemy.state(&text)
                {
                    return location_of(target, state.name_span.clone());
                }
                match self.workspace.enemy_near(file, &text) {
                    Some((found, enemy)) => location_of(found, enemy.name_span.clone()),
                    None => Value::Null,
                }
            }

            // On the name in `state x` there is nowhere else to go: this is the declaration.
            What::State {
                declaration: true, ..
            } => Value::Null,

            What::State { name, .. } => match file
                .enemy_at(offset)
                .and_then(|enemy| enemy.state(&name))
            {
                Some(state) => location_of(file, state.name_span.clone()),
                None => Value::Null,
            },

            What::Enemy { name, .. } => match self.workspace.enemy_near(file, &name) {
                Some((found, enemy)) => location_of(found, enemy.name_span.clone()),
                None => Value::Null,
            },

            _ => Value::Null,
        }
    }

    pub fn references(&self, uri: &str, line: u32, character: u32) -> Value {
        let Some(file) = self.workspace.get(uri) else {
            return json!([]);
        };
        let source = &file.source;
        let offset = source.offset(line, character);
        let Some(what) = cursor::at(source, offset) else {
            return json!([]);
        };

        let found: Vec<Value> = match what {
            What::Enemy { name, .. }
            | What::Value {
                text: name,
                quoted: true,
                ..
            } => self
                .workspace
                .mentions(&name)
                .into_iter()
                .map(|(file, span)| location_of(file, span))
                .collect(),

            What::State { name, .. } => {
                let Some(enemy) = file.enemy_at(offset) else {
                    return json!([]);
                };
                let mut found = Vec::new();
                if let Some(state) = enemy.state(&name) {
                    found.push(location_of(file, state.name_span.clone()));
                }
                for state in enemy.all_states() {
                    for (target, span) in &state.targets {
                        if *target == name {
                            found.push(location_of(file, span.clone()));
                        }
                    }
                }
                found
            }

            _ => Vec::new(),
        };

        json!(found)
    }

    // -- outlines -------------------------------------------------------------------------------

    pub fn document_symbols(&self, uri: &str) -> Value {
        let Some(file) = self.workspace.get(uri) else {
            return json!([]);
        };

        let symbols: Vec<Value> = file
            .enemies
            .iter()
            .map(|enemy| {
                json!({
                    "name": enemy.name,
                    "detail": counted(enemy.all_states().len(), "state"),
                    "kind": 5,
                    "range": range_of(&file.source, enemy.span.clone()),
                    "selectionRange": range_of(&file.source, enemy.name_span.clone()),
                    "children": enemy
                        .states
                        .iter()
                        .map(|state| state_symbol(&file.source, state))
                        .collect::<Vec<Value>>(),
                })
            })
            .collect();

        json!(symbols)
    }

    /// Every enemy in the content, so that one can be opened by name from anywhere.
    pub fn workspace_symbols(&self, query: &str) -> Value {
        let wanted = query.to_ascii_lowercase();
        let found: Vec<Value> = self
            .workspace
            .enemies()
            .filter(|(_, enemy)| {
                wanted.is_empty() || enemy.name.to_ascii_lowercase().contains(&wanted)
            })
            .take(500)
            .map(|(file, enemy)| {
                json!({
                    "name": enemy.name,
                    "kind": 5,
                    "containerName": file.name(),
                    "location": {
                        "uri": file.uri,
                        "range": range_of(&file.source, enemy.name_span.clone()),
                    },
                })
            })
            .collect();

        json!(found)
    }

    // -- completion -----------------------------------------------------------------------------

    pub fn completion(&self, uri: &str, line: u32, character: u32) -> Value {
        let Some(file) = self.workspace.get(uri) else {
            return json!([]);
        };
        let source = &file.source;
        let offset = source.offset(line, character);
        let items = self.completions(file, source, offset);
        json!(items)
    }

    fn completions(&self, file: &File, source: &Source, offset: usize) -> Vec<Value> {
        // The last token that starts before the cursor. When the cursor is inside it, that token
        // is the half-written name being completed, and the clue is whatever came before it.
        let typing = source
            .tokens
            .iter()
            .rposition(|token| token.span.start < offset && token.kind != Kind::Comment);

        let partial = typing.filter(|index| {
            source.tokens[*index].span.end >= offset
                && matches!(
                    source.tokens[*index].kind,
                    Kind::Word | Kind::Text | Kind::Number
                )
        });
        let context = match partial {
            Some(index) => source.before(index),
            None => typing,
        };
        // Where to start looking outwards for enclosing brackets.
        let here = typing.map(|index| index + 1).unwrap_or(0);

        // A transition's target, which is a state of this enemy.
        if context.is_some_and(|at| source.kind(at) == Some(Kind::Arrow)) {
            return match file.enemy_at(offset) {
                Some(enemy) => enemy
                    .all_states()
                    .into_iter()
                    .map(|state| {
                        item(
                            &state.name,
                            14,
                            &counted(state.behaviours, "behaviour"),
                            None,
                        )
                    })
                    .collect(),
                None => Vec::new(),
            };
        }

        // Inside a call's brackets.
        if let Some(call) = cursor::enclosing_call(source, here) {
            let quoted = partial.is_some_and(|index| source.kind(index) == Some(Kind::Text));
            return self.argument_completions(source, context, &call, quoted);
        }

        // Otherwise a name is being written where a call goes.
        let after_on = context.is_some_and(|at| source.is_word(at, "on"));
        let table = if after_on {
            docs::CONDITIONS
        } else if context.is_some_and(|at| cursor::in_loot(source, at + 1)) {
            docs::LOOT
        } else {
            docs::BEHAVIOURS
        };

        let mut items: Vec<Value> = table
            .iter()
            .map(|entry| {
                item(
                    entry.name,
                    3,
                    entry.kind.label(),
                    Some(snippet(entry)),
                )
            })
            .collect();

        if !after_on {
            for (keyword, detail) in [
                ("state", "a state of this enemy"),
                ("on", "a transition out of this state"),
                ("loot", "what this enemy drops"),
            ] {
                items.push(item(keyword, 14, detail, None));
            }
        }
        items
    }

    /// What can be written inside a call: the names of its arguments, or the values one takes.
    fn argument_completions(
        &self,
        source: &Source,
        context: Option<usize>,
        call: &CallSite,
        quoted: bool,
    ) -> Vec<Value> {
        let Some(entry) = call.entry() else {
            // A name still worth completing inside: whatever the call is, it may take an enemy.
            return if quoted { self.enemy_completions(false) } else { Vec::new() };
        };

        // The argument a value is being written for, when the cursor follows a `name:`.
        let argument = context.and_then(|at| {
            let colon = if source.kind(at) == Some(Kind::Colon) {
                at
            } else {
                return None;
            };
            let name = source.before(colon)?;
            (source.kind(name) == Some(Kind::Word)).then(|| source.tokens[name].value.clone())
        });

        let argument = argument.or_else(|| {
            // Inside a half-written string, the `name:` is two tokens back instead of one.
            let at = context?;
            let colon = source.before(at)?;
            (source.kind(colon) == Some(Kind::Colon))
                .then(|| source.before(colon))
                .flatten()
                .filter(|name| source.kind(*name) == Some(Kind::Word))
                .map(|name| source.tokens[name].value.clone())
        });

        if let Some(name) = argument {
            let param = entry.param(&name);
            let value = param.map(|param| param.value).unwrap_or("");

            if value.contains("effect") {
                return docs::EFFECTS
                    .iter()
                    .map(|(effect, code)| item(effect, 21, &format!("effect {code}"), None))
                    .collect();
            }
            if value.contains("true or false") {
                return ["true", "false"]
                    .into_iter()
                    .map(|word| item(word, 21, "", None))
                    .collect();
            }
            if value.contains("name") || value.contains("group") || value.contains("portal") {
                return self.enemy_completions(!quoted);
            }
            return Vec::new();
        }

        if quoted {
            return self.enemy_completions(false);
        }

        // Otherwise the name of an argument.
        let mut items: Vec<Value> = entry
            .params
            .iter()
            .map(|param| {
                item(
                    param.name,
                    5,
                    &format!("{}, default {}", param.value, param.default),
                    Some(format!("{}: $0", param.name)),
                )
            })
            .collect();

        if entry.name == "conditional_effect" || entry.name == "remove_conditional_effect" {
            items.extend(
                docs::EFFECTS
                    .iter()
                    .map(|(effect, code)| item(effect, 21, &format!("effect {code}"), None)),
            );
        }
        items
    }

    /// Every enemy the content defines, quoted when the cursor is not already inside quotes.
    fn enemy_completions(&self, quote: bool) -> Vec<Value> {
        self.workspace
            .enemies()
            .map(|(file, enemy)| {
                let label = if quote {
                    format!("\"{}\"", enemy.name)
                } else {
                    enemy.name.clone()
                };
                item(&label, 21, &file.name(), None)
            })
            .collect()
    }
}

// -- rendering ----------------------------------------------------------------------------------

fn state_symbol(source: &Source, state: &StateBlock) -> Value {
    json!({
        "name": state.name,
        "detail": format!(
            "{}, {}",
            counted(state.behaviours, "behaviour"),
            counted(state.targets.len(), "way out"),
        ),
        "kind": 6,
        "range": range_of(source, state.span.clone()),
        "selectionRange": range_of(source, state.name_span.clone()),
        "children": state
            .children
            .iter()
            .map(|child| state_symbol(source, child))
            .collect::<Vec<Value>>(),
    })
}

/// A state as a tooltip: what it does, where it leads, and the text itself.
fn state_summary(source: &Source, enemy: &EnemyBlock, state: &StateBlock) -> String {
    let mut out = format!("**state {}** of **{}**\n\n", state.name, enemy.name);
    out.push_str(&format!(
        "{}, {}",
        counted(state.behaviours, "behaviour"),
        counted(state.targets.len(), "way out"),
    ));
    if !state.targets.is_empty() {
        let targets: Vec<String> = state
            .targets
            .iter()
            .map(|(name, _)| format!("`{name}`"))
            .collect();
        out.push_str(&format!(" → {}", targets.join(", ")));
    }
    out.push_str("\n\n```beh\n");
    out.push_str(&excerpt(&source.text[state.span.clone()], 14));
    out.push_str("\n```");
    out
}

fn write_state_tree(out: &mut String, state: &StateBlock, depth: usize) {
    out.push_str(&format!("{}{}\n", "  ".repeat(depth), state.name));
    for child in &state.children {
        write_state_tree(out, child, depth + 1);
    }
}

/// The first lines of something, with a mark when there is more.
fn excerpt(text: &str, lines: usize) -> String {
    let mut kept: Vec<&str> = text.lines().take(lines).collect();
    let indent = kept
        .iter()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    let trimmed: Vec<String> = kept
        .drain(..)
        .enumerate()
        .map(|(at, line)| {
            if at == 0 || line.len() < indent {
                line.to_string()
            } else {
                line[indent..].to_string()
            }
        })
        .collect();

    let mut out = trimmed.join("\n");
    if text.lines().count() > lines {
        out.push_str("\n…");
    }
    out
}

fn keyword_help(word: &str) -> &'static str {
    match word {
        "enemy" => {
            "**enemy** — everything one object does, by the name the content knows it as. A file \
             holds as many as its author found convenient."
        }
        "state" => {
            "**state** — one phase of behaviour. What is written in a state runs while the enemy \
             is in it; states written inside a state run alongside their parent, and the enemy is \
             in all of them at once."
        }
        "on" => {
            "**on** — a way out of a state. `on <condition> -> <state>` moves the enemy when the \
             condition holds. Conditions are checked in the order they are written."
        }
        "loot" => {
            "**loot** — what drops when the enemy dies. A `threshold` inside it makes what it \
             holds soulbound to whoever did that share of the damage."
        }
        _ => "",
    }
}

/// A snippet that puts the cursor inside the brackets.
fn snippet(entry: &docs::Entry) -> String {
    if entry.params.is_empty() {
        format!("{}()", entry.name)
    } else {
        format!("{}($0)", entry.name)
    }
}

fn item(label: &str, kind: u8, detail: &str, insert: Option<String>) -> Value {
    let mut value = json!({ "label": label, "kind": kind });
    if !detail.is_empty() {
        value["detail"] = json!(detail);
    }
    if let Some(insert) = insert {
        value["insertText"] = json!(insert);
        value["insertTextFormat"] = json!(2);
    }
    value
}

fn counted(count: usize, thing: &str) -> String {
    match count {
        1 => format!("1 {thing}"),
        _ => format!("{count} {thing}s"),
    }
}

fn times(count: usize) -> String {
    match count {
        1 => "once".to_string(),
        2 => "twice".to_string(),
        _ => format!("{count} times"),
    }
}

fn ordinal(index: usize) -> &'static str {
    match index {
        0 => "first",
        1 => "second",
        2 => "third",
        3 => "fourth",
        4 => "fifth",
        5 => "sixth",
        6 => "seventh",
        7 => "eighth",
        8 => "ninth",
        9 => "tenth",
        _ => "later",
    }
}

// -- positions ----------------------------------------------------------------------------------

fn range_of(source: &Source, span: Range<usize>) -> Value {
    let (start_line, start_character) = source.position(span.start);
    let (end_line, end_character) = source.position(span.end);
    json!({
        "start": { "line": start_line, "character": start_character },
        "end": { "line": end_line, "character": end_character },
    })
}

fn location_of(file: &File, span: Range<usize>) -> Value {
    json!({ "uri": file.uri, "range": range_of(&file.source, span) })
}

/// The extent of whatever starts at an offset, so a diagnostic underlines a word rather than a
/// point.
fn word_at(source: &Source, offset: usize) -> Range<usize> {
    match source.token_at(offset) {
        Some(index) => source.tokens[index].span.clone(),
        None => offset..offset,
    }
}

fn parse_finding(source: &Source, error: &hendra_behavior::ParseError) -> Finding {
    use hendra_behavior::{LexError, ParseError};

    let at = match error {
        ParseError::Lex(LexError::Unexpected { at, .. })
        | ParseError::Lex(LexError::UnclosedText { at })
        | ParseError::Lex(LexError::BadNumber { at, .. })
        | ParseError::Lex(LexError::LoneDash { at })
        | ParseError::Expected { at, .. }
        | ParseError::Unexpected { at, .. }
        | ParseError::MissingTarget { at }
        | ParseError::DuplicateState { at, .. }
        | ParseError::UnknownTarget { at, .. } => *at,
    };

    let start = source.from_span(at);
    // The message already says where it is; the editor shows that itself.
    let message = error.to_string();
    let message = message
        .split_once(": ")
        .map(|(_, rest)| rest.to_string())
        .unwrap_or(message);

    Finding {
        span: word_at(source, start),
        severity: ERROR,
        message: capitalised(&message),
    }
}

fn capitalised(text: &str) -> String {
    let mut letters = text.chars();
    match letters.next() {
        Some(first) if first.is_lowercase() && !text.starts_with('`') => {
            first.to_uppercase().collect::<String>() + letters.as_str()
        }
        _ => text.to_string(),
    }
}

// -- reading calls ------------------------------------------------------------------------------

/// The `name:` arguments of a call, given the index of its `(`.
fn named_arguments(source: &Source, open: usize) -> Vec<(String, Range<usize>)> {
    let mut found = Vec::new();
    let mut depth = 1usize;
    let mut at = open + 1;

    while at < source.tokens.len() {
        match source.tokens[at].kind {
            Kind::OpenParen => depth += 1,
            Kind::CloseParen => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Kind::OpenBrace | Kind::CloseBrace => break,
            Kind::Word if depth == 1 => {
                if source.after(at) == Some(at + 1) && source.kind(at + 1) == Some(Kind::Colon) {
                    found.push((
                        source.tokens[at].value.clone(),
                        source.tokens[at].span.clone(),
                    ));
                }
            }
            _ => {}
        }
        at += 1;
    }

    found
}

fn root_of(params: &Value) -> Option<PathBuf> {
    if let Some(folders) = params["workspaceFolders"].as_array()
        && let Some(first) = folders.first()
        && let Some(uri) = first["uri"].as_str()
    {
        return path_of(uri);
    }
    if let Some(uri) = params["rootUri"].as_str() {
        return path_of(uri);
    }
    params["rootPath"].as_str().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABYSS: &str = r#"enemy "Malphas Missile" {
    state Start {
        on timed(time: 50) -> Attacking
    }
    state Attacking {
        follow(speed: 1.1, acquire_range: 10, range: 0.2)
        on player_within(dist: 1.3) -> Explode
    }
    state Explode {
        shoot(radius: 0, count: 8, shoot_angle: 45)
        suicide()
    }
}

enemy "Archdemon Malphas" {
    state alone {
        reproduce(children: "Malphas Missile", density_radius: 24, density_max: 4, cooldown: 1800)
        order(range: 20, children: "Malphas Missile", target_state: "Explode")
    }
}
"#;

    fn server() -> Server {
        let mut server = Server::new();
        server.workspace.set("file:///abyss.beh", ABYSS.into());
        server
    }

    /// The position of the character after a needle's first character, which is inside its word.
    fn position(needle: &str) -> (u32, u32) {
        let offset = ABYSS.find(needle).expect("in the sample") + 1;
        let source = Source::new(ABYSS.into());
        source.position(offset)
    }

    fn hover(needle: &str) -> String {
        let (line, character) = position(needle);
        let hover = server().hover("file:///abyss.beh", line, character);
        hover["contents"]["value"].as_str().unwrap_or("").to_string()
    }

    #[test]
    fn a_behaviour_tooltip_says_what_it_does_and_what_it_takes() {
        let text = hover("follow(");
        assert!(text.contains("Walks towards the nearest player"), "{text}");
        assert!(text.contains("**acquire_range**"), "{text}");
        assert!(text.contains("default `10`"), "{text}");
    }

    #[test]
    fn an_argument_the_compiler_never_reads_says_so() {
        let text = hover("time: 50");
        assert!(text.contains("does not read `time`"), "{text}");
        assert!(text.contains("1000ms"), "{text}");
    }

    #[test]
    fn a_quoted_name_resolves_to_the_enemy_it_names() {
        let text = hover("\"Malphas Missile\", density_radius");
        assert!(text.contains("enemy in `abyss.beh`"), "{text}");
        assert!(text.contains("3 states"), "{text}");
    }

    #[test]
    fn a_transition_target_shows_the_state_it_leads_to() {
        let text = hover("Attacking\n");
        assert!(text.contains("**state Attacking**"), "{text}");
        assert!(text.contains("follow(speed: 1.1"), "{text}");
    }

    #[test]
    fn an_ordered_state_is_looked_up_in_the_enemy_being_ordered() {
        let text = hover("\"Explode\"");
        assert!(text.contains("**state Explode**"), "{text}");
        assert!(text.contains("Malphas Missile"), "{text}");
    }

    #[test]
    fn a_name_jumps_to_the_enemy_that_defines_it() {
        let (line, character) = position("\"Malphas Missile\", density_radius");
        let found = server().definition("file:///abyss.beh", line, character);
        assert_eq!(found["uri"], "file:///abyss.beh");
        assert_eq!(found["range"]["start"]["line"], 0);
    }

    #[test]
    fn a_transition_jumps_to_the_state_it_names() {
        let (line, character) = position("Attacking\n");
        let found = server().definition("file:///abyss.beh", line, character);
        assert_eq!(found["range"]["start"]["line"], 4);
    }

    #[test]
    fn every_use_of_a_name_is_found() {
        let (line, character) = position("\"Malphas Missile\", density_radius");
        let found = server().references("file:///abyss.beh", line, character);
        assert_eq!(found.as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn the_outline_nests_states_under_their_enemy() {
        let symbols = server().document_symbols("file:///abyss.beh");
        let symbols = symbols.as_array().expect("a list");
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0]["name"], "Malphas Missile");
        assert_eq!(symbols[0]["children"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn arguments_nothing_reads_are_reported() {
        let found = server().diagnostics("file:///abyss.beh");
        let messages: Vec<&str> = found
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["message"].as_str().unwrap())
            .collect();
        assert!(
            messages.iter().any(|message| message.contains("does not read `time`")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|message| message.contains("does not read `dist`")),
            "{messages:?}"
        );
    }

    #[test]
    fn a_file_the_parser_rejects_says_where() {
        let mut server = Server::new();
        server
            .workspace
            .set("file:///broken.beh", "enemy \"X\" {\n  state a {\n".into());
        let found = server.diagnostics("file:///broken.beh");
        let found = found.as_array().expect("a list");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["severity"], 1);
    }

    #[test]
    fn completing_after_an_arrow_offers_the_states_of_this_enemy() {
        let source = Source::new(ABYSS.into());
        let offset = ABYSS.find("-> Attacking").unwrap() + 3;
        let (line, character) = source.position(offset);
        let found = server().completion("file:///abyss.beh", line, character);
        let labels: Vec<&str> = found
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"Explode"), "{labels:?}");
        assert!(labels.contains(&"Start"), "{labels:?}");
    }

    #[test]
    fn completing_inside_a_call_offers_its_arguments() {
        let source = Source::new(ABYSS.into());
        let offset = ABYSS.find("speed: 1.1").unwrap();
        let (line, character) = source.position(offset);
        let found = server().completion("file:///abyss.beh", line, character);
        let labels: Vec<&str> = found
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"acquire_range"), "{labels:?}");
    }

    #[test]
    fn every_enemy_can_be_found_by_name() {
        let found = server().workspace_symbols("malphas");
        assert_eq!(found.as_array().map(Vec::len), Some(2));
    }
}
