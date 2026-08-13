//! Turning behaviour source into tokens.
//!
//! The language describes enemy AI, so almost everyone who touches it is editing content rather
//! than writing a compiler. That shapes the priorities here: every token carries where it came
//! from, and every failure says what was wrong at a specific line and column. A content language
//! whose errors are "parse error" is a content language nobody edits twice.

use std::fmt;

/// Where something is in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub column: u32,
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// `enemy`, `state`, `on`, `loot`, and anything else word-shaped.
    Word(String),

    Number(f64),

    /// A duration, already converted to milliseconds. `400ms`, `12s` and `1.5s` all land here.
    Duration(f64),

    Text(String),

    /// `->`
    Arrow,

    OpenBrace,
    CloseBrace,
    OpenParen,
    CloseParen,
    Comma,
    Colon,

    End,
}

impl Token {
    /// How to name this token in an error message.
    pub fn describe(&self) -> String {
        match self {
            Token::Word(word) => format!("`{word}`"),
            Token::Number(value) => format!("the number {value}"),
            Token::Duration(ms) => format!("the duration {ms}ms"),
            Token::Text(text) => format!("the text \"{text}\""),
            Token::Arrow => "`->`".into(),
            Token::OpenBrace => "`{`".into(),
            Token::CloseBrace => "`}`".into(),
            Token::OpenParen => "`(`".into(),
            Token::CloseParen => "`)`".into(),
            Token::Comma => "`,`".into(),
            Token::Colon => "`:`".into(),
            Token::End => "the end of the file".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spanned {
    pub token: Token,
    pub at: Span,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LexError {
    #[error("{at}: unexpected character {found:?}")]
    Unexpected { found: char, at: Span },

    #[error("{at}: a text value was never closed")]
    UnclosedText { at: Span },

    #[error("{at}: {text:?} is not a number")]
    BadNumber { text: String, at: Span },

    #[error("{at}: expected `>` after `-`")]
    LoneDash { at: Span },
}

/// Splits source into tokens.
pub fn tokenize(source: &str) -> Result<Vec<Spanned>, LexError> {
    let mut out = Vec::new();
    let characters: Vec<char> = source.chars().collect();

    let mut at = 0usize;
    let mut line = 1u32;
    let mut column = 1u32;

    macro_rules! here {
        () => {
            Span { line, column }
        };
    }

    while at < characters.len() {
        let current = characters[at];

        // Whitespace, tracking lines so errors can point at one.
        if current == '\n' {
            line += 1;
            column = 1;
            at += 1;
            continue;
        }
        if current.is_whitespace() {
            at += 1;
            column += 1;
            continue;
        }

        // Comments run to the end of the line. Both spellings, because content authors arriving
        // from either the C# or a scripting language will reach for a different one.
        if current == '#' || (current == '/' && characters.get(at + 1) == Some(&'/')) {
            while at < characters.len() && characters[at] != '\n' {
                at += 1;
            }
            continue;
        }

        let start = here!();

        let single = match current {
            '{' => Some(Token::OpenBrace),
            '}' => Some(Token::CloseBrace),
            '(' => Some(Token::OpenParen),
            ')' => Some(Token::CloseParen),
            ',' => Some(Token::Comma),
            ':' => Some(Token::Colon),
            _ => None,
        };
        if let Some(token) = single {
            out.push(Spanned { token, at: start });
            at += 1;
            column += 1;
            continue;
        }

        if current == '-' && characters.get(at + 1) == Some(&'>') {
            out.push(Spanned {
                token: Token::Arrow,
                at: start,
            });
            at += 2;
            column += 2;
            continue;
        }

        if current == '"' {
            at += 1;
            column += 1;
            let mut text = String::new();
            let mut closed = false;

            while at < characters.len() {
                let inner = characters[at];
                if inner == '"' {
                    closed = true;
                    at += 1;
                    column += 1;
                    break;
                }
                if inner == '\n' {
                    break;
                }
                // Escapes, so a name containing a quotation mark is expressible.
                if inner == '\\' && at + 1 < characters.len() {
                    at += 1;
                    column += 1;
                    text.push(match characters[at] {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                } else {
                    text.push(inner);
                }
                at += 1;
                column += 1;
            }

            if !closed {
                return Err(LexError::UnclosedText { at: start });
            }
            out.push(Spanned {
                token: Token::Text(text),
                at: start,
            });
            continue;
        }

        // A number, possibly negative, possibly with a duration suffix.
        if current.is_ascii_digit()
            || (current == '-' && characters.get(at + 1).is_some_and(char::is_ascii_digit))
            || (current == '.' && characters.get(at + 1).is_some_and(char::is_ascii_digit))
        {
            let mut text = String::new();
            if current == '-' {
                text.push('-');
                at += 1;
                column += 1;
            }
            while at < characters.len() && (characters[at].is_ascii_digit() || characters[at] == '.')
            {
                text.push(characters[at]);
                at += 1;
                column += 1;
            }

            let Ok(value) = text.parse::<f64>() else {
                return Err(LexError::BadNumber { text, at: start });
            };

            // Durations read far better than bare milliseconds in a file full of cooldowns, and
            // converting here means nothing downstream has to know the difference.
            let suffix: String = characters[at..]
                .iter()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();

            let token = match suffix.as_str() {
                "ms" => {
                    at += 2;
                    column += 2;
                    Token::Duration(value)
                }
                "s" => {
                    at += 1;
                    column += 1;
                    Token::Duration(value * 1000.0)
                }
                _ => Token::Number(value),
            };

            out.push(Spanned { token, at: start });
            continue;
        }

        if current.is_alphabetic() || current == '_' {
            let mut word = String::new();
            while at < characters.len()
                && (characters[at].is_alphanumeric() || characters[at] == '_')
            {
                word.push(characters[at]);
                at += 1;
                column += 1;
            }
            out.push(Spanned {
                token: Token::Word(word),
                at: start,
            });
            continue;
        }

        if current == '-' {
            return Err(LexError::LoneDash { at: start });
        }

        return Err(LexError::Unexpected {
            found: current,
            at: start,
        });
    }

    out.push(Spanned {
        token: Token::End,
        at: Span { line, column },
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<Token> {
        tokenize(source)
            .unwrap()
            .into_iter()
            .map(|spanned| spanned.token)
            .collect()
    }

    #[test]
    fn punctuation_and_words_come_out_separately() {
        assert_eq!(
            tokens("state ring1 { }"),
            vec![
                Token::Word("state".into()),
                Token::Word("ring1".into()),
                Token::OpenBrace,
                Token::CloseBrace,
                Token::End,
            ]
        );
    }

    #[test]
    fn an_arrow_is_one_token() {
        assert_eq!(
            tokens("-> next"),
            vec![Token::Arrow, Token::Word("next".into()), Token::End]
        );
    }

    #[test]
    fn numbers_keep_their_sign_and_fraction() {
        assert_eq!(
            tokens("1 -3 0.75 -0.5 .5"),
            vec![
                Token::Number(1.0),
                Token::Number(-3.0),
                Token::Number(0.75),
                Token::Number(-0.5),
                Token::Number(0.5),
                Token::End,
            ]
        );
    }

    #[test]
    fn durations_become_milliseconds() {
        assert_eq!(
            tokens("400ms 12s 1.5s 400"),
            vec![
                Token::Duration(400.0),
                Token::Duration(12_000.0),
                Token::Duration(1_500.0),
                Token::Number(400.0),
                Token::End,
            ]
        );
    }

    #[test]
    fn text_survives_escapes_and_spaces() {
        assert_eq!(
            tokens(r#""Hobbit Mage" "a \"quoted\" name""#),
            vec![
                Token::Text("Hobbit Mage".into()),
                Token::Text("a \"quoted\" name".into()),
                Token::End,
            ]
        );
    }

    #[test]
    fn comments_are_ignored_in_both_spellings() {
        assert_eq!(
            tokens("state # a hash comment\nidle // a slash comment\n{"),
            vec![
                Token::Word("state".into()),
                Token::Word("idle".into()),
                Token::OpenBrace,
                Token::End,
            ]
        );
    }

    #[test]
    fn every_token_knows_where_it_came_from() {
        let spanned = tokenize("state\n  idle {").unwrap();
        assert_eq!(spanned[0].at, Span { line: 1, column: 1 });
        assert_eq!(spanned[1].at, Span { line: 2, column: 3 });
        assert_eq!(spanned[2].at, Span { line: 2, column: 8 });
    }

    #[test]
    fn an_unclosed_string_says_where_it_started() {
        match tokenize("name \"never ends\nstate") {
            Err(LexError::UnclosedText { at }) => assert_eq!(at.line, 1),
            other => panic!("expected an unclosed string, got {other:?}"),
        }
    }

    #[test]
    fn a_stray_character_is_named_and_located() {
        match tokenize("state idle {\n  shoot @ 3") {
            Err(LexError::Unexpected { found, at }) => {
                assert_eq!(found, '@');
                assert_eq!(at.line, 2);
            }
            other => panic!("expected an unexpected character, got {other:?}"),
        }
    }

    #[test]
    fn a_lone_dash_is_not_a_broken_arrow() {
        assert!(matches!(tokenize("a - b"), Err(LexError::LoneDash { .. })));
    }

    #[test]
    fn an_empty_source_is_just_the_end() {
        assert_eq!(tokens(""), vec![Token::End]);
        assert_eq!(tokens("   \n\n  # only a comment\n"), vec![Token::End]);
    }
}
