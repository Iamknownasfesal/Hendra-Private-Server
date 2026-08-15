//! Converting the C# behaviour database into this language.
//!
//! Runs once, over 23,000 lines. Hand-porting that much content would dominate the schedule and
//! drift silently, since a mistyped cooldown in one dungeon is invisible until someone plays it, so the
//! conversion is mechanical and the output is checked by parsing it back.
//!
//! # What the mapping is
//!
//! Type names become snake case (`PlayerWithinTransition` → `player_within`), argument names the
//! same (`coolDown` → `cooldown`), and a handful are renamed outright where the C# name describes
//! the implementation rather than the intent. Everything else is carried through unchanged, so a
//! primitive this runtime does not implement still appears in the output, still with its arguments,
//! ready to be implemented later without another pass over the C#.

use std::fmt::Write;

use crate::csharp::{CsCall, CsEnemy, CsValue};

/// What a conversion produced.
#[derive(Debug, Default)]
pub struct Report {
    pub enemies: usize,

    /// Behaviour and transition names encountered, with how often.
    pub primitives: Vec<(String, usize)>,

    /// Enemies that could not be converted, and why.
    pub skipped: Vec<(String, String)>,

    /// `.Init(` entries whose C# could not be read at all.
    ///
    /// Distinct from `skipped`, which knows the enemy's name and gave up later. These are not even
    /// named, because the name is inside the part that would not parse. Counted rather than
    /// ignored: a silent zero here is how three files went missing behind a total that looked
    /// exactly like success.
    pub unreadable: usize,

    /// Transitions dropped because they named a state that does not exist.
    ///
    /// Bugs in the original content, not in the conversion. The C# resolves a target by dictionary
    /// lookup with no fallback, so one of these throws a KeyNotFoundException the moment its enemy
    /// spawns.
    pub dangling: Vec<(String, String)>,

    /// States renamed because their enemy declared the same name twice.
    pub renamed: Vec<(String, String)>,
}

impl Report {
    fn count(&mut self, name: &str) {
        match self.primitives.iter_mut().find(|(known, _)| known == name) {
            Some((_, seen)) => *seen += 1,
            None => self.primitives.push((name.to_string(), 1)),
        }
    }

    /// Names in descending order of use.
    pub fn by_use(&self) -> Vec<(String, usize)> {
        let mut sorted = self.primitives.clone();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        sorted
    }
}

/// Converts one C# source file.
pub fn transpile(source: &str, report: &mut Report) -> String {
    let mut out = String::new();

    // Commented-out code is not code. Two enemies in the database are written inside block
    // comments, and converting them made one of them a second definition of a boss that already
    // exists: whichever of the two the host happened to keep decided how that fight went.
    let source = strip_comments(source);

    let (enemies, unreadable) = crate::csharp::read_enemies_reporting(&source);
    report.unreadable += unreadable;

    for enemy in enemies {
        match emit_enemy(&enemy, report) {
            Ok(text) => {
                out.push_str(&text);
                out.push('\n');
                report.enemies += 1;
            }
            Err(reason) => report.skipped.push((enemy.name.clone(), reason)),
        }
    }

    out
}

/// Removes C# comments, leaving everything else where it was.
///
/// Replaces each comment with spaces rather than deleting it, so every byte that is kept stays at
/// the offset it had: the reader reports positions, and a position into a shortened copy points at
/// the wrong line.
///
/// String literals are respected, or a URL in a message would swallow the rest of its line.
fn strip_comments(source: &str) -> String {
    let bytes: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());

    let mut at = 0;
    let mut in_string = false;
    let mut in_char = false;

    while at < bytes.len() {
        let here = bytes[at];
        let next = bytes.get(at + 1).copied();

        if in_string || in_char {
            out.push(here);

            // An escaped quote does not end the literal, and neither does the character after it.
            if here == '\\'
                && let Some(escaped) = next
            {
                out.push(escaped);
                at += 2;
                continue;
            }

            if (in_string && here == '"') || (in_char && here == '\'') {
                in_string = false;
                in_char = false;
            }

            at += 1;
            continue;
        }

        match (here, next) {
            ('/', Some('/')) => {
                while at < bytes.len() && bytes[at] != '\n' {
                    out.push(' ');
                    at += 1;
                }
            }

            ('/', Some('*')) => {
                let mut depth = 1;
                out.push_str("  ");
                at += 2;

                while at < bytes.len() && depth > 0 {
                    if bytes[at] == '*' && bytes.get(at + 1) == Some(&'/') {
                        depth -= 1;
                        out.push_str("  ");
                        at += 2;
                        continue;
                    }

                    // Newlines are kept so line numbers still count.
                    out.push(if bytes[at] == '\n' { '\n' } else { ' ' });
                    at += 1;
                }
            }

            _ => {
                if here == '"' {
                    in_string = true;
                }
                if here == '\'' {
                    in_char = true;
                }
                out.push(here);
                at += 1;
            }
        }
    }

    out
}

/// The naming decisions for one enemy, made before anything is emitted.
struct Naming {
    /// Each state occurrence, in resolve order, with the name it will be emitted under.
    occurrences: Vec<String>,
    /// Every name a transition may legitimately target.
    valid: Vec<String>,
    seen: usize,
}

impl Naming {
    /// Works out what each state will be called.
    ///
    /// The C# resolves states with `states[Name] = this`, so a name declared twice keeps the *last*
    /// one and the earlier become unreachable. Reproducing that means renaming the earlier
    /// occurrences and leaving the final one with the original name.
    fn plan(enemy: &CsEnemy, report: &mut Report) -> Naming {
        let mut names = Vec::new();
        for argument in &enemy.arguments {
            collect_state_names(argument, &mut names);
        }

        let mut occurrences = Vec::with_capacity(names.len());
        for (index, name) in names.iter().enumerate() {
            let shadowed = names[index + 1..].iter().any(|other| other == name);
            if shadowed {
                let suffix = names[..index].iter().filter(|other| *other == name).count() + 2;
                let renamed = format!("{name}_{suffix}");
                report
                    .renamed
                    .push((enemy.name.clone(), format!("{name} -> {renamed}")));
                occurrences.push(renamed);
            } else {
                occurrences.push(name.clone());
            }
        }

        let valid = occurrences.clone();
        Naming {
            occurrences,
            valid,
            seen: 0,
        }
    }

    fn next_name(&mut self) -> String {
        let name = self
            .occurrences
            .get(self.seen)
            .cloned()
            .unwrap_or_else(|| "unnamed".to_string());
        self.seen += 1;
        name
    }
}

/// Gathers state names in the order the C# resolves them: depth-first, parents before children.
fn collect_state_names(value: &CsValue, into: &mut Vec<String>) {
    let CsValue::Call(call) = value else { return };

    if call.name == "State" {
        if let Some(CsValue::Text(name)) = call.arguments.first().map(|a| &a.value) {
            into.push(identifier(name));
        }
        for child in &call.arguments {
            collect_state_names(&child.value, into);
        }
        return;
    }

    if group_name(&call.name).is_some() {
        for child in &call.arguments {
            collect_state_names(&child.value, into);
        }
    }
}

/// The emitted name of a wrapper whose arguments are behaviours, if it is one.
fn group_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "Prioritize" => "prioritize",
        "Sequence" => "sequence",
        "Timed" => "timed",
        "If" => "if",
        "OnDeathBehavior" => "on_death_behavior",
        _ => return None,
    })
}

fn emit_enemy(enemy: &CsEnemy, report: &mut Report) -> Result<String, String> {
    let mut naming = Naming::plan(enemy, report);
    let mut out = String::new();
    writeln!(out, "enemy {:?} {{", enemy.name).expect("writing to a string");

    let mut loot = Vec::new();
    let mut wrote_body = false;

    for argument in &enemy.arguments {
        match argument {
            CsValue::Call(call) if call.name == "State" => {
                // The root State's contents become the enemy's body directly.
                for item in &call.arguments {
                    if let Some(text) = emit_item(&item.value, 1, report, &enemy.name, &mut naming)
                    {
                        out.push_str(&text);
                        wrote_body = true;
                    }
                }
            }

            CsValue::Call(call) if is_loot(&call.name) => loot.push(call.clone()),

            // A behaviour written directly on the enemy rather than inside a State.
            CsValue::Call(call) => {
                if let Some(text) = emit_item(argument, 1, report, &enemy.name, &mut naming) {
                    out.push_str(&text);
                    wrote_body = true;
                }
                let _ = call;
            }

            _ => {}
        }
    }

    if !loot.is_empty() {
        writeln!(out, "    loot {{").expect("writing to a string");
        for entry in &loot {
            writeln!(out, "        {}", emit_loot(entry)).expect("writing to a string");
        }
        writeln!(out, "    }}").expect("writing to a string");
        wrote_body = true;
    }

    if !wrote_body {
        return Err("nothing convertible in this entry".into());
    }

    out.push_str("}\n");
    Ok(out)
}

/// One item inside a state: a nested state, a transition, a group, or a behaviour.
fn emit_item(
    value: &CsValue,
    depth: usize,
    report: &mut Report,
    enemy: &str,
    naming: &mut Naming,
) -> Option<String> {
    let CsValue::Call(call) = value else {
        return None;
    };
    let pad = "    ".repeat(depth);

    if call.name == "State" {
        // A State's first argument is its name when it has one; anonymous states exist and are
        // flattened into their parent, because this language has no way to enter something unnamed.
        let (name, rest) = match call.arguments.first().map(|argument| &argument.value) {
            Some(CsValue::Text(_)) => (Some(naming.next_name()), &call.arguments[1..]),
            _ => (None, &call.arguments[..]),
        };

        return match name {
            Some(name) => {
                let mut body = String::new();
                for item in rest {
                    if let Some(text) = emit_item(&item.value, depth + 1, report, enemy, naming) {
                        body.push_str(&text);
                    }
                }
                Some(format!("{pad}state {name} {{\n{body}{pad}}}\n"))
            }
            None => {
                // Anonymous: its contents belong to the enclosing state, reindented one level out.
                let mut flattened = String::new();
                for item in rest {
                    if let Some(text) = emit_item(&item.value, depth, report, enemy, naming) {
                        flattened.push_str(&text);
                    }
                }
                Some(flattened)
            }
        };
    }

    if is_transition(&call.name) {
        return emit_transition(call, &pad, report, enemy, naming);
    }

    // The wrappers whose arguments are behaviours rather than settings. Emitting one without its
    // block loses everything inside it, which is how `on_death_behavior()` ended up empty.
    if let Some(group) = group_name(&call.name) {
        report.count(group);
        let mut body = String::new();

        // `Timed` and `OnDeathBehavior` take a leading number or nothing; anything that is not a
        // constructor is a setting rather than a child.
        let mut settings = Vec::new();
        for child in &call.arguments {
            match &child.value {
                CsValue::Call(_) => {
                    if let Some(text) = emit_item(&child.value, depth + 1, report, enemy, naming) {
                        body.push_str(&text);
                    }
                }
                other => settings.push(other.clone()),
            }
        }

        let head = if settings.is_empty() {
            format!("{group}()")
        } else {
            let written: Vec<String> = settings.iter().filter_map(emit_value).collect();
            format!("{group}({})", written.join(", "))
        };
        return Some(format!("{pad}{head} {{\n{body}{pad}}}\n"));
    }

    if is_loot(&call.name) {
        return None;
    }

    let name = snake(&call.name);
    report.count(&name);
    Some(format!("{pad}{}\n", emit_call(&name, call)))
}

/// A transition, and the state it leads to.
///
/// Usually the target is the last string argument, because the C# signatures end with it. Two do
/// not: `EntitiesNotExistsTransition(dist, targetState, params targets)` and
/// `TimedRandomTransition(time, randomised, params states)` both take a variadic tail, so the last
/// string is an entity name or an alternative rather than the destination.
///
/// Reading "the last string" for those produced transitions pointing at entity names, which the
/// parser then rejected as unknown states, in eight separate dungeons. It would otherwise have been
/// a boss that simply never changed phase.
fn emit_transition(
    call: &CsCall,
    pad: &str,
    report: &mut Report,
    enemy: &str,
    naming: &Naming,
) -> Option<String> {
    let strings: Vec<&str> = call
        .arguments
        .iter()
        .filter_map(|argument| argument.value.as_text())
        .collect();

    // Which string is the target state depends on the transition, because the C# constructors put
    // it in different places. Guessing "the last one" is right for most and wrong for these, and
    // wrong here does not read as wrong: the target comes out as some other word, no state has that
    // name, and the transition is dropped as though the original were at fault.
    let target = match call.name.as_str() {
        "EntitiesNotExistsTransition"
        | "EntitiesNotExistTransition"
        | "TimedRandomTransition"
        | "PlayerTextTransition" => strings.first(),
        _ => strings.last(),
    }?
    .to_string();

    let target = identifier(&target);

    // A target naming no state is a bug in the original: the C# looks targets up in a dictionary
    // with no fallback, so this throws the moment its enemy spawns. Dropping the transition keeps
    // the rest of the enemy working and the report says which went.
    if !naming.valid.contains(&target) {
        report
            .dangling
            .push((enemy.to_string(), format!("{} -> {target}", call.name)));
        return None;
    }

    let name = transition_name(&call.name);
    report.count(&name);

    // Everything except the target is a condition argument.
    let kept: Vec<&crate::csharp::CsArgument> = call
        .arguments
        .iter()
        .filter(|argument| argument.value.as_text() != Some(target.as_str()))
        .collect();

    let arguments = kept
        .iter()
        .filter_map(|argument| emit_argument(argument))
        .collect::<Vec<_>>()
        .join(", ");

    Some(format!("{pad}on {name}({arguments}) -> {target}\n"))
}

fn emit_call(name: &str, call: &CsCall) -> String {
    let arguments = call
        .arguments
        .iter()
        .filter_map(emit_argument)
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}({arguments})")
}

fn emit_argument(argument: &crate::csharp::CsArgument) -> Option<String> {
    let value = emit_value(&argument.value)?;
    Some(match &argument.name {
        Some(name) => format!("{}: {value}", argument_name(name)),
        None => value,
    })
}

/// The language's name for an argument.
///
/// Most convert mechanically, but a few C# names would snake-case into something this language does
/// not use. `coolDown` becomes `cool_down`, not `cooldown`. A misspelled argument falls back to a
/// default, so the enemy still works and is simply wrong: a shot fires every 1000ms where the
/// content asked for 1200.
fn argument_name(csharp: &str) -> String {
    match csharp {
        "coolDown" => "cooldown".to_string(),
        "projectileIndex" => "projectile".to_string(),
        "coolDownOffset" => "cooldown_offset".to_string(),
        other => snake(other),
    }
}

fn emit_value(value: &CsValue) -> Option<String> {
    Some(match value {
        CsValue::Number(number) => {
            if number.fract() == 0.0 {
                format!("{}", *number as i64)
            } else {
                format!("{number}")
            }
        }
        CsValue::Text(text) => format!("{text:?}"),
        CsValue::Bool(value) => value.to_string(),

        // A null argument means "left at its default", which this language spells by omission.
        CsValue::Null => return None,

        // `ItemType.Weapon` becomes `weapon`: the qualifier carries no information here.
        CsValue::Path(path) => {
            let tail = path.rsplit('.').next().unwrap_or(path);
            snake(tail)
        }

        // `new[] { "a", "b" }`, which the C# reader hands back as a call named `Array`, and the
        // bare `{ "a", "b" }` of a collection initialiser. Both become a bracketed list, so a
        // constructor taking two arrays keeps the boundary between them.
        CsValue::Call(array) if array.name == "Array" => {
            let items: Vec<String> = array
                .arguments
                .iter()
                .filter_map(|argument| emit_value(&argument.value))
                .collect();
            format!("[{}]", items.join(", "))
        }
        CsValue::List(items) => {
            let items: Vec<String> = items.iter().filter_map(emit_value).collect();
            format!("[{}]", items.join(", "))
        }

        // Any other nested call as an argument, usually a Cooldown, has no representation, so it is
        // dropped and the default applies.
        CsValue::Call(_) => return None,
    })
}

fn emit_loot(call: &CsCall) -> String {
    // `Threshold(share, children...)` carries both a number and a list, and dropping either loses
    // the whole entry: the share decides who is eligible and the children are what they get. Before
    // this it emitted a bare `threshold`, and two hundred of the content's soulbound drops went
    // nowhere.
    if call.name == "Threshold" {
        let share = call
            .arguments
            .first()
            .and_then(|argument| emit_value(&argument.value))
            .unwrap_or_else(|| "0".to_string());

        let children: Vec<String> = call
            .arguments
            .iter()
            .skip(1)
            .flat_map(|argument| match &argument.value {
                CsValue::Call(inner) if is_loot(&inner.name) => vec![emit_loot(inner)],

                // A named bundle, which stands in for a list of entries rather than being one.
                CsValue::Call(inner) => loot_template(&inner.name).unwrap_or_default(),

                _ => Vec::new(),
            })
            .collect();

        return format!("threshold({share}) {{ {} }}", children.join(" "));
    }

    let name = match call.name.as_str() {
        "ItemLoot" => "item",
        "TierLoot" => "tier",
        other => return snake(other),
    };
    emit_call(name, call)
}

fn is_loot(name: &str) -> bool {
    name.ends_with("Loot") || name == "Threshold"
}

/// The six stat potions of `LootTemplates.StatPots()`, in the order the original lists them.
const STAT_POTS: [&str; 6] = [
    "Potion of Defense",
    "Potion of Attack",
    "Potion of Speed",
    "Potion of Vitality",
    "Potion of Wisdom",
    "Potion of Dexterity",
];

/// The entries a named drop bundle stands for, or `None` if the name is not one of them.
///
/// `logic/loot/LootTemplates.cs` returns a `MobDrops[]` that C# splats into the enclosing
/// `Threshold`'s parameter array, so a bundle is a list of entries rather than a single one and has
/// to be spliced in the same way here. Reading it as one unrecognised entry dropped it, which is how
/// three bosses each lost their six potions and were left holding an empty threshold.
///
/// `StatPots` is the six stat potions at a sixth of a chance each (`.cs:27-40`). The eight `Sor*`
/// bundles and `RaidTokens` return an empty array in the original (`.cs:49-66`), so a threshold
/// holding one of them really does drop nothing, and expanding to nothing is the parity answer
/// rather than a gap.
///
/// # Whose rule this is
///
/// `LootTemplates.cs` is not the shipped game's. It was written for this repository when the
/// dungeon scripts were imported from another fork, to stand in for that fork's `OnlyOne`, which
/// picks one entry out of a bundle and which no version of this server has. Six independent sixths
/// give the same expected potion count and a slightly different shape — very occasionally two at
/// once, where `OnlyOne` never gives two.
///
/// Reproduced as written rather than as `OnlyOne` would have it, because the C# server is the
/// specification this crate is measured against and that is what the C# server does; implementing
/// `OnlyOne` here would make the two disagree, and there is no `OnlyOne` in the tree to implement
/// it from. Worth knowing before citing these numbers as the game's.
fn loot_template(name: &str) -> Option<Vec<String>> {
    let bundle = name.strip_prefix("LootTemplates.")?;

    Some(match bundle {
        "StatPots" => {
            let each = emit_value(&CsValue::Number(1.0 / 6.0))?;
            STAT_POTS
                .iter()
                .map(|item| format!("item({item:?}, {each})"))
                .collect()
        }

        "SorRare" | "SorUncommon" | "SorCommon" | "Sor1Perc" | "Sor2Perc" | "Sor3Perc"
        | "Sor4Perc" | "Sor5Perc" | "RaidTokens" => Vec::new(),

        _ => return None,
    })
}

fn is_transition(name: &str) -> bool {
    name.ends_with("Transition") || name.ends_with("Transitions")
}

/// The language's name for a transition.
///
/// The C# names describe their implementation. `HpLessTransition` tests a *fraction*, not an
/// amount, so a few are renamed to say what they mean.
fn transition_name(csharp: &str) -> String {
    let trimmed = csharp
        .trim_end_matches("Transitions")
        .trim_end_matches("Transition");

    match trimmed {
        "HpLess" => "hp_below".to_string(),
        "Timed" => "timed".to_string(),
        "PlayerWithin" => "player_within".to_string(),
        "NoPlayerWithin" => "no_player_within".to_string(),
        other => snake(other),
    }
}

/// Converts a C# name to snake case.
/// The snake case a C# name converts to, for the parity check in `compile`.
#[cfg(test)]
pub(crate) fn snake_for_test(name: &str) -> String {
    snake(name)
}

fn snake(name: &str) -> String {
    let mut out = String::new();
    for (index, character) in name.chars().enumerate() {
        if character.is_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.extend(character.to_lowercase());
        } else {
            out.push(character);
        }
    }
    out
}

/// Makes a state name usable as an identifier.
///
/// State names in the C# are free text, with `"ring 1"`, `"phase-2"` and `"1"` all occurring, so this
/// language's identifiers are not.
fn identifier(name: &str) -> String {
    let mut out = String::new();
    for character in name.chars() {
        if character.is_alphanumeric() || character == '_' {
            out.push(character);
        } else {
            out.push('_');
        }
    }

    if out.is_empty() {
        return "unnamed".into();
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, 's');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::compile;
    use crate::parse::parse;

    const HOBBIT: &str = r#"
        .Init("Hobbit Mage",
            new State(
                new State("idle",
                    new PlayerWithinTransition(12, "ring1")
                    ),
                new State("ring1",
                    new Shoot(1, fixedAngle: 0, count: 15, shootAngle: 24, coolDown: 1200, projectileIndex: 0),
                    new TimedTransition(400, "ring2")
                    ),
                new State("ring2",
                    new Shoot(1, fixedAngle: 8, count: 15, shootAngle: 24, coolDown: 1200, projectileIndex: 1),
                    new TimedTransition(400, "idle")
                    ),
                new Prioritize(
                    new StayAbove(0.4, 9),
                    new Follow(0.75, range: 6),
                    new Wander(0.4)
                    ),
                new Spawn("Hobbit Archer", maxChildren: 4, coolDown: 12000, givesNoXp: false)
                ),
            new TierLoot(2, ItemType.Weapon, 0.3),
            new ItemLoot("Health Potion", 0.02)
        )
    "#;

    #[test]
    fn an_enemy_written_inside_a_comment_is_not_an_enemy() {
        // Two in the database are, and one of them is a second definition of a boss that already
        // exists: converting it meant whichever copy the host happened to keep decided that fight.
        let source = r#"
            /*
            .Init("Ghost Of The Comment",
                new State(
                    new State("idle", new Wander(0.4))
                    )
                )
            */
            .Init("Real Enemy",
                new State(
                    new State("idle", new Wander(0.4))
                    )
                )
        "#;

        let mut report = Report::default();
        let text = transpile(source, &mut report);

        assert!(text.contains(r#"enemy "Real Enemy""#));
        assert!(
            !text.contains("Ghost Of The Comment"),
            "a commented-out enemy was converted into a real one"
        );
    }

    #[test]
    fn a_line_comment_takes_the_rest_of_its_line_and_no_more() {
        let source = r#"
            .Init("Real Enemy",
                new State(
                    // new State("commented", new Wander(0.4)),
                    new State("idle", new Wander(0.4))
                    )
                )
        "#;

        let mut report = Report::default();
        let text = transpile(source, &mut report);

        assert!(text.contains("idle"));
        assert!(!text.contains("commented"));
    }

    #[test]
    fn a_comment_marker_inside_a_string_is_part_of_the_string() {
        // Or a taunt with a web address in it swallows the rest of its line, and the enemy loses
        // whatever was written after it.
        let source = r#"
            .Init("Real Enemy",
                new State(
                    new State("idle",
                        new Taunt("visit http://example.com/help"),
                        new Wander(0.4)
                        )
                    )
                )
        "#;

        let mut report = Report::default();
        let text = transpile(source, &mut report);

        assert!(text.contains("example.com"), "the address was cut: {text}");
        assert!(text.contains("wander"), "what followed it was lost: {text}");
    }

    #[test]
    fn the_target_state_is_taken_from_where_the_transition_actually_puts_it() {
        // The C# constructors do not agree on where the target state goes. Taking the last string
        // is right for most of them and wrong for these, and wrong here does not read as wrong: the
        // target comes out as some other word, no state has that name, and the transition is
        // dropped as though the original were at fault. That is exactly what happened to the eight
        // `PlayerTextTransition` uses in the content, which took Draconis' three dragon souls with
        // them.
        let source = r#"
            .Init("Talker",
                new State(
                    new State("waiting",
                        new PlayerTextTransition("goToRed", "Red", 99, false, true),
                        new EntitiesNotExistsTransition("alone", 10, "Guardian"),
                        new TimedRandomTransition("wander", 1000, 2000, false)
                        ),
                    new State("goToRed"),
                    new State("alone"),
                    new State("wander")
                    )
                )
        "#;

        let mut report = Report::default();
        let text = transpile(source, &mut report);

        for target in ["goToRed", "alone", "wander"] {
            assert!(
                text.contains(&format!("-> {target}")),
                "the target was not read from the front: {text}"
            );
        }

        // And it has to survive compiling, or a transition naming a state that does not exist is
        // dropped later and just as silently.
        let behaviours = parse(&text).expect("the output parses");
        let (programs, diagnostics) = compile(&behaviours);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(programs.programs.len(), 1);
    }

    #[test]
    fn a_real_enemy_converts_and_parses_back() {
        // The property that matters: output this language accepts. A transpiler whose output does
        // not parse has converted nothing.
        let mut report = Report::default();
        let text = transpile(HOBBIT, &mut report);

        assert_eq!(report.enemies, 1);
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);

        let parsed =
            parse(&text).unwrap_or_else(|err| panic!("output should parse: {err}\n{text}"));
        let enemy = parsed.get("Hobbit Mage").expect("by name");

        assert_eq!(enemy.root.states().count(), 3);
        assert_eq!(enemy.loot.len(), 2);
    }

    #[test]
    fn the_converted_enemy_compiles_and_runs() {
        let mut report = Report::default();
        let text = transpile(HOBBIT, &mut report);

        let parsed = parse(&text).unwrap();
        let (programs, _) = compile(&parsed);
        let program = programs.get("Hobbit Mage").expect("compiled");

        assert!(program.state_named("idle").is_some());
        assert!(program.state_named("ring1").is_some());

        // And the shot survived with its numbers intact.
        let ring1 = program.state_named("ring1").unwrap();
        match &program.states[ring1].behaviours[0] {
            crate::program::Primitive::Shoot {
                count,
                spread,
                cooldown_ms,
                projectile,
                ..
            } => {
                assert_eq!(*count, 15);
                assert_eq!(*spread, 24.0);
                assert_eq!(*cooldown_ms, 1200);
                assert_eq!(*projectile, 0);
            }
            other => panic!("expected a shoot, got {other:?}"),
        }
    }

    #[test]
    fn names_become_snake_case() {
        assert_eq!(snake("Shoot"), "shoot");
        assert_eq!(snake("StayAbove"), "stay_above");
        assert_eq!(snake("coolDown"), "cool_down");
        assert_eq!(snake("maxChildren"), "max_children");
        assert_eq!(snake("projectileIndex"), "projectile_index");
    }

    #[test]
    fn transitions_are_renamed_where_the_csharp_name_misleads() {
        // HpLessTransition tests a fraction, so `hp_below` says what it does.
        assert_eq!(transition_name("HpLessTransition"), "hp_below");
        assert_eq!(transition_name("PlayerWithinTransition"), "player_within");
        assert_eq!(
            transition_name("EntityNotExistsTransition"),
            "entity_not_exists"
        );
    }

    #[test]
    fn awkward_state_names_become_identifiers() {
        assert_eq!(identifier("ring1"), "ring1");
        assert_eq!(identifier("ring 1"), "ring_1");
        assert_eq!(identifier("phase-2"), "phase_2");
        assert_eq!(identifier("1"), "s1", "a name cannot start with a digit");
        assert_eq!(identifier(""), "unnamed");
    }

    #[test]
    fn an_anonymous_state_is_flattened_into_its_parent() {
        // Anonymous States exist in the content and there is no way to enter one, so their contents
        // belong to the enclosing state.
        let source = r#".Init("X", new State(new State(new Wander(0.4)), new State("named", new Wander(0.5))))"#;
        let mut report = Report::default();
        let text = transpile(source, &mut report);

        let parsed = parse(&text).expect("should parse");
        let enemy = parsed.get("X").unwrap();

        assert_eq!(
            enemy.root.states().count(),
            1,
            "only the named one is a state"
        );
        assert_eq!(
            enemy.root.behaviours().count(),
            1,
            "the anonymous one's contents moved up"
        );
    }

    #[test]
    fn a_null_argument_is_omitted_so_the_default_applies() {
        let source = r#".Init("X", new State(new Orbit(0.6, 4, target: null)))"#;
        let mut report = Report::default();
        let text = transpile(source, &mut report);

        assert!(
            !text.contains("null"),
            "null should not reach the output: {text}"
        );
        assert!(parse(&text).is_ok());
    }

    #[test]
    fn enum_paths_lose_their_qualifier() {
        let source = r#".Init("X", new State(), new TierLoot(2, ItemType.Weapon, 0.3))"#;
        let mut report = Report::default();
        let text = transpile(source, &mut report);

        assert!(text.contains("tier(2, weapon, 0.3)"), "{text}");
    }

    #[test]
    fn the_report_counts_what_it_saw() {
        let mut report = Report::default();
        transpile(HOBBIT, &mut report);

        let counts = report.by_use();
        let shoot = counts.iter().find(|(name, _)| name == "shoot").unwrap();
        assert_eq!(shoot.1, 2, "two states shoot");

        assert!(counts.iter().any(|(name, _)| name == "prioritize"));
        assert!(counts.iter().any(|(name, _)| name == "stay_above"));
    }

    #[test]
    fn an_entry_with_nothing_convertible_is_reported_not_emitted() {
        let mut report = Report::default();
        let text = transpile(r#".Init("Empty")"#, &mut report);

        assert!(text.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].0, "Empty");
    }

    #[test]
    fn a_named_drop_bundle_is_spliced_into_the_threshold_that_holds_it() {
        // `Threshold(0.05, LootTemplates.StatPots())` is six entries in the original, not one and
        // not none. Dropping the call left `threshold(0.05) { }` on three bosses, each of which
        // then had a soulbound bag that could contain nothing at all.
        let mut report = Report::default();
        let text = transpile(
            r#".Init("X", new State(new Wander(0.4)), new Threshold(0.05, LootTemplates.StatPots()))"#,
            &mut report,
        );

        for potion in STAT_POTS {
            assert!(text.contains(potion), "{potion} is missing from:\n{text}");
        }
    }

    #[test]
    fn an_empty_drop_bundle_stays_empty() {
        // The fork-currency bundles return `new MobDrops[0]`, so a threshold holding one really
        // does drop nothing. Inventing an item for them would be worse than the gap.
        let mut report = Report::default();
        let text = transpile(
            r#".Init("X", new State(new Wander(0.4)), new Threshold(0.05, LootTemplates.Sor3Perc()))"#,
            &mut report,
        );

        assert!(text.contains("threshold(0.05) {  }"), "{text}");
    }

    #[test]
    fn every_bundle_the_original_offers_has_an_answer_here() {
        // The table above is a copy of someone else's file, so it goes stale the moment a bundle is
        // added. This reads the original's own list and demands a verdict for each name, because a
        // bundle with no entry here is silently dropped rather than reported.
        const TEMPLATES: &str = "../../../Server-Side/wServer/logic/loot/LootTemplates.cs";

        let Ok(source) = std::fs::read_to_string(TEMPLATES) else {
            eprintln!("skipping: the original's sources are not where the test looks for them");
            return;
        };

        let mut found = 0usize;
        for line in source.lines() {
            let Some(after) = line.split_once("public static MobDrops[] ") else {
                continue;
            };
            let Some(name) = after.1.split('(').next() else {
                continue;
            };
            found += 1;
            assert!(
                loot_template(&format!("LootTemplates.{name}")).is_some(),
                "LootTemplates.{name} has no expansion, so every threshold holding it drops it"
            );
        }

        assert!(found > 0, "no bundles were read out of {TEMPLATES}");
    }

    /// Every `.beh` the server loads is the converter's own output over the original's `logic/db`.
    ///
    /// The shipped content is a generated artefact with no marking that says so, which invites the
    /// repair that is applied to the artefact instead of to the converter: the next conversion then
    /// silently undoes it. Comparing the two here is what makes that repair impossible to leave
    /// half-done, and it is also what caught a second, stale copy of the same files sitting beside
    /// the C# they came from, still holding two enemies the original has commented out.
    #[test]
    fn the_shipped_behaviours_are_what_the_converter_produces() {
        const CSHARP: &str = "../../../Server-Side/wServer/logic/db";
        const SHIPPED: &str = "../../content/behaviours";

        let Ok(entries) = std::fs::read_dir(CSHARP) else {
            eprintln!("skipping: the original's sources are not where the test looks for them");
            return;
        };

        let mut sources: Vec<std::path::PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("cs"))
            .collect();
        sources.sort();
        assert!(!sources.is_empty(), "no C# was found in {CSHARP}");

        let mut report = Report::default();
        let mut wrong = Vec::new();
        let mut checked = 0usize;

        for path in &sources {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let converted = transpile(&text, &mut report);
            if converted.trim().is_empty() {
                continue;
            }

            let name = format!(
                "{}.beh",
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .replace("BehaviorDb.", "")
                    .to_lowercase()
            );

            match std::fs::read_to_string(std::path::Path::new(SHIPPED).join(&name)) {
                Ok(shipped) if shipped == converted => checked += 1,
                Ok(_) => wrong.push(format!("{name} differs from what the converter produces")),
                Err(_) => wrong.push(format!("{name} is not in {SHIPPED} at all")),
            }
        }

        assert!(
            wrong.is_empty(),
            "the shipped behaviours are hand-edited or stale; re-run \
             `cargo run --release -p hendra-behavior --example convert -- content/behaviours`:\n  {}",
            wrong.join("\n  ")
        );
        assert!(checked > 0, "nothing was compared");
    }

    /// `Server-Side/wServer/logic/db` also holds a `.beh` beside each `.cs`, and nothing compares
    /// them to anything on purpose.
    ///
    /// Those files are an old snapshot of our own converter's output, committed into the C# tree in
    /// August 2026. Only the `.cs` there is the original — and even those were imported from a
    /// different fork rather than shipped in 2020, which `docs/audit/11-the-reference-itself.md`
    /// records file by file. The server reads `content/behaviours`, which the test above holds to
    /// the converter's current output; the copy beside the C# is read by nothing and is expected to
    /// fall behind as the converter improves.
    ///
    /// There is deliberately no test tying the two together. One that demanded they agree would
    /// have to be satisfied by writing into a tree this project treats as read-only, which is the
    /// trap: the specification must never be edited to match us, and a green test is not worth
    /// making that the easy way out.
    #[test]
    fn the_copy_beside_the_original_is_not_treated_as_anything() {
        const BESIDE: &str = "../../../Server-Side/wServer/logic/db";

        let Ok(entries) = std::fs::read_dir(BESIDE) else {
            eprintln!("skipping: the original's sources are not where the test looks for them");
            return;
        };

        let originals = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("cs"))
            .count();

        assert!(
            originals > 0,
            "the C# behaviour database is missing from {BESIDE}; the converter reads those `.cs` \
             files and cannot run without them"
        );
    }
}
