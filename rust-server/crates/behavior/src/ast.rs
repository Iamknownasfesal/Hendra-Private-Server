//! What a behaviour file says.
//!
//! # Why calls are generic
//!
//! There are seventy-five behaviour primitives and twenty transitions, and the content uses them in
//! wildly uneven proportion: `Shoot` appears 4,498 times and a dozen others appear once each.
//! Giving each its own syntax would be seventy-five pieces of grammar to write, test and keep in
//! step with the runtime.
//!
//! So a behaviour is a name and some arguments, and the parser neither knows nor cares which ones
//! exist. An unknown name is a *compile* error with a suggestion, not a *parse* error, which means
//! adding a primitive is a change to one table rather than to the grammar.

use std::fmt;

use crate::lex::Span;

/// A value passed to a behaviour or transition.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(f64),

    /// A duration in milliseconds. Distinct from a number so a cooldown written as a bare `400`
    /// can be told apart from one written `400ms`, and the compiler can say which was meant.
    Duration(f64),

    Text(String),

    /// A bare word: `true`, `false`, or a name from a fixed set such as an item type.
    Word(String),
}

impl Value {
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(value) | Value::Duration(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(text) | Value::Word(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Word(word) if word == "true" => Some(true),
            Value::Word(word) if word == "false" => Some(false),
            _ => None,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Value::Number(value) => format!("the number {value}"),
            Value::Duration(value) => format!("the duration {value}ms"),
            Value::Text(text) => format!("the text {text:?}"),
            Value::Word(word) => format!("`{word}`"),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Number(value) => write!(f, "{value}"),
            Value::Duration(value) => write!(f, "{value}ms"),
            Value::Text(text) => write!(f, "{text:?}"),
            Value::Word(word) => write!(f, "{word}"),
        }
    }
}

/// One argument, named or not.
#[derive(Debug, Clone, PartialEq)]
pub struct Argument {
    /// `None` for a positional argument.
    pub name: Option<String>,
    pub value: Value,
    pub at: Span,
}

/// A call to a behaviour or a transition.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub name: String,
    pub arguments: Vec<Argument>,
    pub at: Span,
}

impl Call {
    /// The value of a named argument.
    pub fn named(&self, name: &str) -> Option<&Value> {
        self.arguments
            .iter()
            .find(|argument| argument.name.as_deref() == Some(name))
            .map(|argument| &argument.value)
    }

    /// The value at a positional index, counting only unnamed arguments.
    pub fn positional(&self, index: usize) -> Option<&Value> {
        self.arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .nth(index)
            .map(|argument| &argument.value)
    }

    /// A named argument, falling back to a position.
    ///
    /// The transpiler emits whichever form the C# used, so both spellings occur in real files and
    /// every reader has to accept both.
    pub fn argument(&self, name: &str, index: usize) -> Option<&Value> {
        self.named(name).or_else(|| self.positional(index))
    }
}

/// A transition out of a state.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub condition: Call,
    pub target: String,
    pub at: Span,
}

/// Anything that can appear inside a state.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Behaviour(Call),
    Transition(Transition),
    State(State),

    /// A behaviour that contains others, such as `prioritize`.
    Group {
        call: Call,
        children: Vec<Item>,
    },
}

/// A state, which is also the shape of an enemy's root.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    /// Empty for the root.
    pub name: String,
    pub items: Vec<Item>,
    pub at: Option<Span>,
}

impl State {
    pub fn behaviours(&self) -> impl Iterator<Item = &Call> {
        self.items.iter().filter_map(|item| match item {
            Item::Behaviour(call) => Some(call),
            _ => None,
        })
    }

    pub fn transitions(&self) -> impl Iterator<Item = &Transition> {
        self.items.iter().filter_map(|item| match item {
            Item::Transition(transition) => Some(transition),
            _ => None,
        })
    }

    pub fn states(&self) -> impl Iterator<Item = &State> {
        self.items.iter().filter_map(|item| match item {
            Item::State(state) => Some(state),
            _ => None,
        })
    }

    /// Every state name in this subtree, including this one.
    pub fn state_names(&self, into: &mut Vec<String>) {
        if !self.name.is_empty() {
            into.push(self.name.clone());
        }
        for state in self.states() {
            state.state_names(into);
        }
    }
}

/// One entry in a loot table.
#[derive(Debug, Clone, PartialEq)]
pub struct Loot {
    pub call: Call,

    /// Entries nested inside this one, for `threshold`, which is a rule about who may have what is
    /// inside it rather than a drop of its own.
    pub children: Vec<Loot>,
}

/// One enemy's behaviour.
#[derive(Debug, Clone, PartialEq)]
pub struct Enemy {
    /// The object id this describes, as it appears in the content.
    pub name: String,
    pub root: State,
    pub loot: Vec<Loot>,
    pub at: Span,
}

/// A whole file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Behaviours {
    pub enemies: Vec<Enemy>,
}

impl Behaviours {
    pub fn get(&self, name: &str) -> Option<&Enemy> {
        self.enemies.iter().find(|enemy| enemy.name == name)
    }

    pub fn len(&self) -> usize {
        self.enemies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.enemies.is_empty()
    }
}
