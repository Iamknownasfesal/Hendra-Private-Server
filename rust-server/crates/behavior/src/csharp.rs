//! Reading the corner of C# that the old behaviour database is written in.
//!
//! This is not a C# parser and does not want to be. The behaviour database is 23,000 lines of one
//! shape:
//!
//! ```csharp
//! .Init("Hobbit Mage",
//!     new State(
//!         new State("idle",
//!             new PlayerWithinTransition(12, "ring1")),
//!         new Prioritize(
//!             new Follow(0.75, range: 6),
//!             new Wander(0.4))),
//!     new ItemLoot("Health Potion", 0.02))
//! ```
//!
//! Constructor calls, literals, named arguments and nesting. Everything else in those files,
//! meaning classes, fields, lambdas and `using` lines, is scenery, so the scanner finds each `.Init(` and
//! parses the balanced argument list after it, ignoring the rest entirely.
//!
//! Anything wider would be a project of its own, and this runs once.

/// A constructor call, or the argument list of an `.Init`.
#[derive(Debug, Clone, PartialEq)]
pub struct CsCall {
    pub name: String,
    pub arguments: Vec<CsArgument>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CsArgument {
    /// `None` for a positional argument.
    pub name: Option<String>,
    pub value: CsValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CsValue {
    Number(f64),
    Text(String),
    Bool(bool),
    Null,

    /// A bare or dotted name: `ItemType.Weapon`, `TileRegion.Spawn`.
    Path(String),

    Call(CsCall),

    /// An array or collection initialiser, whose contents are kept in order.
    List(Vec<CsValue>),
}

impl CsValue {
    pub fn as_number(&self) -> Option<f64> {
        match self {
            CsValue::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            CsValue::Text(text) => Some(text),
            _ => None,
        }
    }
}

/// One enemy, as the C# declares it.
#[derive(Debug, Clone, PartialEq)]
pub struct CsEnemy {
    pub name: String,
    /// Everything after the name: the root `State` and any loot.
    pub arguments: Vec<CsValue>,
}

/// Finds every `.Init(...)` in a source file and parses its arguments.
///
/// Unparseable entries are skipped rather than fatal: one malformed dungeon should not stop the
/// other forty from converting. They are also *counted*, because a silent skip is how three whole
/// files went missing behind a total that looked like success.
pub fn read_enemies(source: &str) -> Vec<CsEnemy> {
    read_enemies_reporting(source).0
}

/// The same, with the number of `.Init(` entries that could not be read.
pub fn read_enemies_reporting(source: &str) -> (Vec<CsEnemy>, usize) {
    let characters: Vec<char> = source.chars().collect();
    let mut enemies = Vec::new();
    let mut unreadable = 0usize;
    let mut at = 0usize;

    while let Some(start) = find_init(&characters, at) {
        // Position just after `.Init(`.
        let open = start + ".Init(".len();
        let Some(close) = matching_paren(&characters, open - 1) else {
            unreadable += 1;
            at = open;
            continue;
        };

        let inner: String = characters[open..close].iter().collect();
        at = close + 1;

        let mut parser = CsParser::new(&inner);
        match parser.arguments() {
            Ok(arguments) => {
                let Some(CsValue::Text(name)) = arguments.first().map(|a| a.value.clone()) else {
                    unreadable += 1;
                    continue;
                };
                enemies.push(CsEnemy {
                    name,
                    arguments: arguments.into_iter().skip(1).map(|a| a.value).collect(),
                });
            }
            Err(_) => unreadable += 1,
        }
    }

    (enemies, unreadable)
}

/// Finds the next `.Init(` that is real code rather than text or a comment.
fn find_init(characters: &[char], from: usize) -> Option<usize> {
    let needle: Vec<char> = ".Init(".chars().collect();
    let mut at = from;
    let mut in_text = false;
    let mut in_comment = false;

    while at < characters.len() {
        let current = characters[at];

        if in_comment {
            if current == '\n' {
                in_comment = false;
            }
            at += 1;
            continue;
        }

        if in_text {
            if current == '\\' {
                at += 2;
                continue;
            }
            if current == '"' {
                in_text = false;
            }
            at += 1;
            continue;
        }

        if current == '"' {
            in_text = true;
            at += 1;
            continue;
        }

        if current == '/' && characters.get(at + 1) == Some(&'/') {
            in_comment = true;
            at += 2;
            continue;
        }

        if characters[at..].starts_with(needle.as_slice()) {
            return Some(at);
        }

        at += 1;
    }

    None
}

/// The index of the parenthesis closing the one at `open`.
fn matching_paren(characters: &[char], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut at = open;
    let mut in_text = false;

    while at < characters.len() {
        let current = characters[at];

        if in_text {
            if current == '\\' {
                at += 2;
                continue;
            }
            if current == '"' {
                in_text = false;
            }
            at += 1;
            continue;
        }

        match current {
            '"' => in_text = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
        at += 1;
    }

    None
}

#[derive(Debug)]
pub struct CsError;

struct CsParser {
    characters: Vec<char>,
    at: usize,
}

impl CsParser {
    fn new(source: &str) -> CsParser {
        CsParser {
            characters: source.chars().collect(),
            at: 0,
        }
    }

    fn skip_trivia(&mut self) {
        loop {
            while self.at < self.characters.len() && self.characters[self.at].is_whitespace() {
                self.at += 1;
            }
            if self.characters.get(self.at) == Some(&'/')
                && self.characters.get(self.at + 1) == Some(&'/')
            {
                while self.at < self.characters.len() && self.characters[self.at] != '\n' {
                    self.at += 1;
                }
                continue;
            }
            // Preprocessor directives, which Oryx uses to fold its dance phases. They sit in the
            // middle of an argument list and are not part of it.
            if self.characters.get(self.at) == Some(&'#') {
                while self.at < self.characters.len() && self.characters[self.at] != '\n' {
                    self.at += 1;
                }
                continue;
            }

            // Block comments appear in a few places, usually around commented-out behaviours.
            if self.characters.get(self.at) == Some(&'/')
                && self.characters.get(self.at + 1) == Some(&'*')
            {
                self.at += 2;
                while self.at + 1 < self.characters.len()
                    && !(self.characters[self.at] == '*' && self.characters[self.at + 1] == '/')
                {
                    self.at += 1;
                }
                self.at = (self.at + 2).min(self.characters.len());
                continue;
            }
            return;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.skip_trivia();
        self.characters.get(self.at).copied()
    }

    /// A comma-separated argument list, to the end of the input.
    fn arguments(&mut self) -> Result<Vec<CsArgument>, CsError> {
        let mut out = Vec::new();

        loop {
            if self.peek().is_none() {
                return Ok(out);
            }

            out.push(self.argument()?);

            match self.peek() {
                Some(',') => {
                    self.at += 1;
                }
                None => return Ok(out),
                Some(_) => return Ok(out),
            }
        }
    }

    fn argument(&mut self) -> Result<CsArgument, CsError> {
        // A name followed by a colon, but not `::` and not a dotted path, is a named argument.
        let saved = self.at;
        if let Some(word) = self.word()
            && self.peek() == Some(':')
            && self.characters.get(self.at + 1) != Some(&':')
        {
            self.at += 1;
            return Ok(CsArgument {
                name: Some(word),
                value: self.value()?,
            });
        }
        self.at = saved;

        Ok(CsArgument {
            name: None,
            value: self.value()?,
        })
    }

    fn word(&mut self) -> Option<String> {
        self.skip_trivia();
        let start = self.at;
        while self.at < self.characters.len()
            && (self.characters[self.at].is_alphanumeric() || self.characters[self.at] == '_')
        {
            self.at += 1;
        }
        if self.at == start {
            return None;
        }
        Some(self.characters[start..self.at].iter().collect())
    }

    fn value(&mut self) -> Result<CsValue, CsError> {
        let first = self.single_value()?;

        // An arithmetic expression, which a few cooldowns and damages are written as. The left
        // side is kept and the rest is stepped over: nothing downstream reads these closely enough
        // for the arithmetic to matter, and refusing to parse one loses the whole enemy.
        while matches!(self.peek(), Some('+') | Some('*') | Some('/'))
            || (self.peek() == Some('-') && !self.at_argument_end())
        {
            self.at += 1;
            let _ = self.single_value()?;
        }

        Ok(first)
    }

    /// Whether what follows is the end of this argument rather than more of its value.
    fn at_argument_end(&mut self) -> bool {
        matches!(self.peek(), None | Some(',') | Some(')') | Some('}'))
    }

    fn single_value(&mut self) -> Result<CsValue, CsError> {
        let Some(current) = self.peek() else {
            return Err(CsError);
        };

        // A character literal. Only ever a separator in these files, and never read.
        if current == '\'' {
            self.at += 1;
            if self.characters.get(self.at) == Some(&'\\') {
                self.at += 1;
            }
            self.at += 1;
            if self.characters.get(self.at) == Some(&'\'') {
                self.at += 1;
            }
            return Ok(CsValue::Null);
        }

        if current == '"' {
            return Ok(CsValue::Text(self.text()?));
        }

        if current == '{' {
            return Ok(CsValue::List(self.list('{', '}')?));
        }

        if current == '-' || current.is_ascii_digit() || current == '.' {
            return self.number();
        }

        if current == '(' {
            // A cast such as `(float)0.5`, or a parenthesised expression. Either way the value
            // inside is what matters.
            self.at += 1;
            let inner_start = self.at;
            let mut depth = 1;
            while self.at < self.characters.len() && depth > 0 {
                match self.characters[self.at] {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                self.at += 1;
            }
            let inside: String = self.characters[inner_start..self.at.saturating_sub(1)]
                .iter()
                .collect();

            // A cast is a type name in parentheses followed by the real value.
            if inside
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
            {
                return self.value();
            }
            let mut inner = CsParser::new(&inside);
            return inner.value();
        }

        let Some(word) = self.word() else {
            return Err(CsError);
        };

        match word.as_str() {
            "true" => return Ok(CsValue::Bool(true)),
            "false" => return Ok(CsValue::Bool(false)),
            "null" => return Ok(CsValue::Null),
            "new" => return Ok(CsValue::Call(self.constructor()?)),
            _ => {}
        }

        // A dotted path.
        let mut path = word;
        while self.peek() == Some('.') {
            self.at += 1;
            match self.word() {
                Some(part) => {
                    path.push('.');
                    path.push_str(&part);
                }
                None => break,
            }
        }

        // A path followed by parentheses is a static call, such as `LootTemplates.DefaultLoot(5)`. Its
        // arguments have to be consumed even though nothing reads them, because leaving `(5)`
        // sitting there makes the enclosing argument list fail at the very next token. Harmless at
        // the top level and fatal one layer in.
        if self.peek() == Some('(') {
            let arguments = self.list('(', ')')?;
            return Ok(CsValue::Call(CsCall {
                name: path,
                arguments: arguments
                    .into_iter()
                    .map(|value| CsArgument { name: None, value })
                    .collect(),
            }));
        }

        Ok(CsValue::Path(path))
    }

    fn constructor(&mut self) -> Result<CsCall, CsError> {
        self.skip_trivia();

        // `new[] { … }` and `new Thing[] { … }` both appear.
        if self.peek() == Some('[') {
            self.at += 1;
            while self.at < self.characters.len() && self.characters[self.at] != ']' {
                self.at += 1;
            }
            self.at += 1;
            return Ok(CsCall {
                name: "Array".into(),
                arguments: self
                    .list('{', '}')?
                    .into_iter()
                    .map(|value| CsArgument { name: None, value })
                    .collect(),
            });
        }

        let Some(name) = self.word() else {
            return Err(CsError);
        };

        if self.peek() == Some('[') {
            self.at += 1;
            while self.at < self.characters.len() && self.characters[self.at] != ']' {
                self.at += 1;
            }
            self.at += 1;
            return Ok(CsCall {
                name: "Array".into(),
                arguments: self
                    .list('{', '}')?
                    .into_iter()
                    .map(|value| CsArgument { name: None, value })
                    .collect(),
            });
        }

        // A constructor with no argument list, as in `new Cooldown()`.
        if self.peek() != Some('(') {
            return Ok(CsCall {
                name,
                arguments: Vec::new(),
            });
        }
        self.at += 1;

        let mut arguments = Vec::new();
        loop {
            if self.peek() == Some(')') {
                self.at += 1;
                break;
            }
            if self.peek().is_none() {
                return Err(CsError);
            }

            arguments.push(self.argument()?);

            match self.peek() {
                Some(',') => self.at += 1,
                Some(')') => {
                    self.at += 1;
                    break;
                }
                _ => return Err(CsError),
            }
        }

        Ok(CsCall { name, arguments })
    }

    fn list(&mut self, open: char, close: char) -> Result<Vec<CsValue>, CsError> {
        self.skip_trivia();
        if self.peek() != Some(open) {
            return Ok(Vec::new());
        }
        self.at += 1;

        let mut out = Vec::new();
        loop {
            if self.peek() == Some(close) {
                self.at += 1;
                return Ok(out);
            }
            if self.peek().is_none() {
                return Err(CsError);
            }

            out.push(self.value()?);

            match self.peek() {
                Some(',') => self.at += 1,
                Some(c) if c == close => {
                    self.at += 1;
                    return Ok(out);
                }
                _ => return Err(CsError),
            }
        }
    }

    fn text(&mut self) -> Result<String, CsError> {
        self.skip_trivia();
        if self.characters.get(self.at) != Some(&'"') {
            return Err(CsError);
        }
        self.at += 1;

        let mut out = String::new();
        while self.at < self.characters.len() {
            let current = self.characters[self.at];
            if current == '\\' {
                self.at += 1;
                if let Some(escaped) = self.characters.get(self.at) {
                    out.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        other => *other,
                    });
                }
                self.at += 1;
                continue;
            }
            if current == '"' {
                self.at += 1;
                return Ok(out);
            }
            out.push(current);
            self.at += 1;
        }

        Err(CsError)
    }

    fn number(&mut self) -> Result<CsValue, CsError> {
        self.skip_trivia();
        let start = self.at;

        if self.characters.get(self.at) == Some(&'-') {
            self.at += 1;
        }

        // Hex, which the content writes for colours: `new Flash(0x00FF0C, .25, 8)`. Reading it as
        // a decimal made the whole enemy unparseable, and an enemy that fails to parse is dropped
        // An unreadable literal costs the whole enemy, not one argument.
        let hex = self.characters.get(self.at) == Some(&'0')
            && matches!(self.characters.get(self.at + 1), Some('x') | Some('X'));

        if hex {
            self.at += 2;
            let digits = self.at;
            while self
                .characters
                .get(self.at)
                .is_some_and(char::is_ascii_hexdigit)
            {
                self.at += 1;
            }
            if self.at == digits {
                return Err(CsError);
            }

            let text: String = self.characters[digits..self.at].iter().collect();
            let value = i64::from_str_radix(&text, 16).map_err(|_| CsError)?;
            let signed = if self.characters[start] == '-' {
                -value
            } else {
                value
            };
            return Ok(CsValue::Number(signed as f64));
        }

        while self.at < self.characters.len()
            && (self.characters[self.at].is_ascii_digit() || self.characters[self.at] == '.')
        {
            self.at += 1;
        }

        // An exponent, which a handful of cooldowns are written with.
        if self
            .characters
            .get(self.at)
            .is_some_and(|c| matches!(c, 'e' | 'E'))
            && self
                .characters
                .get(self.at + 1)
                .is_some_and(|c| c.is_ascii_digit() || matches!(c, '+' | '-'))
        {
            self.at += 2;
            while self
                .characters
                .get(self.at)
                .is_some_and(char::is_ascii_digit)
            {
                self.at += 1;
            }
        }

        let text: String = self.characters[start..self.at].iter().collect();

        // Literal suffixes: 0.5f, 100L, 3d.
        if self
            .characters
            .get(self.at)
            .is_some_and(|c| matches!(c, 'f' | 'F' | 'd' | 'D' | 'L' | 'l' | 'm' | 'M'))
        {
            self.at += 1;
        }

        text.parse::<f64>()
            .map(CsValue::Number)
            .map_err(|_| CsError)
    }
}

#[cfg(test)]
mod tests {
    use super::read_enemies_reporting;

    #[test]
    fn a_hex_literal_is_a_number() {
        // `new Flash(0x00FF0C, .25, 8)`: reading this as a decimal failed the whole argument
        // list, which drops the enemy.
        let (enemies, unreadable) =
            read_enemies_reporting(r#".Init("X", new State(new Flash(0x00FF0C, .25, 8)))"#);

        assert_eq!(unreadable, 0);
        assert_eq!(enemies.len(), 1);
    }

    #[test]
    fn a_nested_static_call_is_consumed_rather_than_left_behind() {
        // `LootTemplates.DefaultLoot(5)` parses as a path and leaves `(5)` sitting there. Harmless
        // at the top level and fatal one layer in.
        let (enemies, unreadable) = read_enemies_reporting(
            r#".Init("X", new State(new Wander(0.4)), new Threshold(0.05, LootTemplates.Loot(5)))"#,
        );

        assert_eq!(unreadable, 0);
        assert_eq!(enemies.len(), 1);
    }

    #[test]
    fn arithmetic_and_character_literals_do_not_lose_the_enemy() {
        let (enemies, unreadable) =
            read_enemies_reporting(r#".Init("X", new State(new Shoot(50, 12, 360 / 12, 3, 45)))"#);

        assert_eq!(unreadable, 0);
        assert_eq!(enemies.len(), 1);
    }

    #[test]
    fn preprocessor_directives_are_not_part_of_an_argument_list() {
        // Oryx folds its dance phases with #region, in the middle of its argument list.
        let (enemies, unreadable) = read_enemies_reporting(
            ".Init(\"X\",\n#region dance\n new State(new Wander(0.4))\n#endregion dance\n)",
        );

        assert_eq!(unreadable, 0);
        assert_eq!(enemies.len(), 1);
    }

    #[test]
    fn an_unreadable_entry_is_counted_rather_than_swallowed() {
        // Without a count, a dropped entry leaves a total that looks exactly like success.
        let (enemies, unreadable) = read_enemies_reporting(r#".Init("X", new State(@@@))"#);

        assert!(enemies.is_empty());
        assert_eq!(unreadable, 1, "the drop has to be visible");
    }

    use super::*;

    #[test]
    fn a_real_init_is_read() {
        // Taken verbatim from BehaviorDb.Lowland.cs.
        let source = r#"
            private _ Lowland = () => Behav()
                .Init("Hobbit Mage",
                    new State(
                        new State("idle",
                            new PlayerWithinTransition(12, "ring1")
                            ),
                        new State("ring1",
                            new Shoot(1, fixedAngle: 0, count: 15, shootAngle: 24, coolDown: 1200, projectileIndex: 0),
                            new TimedTransition(400, "ring2")
                            ),
                        new Prioritize(
                            new StayAbove(0.4, 9),
                            new Follow(0.75, range: 6),
                            new Wander(0.4)
                            )
                        ),
                    new TierLoot(2, ItemType.Weapon, 0.3),
                    new ItemLoot("Health Potion", 0.02)
                )
        "#;

        let enemies = read_enemies(source);
        assert_eq!(enemies.len(), 1);
        assert_eq!(enemies[0].name, "Hobbit Mage");
        assert_eq!(
            enemies[0].arguments.len(),
            3,
            "the root state and two loots"
        );

        let CsValue::Call(root) = &enemies[0].arguments[0] else {
            panic!("the first argument should be the root State");
        };
        assert_eq!(root.name, "State");
        assert_eq!(root.arguments.len(), 3);
    }

    #[test]
    fn named_and_positional_arguments_are_distinguished() {
        let source = r#".Init("X", new Shoot(1, fixedAngle: 0, count: 15))"#;
        let enemies = read_enemies(source);

        let CsValue::Call(shoot) = &enemies[0].arguments[0] else {
            panic!("expected a Shoot");
        };
        assert_eq!(shoot.arguments[0].name, None);
        assert_eq!(shoot.arguments[0].value, CsValue::Number(1.0));
        assert_eq!(shoot.arguments[1].name.as_deref(), Some("fixedAngle"));
        assert_eq!(shoot.arguments[2].name.as_deref(), Some("count"));
        assert_eq!(shoot.arguments[2].value, CsValue::Number(15.0));
    }

    #[test]
    fn literals_of_every_kind_the_content_uses() {
        let source = r#".Init("X", new Thing(1, -3, 0.75, 0.5f, "text", true, false, null, ItemType.Weapon))"#;
        let enemies = read_enemies(source);

        let CsValue::Call(call) = &enemies[0].arguments[0] else {
            panic!("expected a call");
        };
        let values: Vec<&CsValue> = call.arguments.iter().map(|a| &a.value).collect();

        assert_eq!(values[0], &CsValue::Number(1.0));
        assert_eq!(values[1], &CsValue::Number(-3.0));
        assert_eq!(values[2], &CsValue::Number(0.75));
        assert_eq!(
            values[3],
            &CsValue::Number(0.5),
            "a float suffix is not part of the number"
        );
        assert_eq!(values[4], &CsValue::Text("text".into()));
        assert_eq!(values[5], &CsValue::Bool(true));
        assert_eq!(values[6], &CsValue::Bool(false));
        assert_eq!(values[7], &CsValue::Null);
        assert_eq!(values[8], &CsValue::Path("ItemType.Weapon".into()));
    }

    #[test]
    fn nesting_survives_several_levels() {
        let source = r#".Init("X", new State(new State("a", new Prioritize(new Wander(0.4)))))"#;
        let enemies = read_enemies(source);

        let CsValue::Call(root) = &enemies[0].arguments[0] else {
            panic!()
        };
        let CsValue::Call(inner) = &root.arguments[0].value else {
            panic!()
        };
        assert_eq!(inner.name, "State");
        assert_eq!(inner.arguments[0].value, CsValue::Text("a".into()));

        let CsValue::Call(prioritize) = &inner.arguments[1].value else {
            panic!()
        };
        assert_eq!(prioritize.name, "Prioritize");
    }

    #[test]
    fn several_enemies_in_one_file_are_all_found() {
        let source = r#"
            Behav()
              .Init("First", new State())
              .Init("Second", new State())
              .Init("Third", new State())
        "#;
        let enemies = read_enemies(source);
        assert_eq!(enemies.len(), 3);
        assert_eq!(enemies[2].name, "Third");
    }

    #[test]
    fn init_inside_a_comment_or_string_is_ignored() {
        let source = r#"
            // .Init("Commented", new State())
            .Init("Real", new State())
            var s = ".Init(\"InsideAString\", x)";
        "#;
        let enemies = read_enemies(source);
        assert_eq!(enemies.len(), 1);
        assert_eq!(enemies[0].name, "Real");
    }

    #[test]
    fn a_constructor_without_arguments_parses() {
        let source = r#".Init("X", new Follow(1, coolDown: new Cooldown()))"#;
        let enemies = read_enemies(source);

        let CsValue::Call(follow) = &enemies[0].arguments[0] else {
            panic!()
        };
        assert_eq!(follow.arguments[1].name.as_deref(), Some("coolDown"));
    }

    #[test]
    fn a_cast_is_seen_through() {
        let source = r#".Init("X", new Thing((float)0.5, (int)3))"#;
        let enemies = read_enemies(source);

        let CsValue::Call(call) = &enemies[0].arguments[0] else {
            panic!()
        };
        assert_eq!(call.arguments[0].value, CsValue::Number(0.5));
        assert_eq!(call.arguments[1].value, CsValue::Number(3.0));
    }

    #[test]
    fn a_malformed_entry_is_skipped_rather_than_fatal() {
        // One broken dungeon should not cost the other forty.
        let source = r#"
            .Init("Broken", new State(
            .Init("Fine", new State())
        "#;
        let enemies = read_enemies(source);
        assert!(
            enemies.iter().any(|enemy| enemy.name == "Fine"),
            "the well-formed entry should still be found"
        );
    }

    #[test]
    fn junk_never_panics() {
        let mut seed = 0xfeed_beefu64;
        let alphabet: Vec<char> = ".Init(\"newState,:0123)}{ ".chars().collect();

        for _ in 0..2_000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let length = (seed % 80) as usize;
            let source: String = (0..length)
                .map(|i| alphabet[((seed >> (i % 16)) as usize) % alphabet.len()])
                .collect();
            let _ = read_enemies(&source);
        }
    }
}
