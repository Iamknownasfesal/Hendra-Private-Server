//! Turning tokens into a behaviour tree.
//!
//! Recursive descent, because the grammar is small and nested, and because a hand-written parser
//! can say what it expected — which is most of the value a content language gives its authors.

use crate::ast::*;
use crate::lex::{LexError, Span, Spanned, Token, tokenize};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ParseError {
    #[error(transparent)]
    Lex(#[from] LexError),

    #[error("{at}: expected {expected}, found {found}")]
    Expected {
        expected: String,
        found: String,
        at: Span,
    },

    #[error("{at}: `{name}` is not something that can appear here")]
    Unexpected { name: String, at: Span },

    #[error("{at}: a transition needs a target state after `->`")]
    MissingTarget { at: Span },

    #[error("{at}: state `{name}` is declared twice in the same enemy")]
    DuplicateState { name: String, at: Span },

    #[error("{at}: `{target}` is not a state of this enemy")]
    UnknownTarget { target: String, at: Span },
}

/// Parses a whole file.
pub fn parse(source: &str) -> Result<Behaviours, ParseError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser { tokens, at: 0 };
    parser.file()
}

struct Parser {
    tokens: Vec<Spanned>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.at.min(self.tokens.len() - 1)].token
    }

    fn span(&self) -> Span {
        self.tokens[self.at.min(self.tokens.len() - 1)].at
    }

    fn advance(&mut self) -> Token {
        let token = self.tokens[self.at.min(self.tokens.len() - 1)].token.clone();
        if self.at < self.tokens.len() - 1 {
            self.at += 1;
        }
        token
    }

    fn expect(&mut self, wanted: &Token, description: &str) -> Result<(), ParseError> {
        if self.peek() == wanted {
            self.advance();
            return Ok(());
        }
        Err(ParseError::Expected {
            expected: description.to_string(),
            found: self.peek().describe(),
            at: self.span(),
        })
    }

    fn word(&mut self, description: &str) -> Result<String, ParseError> {
        match self.peek().clone() {
            Token::Word(word) => {
                self.advance();
                Ok(word)
            }
            other => Err(ParseError::Expected {
                expected: description.to_string(),
                found: other.describe(),
                at: self.span(),
            }),
        }
    }

    fn text(&mut self, description: &str) -> Result<String, ParseError> {
        match self.peek().clone() {
            Token::Text(text) => {
                self.advance();
                Ok(text)
            }
            other => Err(ParseError::Expected {
                expected: description.to_string(),
                found: other.describe(),
                at: self.span(),
            }),
        }
    }

    fn file(&mut self) -> Result<Behaviours, ParseError> {
        let mut enemies = Vec::new();

        while self.peek() != &Token::End {
            let at = self.span();
            let keyword = self.word("`enemy`")?;
            if keyword != "enemy" {
                return Err(ParseError::Unexpected { name: keyword, at });
            }
            enemies.push(self.enemy(at)?);
        }

        Ok(Behaviours { enemies })
    }

    fn enemy(&mut self, at: Span) -> Result<Enemy, ParseError> {
        let name = self.text("the enemy's name, in quotes")?;
        self.expect(&Token::OpenBrace, "`{` to open the enemy")?;

        let mut root = State {
            name: String::new(),
            items: Vec::new(),
            at: Some(at),
        };
        let mut loot = Vec::new();

        while self.peek() != &Token::CloseBrace && self.peek() != &Token::End {
            if self.peek() == &Token::Word("loot".into()) {
                self.advance();
                loot.extend(self.loot()?);
                continue;
            }
            root.items.push(self.item()?);
        }

        self.expect(&Token::CloseBrace, "`}` to close the enemy")?;

        let enemy = Enemy {
            name,
            root,
            loot,
            at,
        };
        check_states(&enemy)?;
        Ok(enemy)
    }

    /// One thing inside a state: a nested state, a transition, or a behaviour.
    fn item(&mut self) -> Result<Item, ParseError> {
        let at = self.span();

        match self.peek().clone() {
            Token::Word(word) if word == "state" => {
                self.advance();
                Ok(Item::State(self.state()?))
            }

            Token::Word(word) if word == "on" => {
                self.advance();
                Ok(Item::Transition(self.transition()?))
            }

            Token::Word(name) => {
                self.advance();
                let call = self.call(name, at)?;

                // A behaviour followed by a block contains others — `prioritize { … }`. Nothing
                // distinguishes the two at the call site, which is why the block decides.
                if self.peek() == &Token::OpenBrace {
                    self.advance();
                    let mut children = Vec::new();
                    while self.peek() != &Token::CloseBrace && self.peek() != &Token::End {
                        children.push(self.item()?);
                    }
                    self.expect(&Token::CloseBrace, "`}` to close the group")?;
                    return Ok(Item::Group { call, children });
                }

                Ok(Item::Behaviour(call))
            }

            other => Err(ParseError::Expected {
                expected: "a behaviour, `state`, `on` or `loot`".into(),
                found: other.describe(),
                at,
            }),
        }
    }

    fn state(&mut self) -> Result<State, ParseError> {
        let at = self.span();
        let name = self.word("a state name")?;
        self.expect(&Token::OpenBrace, "`{` to open the state")?;

        let mut items = Vec::new();
        while self.peek() != &Token::CloseBrace && self.peek() != &Token::End {
            items.push(self.item()?);
        }
        self.expect(&Token::CloseBrace, "`}` to close the state")?;

        Ok(State {
            name,
            items,
            at: Some(at),
        })
    }

    fn transition(&mut self) -> Result<Transition, ParseError> {
        let at = self.span();
        let name = self.word("a transition condition")?;
        let condition = self.call(name, at)?;

        if self.peek() != &Token::Arrow {
            return Err(ParseError::MissingTarget { at: self.span() });
        }
        self.advance();

        let target = self.word("the state to move to")?;
        Ok(Transition {
            condition,
            target,
            at,
        })
    }

    /// A name, optionally followed by a parenthesised argument list.
    fn call(&mut self, name: String, at: Span) -> Result<Call, ParseError> {
        let mut arguments = Vec::new();

        if self.peek() == &Token::OpenParen {
            self.advance();

            while self.peek() != &Token::CloseParen && self.peek() != &Token::End {
                arguments.push(self.argument()?);

                if self.peek() == &Token::Comma {
                    self.advance();
                } else if self.peek() != &Token::CloseParen {
                    return Err(ParseError::Expected {
                        expected: "`,` between arguments or `)` to close them".into(),
                        found: self.peek().describe(),
                        at: self.span(),
                    });
                }
            }

            self.expect(&Token::CloseParen, "`)` to close the arguments")?;
        }

        Ok(Call {
            name,
            arguments,
            at,
        })
    }

    fn argument(&mut self) -> Result<Argument, ParseError> {
        let at = self.span();

        // A word followed by a colon names the argument; a word alone is a value.
        if let Token::Word(word) = self.peek().clone()
            && self.tokens.get(self.at + 1).map(|next| &next.token) == Some(&Token::Colon)
        {
            self.advance();
            self.advance();
            return Ok(Argument {
                name: Some(word),
                value: self.value()?,
                at,
            });
        }

        Ok(Argument {
            name: None,
            value: self.value()?,
            at,
        })
    }

    fn value(&mut self) -> Result<Value, ParseError> {
        let at = self.span();
        match self.advance() {
            Token::Number(value) => Ok(Value::Number(value)),
            Token::Duration(value) => Ok(Value::Duration(value)),
            Token::Text(text) => Ok(Value::Text(text)),
            Token::Word(word) => Ok(Value::Word(word)),
            other => Err(ParseError::Expected {
                expected: "a number, duration, text or word".into(),
                found: other.describe(),
                at,
            }),
        }
    }

    fn loot(&mut self) -> Result<Vec<Loot>, ParseError> {
        self.expect(&Token::OpenBrace, "`{` to open the loot table")?;

        let mut entries = Vec::new();
        while self.peek() != &Token::CloseBrace && self.peek() != &Token::End {
            let at = self.span();
            let name = self.word("a loot entry")?;
            entries.push(Loot {
                call: self.call(name, at)?,
            });
        }

        self.expect(&Token::CloseBrace, "`}` to close the loot table")?;
        Ok(entries)
    }
}

/// Checks that state names are unique and that every transition leads somewhere real.
///
/// Done at parse time because a transition to a state that does not exist is a file that can never
/// work, and finding it now names the line. Finding it at run time means an enemy that silently
/// stops behaving, months later, in a dungeon nobody has visited.
fn check_states(enemy: &Enemy) -> Result<(), ParseError> {
    let mut names = Vec::new();
    enemy.root.state_names(&mut names);

    let mut seen: Vec<&String> = Vec::new();
    for name in &names {
        if seen.contains(&name) {
            return Err(ParseError::DuplicateState {
                name: name.clone(),
                at: enemy.at,
            });
        }
        seen.push(name);
    }

    check_targets(&enemy.root, &names)
}

fn check_targets(state: &State, names: &[String]) -> Result<(), ParseError> {
    for item in &state.items {
        match item {
            Item::Transition(transition) => {
                if !names.contains(&transition.target) {
                    return Err(ParseError::UnknownTarget {
                        target: transition.target.clone(),
                        at: transition.at,
                    });
                }
            }
            Item::State(nested) => check_targets(nested, names)?,
            Item::Group { children, .. } => {
                for child in children {
                    if let Item::Transition(transition) = child
                        && !names.contains(&transition.target)
                    {
                        return Err(ParseError::UnknownTarget {
                            target: transition.target.clone(),
                            at: transition.at,
                        });
                    }
                }
            }
            Item::Behaviour(_) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Hobbit Mage, as it appears in the C# content, written in this language.
    const HOBBIT_MAGE: &str = r#"
        enemy "Hobbit Mage" {
            state idle {
                on player_within(12) -> ring1
            }

            state ring1 {
                shoot(1, fixed_angle: 0, count: 15, shoot_angle: 24, cooldown: 1200ms, projectile: 0)
                on timed(400ms) -> ring2
            }

            state ring2 {
                shoot(1, fixed_angle: 8, count: 15, shoot_angle: 24, cooldown: 1200ms, projectile: 1)
                on timed(400ms) -> idle
            }

            prioritize {
                stay_above(0.4, 9)
                follow(0.75, range: 6)
                wander(0.4)
            }

            spawn("Hobbit Archer", max_children: 4, cooldown: 12s, gives_no_xp: false)

            loot {
                tier(2, weapon, 0.3)
                item("Health Potion", 0.02)
            }
        }
    "#;

    #[test]
    fn a_real_enemy_parses() {
        let parsed = parse(HOBBIT_MAGE).expect("should parse");
        assert_eq!(parsed.len(), 1);

        let mage = parsed.get("Hobbit Mage").expect("by name");
        assert_eq!(mage.root.states().count(), 3);
        assert_eq!(mage.loot.len(), 2);
    }

    #[test]
    fn arguments_read_by_name_or_position() {
        let parsed = parse(HOBBIT_MAGE).unwrap();
        let mage = parsed.get("Hobbit Mage").unwrap();

        let ring1 = mage.root.states().find(|state| state.name == "ring1").unwrap();
        let shoot = ring1.behaviours().next().unwrap();

        assert_eq!(shoot.name, "shoot");
        assert_eq!(shoot.positional(0), Some(&Value::Number(1.0)));
        assert_eq!(shoot.named("count"), Some(&Value::Number(15.0)));
        assert_eq!(shoot.named("cooldown"), Some(&Value::Duration(1200.0)));

        // `argument` accepts either spelling, which the transpiler relies on.
        assert_eq!(shoot.argument("radius", 0), Some(&Value::Number(1.0)));
        assert_eq!(shoot.argument("count", 9), Some(&Value::Number(15.0)));
    }

    #[test]
    fn a_group_holds_its_children() {
        let parsed = parse(HOBBIT_MAGE).unwrap();
        let mage = parsed.get("Hobbit Mage").unwrap();

        let group = mage
            .root
            .items
            .iter()
            .find_map(|item| match item {
                Item::Group { call, children } if call.name == "prioritize" => Some(children),
                _ => None,
            })
            .expect("a prioritize group");

        assert_eq!(group.len(), 3);
    }

    #[test]
    fn transitions_name_their_target() {
        let parsed = parse(HOBBIT_MAGE).unwrap();
        let mage = parsed.get("Hobbit Mage").unwrap();

        let idle = mage.root.states().find(|state| state.name == "idle").unwrap();
        let transition = idle.transitions().next().unwrap();

        assert_eq!(transition.condition.name, "player_within");
        assert_eq!(transition.condition.positional(0), Some(&Value::Number(12.0)));
        assert_eq!(transition.target, "ring1");
    }

    #[test]
    fn a_transition_to_a_state_that_does_not_exist_is_refused() {
        // The failure this prevents: an enemy that silently stops behaving, months later, in a
        // dungeon nobody has visited.
        let outcome = parse(
            r#"enemy "X" {
                 state idle { on timed(400ms) -> nowhere }
               }"#,
        );

        match outcome {
            Err(ParseError::UnknownTarget { target, .. }) => assert_eq!(target, "nowhere"),
            other => panic!("expected an unknown target, got {other:?}"),
        }
    }

    #[test]
    fn a_state_declared_twice_is_refused() {
        let outcome = parse(
            r#"enemy "X" {
                 state idle { }
                 state idle { }
               }"#,
        );
        assert!(matches!(outcome, Err(ParseError::DuplicateState { .. })));
    }

    #[test]
    fn errors_say_what_was_expected_and_where() {
        let outcome = parse("enemy \"X\" {\n  state idle\n}");
        match outcome {
            Err(ParseError::Expected { expected, at, .. }) => {
                assert!(expected.contains('{'), "expected mentions the brace: {expected}");
                assert_eq!(at.line, 3, "and points at the line that went wrong");
            }
            other => panic!("expected a specific complaint, got {other:?}"),
        }
    }

    #[test]
    fn a_transition_without_a_target_is_named_as_such() {
        let outcome = parse(r#"enemy "X" { state idle { on timed(400ms) } }"#);
        assert!(matches!(outcome, Err(ParseError::MissingTarget { .. })));
    }

    #[test]
    fn several_enemies_can_share_a_file() {
        let parsed = parse(
            r#"
            enemy "First"  { state idle { } }
            enemy "Second" { state idle { } }
            "#,
        )
        .unwrap();

        assert_eq!(parsed.len(), 2);
        assert!(parsed.get("First").is_some());
        assert!(parsed.get("Second").is_some());
    }

    #[test]
    fn state_names_may_repeat_across_enemies() {
        // They are scoped per enemy, so two enemies each having an `idle` is not a clash.
        assert!(
            parse(
                r#"
                enemy "A" { state idle { on timed(1s) -> idle } }
                enemy "B" { state idle { on timed(1s) -> idle } }
                "#
            )
            .is_ok()
        );
    }

    #[test]
    fn an_empty_file_parses_to_nothing() {
        let parsed = parse("# nothing here\n").unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn junk_never_panics() {
        let mut seed = 0x1234_5678u64;
        let alphabet: Vec<char> = "enemy state on {}()-> \"abc\",:0123.".chars().collect();

        for _ in 0..2_000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let length = (seed % 60) as usize;
            let source: String = (0..length)
                .map(|i| alphabet[((seed >> (i % 16)) as usize) % alphabet.len()])
                .collect();
            let _ = parse(&source);
        }
    }
}
