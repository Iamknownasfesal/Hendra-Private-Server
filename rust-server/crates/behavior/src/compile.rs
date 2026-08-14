//! Turning a parsed file into something that runs.
//!
//! Compiling flattens the state tree into a list, resolves every transition target to an index, and
//! turns each generic call into a typed primitive.
//!
//! # Unknown names are reported, not fatal
//!
//! The content uses seventy-five primitives and this runtime implements the ones that carry the
//! weight. A behaviour it does not know becomes [`Primitive::Unsupported`], which does nothing, and
//! is reported once by name with a suggestion. Refusing the whole file instead would mean a single
//! unimplemented cosmetic behaviour costs a whole boss, which is the wrong trade while the runtime
//! is being filled in.

use std::sync::Arc;

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
    "stay_above",
    "heal_self",
    "heal_group",
    "heal_entity",
    "heal_player",
    "spawn",
    "spawn_group",
    "reproduce",
    "toss_object",
    "grenade",
    "suicide",
    "decay",
    "conditional_effect",
    "set_alt_texture",
    "change_size",
    "taunt",
    "order",
    "transform",
    "protect",
    "move_to",
    "move_line",
    "back_and_forth",
    "charge",
    "swirl",
    "return_to_spawn",
    "set_no_x_p",
    "remove_tile_object",
    "ground_transform",
    "transform_on_death",
    "drop_portal_on_death",
    "change_ground_on_death",
    "remove_object_on_death",
    "order_on_death",
    "transfer_damage_on_death",
    "prioritize",
    "sequence",
    "timed",
    "if",
    "flash",
    "invisi_toss",
    "order_once",
    "remove_conditional_effect",
    "scale_h_p",
    "on_death_behavior",
];

/// Every transition condition the runtime understands.
const KNOWN_CONDITIONS: &[&str] = &[
    "timed",
    "timed_random",
    "player_within",
    "no_player_within",
    "hp_below",
    "entity_exists",
    "entity_not_exists",
    "entities_not_exists",
    "damage_taken",
    "not_moving",
];

/// Compiles a parsed file.
///
/// Returns the programs and everything worth telling the author. Diagnostics are not failures:
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

/// Collects the entity names a program mentions, giving each one an index.
///
/// One table per enemy rather than one per file: an enemy is what gets resolved against the
/// catalog, and a shared table would make every enemy carry every other enemy's names.
#[derive(Default)]
pub struct Names {
    entries: Vec<String>,
}

impl Names {
    fn intern(&mut self, name: &str) -> NameRef {
        let name = name.trim();
        if let Some(at) = self.entries.iter().position(|held| held == name) {
            return NameRef(at as u32);
        }
        self.entries.push(name.to_string());
        NameRef((self.entries.len() - 1) as u32)
    }
}

fn compile_enemy(enemy: &ast::Enemy, diagnostics: &mut Vec<Diagnostic>) -> Program {
    let mut interner = Names::default();
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
                    behaviours.push(behaviour(call, &mut interner, diagnostics));
                }

                Item::Group { call, children } => {
                    let inner: Vec<Primitive> = children
                        .iter()
                        .filter_map(|child| match child {
                            Item::Behaviour(call) => {
                                Some(behaviour(call, &mut interner, diagnostics))
                            }
                            _ => None,
                        })
                        .collect();

                    match call.name.as_str() {
                        "prioritize" => behaviours.push(Primitive::Prioritize(inner)),

                        // One child per turn, which is what makes an attack pattern a pattern.
                        "sequence" => behaviours.push(Primitive::Sequence { children: inner }),

                        // A wrapper whose child runs at death rather than during a tick. Its
                        // contents are kept as they are: whether a primitive is a death effect is
                        // already decided by what it is.
                        "on_death_behavior" => behaviours.extend(inner),

                        // A timer around a group: everything inside runs together, on a period.
                        // The C# writes it as `Timed(600, new Shoot(...))`.
                        "timed" => behaviours.push(Primitive::Every {
                            period_ms: number(call, "period", 0, 1000.0).max(0.0) as u32,
                            children: inner,
                        }),

                        // A guard around a group, which the C# writes as `If(condition, ...)`.
                        // The condition is the call's first argument.
                        "if" => behaviours.push(Primitive::When {
                            condition: Box::new(guard(call, &mut interner, diagnostics)),
                            children: inner,
                        }),

                        _ => {
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
                }

                Item::Transition(transition) => {
                    // Already checked by the parser, so a miss here is a compiler bug rather than a
                    // content one.
                    let Some(target) = names.iter().position(|name| *name == transition.target)
                    else {
                        continue;
                    };
                    transitions.push(CompiledTransition {
                        condition: condition(&transition.condition, &mut interner, diagnostics),
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
        // Unresolved: the host fills these in at load, when it has a catalog.
        kinds: vec![Vec::new(); interner.entries.len()],
        names: interner.entries,
        states,
        loot: enemy.loot.iter().filter_map(loot).collect(),
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

/// How far a health-scaling behaviour counts players when the content gives no distance.
///
/// A boss written for a crowd is trivial when two people find it and impossible when forty do, so
/// "no distance" has to mean the room rather than nowhere.
const SCALE_RADIUS: f32 = 20.0;

/// How far around itself a `reproduce_children` counts its own kind.
///
/// The C# form of this behaviour has no radius at all, it counts children it has made. Counting
/// what is standing nearby needs one, and this is wide enough to cover a room.
const REPRODUCE_RADIUS: f32 = 15.0;

/// The text of an argument, by name or position.
fn text<'a>(call: &'a Call, name: &str, index: usize) -> Option<&'a str> {
    call.argument(name, index).and_then(Value::as_text)
}

/// An entity name, interned. Missing names become an empty entry rather than a missing one, so a
/// behaviour that names nothing still compiles and is reported at resolve time.
fn entity(call: &Call, names: &mut Names, name: &str, index: usize) -> NameRef {
    names.intern(text(call, name, index).unwrap_or_default())
}

/// An optional entity name: absent, or empty, means "anything".
fn maybe_entity(call: &Call, names: &mut Names, name: &str, index: usize) -> Option<NameRef> {
    let found = text(call, name, index)?.trim();
    (!found.is_empty()).then(|| names.intern(found))
}

/// Every string argument from a position onward.
///
/// The variadic behaviours put their entity names last, so this is how `order` and the
/// `entity_not_exists` family read theirs.
fn text_list(call: &Call, from: usize) -> Vec<&str> {
    call.arguments
        .iter()
        .skip(from)
        .filter_map(|argument| argument.value.as_text())
        .filter(|found| !found.trim().is_empty())
        .collect()
}

/// A condition effect, by name or by number.
///
/// The content writes these both ways. `ConditionEffectIndex.Invulnerable` transpiles to a name,
/// while a few files use the raw index. Unknown names become `Nothing` rather than refusing, and
/// the compiler says so.
fn effect_of(call: &Call, name: &str, index: usize, diagnostics: &mut Vec<Diagnostic>) -> u8 {
    if let Some(number) = call.argument(name, index).and_then(Value::as_number) {
        return number.clamp(0.0, 255.0) as u8;
    }

    let Some(written) = text(call, name, index) else {
        return 0;
    };

    match effect_number(written) {
        Some(found) => found,
        None => {
            diagnostics.push(Diagnostic {
                message: format!("`{written}` is not a condition effect; nothing will be applied"),
                at: call.at,
            });
            0
        }
    }
}

/// The game's effect numbers, which are fixed by the wire format rather than chosen here.
///
/// Matched case-insensitively and ignoring separators, because the content writes
/// `ArmorBroken`, `armor_broken` and `ARMORBROKEN` in different files.
fn effect_number(written: &str) -> Option<u8> {
    let tidy: String = written
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();

    Some(match tidy.as_str() {
        "dead" => 0,
        "quiet" => 1,
        "weak" => 2,
        "slowed" => 3,
        "sick" => 4,
        "dazed" => 5,
        "stunned" => 6,
        "blind" => 7,
        "hallucinating" => 8,
        "drunk" => 9,
        "confused" => 10,
        "stunimmune" => 11,
        "invisible" => 12,
        "paralyzed" | "paralysed" => 13,
        "speedy" => 14,
        "bleeding" => 15,
        "armorbreakimmune" => 16,
        "healing" => 17,
        "damaging" => 18,
        "berserk" => 19,
        "paused" => 20,
        "stasis" => 21,
        "stasisimmune" => 22,
        "invincible" => 23,
        "invulnerable" => 24,
        "armored" => 25,
        "armorbroken" => 26,
        "hexed" => 27,
        "ninjaspeedy" => 28,
        "unstable" => 29,
        "darkness" => 30,
        "slowedimmune" => 31,
        "dazedimmune" => 32,
        "paralyzeimmune" | "paralyseimmune" => 33,
        "petrify" | "petrified" => 34,
        "petrifyimmune" => 35,
        "petdisable" => 36,
        "curse" | "cursed" => 37,
        "curseimmune" => 38,
        "hpboost" => 39,
        "nothing" | "none" => 0,
        _ => return None,
    })
}

/// Who a `conditional_effect` lands on.
fn effect_target(call: &Call) -> EffectTarget {
    // The C# spells this as a target enum on the behaviour. Anything unrecognised means the entity
    // itself, which is what the overwhelming majority of uses mean.
    match text(call, "target", 3)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("players") | Some("player") => EffectTarget::Players,
        Some("enemies") | Some("others") | Some("allies") => EffectTarget::Others,
        _ => EffectTarget::Myself,
    }
}

/// A duration written either in seconds or milliseconds.
///
/// The C# durations for effects are seconds as floats, and everything else in these files is
/// milliseconds. A value small enough to be meaningless as milliseconds is read as seconds.
fn duration_ms(call: &Call, name: &str, index: usize, fallback: f64) -> u32 {
    let raw = number(call, name, index, fallback);
    if raw > 0.0 && raw < 60.0 {
        (raw * 1000.0) as u32
    } else {
        raw.max(0.0) as u32
    }
}

fn behaviour(call: &Call, names: &mut Names, diagnostics: &mut Vec<Diagnostic>) -> Primitive {
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
            child: entity(call, names, "children", 0),
            max_children: number(call, "max_children", 1, 5.0).max(0.0) as u32,
            cooldown_ms: number(call, "cooldown", 2, 1000.0).max(0.0) as u32,
        },

        // `Reproduce(children, densityRadius, densityMax, coolDown)`.
        "reproduce" => Primitive::Reproduce {
            child: entity(call, names, "children", 0),
            density_radius: number(call, "density_radius", 1, 10.0) as f32,
            density_max: number(call, "density_max", 2, 5.0).max(0.0) as u32,
            cooldown_ms: number(call, "cooldown", 3, 1000.0).max(0.0) as u32,
        },

        // `ReproduceChildren(maxChildren, initialSpawn, coolDown, params children)`: the numbers
        // come first here and the name last, which is the reverse of `reproduce`.
        "reproduce_children" => Primitive::Reproduce {
            child: names.intern(text_list(call, 0).last().copied().unwrap_or_default()),
            density_radius: REPRODUCE_RADIUS,
            density_max: number(call, "max_children", 0, 5.0).max(0.0) as u32,
            cooldown_ms: number(call, "cooldown", 2, 1000.0).max(0.0) as u32,
        },

        // The C# spells the group form `SpawnGroup(group, max, initial, cooldown)`. A group is a
        // name like any other here; what it resolves to is the host's problem.
        "spawn_group" => Primitive::Spawn {
            child: entity(call, names, "group", 0),
            max_children: number(call, "max_children", 1, 5.0).max(0.0) as u32,
            cooldown_ms: number(call, "cooldown", 3, 1000.0).max(0.0) as u32,
        },

        // `InvisiToss` throws the same way but without the thrower being seen doing it. Nothing
        // downstream distinguishes them, since the difference is what the client draws.
        "toss_object" | "invisi_toss" => Primitive::TossObject {
            child: entity(call, names, "child", 0),
            // Written `range` rather than `radius` in every real use.
            radius: number(call, "range", 1, 5.0) as f32,
            fixed_angle: call
                .named("angle")
                .and_then(Value::as_number)
                .map(|degrees| degrees as f32),
            cooldown_ms: number(call, "cooldown", 3, 1000.0).max(0.0) as u32,
            // The telegraph. Without it a thrown object is an unavoidable hit, which is the
            // difference between a hard fight and an unfair one.
            warning_ms: number(call, "throw_delay", 9, 800.0).max(0.0) as u32,
        },

        "grenade" => Primitive::Grenade {
            radius: number(call, "radius", 0, 2.0) as f32,
            damage: number(call, "damage", 1, 100.0) as i32,
            range: number(call, "range", 2, 5.0) as f32,
            cooldown_ms: number(call, "cooldown", 3, 1000.0).max(0.0) as u32,
            effect: call
                .named("effect")
                .map(|_| effect_of(call, "effect", usize::MAX, diagnostics)),
            effect_ms: duration_ms(call, "effect_duration", usize::MAX, 0.0),
        },

        "suicide" => Primitive::Suicide,

        // `RemoveEntity(dist, children)` removes *other* entities rather than itself. Reading it
        // as a suicide would have made every boss that tidies up its summons kill itself instead.
        "remove_entity" => Primitive::RemoveNearby {
            radius: number(call, "radius", 0, 10.0) as f32,
            kind: maybe_entity(call, names, "children", 1),
        },

        "decay" => Primitive::Decay {
            // A written zero means the argument was elided rather than that it should vanish at
            // once, which is what the C# default of ten seconds says.
            after_ms: match number(call, "duration", 0, 0.0).max(0.0) as u32 {
                0 => 10_000,
                given => given,
            },
        },

        "conditional_effect" => Primitive::ConditionalEffect {
            effect: effect_of(call, "effect", 0, diagnostics),
            // Zero means "for as long as this state lasts". `perm` says the same thing in the
            // content's own words, and is how nearly every use of this is written.
            duration_ms: if call.named("perm").is_some() {
                0
            } else {
                duration_ms(call, "duration", 1, 0.0)
            },
            target: effect_target(call),
            radius: number(call, "range", 2, 0.0) as f32,
        },

        "set_alt_texture" => Primitive::SetAltTexture {
            index: number(call, "index", 0, 0.0).clamp(0.0, 255.0) as u8,
        },

        // `Flash(color, flashPeriod, flashRepeats)`. The period is seconds here, unlike almost
        // everything else in these files.
        "flash" => Primitive::Flash {
            colour: number(call, "color", 0, 0.0).clamp(0.0, u32::MAX as f64) as u32,
            period_ms: (number(call, "flash_period", 1, 0.5).max(0.0) * 1000.0) as u32,
            repeats: number(call, "flash_repeats", 2, 1.0).clamp(0.0, 255.0) as u32,
        },

        "remove_conditional_effect" => Primitive::RemoveEffect {
            effect: effect_of(call, "effect", 0, diagnostics),
        },

        // `ScaleHP(amountPerPlayer, maxAdditional, healAfterMax, dist, scaleAfter)`.
        "scale_h_p" => Primitive::ScaleHealth {
            per_player: number(call, "amount_per_player", 0, 0.0) as i32,
            maximum_extra: number(call, "max_additional", 1, 0.0).max(0.0) as i32,
            radius: {
                // A distance of zero means the whole room rather than nowhere.
                let written = number(call, "dist", 3, 0.0) as f32;
                if written <= 0.0 {
                    SCALE_RADIUS
                } else {
                    written
                }
            },
        },

        "change_size" => Primitive::ChangeSize {
            rate: number(call, "rate", 0, 0.0) as f32,
            target: number(call, "target", 1, 100.0).clamp(0.0, 65_535.0) as u16,
        },

        "taunt" => {
            // Every string argument is a line it might say; the numbers around them are the
            // probability and the cooldown.
            let lines: Vec<Arc<str>> = text_list(call, 0)
                .into_iter()
                .map(|line| Arc::from(line.trim()))
                .collect();

            Primitive::Taunt {
                probability: {
                    let raw = number(call, "probability", usize::MAX, 1.0) as f32;
                    if raw > 1.0 { raw / 100.0 } else { raw }.clamp(0.0, 1.0)
                },
                cooldown_ms: number(call, "cooldown", usize::MAX, 5_000.0).max(0.0) as u32,
                broadcast: call
                    .named("broadcast")
                    .and_then(Value::as_number)
                    .is_some_and(|value| value != 0.0),
                lines,
            }
        }

        // `Order(range, children, targetState)`, and its once-only twin.
        "order" | "order_once" => Primitive::Order {
            radius: number(call, "range", 0, 10.0) as f32,
            kind: maybe_entity(call, names, "children", 1),
            state: Arc::from(text(call, "target_state", 2).unwrap_or_default().trim()),
            once: call.name == "order_once",
        },

        "transform" => Primitive::Transform {
            into: entity(call, names, "target", 0),
        },

        "protect" => Primitive::Protect {
            speed: number(call, "speed", 0, 1.0) as f32,
            protectee: entity(call, names, "protectee", 1),
            acquire_range: number(call, "acquire_range", 2, 10.0) as f32,
            protect_range: number(call, "protection_range", 3, 4.0) as f32,
            reprotect_range: number(call, "reprotect_range", 4, 2.0) as f32,
        },

        "heal_group" => Primitive::HealOthers {
            radius: number(call, "range", 0, 10.0) as f32,
            amount: number(call, "amount", 2, 100.0) as i32,
            kind: maybe_entity(call, names, "group", 1),
            players: false,
            cooldown_ms: number(call, "cooldown", 3, 1000.0).max(0.0) as u32,
        },

        "heal_entity" => Primitive::HealOthers {
            radius: number(call, "range", 0, 10.0) as f32,
            amount: number(call, "amount", 2, 100.0) as i32,
            kind: maybe_entity(call, names, "name", 1),
            players: false,
            cooldown_ms: number(call, "cooldown", 3, 1000.0).max(0.0) as u32,
        },

        "heal_player" => Primitive::HealOthers {
            radius: number(call, "range", 0, 10.0) as f32,
            amount: number(call, "amount", 1, 100.0) as i32,
            kind: None,
            players: true,
            cooldown_ms: number(call, "cooldown", 2, 1000.0).max(0.0) as u32,
        },

        // `MoveTo(speed, x, y)`.
        "move_to" => Primitive::MoveTo {
            x: number(call, "x", 1, 0.0) as f32,
            y: number(call, "y", 2, 0.0) as f32,
            speed: number(call, "speed", 0, 1.0) as f32,
        },

        // `MoveTo2(x, y, speed)` is the same behaviour with the arguments the other way round.
        "move_to2" => Primitive::MoveTo {
            x: number(call, "x", 0, 0.0) as f32,
            y: number(call, "y", 1, 0.0) as f32,
            speed: number(call, "speed", 2, 2.0) as f32,
        },

        "move_line" => Primitive::MoveLine {
            speed: number(call, "speed", 0, 1.0) as f32,
            angle: number(call, "direction", 1, 0.0) as f32,
        },

        "back_and_forth" => Primitive::BackAndForth {
            speed: number(call, "speed", 0, 1.0) as f32,
            distance: number(call, "distance", 1, 5.0) as f32,
        },

        "charge" => Primitive::Charge {
            speed: number(call, "speed", 0, 1.0) as f32,
            range: number(call, "range", 1, 10.0) as f32,
            cooldown_ms: number(call, "cooldown", 2, 1000.0).max(0.0) as u32,
        },

        "swirl" => Primitive::Swirl {
            speed: number(call, "speed", 0, 1.0) as f32,
            radius: number(call, "radius", 1, 5.0) as f32,
            targeted: call
                .argument("targeted", 2)
                .and_then(Value::as_number)
                .is_some_and(|value| value != 0.0),
        },

        "return_to_spawn" => Primitive::ReturnToSpawn {
            speed: number(call, "speed", 0, 1.0) as f32,
            tolerance: number(call, "tolerance", 1, 0.5) as f32,
        },

        "stay_above" => Primitive::StayAbove {
            speed: number(call, "speed", 0, 1.0) as f32,
            altitude: number(call, "altitude", 1, 5.0) as f32,
        },

        "set_no_x_p" => Primitive::NoExperience,

        "remove_tile_object" => Primitive::RemoveNearby {
            radius: number(call, "radius", 1, 1.0) as f32,
            kind: maybe_entity(call, names, "target", 0),
        },

        // `ApplySetpiece(name)`: draws one of the structures a realm is built with, where the
        // entity is standing. Whether the name is one the simulation knows is checked there, since
        // that is where the drawings live.
        "apply_setpiece" => Primitive::ApplySetpiece {
            name: text_list(call, 0)
                .first()
                .copied()
                .unwrap_or_default()
                .to_string(),
        },

        // `GroundTransform(tileId, radius, ...)`.
        "ground_transform" => Primitive::GroundTransform {
            tile: entity(call, names, "tile", 0),
            radius: number(call, "radius", 1, 1.0) as f32,
            cooldown_ms: number(call, "cooldown", 2, 0.0).max(0.0) as u32,
        },

        // `ReplaceTile(objName, replacedObjName, range)`: the *second* name is what the ground
        // becomes. Taking the first would replace the ground with what was already there.
        "replace_tile" => Primitive::GroundTransform {
            tile: names.intern(text_list(call, 0).get(1).copied().unwrap_or_default()),
            radius: number(call, "range", 0, 1.0) as f32,
            cooldown_ms: 0,
        },

        // -- what happens at death ------------------------------------------------------------

        // `TransformOnDeath(target, min, max, probability)`.
        "transform_on_death" => Primitive::OnDeath(Box::new(DeathEffect::TransformInto {
            child: entity(call, names, "target", 0),
        })),

        "drop_portal_on_death" | "realm_portal_drop" => {
            Primitive::OnDeath(Box::new(DeathEffect::Portal {
                name: entity(call, names, "target", 0),
                probability: {
                    let raw = number(call, "probability", 1, 1.0) as f32;
                    if raw > 1.0 { raw / 100.0 } else { raw }.clamp(0.0, 1.0)
                },
                duration_ms: number(call, "timeout", 2, 30_000.0).max(0.0) as u32,
            }))
        }

        // `ChangeGroundOnDeath(groundToChange, changeTo, dist)`: again the second name is what
        // the ground becomes, falling back to the only name when just one was written.
        "change_ground_on_death" => Primitive::OnDeath(Box::new(DeathEffect::ChangeGround {
            tile: {
                let written = text_list(call, 0);
                names.intern(
                    written
                        .get(1)
                        .or_else(|| written.first())
                        .copied()
                        .unwrap_or_default(),
                )
            },
            radius: number(call, "radius", 0, 1.0) as f32,
        })),

        "remove_object_on_death" => Primitive::OnDeath(Box::new(DeathEffect::RemoveObjects {
            radius: number(call, "radius", 1, 10.0) as f32,
            kind: maybe_entity(call, names, "target", 0),
        })),

        "order_on_death" => Primitive::OnDeath(Box::new(DeathEffect::Order {
            radius: number(call, "range", 0, 10.0) as f32,
            kind: maybe_entity(call, names, "children", 1),
            state: Arc::from(text(call, "target_state", 2).unwrap_or_default().trim()),
        })),

        // `TransferDamageOnDeath(target, radius)`: name first, radius second.
        "transfer_damage_on_death" | "copy_damage_on_death" => {
            Primitive::OnDeath(Box::new(DeathEffect::TransferDamage {
                radius: number(call, "radius", 1, 50.0) as f32,
                kind: maybe_entity(call, names, "target", 0),
            }))
        }

        // A `prioritize` written without a block has nothing to prioritise, and the same is true
        // of the other two group forms.
        "prioritize" => Primitive::Prioritize(Vec::new()),
        "sequence" => Primitive::Sequence {
            children: Vec::new(),
        },
        "on_death_behavior" => Primitive::Unsupported {
            name: "on_death_behavior".to_string(),
        },
        "timed" => Primitive::Every {
            period_ms: number(call, "period", 0, 1000.0).max(0.0) as u32,
            children: Vec::new(),
        },
        "if" => Primitive::When {
            condition: Box::new(Condition::Unsupported {
                name: "if".to_string(),
            }),
            children: Vec::new(),
        },

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

/// The condition guarding an `if` group, which the C# writes as the call's first argument.
fn guard(call: &Call, names: &mut Names, diagnostics: &mut Vec<Diagnostic>) -> Condition {
    // The transpiler flattens the guard into this call's arguments, so an `if` whose guard did not
    // survive is one that should not run its children rather than one that always does.
    let Some(inner) = call
        .arguments
        .first()
        .and_then(|argument| argument.value.as_text())
    else {
        return Condition::Unsupported {
            name: "if".to_string(),
        };
    };

    let rebuilt = Call {
        name: inner.to_string(),
        arguments: call.arguments.iter().skip(1).cloned().collect(),
        at: call.at,
    };
    condition(&rebuilt, names, diagnostics)
}

fn condition(call: &Call, names: &mut Names, diagnostics: &mut Vec<Diagnostic>) -> Condition {
    match call.name.as_str() {
        "timed" => Condition::Timed {
            after_ms: number(call, "after", 0, 1000.0).max(0.0) as u32,
        },

        "player_within" => Condition::PlayerWithin {
            radius: number(call, "radius", 0, 10.0) as f32,
            see_invis: call
                .argument("see_invis", 1)
                .and_then(Value::as_bool)
                .unwrap_or(false),
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

        // `EntityExistsTransition(target, dist, targetState)`: the name comes first and the
        // radius second, which is the opposite way round from the plural form below.
        "entity_exists" | "entity_count_greater_than" => Condition::EntityWithin {
            kind: names.intern(text_list(call, 0).first().copied().unwrap_or_default()),
            radius: number(call, "radius", 1, 10.0) as f32,
        },

        // `EntityNotExistsTransition(target, dist, targetState)`. Singular: name first.
        "entity_not_exists" => Condition::NoneWithin {
            kinds: text_list(call, 0)
                .into_iter()
                .map(|found| names.intern(found))
                .collect(),
            radius: number(call, "radius", 1, 10.0) as f32,
        },

        // `EntitiesNotExistsTransition(dist, targetState, params targets)`. Plural: radius first,
        // then the names. The two forms really are ordered differently, and reading one with the
        // other's order gives a radius of ten where the content asked for a hundred.
        "entities_not_exists" => Condition::NoneWithin {
            kinds: text_list(call, 0)
                .into_iter()
                .map(|found| names.intern(found))
                .collect(),
            radius: number(call, "radius", 0, 10.0) as f32,
        },

        // `TimedRandomTransition(time, randomized)`: the wait is drawn from zero to `time` when
        // randomized, and is exactly `time` when not. Not a min and a max.
        "timed_random" => {
            let time = number(call, "time", 0, 1000.0).max(0.0) as u32;
            let randomized = call
                .argument("randomized", 1)
                .and_then(Value::as_number)
                .is_some_and(|value| value != 0.0);

            Condition::TimedRandom {
                min_ms: if randomized { 0 } else { time },
                max_ms: time,
            }
        }

        "damage_taken" => Condition::DamageTaken {
            amount: number(call, "amount", 0, 1.0).max(0.0) as i32,
        },

        // `PlayerTextTransition(target, regex, dist, setAttackTarget, ignoreCase)`. The target is
        // taken as the state by the transpiler, so what arrives here is the rest.
        "player_text" => Condition::PlayerSaid {
            word: call
                .argument("word", 0)
                .and_then(Value::as_text)
                .unwrap_or_default()
                .to_string(),
            within: call
                .argument("within", 1)
                .and_then(Value::as_number)
                .map(|value| value as f32),
            // The C# argument is `ignoreCase` and defaults to true, so the sense is inverted here to
            // say what it means rather than what it negates.
            exact_case: !call
                .argument("ignore_case", 3)
                .and_then(Value::as_bool)
                .unwrap_or(true),
        },

        "not_moving" => Condition::NotMoving {
            after_ms: number(call, "delay", 0, 250.0).max(0.0) as u32,
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

fn loot(entry: &crate::ast::Loot) -> Option<LootEntry> {
    let call = &entry.call;

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
        // `Threshold(share, children...)`: everything inside belongs to whoever earned it.
        //
        // Silently dropped before this existed, which is two hundred uses of the content's own
        // soulbound loot going nowhere and nothing saying so.
        "threshold" => Some(LootEntry::Threshold {
            share: number(call, "threshold", 0, 0.0) as f32,
            children: entry.children.iter().filter_map(loot).collect(),
        }),

        _ => None,
    }
}

/// A "did you mean" for a misspelled name.
///
/// Cheap edit distance over a list of ten names. The value is not the algorithm but the fact that a
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
