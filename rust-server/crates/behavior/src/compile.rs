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
    "buzz",
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
    "copy_damage_on_death",
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
    let mut transition_slot = 0usize;
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
                    group(call, children, &mut interner, diagnostics, &mut behaviours);
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
                        slot: transition_slot,
                    });
                    transition_slot += 1;
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
        transition_slots: transition_slot,
        // Unresolved: the host fills these in at load, when it has a catalog.
        kinds: vec![Vec::new(); interner.entries.len()],
        names: interner.entries,
        states,
        loot: enemy.loot.iter().filter_map(loot).collect(),
    }
}

/// Compiles one wrapper and everything inside it, appending to `into`.
///
/// Most wrappers become one primitive holding their children; `on_death_behavior` becomes its
/// children directly, which is why this appends rather than returning.
fn group(
    call: &Call,
    children: &[Item],
    names: &mut Names,
    diagnostics: &mut Vec<Diagnostic>,
    into: &mut Vec<Primitive>,
) {
    let inner = group_children(children, names, diagnostics);

    match call.name.as_str() {
        "prioritize" => into.push(Primitive::Prioritize(inner)),

        // One child per turn, which is what makes an attack pattern a pattern.
        "sequence" => into.push(Primitive::Sequence { children: inner }),

        // A wrapper whose child runs at death rather than during a tick, and only ever through
        // `OnStateEntry` (`OnDeathBehavior.cs:15-20`). Anything that does its work in `TickCore`
        // therefore does nothing at all: the content's one use wraps a `Shoot`, whose
        // `OnStateEntry` sets a cooldown and returns (`Shoot.cs:58-61`), so the original fires
        // nothing. Keeping the children as ordinary behaviours had that boss firing a fifty-shot
        // spread every tick of the state instead of never.
        "on_death_behavior" => {
            let (deaths, ticked): (Vec<_>, Vec<_>) = inner
                .into_iter()
                .partition(|child| child.death_effect().is_some());

            into.extend(deaths);

            if !ticked.is_empty() {
                diagnostics.push(Diagnostic {
                    message: format!(
                        "{} of `on_death_behavior`'s children do their work while ticking, which \
                         death never reaches; they will not run",
                        ticked.len()
                    ),
                    at: call.at,
                });
            }
        }

        // A timer around a group: everything inside runs together, and the period says how long a
        // cycle of it lasts. The C# writes it as `Timed(600, new Shoot(...))`.
        "timed" => into.push(Primitive::Every {
            period_ms: number(call, "period", 0, 1000.0).max(0.0) as u32,
            children: inner,
        }),

        // A guard around a group, which the C# writes as `If(condition, ...)`. The condition is the
        // call's first argument.
        "if" => into.push(Primitive::When {
            condition: Box::new(guard(call, names, diagnostics)),
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
            into.push(Primitive::Unsupported {
                name: call.name.clone(),
            });
        }
    }
}

/// What is inside a wrapper: behaviours, and in two dungeons further wrappers.
///
/// `Sequence(new Timed(1250, new ReturnToSpawn(0.6)), ...)` is how a boss is written to do one
/// thing for a while and then another, and reading only the behaviours left twenty-one of these
/// empty. An empty `sequence()` is a boss standing still, and nothing said so.
fn group_children(
    items: &[Item],
    names: &mut Names,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Primitive> {
    let mut out = Vec::new();

    for item in items {
        match item {
            Item::Behaviour(call) => out.push(behaviour(call, names, diagnostics)),
            Item::Group { call, children } => group(call, children, names, diagnostics, &mut out),
            Item::Transition(_) | Item::State(_) => {}
        }
    }

    out
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

/// A flag, by name or position, absent when the call does not give one.
fn maybe_flag(call: &Call, name: &str, index: usize) -> Option<bool> {
    call.argument(name, index).and_then(Value::as_bool)
}

/// A number that means something different when it is absent.
///
/// The original spells these `double?`, and the distinction is not cosmetic: `Shoot`'s fixed angle
/// is either "aim at this bearing" or "aim at whoever is nearest", and zero is a bearing.
fn maybe_number(call: &Call, name: &str, index: usize) -> Option<f64> {
    call.argument(name, index).and_then(Value::as_number)
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

/// The interned names of one bracketed list argument.
///
/// Distinct from [`text_list`], which sweeps up every trailing string: this reads *one* argument
/// that is itself a list, so a constructor taking two arrays keeps them apart. A single unbracketed
/// name counts as a list of one.
fn entity_list(call: &Call, names: &mut Names, name: &str, index: usize) -> Vec<NameRef> {
    let Some(value) = call.argument(name, index) else {
        return Vec::new();
    };
    value
        .as_text_list()
        .into_iter()
        .filter(|found| !found.trim().is_empty())
        .map(|found| names.intern(found))
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
        // Positional indices follow the C# constructor, which is the order the converter writes:
        // radius, count, shootAngle, projectileIndex, fixedAngle, rotateAngle, angleOffset,
        // defaultAngle, predictive, coolDownOffset, coolDown. Fourteen hundred shots in the content
        // pass the fixed angle positionally rather than by name, so the two angles that mean
        // something different when absent are read by position as well: a `fixedAngle` of zero is
        // an enemy firing due east forever, and reading it as "not given" turns that into an
        // enemy that aims.
        "shoot" => Primitive::Shoot {
            acquire_range: number(call, "radius", 0, 20.0).max(0.0) as f32,
            count: number(call, "count", 1, 1.0).max(1.0) as u32,
            spread: number(call, "shoot_angle", 2, 0.0) as f32,
            projectile: number(call, "projectile", 3, 0.0).clamp(0.0, 255.0) as u8,
            fixed_angle: maybe_number(call, "fixed_angle", 4).map(|degrees| degrees as f32),
            rotate_angle: number(call, "rotate_angle", 5, 0.0) as f32,
            angle_offset: number(call, "angle_offset", 6, 0.0) as f32,
            default_angle: maybe_number(call, "default_angle", 7).map(|degrees| degrees as f32),
            predictive: number(call, "predictive", 8, 0.0) as f32,
            cooldown_offset_ms: number(call, "cooldown_offset", 9, 0.0).max(0.0) as u32,
            cooldown_ms: number(call, "cooldown", 10, 1000.0).max(0.0) as u32,
        },

        "buzz" => Primitive::Buzz {
            speed: number(call, "speed", 0, 2.0) as f32,
            distance: number(call, "dist", 1, 0.5) as f32,
            cooldown_ms: number(call, "cooldown", 2, 0.0).max(0.0) as u32,
        },

        "wander" => Primitive::Wander {
            speed: number(call, "speed", 0, 0.4) as f32,
        },

        // `Follow(speed, acquireRange, range, duration, coolDown)`. A duration of zero means the
        // original's default, which is to follow for as long as the state lasts.
        "follow" => Primitive::Follow {
            speed: number(call, "speed", 0, 1.0) as f32,
            acquire_range: number(call, "acquire_range", 1, 10.0) as f32,
            range: number(call, "range", 2, 6.0) as f32,
            duration_ms: number(call, "duration", 3, 0.0).max(0.0) as u32,
            cooldown_ms: number(call, "cooldown", 4, 0.0).max(0.0) as u32,
        },

        // Both variances default to a tenth of the *speed* rather than to nothing, so every orbit
        // in the content varies even though only ninety-two of them say so.
        "orbit" => Primitive::Orbit {
            speed: number(call, "speed", 0, 1.0) as f32,
            radius: number(call, "radius", 1, 4.0) as f32,
            acquire_range: number(call, "acquire_range", 2, 10.0) as f32,
            target: maybe_entity(call, names, "target", 3),
            speed_variance: number(
                call,
                "speed_variance",
                4,
                number(call, "speed", 0, 1.0) * 0.1,
            ) as f32,
            radius_variance: number(
                call,
                "radius_variance",
                5,
                number(call, "speed", 0, 1.0) * 0.1,
            ) as f32,
            clockwise: maybe_flag(call, "orbit_clockwise", 6).or(Some(false)),
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

        // `Spawn(children, maxChildren, initialSpawn, coolDown, givesNoXp)`, where `initialSpawn`
        // is a fraction of `maxChildren` and `givesNoXp` defaults to *true*.
        "spawn" => Primitive::Spawn {
            child: entity(call, names, "children", 0),
            max_children: number(call, "max_children", 1, 5.0).max(0.0) as u32,
            initial_spawn: number(call, "initial_spawn", 2, 0.5).clamp(0.0, 1.0) as f32,
            cooldown_ms: number(call, "cooldown", 3, 1000.0).max(0.0) as u32,
            gives_no_xp: call
                .named("gives_no_xp")
                .and_then(Value::as_bool)
                .unwrap_or(true),
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
        // `SpawnGroup(group, maxChildren, initialSpawn, coolDown, radius)`.
        "spawn_group" => Primitive::Spawn {
            child: entity(call, names, "group", 0),
            max_children: number(call, "max_children", 1, 5.0).max(0.0) as u32,
            initial_spawn: number(call, "initial_spawn", 2, 0.5).clamp(0.0, 1.0) as f32,
            cooldown_ms: number(call, "cooldown", 3, 1000.0).max(0.0) as u32,
            // `SpawnGroup` never touches `GivesNoXp`, where `Spawn` writes it from an argument that
            // defaults to true (`Spawn.cs:46`) and `Reproduce` hard-codes it
            // (`Reproduce.cs:97`). A dwarf out of a group spawner is therefore worth what a dwarf
            // is worth, and there is no argument in the original that can say otherwise.
            gives_no_xp: false,
        },

        // `InvisiToss` throws without the thrower being seen doing it, and its child lands the
        // moment it is thrown: the original arms a `WorldTimer(0, ...)` (`InvisiToss.cs:56`) where
        // `TossObject` arms a `WorldTimer(1500, ...)` (`TossObject.cs:167`). Every one of the
        // content's forty-six invisible tosses gives a bearing, which is the other difference:
        // there is no nearest-target path in `InvisiToss` at all.
        "toss_object" | "invisi_toss" => Primitive::TossObject {
            child: entity(call, names, "child", 0),
            // Written `range` rather than `radius` in every real use.
            radius: number(call, "range", 1, 5.0) as f32,
            // Read by position too: a hundred and twelve tosses in the content give the bearing
            // that way, and reading it as absent throws them at whoever is nearest instead.
            fixed_angle: maybe_number(call, "angle", 2).map(|degrees| degrees as f32),
            cooldown_ms: number(call, "cooldown", 3, 1000.0).max(0.0) as u32,
            cooldown_offset_ms: number(call, "cooldown_offset", 4, 0.0).max(0.0) as u32,
            min_range: maybe_number(call, "min_range", 10).map(|range| range as f32),
            max_range: maybe_number(call, "max_range", 11).map(|range| range as f32),
            // The telegraph, and not an argument in the original at all: `TossObject` arms a
            // `WorldTimer(1500, ...)` between showing the throw and the thing landing. Without it
            // a thrown object is an unavoidable hit, which is the difference between a hard fight
            // and an unfair one, and 800 was little over half the warning the content expects.
            // An invisible toss has no telegraph and no wait, so its child is already standing
            // there when the volley that hides it arrives.
            warning_ms: if call.name == "invisi_toss" { 0 } else { 1500 },
        },

        // `Grenade(radius, damage, range, fixedAngle, coolDown, effect, effectDuration, color)`.
        "grenade" => Primitive::Grenade {
            radius: number(call, "radius", 0, 2.0) as f32,
            damage: number(call, "damage", 1, 100.0) as i32,
            range: number(call, "range", 2, 5.0) as f32,
            fixed_angle: call
                .named("fixed_angle")
                .and_then(Value::as_number)
                .map(|degrees| degrees as f32),
            cooldown_ms: number(call, "cooldown", 4, 1000.0).max(0.0) as u32,
            effect: call
                .named("effect")
                .map(|_| effect_of(call, "effect", usize::MAX, diagnostics)),
            effect_ms: duration_ms(call, "effect_duration", usize::MAX, 0.0),
        },

        "suicide" => Primitive::Suicide,

        // `RemoveEntity(dist, children)` removes *other* entities rather than itself. Reading it
        // as a suicide would have made every boss that tidies up its summons kill itself instead.
        "remove_entity" => Primitive::RemoveNearby {
            radius: number(call, "dist", 0, 10.0) as f32,
            kind: maybe_entity(call, names, "children", 1),
            dies: true,
        },

        "decay" => Primitive::Decay {
            // A written zero means the argument was elided rather than that it should vanish at
            // once, which is what the C# default of ten seconds says.
            after_ms: match number(call, "time", 0, 0.0).max(0.0) as u32 {
                0 => 10_000,
                given => given,
            },
        },

        // The imported scripts' spelling is a subclass that adds nothing (`Ported.cs:36-40`), so
        // it compiles to the same thing rather than to a second implementation of it.
        "conditional_effect" | "condition_effect_behavior" => Primitive::ConditionalEffect {
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

        // `SetAltTexture(minValue, maxValue, cooldown, loop)`, where a `maxValue` of minus one
        // means the one sprite rather than a run of them.
        "set_alt_texture" => Primitive::SetAltTexture {
            index: number(call, "min_value", 0, 0.0).clamp(0.0, 255.0) as u8,
            last: match number(call, "max_value", 1, -1.0) {
                last if last < 0.0 => None,
                last => Some(last.clamp(0.0, 255.0) as u8),
            },
            step_ms: number(call, "cooldown", 2, 0.0).max(0.0) as u32,
            looping: maybe_flag(call, "loop", 3).unwrap_or(false),
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

            // Nought is the original's word for "no limit", not for "no scaling". Both of the
            // caps in `ScaleHP.TickCore` are written `if (maxAdditional != 0)`
            // (`ScaleHP.cs:78-79` and `:93-96`), so a zero skips them and the enemy grows without
            // bound; the field's own comment says as much (`ScaleHP.cs:23`, "leave as 0 for no
            // limit"). Reading it as a ceiling of zero clamped the growth to nothing, which is the
            // opposite behaviour: Oryx is written `ScaleHP(50000)` and did not scale by a point.
            maximum_extra: match number(call, "max_additional", 1, 0.0) as i32 {
                0 => i32::MAX,
                written => written.max(0),
            },
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

        // `HealGroup(range, group, coolDown, healAmount)` — the cooldown comes *before* the
        // amount here and after it in `HealEntity`, which is why they cannot share an index.
        "heal_group" => Primitive::HealOthers {
            radius: number(call, "range", 0, 10.0) as f32,
            amount: number(call, "heal_amount", 3, 100.0) as i32,
            kind: maybe_entity(call, names, "group", 1),
            players: false,
            cooldown_ms: number(call, "cooldown", 2, 1000.0).max(0.0) as u32,
        },

        // `HealEntity(range, name, healAmount, coolDown)`.
        "heal_entity" => Primitive::HealOthers {
            radius: number(call, "range", 0, 10.0) as f32,
            amount: number(call, "heal_amount", 2, 100.0) as i32,
            kind: maybe_entity(call, names, "name", 1),
            players: false,
            cooldown_ms: number(call, "cooldown", 3, 1000.0).max(0.0) as u32,
        },

        // `HealPlayer(range, coolDown, healAmount)`.
        "heal_player" => Primitive::HealOthers {
            radius: number(call, "range", 0, 10.0) as f32,
            amount: number(call, "heal_amount", 2, 100.0) as i32,
            kind: None,
            players: true,
            cooldown_ms: number(call, "cooldown", 1, 1000.0).max(0.0) as u32,
        },

        // `MoveTo(speed, x, y)`.
        "move_to" => Primitive::MoveTo {
            x: number(call, "x", 1, 0.0) as f32,
            y: number(call, "y", 2, 0.0) as f32,
            speed: number(call, "speed", 0, 1.0) as f32,
            relative: false,
        },

        // `MoveTo2(x, y, speed, once, isMapPosition, instant)` is the same behaviour with the
        // arguments the other way round, and with the point read as an offset from where the enemy
        // was standing unless the script says otherwise.
        "move_to2" => Primitive::MoveTo {
            x: number(call, "x", 0, 0.0) as f32,
            y: number(call, "y", 1, 0.0) as f32,
            speed: number(call, "speed", 2, 2.0) as f32,
            relative: !maybe_flag(call, "is_map_position", 4).unwrap_or(false),
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
            tolerance: number(call, "return_within_radius", 1, 0.5) as f32,
        },

        "stay_above" => Primitive::StayAbove {
            speed: number(call, "speed", 0, 1.0) as f32,
            altitude: number(call, "altitude", 1, 5.0) as f32,
        },

        "set_no_x_p" => Primitive::NoExperience,

        // `RemoveTileObject(objName, range)` clears map squares rather than killing anything, so
        // what stands on those squares goes without a death.
        "remove_tile_object" => Primitive::RemoveNearby {
            radius: number(call, "radius", 1, 1.0) as f32,
            kind: maybe_entity(call, names, "target", 0),
            dies: false,
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
        // `GroundTransform(tileId, radius, relativeX, relativeY, persist)`. The pair of offsets
        // replaces the circle with a single square, and the original only honours them together.
        "ground_transform" => Primitive::GroundTransform {
            tile: entity(call, names, "tile", 0),
            radius: number(call, "radius", 1, 1.0) as f32,
            cooldown_ms: number(call, "cooldown", 5, 0.0).max(0.0) as u32,
            offset: match (
                maybe_number(call, "relative_x", 2),
                maybe_number(call, "relative_y", 3),
            ) {
                (Some(x), Some(y)) => Some((x as f32, y as f32)),
                _ => None,
            },
        },

        // `ReplaceTile(objName, replacedObjName, range)`: the *second* name is what the ground
        // becomes. Taking the first would replace the ground with what was already there, and the
        // range is the third argument rather than the first, which is where the two names are. Both
        // uses in the content write a range of nought, meaning the one square the enemy stands on;
        // read off the wrong index that became the default of one, and the Shatters' king laid a
        // disc of lava where the original lays a tile.
        "replace_tile" => Primitive::GroundTransform {
            tile: names.intern(text_list(call, 0).get(1).copied().unwrap_or_default()),
            radius: number(call, "range", 2, 1.0) as f32,
            cooldown_ms: 0,
            offset: None,
        },

        // -- what happens at death ------------------------------------------------------------

        // `TransformOnDeath(target, min, max, probability)`. The count is drawn inclusively at both
        // ends, as `Random.Next(min, max + 1)` is.
        "transform_on_death" => Primitive::OnDeath(Box::new(DeathEffect::TransformInto {
            child: entity(call, names, "target", 0),
            min: number(call, "min", 1, 1.0).max(0.0) as u32,
            max: number(call, "max", 2, 1.0).max(0.0) as u32,
            probability: number(call, "probability", 3, 1.0).clamp(0.0, 1.0) as f32,
        })),

        "drop_portal_on_death" => Primitive::OnDeath(Box::new(DeathEffect::Portal {
            name: entity(call, names, "target", 0),
            probability: {
                let raw = number(call, "probability", 1, 1.0) as f32;
                if raw > 1.0 { raw / 100.0 } else { raw }.clamp(0.0, 1.0)
            },
            duration_ms: number(call, "timeout", 2, 30_000.0).max(0.0) as u32,
        })),

        // `RealmPortalDrop` takes no arguments at all (`RealmPortalDrop.cs:12`): the portal it
        // leaves is always the one named "Realm Portal", it always leaves it, and it never times
        // out, since the original calls `EnterWorld` and arms no timer. Reading a name off an
        // argument list that has none left every realm boss dropping a portal with an empty name,
        // which resolves to nothing, so the twelve bosses that are the only way out of a realm
        // dropped no way out.
        "realm_portal_drop" => Primitive::OnDeath(Box::new(DeathEffect::Portal {
            name: names.intern("Realm Portal"),
            probability: 1.0,
            duration_ms: 0,
        })),

        // `ChangeGroundOnDeath(GroundToChange, ChangeTo, dist)`. Both name lists are C# arrays and
        // arrive here as bracketed lists, so the ground being replaced stays separate from the
        // ground replacing it even where one of them holds two names, which four of the fourteen
        // calls do. The distance is the third argument; reading it off slot nought, which is where
        // the first array sits, is how it used to be read.
        "change_ground_on_death" => Primitive::OnDeath(Box::new(DeathEffect::ChangeGround {
            sources: entity_list(call, names, "ground_to_change", 0),
            targets: entity_list(call, names, "change_to", 1),
            dist: number(call, "dist", 2, 0.0).max(0.0) as u32,
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
        "transfer_damage_on_death" => Primitive::OnDeath(Box::new(DeathEffect::TransferDamage {
            radius: number(call, "radius", 1, 50.0) as f32,
            kind: maybe_entity(call, names, "target", 0),
        })),

        // `CopyDamageOnDeath(child, dist)`, whose arguments read the same way and whose effect is
        // nothing at all.
        "copy_damage_on_death" => Primitive::OnDeath(Box::new(DeathEffect::CopyDamage {
            radius: number(call, "radius", 1, 50.0) as f32,
            kind: maybe_entity(call, names, "target", 0),
        })),

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

        // `EntityNotExistsTransition(target, dist, targetState)`. Singular: name first. The
        // imported scripts spell it a letter shorter, and the original answers with a subclass that
        // adds nothing (`PortedTransitions.cs:11-17`), so both spellings compile the same way.
        "entity_not_exists" | "entity_not_exist" => Condition::NoneWithin {
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

/// A loot argument under any of the names it goes by, or at its position.
///
/// The transpiler copies the C# argument names through verbatim, so the same field arrives spelled
/// either way: `ItemLoot(item: "X", probability: 0.01)` becomes `item(item: "X", probability: 0.01)`
/// while `ItemLoot("X", 0.01)` becomes `item("X", 0.01)`. Reading only this language's own spelling
/// finds nothing in the first form, and a name that does not resolve is a drop that never happens.
fn loot_argument<'a>(call: &'a Call, names: &[&str], index: usize) -> Option<&'a Value> {
    names
        .iter()
        .find_map(|name| call.named(name))
        .or_else(|| call.positional(index))
}

/// A loot number under any of the names it goes by.
fn loot_number(call: &Call, names: &[&str], index: usize, fallback: f64) -> f64 {
    loot_argument(call, names, index)
        .and_then(Value::as_number)
        .unwrap_or(fallback)
}

fn loot(entry: &crate::ast::Loot) -> Option<LootEntry> {
    let call = &entry.call;

    // An omitted probability is one, not zero: `ItemLoot` and `TierLoot` both default it to a
    // certain drop (`logic/loot/MobDrops.cs:47,72`), which is how the content spells a guaranteed
    // one. The positions are the C# constructors' own: item, probability, numRequired for a named
    // item, and tier, type, probability, numRequired for a whole tier.
    match call.name.as_str() {
        "item" => Some(LootEntry::Item {
            name: loot_argument(call, &["name", "item"], 0)
                .and_then(Value::as_text)?
                .to_string(),
            chance: loot_number(call, &["chance", "probability"], 1, 1.0) as f32,
            required: loot_number(call, &["num_required"], 2, 0.0).max(0.0) as u32,
            threshold: loot_number(call, &["threshold"], 3, 0.0).max(0.0) as f32,
        }),
        "tier" => Some(LootEntry::Tier {
            tier: loot_number(call, &["tier"], 0, 0.0).clamp(0.0, 255.0) as u8,
            kind: loot_argument(call, &["kind", "type"], 1)
                .and_then(Value::as_text)
                .unwrap_or("any")
                .to_string(),
            chance: loot_number(call, &["chance", "probability"], 2, 1.0) as f32,
            required: loot_number(call, &["num_required"], 3, 0.0).max(0.0) as u32,
            threshold: loot_number(call, &["threshold"], 4, 0.0).max(0.0) as f32,
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
                acquire_range,
                cooldown_offset_ms,
                ..
            } => {
                assert_eq!(*count, 15);
                assert_eq!(*spread, 24.0);
                assert_eq!(*cooldown_ms, 1200);
                assert_eq!(*projectile, 2);
                assert_eq!(*fixed_angle, None, "unset means aim at the nearest player");
                assert_eq!(*acquire_range, 1.0, "the first positional is the range");
                assert_eq!(*cooldown_offset_ms, 0);
            }
            other => panic!("expected a shoot, got {other:?}"),
        }
    }

    #[test]
    fn a_shoot_reads_its_fixed_angle_by_position_too() {
        // Fourteen hundred of the content's shots give it that way, and a bearing of zero is a
        // bearing: the Avatar's six shots at 0, 60, 120, 180, 240 and 300 are one pattern read
        // positionally, and taking "not named" for "not given" turns them into six identical shots
        // aimed at whoever is closest.
        let (programs, diagnostics) = compiled(
            r#"enemy "X" {
                 state a {
                   shoot(20, 1, 0, 1, 0)
                   shoot(20, 1, 0, 1, 60)
                   shoot(20, 1, 0, 1)
                 }
               }"#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        let program = programs.get("X").unwrap();
        let state = program.state_named("a").unwrap();

        let angle_of = |at: usize| match &program.states[state].behaviours[at] {
            Primitive::Shoot { fixed_angle, .. } => *fixed_angle,
            other => panic!("expected a shoot, got {other:?}"),
        };

        assert_eq!(angle_of(0), Some(0.0), "zero is due east, not `unset`");
        assert_eq!(angle_of(1), Some(60.0));
        assert_eq!(
            angle_of(2),
            None,
            "not given at all is still aim at somebody"
        );
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
                    chance: 0.02,
                    required: 0,
                    threshold: 0.0,
                },
                LootEntry::Tier {
                    tier: 2,
                    kind: "weapon".into(),
                    chance: 0.3,
                    required: 0,
                    threshold: 0.0,
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

#[cfg(test)]
mod ported {
    //! The behaviours whose C# class is not the one their name suggests.
    //!
    //! Five names in the content compile to a primitive this runtime already had, and each one is
    //! a different C# class with its own rules. Reading them as the primitive without the rules is
    //! how a boss came to shoot every tick where the original shoots never, and how twelve realm
    //! bosses came to drop a portal with no name.

    use super::*;
    use crate::parse::parse;

    fn behaviours(source: &str) -> Vec<Primitive> {
        let (programs, _) = compile(&parse(source).expect("should parse"));
        let program = programs.get("X").expect("the enemy");
        program
            .states
            .iter()
            .flat_map(|state| state.behaviours.clone())
            .collect()
    }

    #[test]
    fn an_invisible_toss_lands_at_once_and_a_visible_one_is_telegraphed() {
        // `InvisiToss.cs:52` arms a `WorldTimer(0, ...)`; `TossObject.cs:167` arms one of 1500.
        // The forty-six invisible tosses in the content are a boss laying summons under a volley,
        // and a second and a half of grace turns that into a summon the player has already left.
        let found = behaviours(
            r#"enemy "X" {
                 state a {
                   invisi_toss("Egg", 5, 90)
                   toss_object("Egg", 5, 90)
                 }
               }"#,
        );

        let warnings: Vec<u32> = found
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::TossObject { warning_ms, .. } => Some(*warning_ms),
                _ => None,
            })
            .collect();

        assert_eq!(warnings, vec![0, 1500]);
    }

    #[test]
    fn a_realm_portal_drop_drops_a_realm_portal() {
        // `RealmPortalDrop` takes no arguments and always leaves "Realm Portal"
        // (`RealmPortalDrop.cs:23`). Read as `DropPortalOnDeath`, the name came off an argument
        // list with nothing in it, so every realm boss dropped a portal named "".
        let (programs, _) = compile(
            &parse(
                r#"enemy "X" {
                     realm_portal_drop()
                   }"#,
            )
            .expect("should parse"),
        );
        let program = programs.get("X").expect("the enemy");

        let found = program
            .states
            .iter()
            .flat_map(|state| &state.behaviours)
            .find_map(Primitive::death_effect)
            .expect("a death effect");

        let DeathEffect::Portal {
            name,
            probability,
            duration_ms,
        } = found
        else {
            panic!("expected a portal, found {found:?}");
        };

        assert_eq!(program.text_of(*name), "Realm Portal");
        assert_eq!(*probability, 1.0);
        // The original arms no closing timer at all, and zero is how this says so.
        assert_eq!(*duration_ms, 0);
    }

    #[test]
    fn on_death_behavior_does_not_run_its_child_while_alive() {
        // `OnDeathBehavior` hands its child to `parent.Death` and calls only `OnStateEntry`
        // (`OnDeathBehavior.cs:15-20`). `Shoot.OnStateEntry` sets a cooldown and returns
        // (`Shoot.cs:58-61`), so the original fires nothing; keeping the child as an ordinary
        // behaviour had the Shatters' boss firing a fifty-shot spread every tick of the state.
        let (programs, diagnostics) = compile(
            &parse(
                r#"enemy "X" {
                     state a {
                       on_death_behavior() {
                         shoot(50, 12, 360, 3, 45)
                       }
                     }
                   }"#,
            )
            .expect("should parse"),
        );
        let program = programs.get("X").expect("the enemy");

        assert!(
            program
                .states
                .iter()
                .all(|state| state.behaviours.is_empty()),
            "nothing should tick"
        );
        assert!(
            diagnostics
                .iter()
                .any(|note| note.message.contains("ticking")),
            "the dropped child should be reported: {diagnostics:?}"
        );
    }

    #[test]
    fn a_death_effect_inside_on_death_behavior_still_happens() {
        // The wrapper is not inert in general: a child whose whole implementation is a `Death`
        // handler runs exactly as it would outside the wrapper.
        let found = behaviours(
            r#"enemy "X" {
                 state a {
                   on_death_behavior() {
                     transform_on_death("Ghost")
                   }
                 }
               }"#,
        );

        assert!(
            found.iter().any(|primitive| matches!(
                primitive.death_effect(),
                Some(DeathEffect::TransformInto { .. })
            )),
            "found {found:?}"
        );
    }

    #[test]
    fn a_group_spawner_makes_children_worth_something() {
        // `SpawnGroup` never writes `GivesNoXp`, where `Spawn` writes it from an argument
        // defaulting to true (`Spawn.cs:46`). Reading the group form as the plain one made every
        // dwarf, warg and pyre out of a group spawner worth nothing.
        let found = behaviours(
            r#"enemy "X" {
                 state a {
                   spawn_group("Dwarves", max_children: 10, cooldown: 8000)
                   spawn("Dwarf")
                 }
               }"#,
        );

        let xp: Vec<bool> = found
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::Spawn { gives_no_xp, .. } => Some(*gives_no_xp),
                _ => None,
            })
            .collect();

        assert_eq!(xp, vec![false, true], "group first, plain second");
    }

    #[test]
    fn replacing_tiles_reads_its_range_past_the_two_names() {
        // `ReplaceTile(objName, replacedObjName, range)`: the range is the third argument, and
        // both uses in the content pass nought, which is the single square the enemy stands on.
        let found = behaviours(
            r#"enemy "X" {
                 state a {
                   replace_tile("Dark Cobblestone", "Hot Lava", 0)
                 }
               }"#,
        );

        let Some(Primitive::GroundTransform { radius, .. }) = found.first() else {
            panic!("expected a ground transform, found {found:?}");
        };

        assert_eq!(*radius, 0.0);
    }

    #[test]
    fn the_imported_spellings_compile_to_what_they_subclass() {
        // `ConditionEffectBehavior` and `EntityNotExistTransition` are subclasses that add nothing
        // (`Ported.cs:36-40`, `PortedTransitions.cs:11-17`), so they must not be a second
        // implementation and must not be a gap either.
        let (programs, _) = compile(
            &parse(
                r#"enemy "X" {
                     state a {
                       condition_effect_behavior(invincible)
                       on entity_not_exist("Egg", 10) -> b
                     }
                     state b { }
                   }"#,
            )
            .expect("should parse"),
        );
        let program = programs.get("X").expect("the enemy");
        let a = program.state_named("a").expect("state a");

        assert!(matches!(
            program.states[a].behaviours.first(),
            Some(Primitive::ConditionalEffect { .. })
        ));
        assert!(matches!(
            program.states[a].transitions.first().map(|t| &t.condition),
            Some(Condition::NoneWithin { .. })
        ));
    }
}

#[cfg(test)]
mod parity {
    //! Every argument the original's constructors take is either read or listed here.
    //!
    //! The censuses this project relies on count *names*: `shoot` resolves to `Primitive::Shoot`,
    //! so it reports as covered, and `Primitive::Shoot` had five fields where the C# constructor
    //! has twelve parameters. Nothing measured the difference, which is how an enemy came to fire
    //! at anything within twenty tiles and a boss's staggered volley came to fire on one frame.
    //!
    //! This reads the original's own signatures and checks each parameter against the arm that
    //! compiles it. An argument that is deliberately not read belongs in [`IGNORED`] with the
    //! reason, so that "we decided not to" and "we forgot" stop looking the same.
    //!
    //! Each excuse also carries a [`Passed`] claim about the original's own database, and
    //! [`no_excuse_about_the_corpus_is_only_an_assertion`] re-derives that claim from the
    //! twenty-three thousand lines of `logic/db` rather than believing it. "No call passes it" was
    //! written on an argument four calls pass.

    use std::collections::{BTreeMap, BTreeSet};

    /// Where the original's behaviour and transition sources are, relative to this crate.
    const CSHARP: [&str; 3] = [
        "../../../Server-Side/wServer/logic/behaviors",
        "../../../Server-Side/wServer/logic/transitions",
        "../../../Server-Side/wServer/logic/loot",
    ];

    /// Where the original's enemy scripts are, which is what decides whether an argument matters.
    const DATABASE: &str = "../../../Server-Side/wServer/logic/db";

    /// What the original's database does with an argument nothing here reads.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Passed {
        /// No call passes it, so the C# default is the only value it ever has.
        Never,

        /// Every call that passes it passes this one value, written as the corpus writes it.
        ///
        /// Usually the C# default said out loud, which is the same as not saying it.
        Only(&'static str),

        /// Passed, with values that vary or that are not the default, and the original still does
        /// nothing observable with it. The reason cites the C# line that drops it.
        Inert,

        /// Not an argument in the sense this census means: the parameter is the `params` tail that
        /// holds the nested behaviours, and the language spells those as a block.
        Shape,
    }

    /// One argument no arm reads, what the original's database does with it, and why that is right.
    struct Excuse {
        class: &'static str,
        parameter: &'static str,
        passed: Passed,
        why: &'static str,
    }

    const fn excuse(
        class: &'static str,
        parameter: &'static str,
        passed: Passed,
        why: &'static str,
    ) -> Excuse {
        Excuse {
            class,
            parameter,
            passed,
            why,
        }
    }

    /// Arguments no arm reads, and why that is right.
    ///
    /// Keyed by the C# class. Every entry is checked against `logic/db` rather than asserted, so an
    /// argument the converter drops cannot go on looking like an argument nobody uses.
    const IGNORED: &[Excuse] = &[
        // Nothing in the original's own database passes these at all.
        excuse(
            "Shoot",
            "shootLowHp",
            Passed::Never,
            "no call passes it, across 4,493 shots",
        ),
        excuse("TossObject", "probability", Passed::Never, "no call passes it"),
        excuse("TossObject", "group", Passed::Never, "no call passes it"),
        excuse("TossObject", "densityRange", Passed::Never, "no call passes it"),
        excuse("TossObject", "maxDensity", Passed::Never, "no call passes it"),
        excuse("TossObject", "region", Passed::Never, "no call names a region"),
        excuse("TossObject", "regionRange", Passed::Never, "no call names a region"),
        excuse("Reproduce", "region", Passed::Never, "no call names a region"),
        excuse("Reproduce", "regionRange", Passed::Never, "no call names a region"),
        excuse(
            "Grenade",
            "color",
            Passed::Never,
            "no call passes it; the six `color:` uses are all `Flash`",
        ),
        excuse("DropPortalOnDeath", "XAdjustment", Passed::Never, "no call passes it"),
        excuse("DropPortalOnDeath", "YAdjustment", Passed::Never, "no call passes it"),
        // The two `ScaleHP` excuses rest on a corpus that cannot compile against the pristine
        // signature, so they are stated carefully rather than as a plain "nobody passes it".
        //
        // Pristine `ScaleHP.cs:28` declares `int maxAdditional` with *no* default. The only two
        // calls anywhere are `new ScaleHP(50000)` (`BehaviorDb.Oryx.cs:17,96`), which cannot
        // compile against it — and a previous round of this project reconciled the two by adding
        // `= 0` to the C#, which is editing the specification to fit the content. The behaviour
        // library is the shipped server's; every file in `logic/db` was imported from another fork
        // in August 2026, so there is no pristine caller of `ScaleHP` at all and the argument
        // census here is measuring the imported fork rather than the original.
        //
        // What is *not* in doubt is what a zero means, because that is pristine: `ScaleHP.cs:23`
        // documents `maxAdditional` as "leave as 0 for no limit" and both caps are guarded
        // `if (maxAdditional != 0)`. That is what the arm implements, and it is why neither of
        // these two is read.
        excuse(
            "ScaleHP",
            "healAfterMax",
            Passed::Never,
            "no imported call passes it; it only matters once a cap has been hit, and \
             the two calls set no cap",
        ),
        excuse(
            "ScaleHP",
            "scaleAfter",
            Passed::Never,
            "no imported call passes it; nought means the first player already counts, \
             which is what the arm does",
        ),
        excuse(
            "SpawnGroup",
            "radius",
            Passed::Never,
            "no call passes it, so the group does land together",
        ),
        // Passed, but only ever the value the C# would have used anyway.
        excuse(
            "StayBack",
            "entity",
            Passed::Only("null"),
            "13 of 50 calls pass it, every one of them `null`, which is the default",
        ),
        // `MoveTo2(X, Y, speed, once, isMapPosition, instant)`. Four calls pass `instant`, all of
        // them `false` — this was written down as "no call passes it", which the census disproved.
        excuse(
            "MoveTo2",
            "instant",
            Passed::Only("false"),
            "4 of 29 calls pass it, every one of them `false`, which is the default",
        ),
        excuse(
            "OrderOnDeath",
            "probability",
            Passed::Only("1"),
            "2 of 2 calls pass it, both at one, which is the default",
        ),
        excuse(
            "ReproduceChildren",
            "initialSpawn",
            Passed::Only("0.5"),
            "1 of 1 calls passes it, at the default of a half",
        ),
        // Every call passes `true`, and true is "leave the ground as it is", which is what happens
        // here: the transform is not reverted when the state is left.
        excuse(
            "GroundTransform",
            "persist",
            Passed::Only("true"),
            "5 of 5 calls pass true, and leaving the ground alone is what this does",
        ),
        // Passed with values that are not the default, and inert in the original all the same.
        //
        // The bearings are read at `TossObject.cs:132`, inside
        // `if (_angle == null && _minAngle != null && _maxAngle != null)`. All three calls that give
        // them give an `angle:` as well (`BehaviorDb.GhostShip.cs:42,140,193`, each
        // `angle: 1, minAngle: 1, maxAngle: 359`), so `_angle` is never null where they are read and
        // that branch never runs.
        excuse(
            "TossObject",
            "minAngle",
            Passed::Inert,
            "3 calls, all alongside an `angle:`, which makes the branch reading it at \
             `TossObject.cs:132` unreachable",
        ),
        excuse(
            "TossObject",
            "maxAngle",
            Passed::Inert,
            "3 calls, all alongside an `angle:`, which makes the branch reading it at \
             `TossObject.cs:132` unreachable",
        ),
        // `if (host.X == X && host.Y == Y && once)`, inside a branch only entered while the entity
        // is still more than half a tile away (`MoveTo2.cs:53-66`): exact equality after a
        // normalised step, guarded by an inequality that rules it out.
        excuse(
            "MoveTo2",
            "once",
            Passed::Inert,
            "21 calls; the latch it arms cannot fire in the original either \
             (`MoveTo2.cs:53-66`)",
        ),
        // Only suppresses the `ShowEffect` throw arc (`TossObject.cs:164`). Nothing in this server
        // sends that effect for any throw, so both forms of the behaviour look the same here.
        excuse(
            "TossObject",
            "tossInvis",
            Passed::Inert,
            "12 calls; it suppresses the `ShowEffect` arc of `TossObject.cs:164`, \
             which this server never sends",
        ),
        // Read out of the string list rather than as a positional argument, because the call takes
        // a set of names and the second name is the one that matters.
        excuse(
            "ReplaceTile",
            "replacedObjName",
            Passed::Inert,
            "taken from the name list by `text_list`, not by position",
        ),
        // The nested block, which the parser reads as a block rather than as an argument.
        excuse("OnDeathBehavior", "behavior", Passed::Shape, "a nested block, read by shape"),
        excuse("Prioritize", "children", Passed::Shape, "a nested block, read by shape"),
        excuse("Sequence", "children", Passed::Shape, "a nested block, read by shape"),
        excuse("Timed", "behaviors", Passed::Shape, "a nested block, read by shape"),
        excuse("ReproduceChildren", "children", Passed::Shape, "a nested block, read by shape"),
        // `Threshold(threshold, params MobDrops[] children)` (`logic/loot/MobDrops.cs:110`). The
        // share is read; the children are the entries written inside the block.
        excuse("Threshold", "children", Passed::Shape, "a nested block, read by shape"),
        // `If(ICondition condition, params Behavior[] behaviors)`. The condition is compiled by
        // `guard`, which reads the call's own arguments rather than a slot of this one, and the
        // behaviours are the block.
        //
        // `If` is not the shipped server's: it is declared in `logic/behaviors/Ported.cs`, which was
        // written for this repository when the dungeon scripts were imported from another fork.
        // Parity against it is parity against ourselves and proves nothing; it is carried here so
        // the census covers every class the tree declares rather than only the ones we trust.
        excuse("If", "condition", Passed::Shape, "compiled by `guard`, from the call itself"),
        excuse("If", "behaviors", Passed::Shape, "a nested block, read by shape"),
    ];

    /// Which arguments the original's own enemy scripts actually pass, and what they pass.
    ///
    /// Keyed class, then parameter, to the set of values written in the corpus, each rendered the
    /// way [`Passed::Only`] spells it. Named arguments are taken at their name; positional ones are
    /// matched against the constructor signature, with anything past the last parameter belonging
    /// to the `params` tail that ends several of these signatures.
    ///
    /// This reads `logic/db` rather than `content/behaviours`, so an argument our own converter
    /// drops cannot be mistaken for an argument nobody uses — which is exactly how
    /// `ChangeGroundOnDeath`'s two name lists stayed unread.
    fn passed_by_the_corpus() -> BTreeMap<String, BTreeMap<String, BTreeSet<String>>> {
        use crate::csharp::{CsCall, CsValue};

        fn written(value: &CsValue) -> String {
            match value {
                CsValue::Number(number) if number.fract() == 0.0 => format!("{}", *number as i64),
                CsValue::Number(number) => format!("{number}"),
                CsValue::Text(text) => format!("{text:?}"),
                CsValue::Bool(value) => value.to_string(),
                CsValue::Null => "null".to_string(),
                CsValue::Path(path) => path.clone(),
                CsValue::Call(call) => format!("{}(..)", call.name),
                CsValue::List(_) => "[..]".to_string(),
            }
        }

        fn collect(value: &CsValue, out: &mut Vec<CsCall>) {
            match value {
                CsValue::Call(call) => {
                    out.push(call.clone());
                    for argument in &call.arguments {
                        collect(&argument.value, out);
                    }
                }
                CsValue::List(items) => {
                    for item in items {
                        collect(item, out);
                    }
                }
                _ => {}
            }
        }

        let constructors = constructors();
        let mut found: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();

        let Ok(entries) = std::fs::read_dir(DATABASE) else {
            return found;
        };
        let mut paths: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("cs"))
            .collect();
        paths.sort();

        for path in paths {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let mut calls = Vec::new();
            for enemy in crate::csharp::read_enemies(&text) {
                for argument in &enemy.arguments {
                    collect(argument, &mut calls);
                }
            }

            for call in calls {
                let Some(parameters) = constructors.get(&call.name) else {
                    continue;
                };
                let entry = found.entry(call.name.clone()).or_default();
                let mut position = 0usize;
                for argument in &call.arguments {
                    let parameter = match &argument.name {
                        Some(name) => name.clone(),
                        None => {
                            // Past the end of the signature is the `params` tail, which every
                            // remaining argument belongs to.
                            let at = position.min(parameters.len().saturating_sub(1));
                            position += 1;
                            parameters[at].clone()
                        }
                    };
                    entry
                        .entry(parameter)
                        .or_default()
                        .insert(written(&argument.value));
                }
            }
        }

        found
    }

    /// The language's name for a C# class, where it is not simply its snake case.
    ///
    /// The three loot constructors are spelt for what they are rather than for the classes that
    /// implement them: a loot table says `item`, `tier` and `threshold`.
    fn dsl_name(class: &str) -> String {
        match class {
            "ItemLoot" => "item".to_string(),
            "TierLoot" => "tier".to_string(),
            "Threshold" => "threshold".to_string(),
            other => crate::transpile::snake_for_test(other),
        }
    }

    /// The C# names that become something other than their snake case.
    fn argument_name(csharp: &str) -> String {
        match csharp {
            "coolDown" => "cooldown".to_string(),
            "projectileIndex" => "projectile".to_string(),
            "coolDownOffset" => "cooldown_offset".to_string(),
            other => crate::transpile::snake_for_test(other),
        }
    }

    /// Every class the original declares in these directories, with its constructor's parameters.
    ///
    /// Keyed on the classes each file *declares* rather than on the file's own name. Three of the
    /// loot constructors — `ItemLoot`, `TierLoot` and `Threshold` — share `MobDrops.cs` between
    /// them, so keying on the file name found none of them and the census was structurally blind to
    /// the whole of `logic/loot`. A census that cannot see a directory reports full coverage of it.
    fn constructors() -> BTreeMap<String, Vec<String>> {
        let mut found = BTreeMap::new();

        for directory in CSHARP {
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("cs") {
                    continue;
                }
                let Ok(source) = std::fs::read_to_string(&path) else {
                    continue;
                };

                for class in classes_declared(&source) {
                    let Some(start) = source.find(&format!("public {class}(")) else {
                        continue;
                    };
                    let open = start + source[start..].find('(').unwrap();
                    let Some(close) = matching(&source, open) else {
                        continue;
                    };

                    let names = source[open + 1..close]
                        .split(',')
                        .filter_map(|part| {
                            let head = part.split('=').next()?.trim();
                            head.split_whitespace().last().map(str::to_string)
                        })
                        .filter(|name| !name.is_empty())
                        .collect::<Vec<_>>();

                    if !names.is_empty() {
                        found.insert(class, names);
                    }
                }
            }
        }

        found
    }

    /// The names of every class declared in one source file.
    fn classes_declared(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut from = 0usize;

        while let Some(at) = source[from..].find("class ") {
            let at = from + at;
            from = at + "class ".len();

            // `class` has to be a word of its own, so `subclass ` and the like are not it.
            let before = source[..at].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }

            let name: String = source[from..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                found.push(name);
            }
        }

        found.sort();
        found.dedup();
        found
    }

    /// Which positional slots an arm reads.
    ///
    /// The readers all take the index as their last-but-one argument — `number(call, "x", 2, 0.0)`,
    /// `entity(call, names, "x", 2)` — so the integers in the arm are the positions it collects.
    fn positions_read(arm: &str) -> Vec<usize> {
        let mut found = Vec::new();
        for reader in [
            "number(call,",
            "entity(call,",
            "text(call,",
            "argument(",
            "text_list(call,",
            "entity_list(call,",
        ] {
            let mut from = 0usize;
            while let Some(at) = arm[from..].find(reader) {
                let at = from + at;
                let tail = &arm[at..];
                let end = tail.find(')').unwrap_or(tail.len());
                for piece in tail[..end].split(',') {
                    if let Ok(index) = piece.trim().parse::<usize>() {
                        found.push(index);
                    }
                }
                from = at + reader.len();
            }
        }
        found
    }

    /// The index of the bracket closing the one at `open`.
    fn matching(source: &str, open: usize) -> Option<usize> {
        let bytes = source.as_bytes();
        let mut depth = 0usize;
        for (offset, byte) in bytes.iter().enumerate().skip(open) {
            match byte {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(offset);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The body of the match arm that compiles a DSL name, if there is one.
    fn arm_for(name: &str) -> Option<&'static str> {
        const SOURCE: &str = include_str!("compile.rs");

        let marker = format!("\"{name}\"");
        let mut from = 0usize;
        while let Some(at) = SOURCE[from..].find(&marker) {
            let at = from + at;
            let line_start = SOURCE[..at].rfind('\n').map_or(0, |n| n + 1);
            let indent = SOURCE[line_start..at].len();

            // The arms of `behaviour` and `transition` sit at eight spaces and are the only place
            // a quoted name is followed by `=>` or by another quoted alternative.
            let tail = &SOURCE[at + marker.len()..];
            let is_arm = indent == 8
                && (tail.trim_start().starts_with("=>") || tail.trim_start().starts_with('|'));

            if is_arm {
                let body_start = at;
                let body_end = SOURCE[body_start..]
                    .find("\n        \"")
                    .map(|end| body_start + end)
                    .unwrap_or(SOURCE.len());
                return Some(&SOURCE[body_start..body_end]);
            }
            from = at + marker.len();
        }
        None
    }

    #[test]
    fn every_argument_the_original_takes_is_read_or_written_down() {
        let constructors = constructors();
        if constructors.is_empty() {
            eprintln!("skipping: the original's sources are not where the test looks for them");
            return;
        }

        let mut missing: Vec<String> = Vec::new();

        for (class, parameters) in &constructors {
            let dsl = dsl_name(class);
            let Some(arm) = arm_for(&dsl) else {
                continue;
            };

            let positions = positions_read(arm);

            for (at, parameter) in parameters.iter().enumerate() {
                // Read by name, or read at the position the constructor puts it. Both are fine:
                // what is not fine is neither, which is an argument the content passes and nothing
                // collects.
                let wanted = argument_name(parameter);
                if arm.contains(&format!("\"{wanted}\"")) || positions.contains(&at) {
                    continue;
                }
                if IGNORED
                    .iter()
                    .any(|entry| entry.class == class && entry.parameter == parameter)
                {
                    continue;
                }
                missing.push(format!(
                    "{class}.{parameter} (looked for {wanted:?} or slot {at})"
                ));
            }
        }

        assert!(
            missing.is_empty(),
            "these arguments are neither read nor listed in IGNORED:\n  {}",
            missing.join("\n  ")
        );
    }

    #[test]
    fn nothing_is_ignored_that_is_actually_read() {
        // An entry that stops being true is worse than none: it says a decision was made where the
        // code has since moved on.
        let constructors = constructors();
        if constructors.is_empty() {
            return;
        }

        for Excuse {
            class,
            parameter,
            why,
            ..
        } in IGNORED
        {
            let Some(parameters) = constructors.get(*class) else {
                panic!("IGNORED names {class}, which has no constructor in the original");
            };
            assert!(
                parameters.iter().any(|name| name == parameter),
                "IGNORED names {class}.{parameter}, which is not one of its arguments"
            );
            assert!(!why.is_empty(), "{class}.{parameter} needs a reason");

            // An excuse for a class this crate has no arm for is an excuse for nothing.
            let dsl = dsl_name(class);
            let Some(arm) = arm_for(&dsl) else {
                panic!("IGNORED names {class}, which this crate compiles no arm for");
            };
            let wanted = argument_name(parameter);
            let at = parameters.iter().position(|name| name == parameter).unwrap();
            assert!(
                !arm.contains(&format!("\"{wanted}\"")) && !positions_read(arm).contains(&at),
                "IGNORED names {class}.{parameter}, which the arm does read"
            );
        }
    }


    #[test]
    fn no_excuse_about_the_corpus_is_only_an_assertion() {
        // Three of the excuses in `IGNORED` are claims about what the original's own scripts do:
        // that nothing passes an argument, or that everything passing it passes one harmless
        // value. Those are facts about twenty-three thousand lines of C# and nobody can hold them
        // in their head, so they are re-derived here instead of believed. `MoveTo2.instant` said
        // "no call passes it" and four calls pass it.
        let corpus = passed_by_the_corpus();
        if corpus.is_empty() {
            eprintln!("skipping: the original's database is not where the test looks for it");
            return;
        }

        let mut wrong: Vec<String> = Vec::new();

        for entry in IGNORED {
            let seen = corpus
                .get(entry.class)
                .and_then(|class| class.get(entry.parameter));

            match (entry.passed, seen) {
                (Passed::Never, Some(values)) => wrong.push(format!(
                    "{}.{} claims no call passes it; the corpus passes {:?}",
                    entry.class, entry.parameter, values
                )),
                (Passed::Never, None) => {}

                (Passed::Only(_), None) => wrong.push(format!(
                    "{}.{} claims a value the corpus never passes at all",
                    entry.class, entry.parameter
                )),
                (Passed::Only(only), Some(values)) => {
                    if values.len() != 1 || !values.contains(only) {
                        wrong.push(format!(
                            "{}.{} claims only {only:?}; the corpus passes {values:?}",
                            entry.class, entry.parameter
                        ));
                    }
                }

                // Inert and Shape make no claim about how often the corpus passes it, because the
                // reason they give is about the C# that reads it rather than the C# that writes it.
                (Passed::Inert | Passed::Shape, _) => {}
            }
        }

        assert!(
            wrong.is_empty(),
            "these excuses do not survive reading `logic/db`:\n  {}",
            wrong.join("\n  ")
        );
    }

    #[test]
    fn the_arrays_the_original_writes_reach_the_arm_that_reads_them() {
        // `ChangeGroundOnDeath(new[] { "Pure Evil" }, new[] { "shtrs Disaster Floor", … }, 30)` is
        // the only place the database writes a C# array literal, and the converter used to drop
        // both of them: all fourteen calls became `change_ground_on_death(30)`, which named no
        // ground to change, no ground to change it to, and read the distance out of the slot the
        // first array had vacated. The whole path is checked here because each half of it looked
        // fine on its own.
        let source = r#".Init("X", new State(
            new ChangeGroundOnDeath(new[] { "shtrs Shattered Floor", "shtrs Disaster Floor" },
                new[] { "shtrs Pure Evil" }, 30)))"#;

        let mut report = crate::transpile::Report::default();
        let converted = crate::transpile::transpile(source, &mut report);
        assert!(
            converted.contains(
                r#"change_ground_on_death(["shtrs Shattered Floor", "shtrs Disaster Floor"], ["shtrs Pure Evil"], 30)"#
            ),
            "the arrays did not survive conversion: {converted}"
        );

        let behaviours = crate::parse::parse(&converted).expect("it parses");
        let (programs, _) = super::compile(&behaviours);
        let found = programs
            .programs
            .iter()
            .flat_map(|program| &program.states)
            .flat_map(|state| &state.behaviours)
            .find_map(|behaviour| match behaviour {
                crate::Primitive::OnDeath(effect) => match effect.as_ref() {
                    crate::program::DeathEffect::ChangeGround {
                        sources,
                        targets,
                        dist,
                    } => Some((sources.len(), targets.len(), *dist)),
                    _ => None,
                },
                _ => None,
            })
            .expect("the ground change");

        assert_eq!(
            found,
            (2, 1, 30),
            "two kinds of ground become one, over thirty squares"
        );
    }
}

#[cfg(test)]
mod classes {
    //! Every behaviour and transition class the original builds is accounted for by name.
    //!
    //! The argument census in [`super::parity`] checks the classes this runtime already compiles,
    //! one parameter at a time. It says nothing about a class that has no arm at all: a behaviour
    //! the original ships and this one has never heard of passes that check by not being looked at.
    //!
    //! This is the other half. It reads the original's own directory listing, keeps only the files
    //! its project actually builds, extracts every class that is a behaviour, a transition or a
    //! condition, and demands a verdict for each. A class with a verdict of [`Verdict::Runs`] is
    //! compiled from a sample written in this language and must produce something other than
    //! `Unsupported`, so deleting an arm fails here rather than at a player's expense. A class with
    //! a verdict of [`Verdict::Unused`] is one the original never instantiates in
    //! `logic/db`, which is the whole of its content; implementing those would be writing
    //! mechanics no enemy asks for.

    use std::collections::{BTreeMap, BTreeSet};

    use crate::program::{Condition, Primitive};

    const PROJECT: &str = "../../../Server-Side/wServer/wServer.csproj";

    /// Where the original's behaviour and transition sources are, relative to this crate.
    const CSHARP: [(&str, &str); 2] = [
        ("behaviors", "../../../Server-Side/wServer/logic/behaviors"),
        (
            "transitions",
            "../../../Server-Side/wServer/logic/transitions",
        ),
    ];

    /// The base types that make a class one of the things this crate has to answer for.
    ///
    /// `ICondition` is in the list because `If` takes one rather than a behaviour, so a condition
    /// class is as much a gap as a behaviour class when it is missing.
    const ROOTS: [&str; 4] = ["Behavior", "CycleBehavior", "Transition", "ICondition"];

    enum Verdict {
        /// Compiled by this runtime. The text is a sample enemy body that must survive compiling.
        Runs(&'static str),

        /// In the original's build, but instantiated nowhere in `logic/db`.
        ///
        /// The reason is the count, so that a class that starts being used stops being excused:
        /// the census in `examples/gaps` reads the same corpus and would report it.
        Unused(&'static str),
    }

    use Verdict::{Runs, Unused};

    /// Every class, and what this runtime does about it.
    const ACCOUNTED: &[(&str, Verdict)] = &[
        // -- behaviours this runtime compiles ------------------------------------------------
        ("Shoot", Runs("shoot(10)")),
        ("Wander", Runs("wander(0.4)")),
        ("Buzz", Runs("buzz(0.4)")),
        ("Follow", Runs("follow(0.75)")),
        ("Orbit", Runs("orbit(0.4, 3)")),
        ("StayBack", Runs("stay_back(0.4, 5)")),
        ("StayCloseToSpawn", Runs("stay_close_to_spawn(0.4, 5)")),
        ("StayAbove", Runs("stay_above(0.4, 5)")),
        ("ReturnToSpawn", Runs("return_to_spawn(0.4)")),
        ("MoveTo", Runs("move_to(1, 5, 5)")),
        ("MoveTo2", Runs("move_to2(5, 5, 1)")),
        ("MoveLine", Runs("move_line(1, 90)")),
        ("BackAndForth", Runs("back_and_forth(1, 5)")),
        ("Charge", Runs("charge(1, 10)")),
        ("Swirl", Runs("swirl(1, 5)")),
        ("Protect", Runs("protect(0.5, \"Guarded\")")),
        ("HealSelf", Runs("heal_self(1000, 100)")),
        ("HealGroup", Runs("heal_group(10, \"Group\", 1000, 100)")),
        ("HealEntity", Runs("heal_entity(10, \"Ally\", 100, 1000)")),
        ("HealPlayer", Runs("heal_player(5, 1000, 100)")),
        ("Spawn", Runs("spawn(\"Child\")")),
        ("SpawnGroup", Runs("spawn_group(\"Group\")")),
        ("Reproduce", Runs("reproduce(\"Child\")")),
        (
            "ReproduceChildren",
            Runs("reproduce_children(5, 0.5, 5000, \"Child\")"),
        ),
        ("TossObject", Runs("toss_object(\"Egg\", 5, 90)")),
        ("InvisiToss", Runs("invisi_toss(\"Egg\", 5, 90)")),
        ("Grenade", Runs("grenade(2, 100, 5)")),
        ("Suicide", Runs("suicide()")),
        ("Decay", Runs("decay(5000)")),
        ("RemoveEntity", Runs("remove_entity(10, \"Summon\")")),
        ("RemoveTileObject", Runs("remove_tile_object(\"Wall\", 10)")),
        ("ReplaceTile", Runs("replace_tile(\"Stone\", \"Lava\", 0)")),
        ("GroundTransform", Runs("ground_transform(\"Lava\", 3)")),
        ("ApplySetpiece", Runs("apply_setpiece(\"LavaSquare\")")),
        ("ConditionalEffect", Runs("conditional_effect(invincible)")),
        (
            "ConditionEffectBehavior",
            Runs("condition_effect_behavior(invincible)"),
        ),
        (
            "RemoveConditionalEffect",
            Runs("remove_conditional_effect(invincible)"),
        ),
        ("SetAltTexture", Runs("set_alt_texture(1)")),
        ("ChangeSize", Runs("change_size(10, 200)")),
        ("Flash", Runs("flash(16711680, 1, 3)")),
        ("Taunt", Runs("taunt(\"hello\")")),
        ("Order", Runs("order(10, \"Minion\", \"angry\")")),
        ("OrderOnce", Runs("order_once(10, \"Minion\", \"angry\")")),
        ("Transform", Runs("transform(\"Second Form\")")),
        ("ScaleHP", Runs("scale_h_p(50000)")),
        ("SetNoXP", Runs("set_no_x_p()")),
        // Groups, which only mean anything with a block after them.
        ("Prioritize", Runs("prioritize() { wander(0.4) }")),
        ("Sequence", Runs("sequence() { wander(0.4) }")),
        ("Timed", Runs("timed(600) { shoot(10) }")),
        ("If", Runs("if(\"player_within\", 10) { shoot(10) }")),
        (
            "OnDeathBehavior",
            Runs("on_death_behavior() { transform_on_death(\"Ghost\") }"),
        ),
        // What happens at death.
        ("TransformOnDeath", Runs("transform_on_death(\"Ghost\")")),
        (
            "DropPortalOnDeath",
            Runs("drop_portal_on_death(\"Portal\")"),
        ),
        ("RealmPortalDrop", Runs("realm_portal_drop()")),
        ("ChangeGroundOnDeath", Runs("change_ground_on_death(30)")),
        (
            "RemoveObjectOnDeath",
            Runs("remove_object_on_death(\"Summon\", 10)"),
        ),
        (
            "OrderOnDeath",
            Runs("order_on_death(10, \"Minion\", \"sad\")"),
        ),
        (
            "TransferDamageOnDeath",
            Runs("transfer_damage_on_death(\"Core\", 50)"),
        ),
        (
            "CopyDamageOnDeath",
            Runs("copy_damage_on_death(\"Core\", 50)"),
        ),
        // -- transitions this runtime compiles ------------------------------------------------
        ("TimedTransition", Runs("on timed(1000) -> other")),
        (
            "TimedRandomTransition",
            Runs("on timed_random(1000, 1) -> other"),
        ),
        (
            "PlayerWithinTransition",
            Runs("on player_within(10) -> other"),
        ),
        (
            "NoPlayerWithinTransition",
            Runs("on no_player_within(10) -> other"),
        ),
        ("HpLessTransition", Runs("on hp_below(0.5) -> other")),
        (
            "EntityExistsTransition",
            Runs("on entity_exists(\"Egg\", 10) -> other"),
        ),
        (
            "EntityNotExistsTransition",
            Runs("on entity_not_exists(\"Egg\", 10) -> other"),
        ),
        (
            "EntityNotExistTransition",
            Runs("on entity_not_exist(\"Egg\", 10) -> other"),
        ),
        (
            "EntitiesNotExistsTransition",
            Runs("on entities_not_exists(10, \"Egg\") -> other"),
        ),
        (
            "DamageTakenTransition",
            Runs("on damage_taken(1000) -> other"),
        ),
        ("NotMovingTransition", Runs("on not_moving(1000) -> other")),
        (
            "PlayerTextTransition",
            Runs("on player_text(\"open\") -> other"),
        ),
        (
            "EntityCountGreaterThan",
            Runs("if(\"entity_count_greater_than\", \"Egg\", 10, 0) { shoot(10) }"),
        ),
        // -- in the build, never built ---------------------------------------------------------
        //
        // Counted with `new <Class>` across `Server-Side/wServer/logic/db/*.cs`, outside comments:
        // the converted corpus in `content/behaviours` is the same census and shows none of these
        // either. Writing them would be writing mechanics nothing plays.
        ("AnnounceOnDeath", Unused("0 uses in logic/db")),
        ("BringEnemy", Unused("0 uses in logic/db")),
        ("ChangeMusic", Unused("0 uses in logic/db")),
        ("ChangeMusicOnDeath", Unused("0 uses in logic/db")),
        ("ConditionEffectRegion", Unused("0 uses in logic/db")),
        ("DestroyOnDeath", Unused("0 uses in logic/db")),
        ("Duration", Unused("0 uses in logic/db")),
        ("EnemyAOE", Unused("0 uses in logic/db")),
        ("HealPlayerMP", Unused("0 uses in logic/db")),
        ("JumpToRandomOffset", Unused("0 uses in logic/db")),
        ("KillPlayer", Unused("0 uses in logic/db")),
        ("MultiplyLootValue", Unused("0 uses in logic/db")),
        ("MutePlayer", Unused("0 uses in logic/db")),
        ("OpenGate", Unused("1 use in logic/db, inside a comment")),
        ("RelativeSpawn", Unused("0 uses in logic/db")),
        ("ReproduceGroup", Unused("0 uses in logic/db")),
        ("ScaleHP2", Unused("0 uses in logic/db")),
        ("TeleporttoTarget", Unused("0 uses in logic/db")),
        ("TossObject2", Unused("0 uses in logic/db")),
        ("WhileEntityNotWithin", Unused("0 uses in logic/db")),
        ("WhileEntityWithin", Unused("0 uses in logic/db")),
        ("WhileWatched", Unused("0 uses in logic/db")),
        ("AnyEntityWithinTransition", Unused("0 uses in logic/db")),
        ("EntityHpLessTransition", Unused("0 uses in logic/db")),
        ("EntityWithinTransition", Unused("0 uses in logic/db")),
        ("GroundTransition", Unused("0 uses in logic/db")),
        ("GroupNotExistTransition", Unused("0 uses in logic/db")),
        ("HpBoundaryTransition", Unused("0 uses in logic/db")),
        ("NoEntityWithinTransition", Unused("0 uses in logic/db")),
        ("OnParentDeathTransition", Unused("0 uses in logic/db")),
    ];

    /// The files the original's project actually compiles, by directory and file name.
    ///
    /// A source file sitting in the directory but left out of the project is not part of the
    /// server, and implementing it would be inventing a mechanic.
    fn in_build() -> BTreeSet<(String, String)> {
        let Ok(project) = std::fs::read_to_string(PROJECT) else {
            return BTreeSet::new();
        };

        let mut found = BTreeSet::new();
        for (directory, _) in CSHARP {
            let marker = format!("logic\\{directory}\\");
            let mut from = 0usize;
            while let Some(at) = project[from..].find(&marker) {
                let at = from + at + marker.len();
                let tail = &project[at..];
                let end = tail.find(".cs").map(|e| e + 3).unwrap_or(tail.len());
                found.insert((directory.to_string(), tail[..end].to_string()));
                from = at + end;
            }
        }

        found
    }

    /// Every class in the built sources that descends from one of [`ROOTS`].
    ///
    /// Classes are found by their declaration rather than by their file name, because seven of the
    /// original's classes do not share a name with the file holding them: `Ported.cs` alone holds
    /// six, and a census keyed on file names misses every one.
    fn classes() -> BTreeMap<String, String> {
        let built = in_build();
        let mut bases: BTreeMap<String, String> = BTreeMap::new();

        for (directory, path) in CSHARP {
            let Ok(entries) = std::fs::read_dir(path) else {
                continue;
            };

            for entry in entries.flatten() {
                let file = entry.path();
                if file.extension().and_then(|e| e.to_str()) != Some("cs") {
                    continue;
                }
                let name = file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                if !built.is_empty() && !built.contains(&(directory.to_string(), name)) {
                    continue;
                }
                let Ok(source) = std::fs::read_to_string(&file) else {
                    continue;
                };

                for line in source.lines() {
                    let line = line.trim();
                    let Some(rest) = line.strip_prefix("class ").or_else(|| {
                        line.strip_prefix("public class ")
                            .or_else(|| line.strip_prefix("internal class "))
                            .or_else(|| line.strip_prefix("private class "))
                    }) else {
                        continue;
                    };
                    let Some((class, base)) = rest.split_once(':') else {
                        continue;
                    };
                    let class = class.trim();
                    let base = base.trim().split_whitespace().next().unwrap_or_default();
                    if class.is_empty() || base.is_empty() {
                        continue;
                    }
                    bases.insert(class.to_string(), base.to_string());
                }
            }
        }

        // A class counts if walking up its bases reaches one of the roots, so a subclass of a
        // behaviour is a behaviour however many steps away it is.
        bases
            .iter()
            .filter(|(_, base)| {
                let mut at = base.as_str();
                for _ in 0..8 {
                    if ROOTS.contains(&at) {
                        return true;
                    }
                    match bases.get(at) {
                        Some(next) => at = next.as_str(),
                        None => return false,
                    }
                }
                false
            })
            .map(|(class, base)| (class.clone(), base.clone()))
            .collect()
    }

    /// Whether a sample body compiles to something that will actually do work.
    fn compiles(body: &str) -> Result<(), String> {
        // Every sample names `other` as somewhere to go, so a transition has a target to resolve.
        let source = format!("enemy \"Sample\" {{\n{body}\nstate other {{ }}\n}}");
        let parsed = crate::parse(&source).map_err(|err| format!("does not parse: {err}"))?;
        let (programs, _) = super::compile(&parsed);
        let program = programs.get("Sample").ok_or("no program")?;

        let mut acted = false;
        for state in &program.states {
            for behaviour in &state.behaviours {
                if !live(behaviour, &mut acted) {
                    return Err("compiles to Unsupported".to_string());
                }
            }
            for transition in &state.transitions {
                if matches!(transition.condition, Condition::Unsupported { .. }) {
                    return Err("the condition compiles to Unsupported".to_string());
                }
                acted = true;
            }
        }

        if acted {
            Ok(())
        } else {
            Err("compiles to nothing at all".to_string())
        }
    }

    /// Walks a primitive and everything nested in it, refusing anything unsupported.
    fn live(behaviour: &Primitive, acted: &mut bool) -> bool {
        match behaviour {
            Primitive::Unsupported { .. } => false,

            Primitive::When {
                condition,
                children,
            } => {
                if matches!(**condition, Condition::Unsupported { .. }) {
                    return false;
                }
                *acted = true;
                children.iter().all(|child| live(child, acted))
            }

            Primitive::Every { children, .. }
            | Primitive::Sequence { children }
            | Primitive::Prioritize(children) => {
                *acted = true;
                children.iter().all(|child| live(child, acted))
            }

            _ => {
                *acted = true;
                true
            }
        }
    }

    #[test]
    fn the_project_builds_every_behaviour_and_transition_source() {
        // The denominator this census rests on. The original's project lists its sources one by
        // one rather than by wildcard, so a file could sit in the directory and never be compiled,
        // and implementing such a file would be inventing a mechanic. Today it lists all of them,
        // and this is what would notice if it stopped.
        let built = in_build();
        if !std::path::Path::new(PROJECT).exists() {
            return;
        }

        assert!(
            !built.is_empty(),
            "the project file was read and no source came out of it, which would silently widen \
             every count below"
        );

        let mut unbuilt = Vec::new();
        for (directory, path) in CSHARP {
            let Ok(entries) = std::fs::read_dir(path) else {
                continue;
            };
            for entry in entries.flatten() {
                let file = entry.path();
                if file.extension().and_then(|e| e.to_str()) != Some("cs") {
                    continue;
                }
                let name = file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                if !built.contains(&(directory.to_string(), name.clone())) {
                    unbuilt.push(format!("{directory}/{name}"));
                }
            }
        }

        assert!(
            unbuilt.is_empty(),
            "these sit in the original's tree and its project does not build them, so nothing \
             here should implement them:\n  {}",
            unbuilt.join("\n  ")
        );
    }

    #[test]
    fn every_class_the_original_builds_is_accounted_for() {
        let classes = classes();
        if classes.is_empty() {
            // The original is not beside us, which is a checkout without it rather than a failure.
            return;
        }

        let named: BTreeSet<&str> = ACCOUNTED.iter().map(|(class, _)| *class).collect();
        let missing: Vec<String> = classes
            .iter()
            .filter(|(class, _)| !named.contains(class.as_str()))
            .map(|(class, base)| format!("{class} : {base}"))
            .collect();

        assert!(
            missing.is_empty(),
            "the original builds these and nothing here says what became of them:\n  {}",
            missing.join("\n  ")
        );
    }

    #[test]
    fn nothing_is_accounted_for_that_the_original_does_not_build() {
        let classes = classes();
        if classes.is_empty() {
            return;
        }

        for (class, _) in ACCOUNTED {
            assert!(
                classes.contains_key(*class),
                "{class} is listed here but the original's project builds no such class"
            );
        }
    }

    #[test]
    fn every_class_said_to_run_still_runs() {
        // The half the argument census cannot see: an arm deleted from `behaviour` or `guard`
        // leaves the name compiling to `Unsupported`, which is silent everywhere else.
        let mut broken = Vec::new();

        for (class, verdict) in ACCOUNTED {
            let Runs(sample) = verdict else {
                continue;
            };
            if let Err(why) = compiles(sample) {
                broken.push(format!("{class}: {sample:?} {why}"));
            }
        }

        assert!(
            broken.is_empty(),
            "these classes are claimed as implemented and are not:\n  {}",
            broken.join("\n  ")
        );
    }

    #[test]
    fn nothing_unused_is_quietly_being_used() {
        // An excuse that has expired is worse than no excuse. If the original's database starts
        // instantiating one of these, the reason on it stops being true and this says so.
        let Ok(entries) = std::fs::read_dir("../../../Server-Side/wServer/logic/db") else {
            return;
        };

        let mut corpus = String::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("cs")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                corpus.push_str(&strip_comments(&text));
            }
        }

        if corpus.is_empty() {
            return;
        }

        let mut used = Vec::new();
        for (class, verdict) in ACCOUNTED {
            let Unused(why) = verdict else {
                continue;
            };
            assert!(
                !why.is_empty(),
                "{class} needs a reason for not being written"
            );
            if corpus.contains(&format!("new {class}(")) {
                used.push(*class);
            }
        }

        assert!(
            used.is_empty(),
            "these are excused as unused and the original's database builds them: {used:?}"
        );
    }

    /// Removes C# comments, so a behaviour written inside one does not count as used.
    fn strip_comments(source: &str) -> String {
        let mut out = String::with_capacity(source.len());
        let bytes = source.as_bytes();
        let mut at = 0usize;

        while at < bytes.len() {
            if bytes[at..].starts_with(b"//") {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
            } else if bytes[at..].starts_with(b"/*") {
                at += 2;
                while at < bytes.len() && !bytes[at..].starts_with(b"*/") {
                    at += 1;
                }
                at = (at + 2).min(bytes.len());
            } else {
                out.push(bytes[at] as char);
                at += 1;
            }
        }

        out
    }
}
