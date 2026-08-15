//! A behaviour file as the editor sees it: bytes, positions, tokens and the block structure.
//!
//! The compiler's own lexer and parser stop at the first mistake, which is right for a build and
//! wrong for an editor — half a file being edited is nearly always broken somewhere. So the pieces
//! an editor needs from a file at every keystroke are rebuilt here over a scanner that never fails:
//! the tokens and their exact extents, and the tree of enemies and states found by matching braces.

use std::ops::Range;

/// What a token is, to the extent an editor needs to care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `shoot`, `state`, `true` — anything word-shaped, keywords included.
    Word,
    /// A quoted name.
    Text,
    Number,
    Arrow,
    OpenBrace,
    CloseBrace,
    OpenParen,
    CloseParen,
    Comma,
    Colon,
    Comment,
    /// Something the language has no meaning for, kept so positions stay honest.
    Junk,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: Kind,
    /// Byte offsets into the source, covering the token including any quotes.
    pub span: Range<usize>,
    /// A word's spelling, or a text's content with escapes resolved. Empty otherwise.
    pub value: String,
}

/// A source file, with everything positional precomputed once.
pub struct Source {
    pub text: String,
    pub tokens: Vec<Token>,
    /// Byte offset of the start of each line.
    line_starts: Vec<usize>,
}

impl Source {
    pub fn new(text: String) -> Self {
        let tokens = scan(&text);
        let mut line_starts = vec![0];
        line_starts.extend(
            text.bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(at, _)| at + 1),
        );
        Self {
            text,
            tokens,
            line_starts,
        }
    }

    /// The byte offset of an LSP position, whose character is a UTF-16 offset into the line.
    pub fn offset(&self, line: u32, character: u32) -> usize {
        let Some(start) = self.line_starts.get(line as usize).copied() else {
            return self.text.len();
        };
        let line_text = &self.text[start..self.line_end(line as usize)];

        let mut units = 0u32;
        for (at, letter) in line_text.char_indices() {
            if units >= character {
                return start + at;
            }
            units += letter.len_utf16() as u32;
        }
        start + line_text.len()
    }

    /// The LSP position of a byte offset.
    pub fn position(&self, offset: usize) -> (u32, u32) {
        let offset = offset.min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(after) => after - 1,
        };
        let character = self.text[self.line_starts[line]..offset]
            .chars()
            .map(|letter| letter.len_utf16() as u32)
            .sum();
        (line as u32, character)
    }

    /// The byte offset of a compiler span, which counts lines and characters from one.
    pub fn offset_of_span(&self, at: hendra_behavior::Span) -> usize {
        let line = at.line.saturating_sub(1) as usize;
        let Some(start) = self.line_starts.get(line).copied() else {
            return self.text.len();
        };
        let line_text = &self.text[start..self.line_end(line)];
        let column = at.column.saturating_sub(1) as usize;

        match line_text.char_indices().nth(column) {
            Some((offset, _)) => start + offset,
            None => start + line_text.len(),
        }
    }

    fn line_end(&self, line: usize) -> usize {
        self.line_starts
            .get(line + 1)
            .map(|next| next - 1)
            .unwrap_or(self.text.len())
    }

    /// The token containing an offset, preferring the one that ends there over the one that starts
    /// there so that a cursor just past a word still points at it.
    pub fn token_at(&self, offset: usize) -> Option<usize> {
        let found = self
            .tokens
            .iter()
            .position(|token| token.span.start <= offset && offset <= token.span.end)?;

        // Two tokens can touch; the earlier one wins only if the later one has not started.
        if self.tokens[found].span.end == offset
            && self
                .tokens
                .get(found + 1)
                .is_some_and(|next| next.span.start == offset)
        {
            return Some(found + 1);
        }
        Some(found)
    }

    /// The index of the previous token that is not a comment.
    pub fn before(&self, index: usize) -> Option<usize> {
        (0..index)
            .rev()
            .find(|at| self.tokens[*at].kind != Kind::Comment)
    }

    /// The index of the next token that is not a comment.
    pub fn after(&self, index: usize) -> Option<usize> {
        (index + 1..self.tokens.len()).find(|at| self.tokens[*at].kind != Kind::Comment)
    }

    pub fn kind(&self, index: usize) -> Option<Kind> {
        self.tokens.get(index).map(|token| token.kind)
    }

    /// Whether a token is a given keyword.
    pub fn is_word(&self, index: usize, word: &str) -> bool {
        self.tokens
            .get(index)
            .is_some_and(|token| token.kind == Kind::Word && token.value == word)
    }
}

/// Splits source into tokens, never failing.
///
/// The rules match the compiler's lexer — same comment spellings, same duration suffixes, same
/// escapes — but an unterminated string ends at the line break and an unknown character becomes a
/// token of its own, because a file being typed into contains both all the time.
fn scan(text: &str) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0usize;

    while at < bytes.len() {
        let start = at;
        let byte = bytes[at];

        if byte.is_ascii_whitespace() {
            at += 1;
            continue;
        }

        if byte == b'#' || (byte == b'/' && bytes.get(at + 1) == Some(&b'/')) {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            tokens.push(Token {
                kind: Kind::Comment,
                span: start..at,
                value: String::new(),
            });
            continue;
        }

        let single = match byte {
            b'{' => Some(Kind::OpenBrace),
            b'}' => Some(Kind::CloseBrace),
            b'(' => Some(Kind::OpenParen),
            b')' => Some(Kind::CloseParen),
            b',' => Some(Kind::Comma),
            b':' => Some(Kind::Colon),
            _ => None,
        };
        if let Some(kind) = single {
            at += 1;
            tokens.push(Token {
                kind,
                span: start..at,
                value: String::new(),
            });
            continue;
        }

        if byte == b'-' && bytes.get(at + 1) == Some(&b'>') {
            at += 2;
            tokens.push(Token {
                kind: Kind::Arrow,
                span: start..at,
                value: String::new(),
            });
            continue;
        }

        if byte == b'"' {
            at += 1;
            let mut value = String::new();
            while at < bytes.len() && bytes[at] != b'"' && bytes[at] != b'\n' {
                if bytes[at] == b'\\' && at + 1 < bytes.len() {
                    at += 1;
                    let escaped = text[at..].chars().next().unwrap_or('\\');
                    value.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                    at += escaped.len_utf8();
                    continue;
                }
                let letter = text[at..].chars().next().unwrap_or('"');
                value.push(letter);
                at += letter.len_utf8();
            }
            if at < bytes.len() && bytes[at] == b'"' {
                at += 1;
            }
            tokens.push(Token {
                kind: Kind::Text,
                span: start..at,
                value,
            });
            continue;
        }

        let numeric = byte.is_ascii_digit()
            || ((byte == b'-' || byte == b'.')
                && bytes.get(at + 1).is_some_and(u8::is_ascii_digit));
        if numeric {
            at += 1;
            while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b'.') {
                at += 1;
            }
            // The duration suffixes, which belong to the number rather than to a word after it.
            if text[at..].starts_with("ms") {
                at += 2;
            } else if text[at..].starts_with('s') {
                at += 1;
            }
            tokens.push(Token {
                kind: Kind::Number,
                span: start..at,
                value: text[start..at].to_string(),
            });
            continue;
        }

        let letter = text[at..].chars().next().unwrap_or('?');
        if letter.is_alphabetic() || letter == '_' {
            while at < text.len() {
                let next = text[at..].chars().next().unwrap_or(' ');
                if !next.is_alphanumeric() && next != '_' {
                    break;
                }
                at += next.len_utf8();
            }
            tokens.push(Token {
                kind: Kind::Word,
                span: start..at,
                value: text[start..at].to_string(),
            });
            continue;
        }

        at += letter.len_utf8();
        tokens.push(Token {
            kind: Kind::Junk,
            span: start..at,
            value: String::new(),
        });
    }

    tokens
}

/// A state, and the states written inside it.
#[derive(Debug, Clone)]
pub struct StateBlock {
    pub name: String,
    /// The name alone, for jumping to.
    pub name_span: Range<usize>,
    /// Everything from `state` to its closing brace, for selecting and for the outline.
    pub span: Range<usize>,
    pub children: Vec<StateBlock>,
    /// The states this one can move to, and where each `->` was written.
    pub targets: Vec<(String, Range<usize>)>,
    /// How many behaviours are written directly in this state.
    pub behaviours: usize,
}

impl StateBlock {
    /// This state and every state under it.
    pub fn flatten<'a>(&'a self, into: &mut Vec<&'a StateBlock>) {
        into.push(self);
        for child in &self.children {
            child.flatten(into);
        }
    }
}

/// One enemy in a file.
#[derive(Debug, Clone)]
pub struct EnemyBlock {
    pub name: String,
    /// The quoted name, including its quotes.
    pub name_span: Range<usize>,
    pub span: Range<usize>,
    pub states: Vec<StateBlock>,
    pub has_loot: bool,
}

impl EnemyBlock {
    pub fn all_states(&self) -> Vec<&StateBlock> {
        let mut out = Vec::new();
        for state in &self.states {
            state.flatten(&mut out);
        }
        out
    }

    pub fn state(&self, name: &str) -> Option<&StateBlock> {
        self.all_states()
            .into_iter()
            .find(|state| state.name == name)
    }
}

/// Finds the enemies and states in a file by matching braces.
///
/// Nothing here parses arguments or checks names: a file mid-edit still has to produce an outline,
/// and a missing closing brace should cost the enemy it was in rather than the whole file.
pub fn outline(source: &Source) -> Vec<EnemyBlock> {
    let tokens = &source.tokens;
    let mut enemies = Vec::new();
    let mut at = 0usize;

    while at < tokens.len() {
        if !source.is_word(at, "enemy") {
            at += 1;
            continue;
        }
        let start = tokens[at].span.start;

        let Some(name_index) = source
            .after(at)
            .filter(|next| tokens[*next].kind == Kind::Text)
        else {
            at += 1;
            continue;
        };
        let Some(open) = source
            .after(name_index)
            .filter(|next| tokens[*next].kind == Kind::OpenBrace)
        else {
            at = name_index;
            continue;
        };

        let mut body = at_block(source, open + 1);
        enemies.push(EnemyBlock {
            name: tokens[name_index].value.clone(),
            name_span: tokens[name_index].span.clone(),
            span: start..body.end,
            states: std::mem::take(&mut body.states),
            has_loot: body.has_loot,
        });
        at = body.after;
    }

    enemies
}

/// What walking one `{ … }` found.
struct Block {
    states: Vec<StateBlock>,
    targets: Vec<(String, Range<usize>)>,
    behaviours: usize,
    has_loot: bool,
    /// The byte offset just past the closing brace.
    end: usize,
    /// The token index just past the closing brace.
    after: usize,
}

/// Walks a block from the token after its `{` to its `}`.
///
/// Group blocks such as `prioritize { … }` are transparent: a state written inside one belongs to
/// the state that holds the group, which is how the compiler flattens them too.
fn at_block(source: &Source, mut at: usize) -> Block {
    let tokens = &source.tokens;
    let mut found = Block {
        states: Vec::new(),
        targets: Vec::new(),
        behaviours: 0,
        has_loot: false,
        end: source.text.len(),
        after: tokens.len(),
    };

    while at < tokens.len() {
        match tokens[at].kind {
            Kind::CloseBrace => {
                found.end = tokens[at].span.end;
                found.after = at + 1;
                return found;
            }
            Kind::Comment => {
                at += 1;
            }
            Kind::Word if tokens[at].value == "state" => {
                let start = tokens[at].span.start;
                let Some(name_index) = source
                    .after(at)
                    .filter(|next| tokens[*next].kind == Kind::Word)
                else {
                    at += 1;
                    continue;
                };
                let Some(open) = source
                    .after(name_index)
                    .filter(|next| tokens[*next].kind == Kind::OpenBrace)
                else {
                    at = name_index + 1;
                    continue;
                };

                let inner = at_block(source, open + 1);
                found.states.push(StateBlock {
                    name: tokens[name_index].value.clone(),
                    name_span: tokens[name_index].span.clone(),
                    span: start..inner.end,
                    children: inner.states,
                    targets: inner.targets,
                    behaviours: inner.behaviours,
                });
                at = inner.after;
            }
            Kind::Word if tokens[at].value == "on" => {
                // `on <condition> -> <state>`: only the target matters to the outline.
                let mut cursor = at + 1;
                while cursor < tokens.len() {
                    match tokens[cursor].kind {
                        Kind::Arrow => {
                            if let Some(target) = source.after(cursor)
                                && tokens[target].kind == Kind::Word
                            {
                                found.targets.push((
                                    tokens[target].value.clone(),
                                    tokens[target].span.clone(),
                                ));
                                cursor = target;
                            }
                            break;
                        }
                        Kind::CloseBrace | Kind::OpenBrace => break,
                        _ => cursor += 1,
                    }
                }
                at = cursor + 1;
            }
            Kind::Word if tokens[at].value == "loot" => {
                found.has_loot = true;
                match source.after(at) {
                    Some(open) if tokens[open].kind == Kind::OpenBrace => {
                        at = skip_braces(source, open + 1);
                    }
                    _ => at += 1,
                }
            }
            Kind::Word => {
                found.behaviours += 1;
                let mut cursor = source.after(at).unwrap_or(tokens.len());
                if source.kind(cursor) == Some(Kind::OpenParen) {
                    cursor = skip_parens(source, cursor + 1);
                    while source.kind(cursor) == Some(Kind::Comment) {
                        cursor += 1;
                    }
                }
                // A call followed by a block is a group, whose contents belong to this state.
                if source.kind(cursor) == Some(Kind::OpenBrace) {
                    let inner = at_block(source, cursor + 1);
                    found.states.extend(inner.states);
                    found.targets.extend(inner.targets);
                    found.behaviours += inner.behaviours;
                    cursor = inner.after;
                }
                at = cursor;
            }
            Kind::OpenBrace => {
                let inner = at_block(source, at + 1);
                found.states.extend(inner.states);
                found.targets.extend(inner.targets);
                at = inner.after;
            }
            _ => at += 1,
        }
    }

    found
}

/// The token index just past the `)` that closes an argument list.
fn skip_parens(source: &Source, mut at: usize) -> usize {
    let mut depth = 1usize;
    while at < source.tokens.len() {
        match source.tokens[at].kind {
            Kind::OpenParen => depth += 1,
            Kind::CloseParen => {
                depth -= 1;
                if depth == 0 {
                    return at + 1;
                }
            }
            _ => {}
        }
        at += 1;
    }
    at
}

/// The token index just past the `}` that closes a block.
fn skip_braces(source: &Source, mut at: usize) -> usize {
    let mut depth = 1usize;
    while at < source.tokens.len() {
        match source.tokens[at].kind {
            Kind::OpenBrace => depth += 1,
            Kind::CloseBrace => {
                depth -= 1;
                if depth == 0 {
                    return at + 1;
                }
            }
            _ => {}
        }
        at += 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_keep_their_exact_extent() {
        let source = Source::new(r#"shoot(radius: 5, target: "Big Guy") # note"#.into());
        let kinds: Vec<Kind> = source.tokens.iter().map(|token| token.kind).collect();
        assert_eq!(
            kinds,
            vec![
                Kind::Word,
                Kind::OpenParen,
                Kind::Word,
                Kind::Colon,
                Kind::Number,
                Kind::Comma,
                Kind::Word,
                Kind::Colon,
                Kind::Text,
                Kind::CloseParen,
                Kind::Comment,
            ]
        );
        let text = source
            .tokens
            .iter()
            .find(|token| token.kind == Kind::Text)
            .unwrap();
        assert_eq!(text.value, "Big Guy");
        assert_eq!(&source.text[text.span.clone()], "\"Big Guy\"");
    }

    #[test]
    fn durations_are_one_token_with_their_suffix() {
        let source = Source::new("timed(400ms) timed(12s) timed(400)".into());
        let numbers: Vec<&str> = source
            .tokens
            .iter()
            .filter(|token| token.kind == Kind::Number)
            .map(|token| token.value.as_str())
            .collect();
        assert_eq!(numbers, vec!["400ms", "12s", "400"]);
    }

    #[test]
    fn an_unclosed_string_ends_at_the_line() {
        let source = Source::new("spawn(children: \"half\nwander()".into());
        assert_eq!(
            source
                .tokens
                .iter()
                .filter(|t| t.kind == Kind::Text)
                .count(),
            1
        );
        assert!(source.tokens.iter().any(|t| t.value == "wander"));
    }

    #[test]
    fn states_nest_and_groups_are_transparent() {
        let source = Source::new(
            r#"enemy "Boss" {
                 state outer {
                   prioritize() {
                     state inner { on timed(time: 5) -> outer }
                   }
                 }
                 loot { item(item: "Potion") }
               }"#
            .into(),
        );
        let enemies = outline(&source);
        assert_eq!(enemies.len(), 1);
        assert_eq!(enemies[0].name, "Boss");
        assert!(enemies[0].has_loot);
        assert_eq!(enemies[0].states.len(), 1);
        assert_eq!(enemies[0].states[0].name, "outer");
        assert_eq!(enemies[0].states[0].children[0].name, "inner");
        assert_eq!(enemies[0].states[0].children[0].targets[0].0, "outer");
    }

    #[test]
    fn an_unclosed_enemy_costs_only_itself() {
        let source = Source::new(
            "enemy \"Broken\" {\n  state a {\n}\n\nenemy \"Fine\" {\n  state b { }\n}\n".into(),
        );
        let enemies = outline(&source);
        assert_eq!(
            enemies.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["Broken"]
        );
        assert!(enemies[0].all_states().iter().any(|s| s.name == "b"));
    }

    #[test]
    fn positions_survive_wide_characters() {
        let source = Source::new("enemy \"Sköll\" {\n  state a { }\n}".into());
        let offset = source.offset(1, 8);
        assert_eq!(&source.text[offset..offset + 1], "a");
        assert_eq!(source.position(offset), (1, 8));
    }
}
