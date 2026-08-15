//! What the cursor is on.
//!
//! Every editor question — what is this, where is it defined, what could go here — is the same
//! question first: which of the language's few kinds of name is under the cursor, and what call is
//! it part of. That is decided here from the tokens alone, so it keeps working while the file is
//! half-written and the parser would not.

use std::ops::Range;

use crate::docs;
use crate::source::{Kind, Source};

/// The call a name belongs to.
#[derive(Debug, Clone)]
pub struct CallSite {
    pub name: String,
    pub kind: docs::Kind,
    pub span: Range<usize>,
}

impl CallSite {
    pub fn entry(&self) -> Option<&'static docs::Entry> {
        docs::lookup(&self.name, self.kind)
    }
}

#[derive(Debug, Clone)]
pub enum What {
    /// `enemy`, `state`, `on` or `loot`.
    Keyword(&'static str),

    /// The quoted name in an `enemy "…"` header.
    Enemy { name: String, span: Range<usize> },

    /// A state name, either where it is declared or where a transition points at it.
    State {
        name: String,
        span: Range<usize>,
        declaration: bool,
    },

    /// The name of a behaviour, transition or loot entry.
    Call(CallSite),

    /// The name written before a `:`.
    Argument {
        call: CallSite,
        name: String,
        span: Range<usize>,
    },

    /// A value inside an argument list.
    Value {
        call: Option<CallSite>,
        /// The argument it was written for, when it was written with a name.
        argument: Option<String>,
        text: String,
        span: Range<usize>,
        quoted: bool,
    },
}

impl What {
    pub fn span(&self) -> Option<Range<usize>> {
        match self {
            What::Keyword(_) => None,
            What::Enemy { span, .. }
            | What::State { span, .. }
            | What::Argument { span, .. }
            | What::Value { span, .. } => Some(span.clone()),
            What::Call(site) => Some(site.span.clone()),
        }
    }
}

/// Works out what a token is from what surrounds it.
pub fn at(source: &Source, offset: usize) -> Option<What> {
    let index = source.token_at(offset)?;
    let token = &source.tokens[index];
    let span = token.span.clone();
    let previous = source.before(index);

    match token.kind {
        Kind::Text => {
            if previous.is_some_and(|at| source.is_word(at, "enemy")) {
                return Some(What::Enemy {
                    name: token.value.clone(),
                    span,
                });
            }
            let call = enclosing_call(source, index);
            Some(What::Value {
                argument: argument_name(source, index),
                call,
                text: token.value.clone(),
                span,
                quoted: true,
            })
        }

        Kind::Number => {
            let call = enclosing_call(source, index);
            Some(What::Value {
                argument: argument_name(source, index),
                call,
                text: token.value.clone(),
                span,
                quoted: false,
            })
        }

        Kind::Word => {
            let word = token.value.as_str();

            if previous.is_some_and(|at| source.tokens[at].kind == Kind::Arrow) {
                return Some(What::State {
                    name: word.to_string(),
                    span,
                    declaration: false,
                });
            }
            if previous.is_some_and(|at| source.is_word(at, "state")) {
                return Some(What::State {
                    name: word.to_string(),
                    span,
                    declaration: true,
                });
            }
            if let Some(keyword) = ["enemy", "state", "on", "loot"]
                .into_iter()
                .find(|keyword| *keyword == word)
            {
                return Some(What::Keyword(keyword));
            }

            let call = enclosing_call(source, index);
            if let Some(call) = call {
                // `name:` is an argument; anything else inside the brackets is a value.
                if source
                    .after(index)
                    .is_some_and(|at| source.kind(at) == Some(Kind::Colon))
                {
                    return Some(What::Argument {
                        call,
                        name: word.to_string(),
                        span,
                    });
                }
                return Some(What::Value {
                    argument: argument_name(source, index),
                    call: Some(call),
                    text: word.to_string(),
                    span,
                    quoted: false,
                });
            }

            Some(What::Call(CallSite {
                name: word.to_string(),
                kind: call_kind(source, index),
                span,
            }))
        }

        _ => None,
    }
}

/// Whether a name written here is a transition, a loot entry, or a behaviour.
fn call_kind(source: &Source, index: usize) -> docs::Kind {
    if source
        .before(index)
        .is_some_and(|at| source.is_word(at, "on"))
    {
        return docs::Kind::Condition;
    }
    if in_loot(source, index) {
        return docs::Kind::Loot;
    }
    // A call with a block after its brackets holds other behaviours.
    let mut after = source.after(index);
    if after.is_some_and(|at| source.kind(at) == Some(Kind::OpenParen)) {
        after = skip_parens(source, after.unwrap() + 1);
    }
    if after.is_some_and(|at| source.kind(at) == Some(Kind::OpenBrace)) {
        return docs::Kind::Group;
    }
    docs::Kind::Behaviour
}

/// The call whose brackets a token is inside.
pub fn enclosing_call(source: &Source, index: usize) -> Option<CallSite> {
    let mut depth = 0usize;
    let mut at = index;

    while at > 0 {
        at -= 1;
        match source.tokens[at].kind {
            Kind::CloseParen => depth += 1,
            Kind::OpenParen => {
                if depth == 0 {
                    let name = source.before(at)?;
                    if source.kind(name) != Some(Kind::Word) {
                        return None;
                    }
                    return Some(CallSite {
                        name: source.tokens[name].value.clone(),
                        kind: call_kind(source, name),
                        span: source.tokens[name].span.clone(),
                    });
                }
                depth -= 1;
            }
            // Brackets do not cross a block, so a stray `(` cannot swallow the rest of the file.
            Kind::OpenBrace | Kind::CloseBrace => return None,
            _ => {}
        }
    }

    None
}

/// The name a value was written under, when it was written with one.
fn argument_name(source: &Source, index: usize) -> Option<String> {
    let colon = source.before(index)?;
    if source.kind(colon) != Some(Kind::Colon) {
        return None;
    }
    let name = source.before(colon)?;
    (source.kind(name) == Some(Kind::Word)).then(|| source.tokens[name].value.clone())
}

/// Whether a token is inside a `loot` table, however deeply a `threshold` nests it.
pub fn in_loot(source: &Source, index: usize) -> bool {
    let mut at = index;

    loop {
        let Some(open) = enclosing_brace(source, at) else {
            return false;
        };
        // What opened this block: `loot {`, `state x {`, `enemy "x" {`, or a call's block.
        let Some(before) = source.before(open) else {
            return false;
        };
        if source.is_word(before, "loot") {
            return true;
        }
        if source.kind(before) != Some(Kind::CloseParen) {
            return false;
        }
        at = open;
    }
}

/// The `{` of the block a token sits in.
fn enclosing_brace(source: &Source, index: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut at = index;

    while at > 0 {
        at -= 1;
        match source.tokens[at].kind {
            Kind::CloseBrace => depth += 1,
            Kind::OpenBrace => {
                if depth == 0 {
                    return Some(at);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn skip_parens(source: &Source, mut at: usize) -> Option<usize> {
    let mut depth = 1usize;
    while at < source.tokens.len() {
        match source.tokens[at].kind {
            Kind::OpenParen => depth += 1,
            Kind::CloseParen => {
                depth -= 1;
                if depth == 0 {
                    return source.after(at);
                }
            }
            _ => {}
        }
        at += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"enemy "Boss" {
    state fight {
        shoot(radius: 8, projectile: 1)
        conditional_effect(invulnerable)
        prioritize() { wander(speed: 0.4) }
        on timed(time: 500) -> rest
    }
    state rest { }
    loot {
        threshold(0.01) { item(item: "Potion", probability: 0.2) }
    }
}"#;

    fn what(needle: &str) -> What {
        let source = Source::new(SOURCE.into());
        let offset = SOURCE.find(needle).expect("in the sample");
        at(&source, offset + 1).expect("something is there")
    }

    #[test]
    fn an_enemy_header_is_a_definition() {
        assert!(matches!(what("\"Boss\""), What::Enemy { name, .. } if name == "Boss"));
    }

    #[test]
    fn a_state_is_told_from_where_a_transition_points() {
        assert!(matches!(
            what("fight {"),
            What::State {
                declaration: true,
                ..
            }
        ));
        assert!(
            matches!(what("rest\n"), What::State { declaration: false, name, .. } if name == "rest")
        );
    }

    #[test]
    fn a_call_knows_which_table_it_belongs_to() {
        assert!(matches!(what("shoot("), What::Call(site) if site.kind == docs::Kind::Behaviour));
        assert!(matches!(what("timed("), What::Call(site) if site.kind == docs::Kind::Condition));
        assert!(matches!(what("prioritize("), What::Call(site) if site.kind == docs::Kind::Group));
        assert!(matches!(what("item(item"), What::Call(site) if site.kind == docs::Kind::Loot));
        assert!(
            matches!(what("threshold(0.01)"), What::Call(site) if site.kind == docs::Kind::Loot)
        );
    }

    #[test]
    fn an_argument_carries_the_call_it_is_in() {
        match what("radius:") {
            What::Argument { call, name, .. } => {
                assert_eq!(name, "radius");
                assert_eq!(call.name, "shoot");
            }
            other => panic!("expected an argument, got {other:?}"),
        }
    }

    #[test]
    fn a_value_carries_the_argument_it_was_written_for() {
        match what("\"Potion\"") {
            What::Value {
                call,
                argument,
                text,
                quoted,
                ..
            } => {
                assert_eq!(text, "Potion");
                assert!(quoted);
                assert_eq!(argument.as_deref(), Some("item"));
                assert_eq!(call.map(|call| call.name).as_deref(), Some("item"));
            }
            other => panic!("expected a value, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_word_inside_brackets_is_a_value_not_a_call() {
        match what("invulnerable") {
            What::Value { call, text, .. } => {
                assert_eq!(text, "invulnerable");
                assert_eq!(
                    call.map(|call| call.name).as_deref(),
                    Some("conditional_effect")
                );
            }
            other => panic!("expected a value, got {other:?}"),
        }
    }

    #[test]
    fn a_behaviour_inside_a_group_is_still_a_behaviour() {
        assert!(matches!(what("wander("), What::Call(site) if site.kind == docs::Kind::Behaviour));
    }
}
