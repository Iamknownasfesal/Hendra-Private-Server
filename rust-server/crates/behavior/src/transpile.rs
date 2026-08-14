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

    let (enemies, unreadable) = crate::csharp::read_enemies_reporting(source);
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

    let target = match call.name.as_str() {
        "EntitiesNotExistsTransition" | "EntitiesNotExistTransition" | "TimedRandomTransition" => {
            strings.first()
        }
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

        // A nested call as an argument, usually a Cooldown, has no representation, so it is
        // dropped and the default applies.
        CsValue::Call(_) | CsValue::List(_) => return None,
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
            .filter_map(|argument| match &argument.value {
                CsValue::Call(inner) if is_loot(&inner.name) => Some(emit_loot(inner)),
                _ => None,
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
}
