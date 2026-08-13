//! The behaviour language: what enemies do, as content rather than code.
//!
//! The game's enemy AI is 23,000 lines of C# constructor calls — declarative in everything but
//! spelling, and requiring a recompile to change a cooldown. This is the same thing as a language,
//! so a boss's behaviour is a file an author can edit.
//!
//! ```text
//!   lex    source            -> tokens, each knowing where it came from
//!   parse  tokens            -> a tree of states, behaviours and transitions
//! ```
//!
//! Behaviours and transitions are parsed as generic calls: a name and some arguments. With
//! seventy-five primitives in wildly uneven use — `Shoot` appears 4,498 times in the content and a
//! dozen others appear once — giving each its own grammar would be seventy-five things to keep in
//! step. An unknown name is a compile error against one table, not a parse error against the
//! grammar.

pub mod ast;
pub mod compile;
pub mod csharp;
pub mod lex;
pub mod parse;
pub mod program;
pub mod run;
pub mod transpile;

pub use ast::{Argument, Behaviours, Call, Enemy, Item, Loot, State, Transition, Value};
pub use compile::{Diagnostic, compile};
pub use lex::{LexError, Span, Token, tokenize};
pub use parse::{ParseError, parse};
pub use program::{
    Action, CompiledState, Condition, LootEntry, Nearby, Primitive, Program, Programs, Senses,
};
pub use run::Mind;
