//! Turning a parsed file into something that runs.
//!
//! Compiling flattens the state tree into a list, resolves every transition target to an index, and
//! turns each generic call into a typed primitive.
//!
//! # Unknown names are reported, not fatal
//!
//! The content uses seventy-five primitives and this runtime implements the ones that carry the
//! weight. A behaviour it does not know becomes [`Primitive::Unsupported`], which does nothing, and
//! is reported once by name with a suggestion. The alternative — refusing the file — means a single
//! unimplemented cosmetic behaviour costs a whole boss, which is the wrong trade while the runtime
//! is being filled in.

use crate::ast::{self, Behaviours, Call, Item, Value};
use crate::lex::Span;
use crate::program::*;

/// Something the compiler wants the author to know.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    pub at: Span,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.at, self.message)
    }
}

/// Every behaviour name the runtime understands.
const KNOWN_BEHAVIOURS: &[&str] = &[
    "shoot",
    "wander",
    "follow",
    "orbit",
    "stay_back",
    "stay_close_to_spawn",
    "heal_self",
    "spawn",
    "suicide",
    "prioritize",
];

/// Every transition condition the runtime understands.
const KNOWN_CONDITIONS: &[&str] = &["timed", "player_within", "no_player_within", "hp_below"];

/// Compiles a parsed file.
///
/// Returns the programs and everything worth telling the author. Diagnostics are not failures —
/// they are the list of things that will silently do nothing.
pub fn compile(parsed: &Behaviours) -> (Programs, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let programs = parsed
        .enemies
        .iter()
        .map(|enemy| compile_enemy(enemy, &mut diagnostics))
        .collect();

    (Programs { programs }, diagnostics)
}

fn compile_enemy(enemy: &ast::Enemy, diagnostics: &mut Vec<Diagnostic>) -> Program {
    // Flatten the tree first, so transition targets can be resolved to indices.
    let mut states: Vec<CompiledState> = Vec::new();
    flatten(&enemy.root, None, &mut states);

    let names: Vec<String> = states.iter().map(|state| state.name.clone()).collect();

    // Fill in behaviours and transitions now that every state has an index.
    let mut slot_base = 0usize;
    let mut sources = Vec::new();
    collect_sources(&enemy.root, &mut sources);

    for (index, source) in sources.iter().enumerate() {
        let mut behaviours = Vec::new();
        let mut transitions = Vec::new();

        for item in &source.items {
            match item {
                Item::Behaviour(call) => {
                    behaviours.push(behaviour(call, diagnostics));
                }

                Item::Group { call, children } => {
                    let inner: Vec<Primitive> = children
                        .iter()
                        .filter_map(|child| match child {
                            Item::Behaviour(call) => Some(behaviour(call, diagnostics)),
                            _ => None,
                        })
                        .collect();

                    if call.name == "prioritize" {
                        behaviours.push(Primitive::Prioritize(inner));
                    } else {
                        diagnostics.push(Diagnostic {
                            message: format!(
                                "`{}` is not a group; its contents will not run{}",
                                call.name,
                                suggestion(&call.name, KNOWN_BEHAVIOURS)
                            ),
                            at: call.at,
                        });
                        behaviours.push(Primitive::Unsupported {
                            name: call.name.clone(),
                        });
                    }
                }

                Item::Transition(transition) => {
                    // Already checked by the parser, so a miss here is a compiler bug rather than a
                    // content one.
                    let Some(target) = names.iter().position(|name| *name == transition.target)
                    else {
                        continue;
                    };
                    transitions.push(CompiledTransition {
                        condition: condition(&transition.condition, diagnostics),
                        target,
                    });
                }

                Item::State(_) => {}
            }
        }

        let needed: usize = behaviours.iter().map(Primitive::slots).sum();
        states[index].behaviours = behaviours;
        states[index].transitions = transitions;
        states[index].slot_base = slot_base;
        slot_base += needed;
    }

    // Entering a state means entering its innermost first child.
    for index in 0..states.len() {
        states[index].entry = descend(&states, index);
    }

    Program {
        name: enemy.name.clone(),
        root: 0,
        slots: slot_base,
        states,
        loot: enemy
            .loot
            .iter()
            .filter_map(|entry| loot(&entry.call))
            .collect(),
    }
}

/// Walks to the innermost first child of a state.
fn descend(states: &[CompiledState], from: usize) -> usize {
    let mut current = from;
    loop {
        let Some(child) = states
            .iter()
            .position(|state| state.parent == Some(current))
        else {
            return current;
        };
        current = child;
    }
}

fn flatten(state: &ast::State, parent: Option<usize>, into: &mut Vec<CompiledState>) {
    let index = into.len();
    into.push(CompiledState {
        name: state.name.clone(),
        parent,
        entry: index,
        behaviours: Vec::new(),
        transitions: Vec::new(),
        slot_base: 0,
    });

    for child in state.states() {
        flatten(child, Some(index), into);
    }
}

/// The same walk as [`flatten`], gathering the source states in the same order.
fn collect_sources<'a>(state: &'a ast::State, into: &mut Vec<&'a ast::State>) {
    into.push(state);
    for child in state.states() {
        collect_sources(child, into);
    }
}

fn number(call: &Call, name: &str, index: usize, fallback: f64) -> f64 {
    call.argument(name, index)
        .and_then(Value::as_number)
        .unwrap_or(fallback)
}

fn behaviour(call: &Call, diagnostics: &mut Vec<Diagnostic>) -> Primitive {
    match call.name.as_str() {
        "shoot" => Primitive::Shoot {
            // The first positional argument in the C# is a radius that nothing reads; count is what
            // matters, and it is named in nearly every use.
            count: number(call, "count", 1, 1.0).max(1.0) as u32,
            spread: number(call, "shoot_angle", 9, 0.0) as f32,
            fixed_angle: call
                .named("fixed_angle")
                .and_then(Value::as_number)
                .map(|degrees| degrees as f32),
            cooldown_ms: number(call, "cooldown", 8, 1000.0).max(0.0) as u32,
            projectile: number(call, "projectile", 7, 0.0).clamp(0.0, 255.0) as u8,
        },

        "wander" => Primitive::Wander {
            speed: number(call, "speed", 0, 0.4) as f32,
        },

        "follow" => Primitive::Follow {
            speed: number(call, "speed", 0, 1.0) as f32,
            acquire_range: number(call, "acquire_range", 1, 10.0) as f32,
            range: number(call, "range", 2, 6.0) as f32,
        },

        "orbit" => Primitive::Orbit {
            speed: number(call, "speed", 0, 1.0) as f32,
            radius: number(call, "radius", 1, 4.0) as f32,
            acquire_range: number(call, "acquire_range", 2, 10.0) as f32,
        },

        "stay_back" => Primitive::StayBack {
            speed: number(call, "speed", 0, 1.0) as f32,
            distance: number(call, "distance", 1, 8.0) as f32,
        },

        "stay_close_to_spawn" => Primitive::StayCloseToSpawn {
            speed: number(call, "speed", 0, 1.0) as f32,
            range: number(call, "range", 1, 5.0) as f32,
        },

        "heal_self" => Primitive::HealSelf {
            amount: number(call, "amount", 0, 100.0) as i32,
            cooldown_ms: number(call, "cooldown", 1, 1000.0).max(0.0) as u32,
        },

        "spawn" => Primitive::Spawn {
            child: call
                .argument("children", 0)
                .and_then(Value::as_text)
                .unwrap_or_default()
                .to_string(),
            max_children: number(call, "max_children", 1, 5.0).max(0.0) as u32,
            cooldown_ms: number(call, "cooldown", 2, 1000.0).max(0.0) as u32,
        },

        "suicide" => Primitive::Suicide,

        // A `prioritize` written without a block has nothing to prioritise.
        "prioritize" => Primitive::Prioritize(Vec::new()),

        other => {
            diagnostics.push(Diagnostic {
                message: format!(
                    "`{other}` is not a behaviour this runtime knows; it will do nothing{}",
                    suggestion(other, KNOWN_BEHAVIOURS)
                ),
                at: call.at,
            });
            Primitive::Unsupported {
                name: other.to_string(),
            }
        }
    }
}

fn condition(call: &Call, diagnostics: &mut Vec<Diagnostic>) -> Condition {
    match call.name.as_str() {
        "timed" => Condition::Timed {
            after_ms: number(call, "after", 0, 1000.0).max(0.0) as u32,
        },

        "player_within" => Condition::PlayerWithin {
            radius: number(call, "radius", 0, 10.0) as f32,
        },

        "no_player_within" => Condition::NoPlayerWithin {
            radius: number(call, "radius", 0, 10.0) as f32,
        },

        "hp_below" => Condition::HpBelow {
            // Written either as a fraction or as a percentage, because the C# uses both.
            fraction: {
                let raw = number(call, "fraction", 0, 0.5) as f32;
                if raw > 1.0 { raw / 100.0 } else { raw }
            },
        },

        other => {
            diagnostics.push(Diagnostic {
                message: format!(
                    "`{other}` is not a transition this runtime knows; it will never fire{}",
                    suggestion(other, KNOWN_CONDITIONS)
                ),
                at: call.at,
            });
            Condition::Unsupported {
                name: other.to_string(),
            }
        }
    }
}

fn loot(call: &Call) -> Option<LootEntry> {
    match call.name.as_str() {
        "item" => Some(LootEntry::Item {
            name: call
                .argument("name", 0)
                .and_then(Value::as_text)?
                .to_string(),
            chance: number(call, "chance", 1, 0.0) as f32,
        }),
        "tier" => Some(LootEntry::Tier {
            tier: number(call, "tier", 0, 0.0).clamp(0.0, 255.0) as u8,
            kind: call
                .argument("kind", 1)
                .and_then(Value::as_text)
                .unwrap_or("any")
                .to_string(),
            chance: number(call, "chance", 2, 0.0) as f32,
        }),
        _ => None,
    }
}

/// A "did you mean" for a misspelled name.
///
/// Cheap edit distance over a list of ten names. The value is not the algorithm — it is that a
/// typo in a content file says what was probably meant instead of quietly doing nothing.
fn suggestion(found: &str, known: &[&str]) -> String {
    let best = known
        .iter()
        .map(|candidate| (candidate, distance(found, candidate)))
        .filter(|(_, distance)| *distance <= 3)
        .min_by_key(|(_, distance)| *distance);

    match best {
        Some((candidate, _)) => format!(" (did you mean `{candidate}`?)"),
        None => String::new(),
    }
}

fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];

    for (i, left) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, right) in b.iter().enumerate() {
            let cost = usize::from(left != right);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn compiled(source: &str) -> (Programs, Vec<Diagnostic>) {
        compile(&parse(source).expect("should parse"))
    }

    #[test]
    fn states_flatten_with_their_parents_resolved() {
        let (programs, _) = compiled(
            r#"enemy "X" {
                 state outer {
                   state inner { }
                 }
                 state other { }
               }"#,
        );

        let program = programs.get("X").unwrap();
        // Root, outer, inner, other.
        assert_eq!(program.states.len(), 4);

        let inner = program.state_named("inner").unwrap();
        let outer = program.state_named("outer").unwrap();
        assert_eq!(program.states[inner].parent, Some(outer));
        assert_eq!(program.states[outer].parent, Some(program.root));
    }

    #[test]
    fn entering_a_state_lands_in_its_innermost_child() {
        let (programs, _) = compiled(
            r#"enemy "X" {
                 state outer {
                   state middle {
                     state innermost { }
                   }
                 }
               }"#,
        );

        let program = programs.get("X").unwrap();
        let outer = program.state_named("outer").unwrap();
        let innermost = program.state_named("innermost").unwrap();

        assert_eq!(
            program.states[outer].entry, innermost,
            "a state with children is entered by entering them"
        );
    }

    #[test]
    fn ancestry_runs_innermost_first() {
        let (programs, _) = compiled(
            r#"enemy "X" {
                 state outer { state inner { } }
               }"#,
        );

        let program = programs.get("X").unwrap();
        let inner = program.state_named("inner").unwrap();

        let mut chain = Vec::new();
        program.ancestry(inner, &mut chain);

        assert_eq!(chain.len(), 3, "inner, outer, root");
        assert_eq!(chain[0], inner);
        assert_eq!(chain[2], program.root);
    }

    #[test]
    fn shoot_reads_its_arguments_by_name() {
        let (programs, diagnostics) = compiled(
            r#"enemy "X" {
                 state a {
                   shoot(1, count: 15, shoot_angle: 24, cooldown: 1200ms, projectile: 2)
                 }
               }"#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        let program = programs.get("X").unwrap();
        let state = program.state_named("a").unwrap();

        match &program.states[state].behaviours[0] {
            Primitive::Shoot {
                count,
                spread,
                cooldown_ms,
                projectile,
                fixed_angle,
            } => {
                assert_eq!(*count, 15);
                assert_eq!(*spread, 24.0);
                assert_eq!(*cooldown_ms, 1200);
                assert_eq!(*projectile, 2);
                assert_eq!(*fixed_angle, None, "unset means aim at the nearest player");
            }
            other => panic!("expected a shoot, got {other:?}"),
        }
    }

    #[test]
    fn a_prioritize_block_becomes_one_primitive_holding_its_children() {
        let (programs, _) = compiled(
            r#"enemy "X" {
                 prioritize {
                   follow(0.75)
                   wander(0.4)
                 }
               }"#,
        );

        let program = programs.get("X").unwrap();
        match &program.states[program.root].behaviours[0] {
            Primitive::Prioritize(children) => assert_eq!(children.len(), 2),
            other => panic!("expected a prioritize, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_behaviour_is_reported_with_a_suggestion_and_does_nothing() {
        let (programs, diagnostics) = compiled(
            r#"enemy "X" {
                 state a { wnader(0.4) }
               }"#,
        );

        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].message.contains("did you mean `wander`"),
            "should suggest the near miss: {}",
            diagnostics[0].message
        );

        // The rest of the enemy still compiles: one unknown behaviour costs that behaviour.
        let program = programs.get("X").unwrap();
        let state = program.state_named("a").unwrap();
        assert!(matches!(
            program.states[state].behaviours[0],
            Primitive::Unsupported { .. }
        ));
    }

    #[test]
    fn an_unknown_transition_never_fires_but_the_enemy_survives() {
        let (programs, diagnostics) = compiled(
            r#"enemy "X" {
                 state a { on ground_within(3) -> b }
                 state b { }
               }"#,
        );

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("never fire"));

        let program = programs.get("X").unwrap();
        let a = program.state_named("a").unwrap();
        assert!(matches!(
            program.states[a].transitions[0].condition,
            Condition::Unsupported { .. }
        ));
    }

    #[test]
    fn transitions_resolve_to_state_indices() {
        let (programs, _) = compiled(
            r#"enemy "X" {
                 state a { on timed(400ms) -> b }
                 state b { }
               }"#,
        );

        let program = programs.get("X").unwrap();
        let a = program.state_named("a").unwrap();
        let b = program.state_named("b").unwrap();

        assert_eq!(program.states[a].transitions[0].target, b);
    }

    #[test]
    fn health_thresholds_accept_a_fraction_or_a_percentage() {
        let (programs, _) = compiled(
            r#"enemy "X" {
                 state a { on hp_below(0.25) -> b }
                 state b { on hp_below(50) -> a }
               }"#,
        );

        let program = programs.get("X").unwrap();
        let a = program.state_named("a").unwrap();
        let b = program.state_named("b").unwrap();

        assert_eq!(
            program.states[a].transitions[0].condition,
            Condition::HpBelow { fraction: 0.25 }
        );
        assert_eq!(
            program.states[b].transitions[0].condition,
            Condition::HpBelow { fraction: 0.5 },
            "a percentage should mean the same thing"
        );
    }

    #[test]
    fn loot_entries_compile() {
        let (programs, _) = compiled(
            r#"enemy "X" {
                 loot {
                   item("Health Potion", 0.02)
                   tier(2, weapon, 0.3)
                 }
               }"#,
        );

        let program = programs.get("X").unwrap();
        assert_eq!(
            program.loot,
            vec![
                LootEntry::Item {
                    name: "Health Potion".into(),
                    chance: 0.02
                },
                LootEntry::Tier {
                    tier: 2,
                    kind: "weapon".into(),
                    chance: 0.3
                },
            ]
        );
    }

    #[test]
    fn every_behaviour_gets_its_own_cooldown_slots() {
        let (programs, _) = compiled(
            r#"enemy "X" {
                 state a {
                   shoot(count: 3)
                   prioritize { follow(1) wander(0.4) }
                 }
                 state b { shoot(count: 1) }
               }"#,
        );

        let program = programs.get("X").unwrap();
        // One for the shoot, one for the prioritize and one per child, one for b's shoot.
        assert_eq!(program.slots, 5);

        let a = program.state_named("a").unwrap();
        let b = program.state_named("b").unwrap();
        assert_ne!(
            program.states[a].slot_base, program.states[b].slot_base,
            "two states must not share cooldown slots"
        );
    }
}
