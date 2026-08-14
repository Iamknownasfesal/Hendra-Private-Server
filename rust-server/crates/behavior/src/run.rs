//! Running a compiled behaviour.
//!
//! One [`Mind`] per enemy: which state it is in, how long it has been there, and when each of its
//! behaviours may next act. Everything else it needs arrives as [`Senses`] and everything it wants
//! leaves as [`Action`], so a mind can be driven and inspected with no world present.
//!
//! # Order
//!
//! Transitions are checked before behaviours, innermost state outwards, and the first that fires
//! wins. Checking inner first is what lets a boss's attack phase override the movement pattern it
//! sits inside; checking before acting means the tick a state is entered on is spent in the new
//! state rather than the old one.
//!
//! At most one movement takes effect per tick. Several behaviours may want to move, such as a
//! `follow` inside a state that also wanders, and applying both would produce a diagonal neither asked
//! for. The innermost one wins, which is the same precedence transitions use.

use crate::program::*;

/// How long a `conditional_effect` with no duration is renewed for each tick.
///
/// Long enough to outlast a tick at any plausible rate, short enough that leaving the state clears
/// it almost at once. An effect that lasted a whole second past its state would let a boss stay
/// invulnerable well into the phase where it is meant to be hit.
const RENEWAL_MS: u32 = 250;

/// The least time between two sweeps of `remove_nearby`.
const ORDER_INTERVAL_MS: u32 = 1000;

/// The least time between two ground changes from the same behaviour.
const GROUND_INTERVAL_MS: u32 = 500;

/// How long a charge is committed to before it can be re-aimed.
const CHARGE_MS: u32 = 600;

/// How close counts as having arrived somewhere.
const ARRIVED: f32 = 0.3;

/// How often health is rescaled to the crowd.
///
/// Not every tick: the count changes as people walk in and out, and rescaling constantly would
/// make a boss's health bar jitter throughout the fight.
const SCALE_INTERVAL_MS: u32 = 2000;

/// One enemy's running behaviour.
#[derive(Debug, Clone)]
pub struct Mind {
    current: usize,
    in_state_ms: u32,

    /// When each behaviour may next act, in milliseconds remaining.
    cooldowns: Vec<u32>,

    /// How many children this has spawned and not yet lost.
    pub children: u32,

    /// Deterministic, so a misbehaving enemy misbehaves identically on the next run.
    seed: u32,

    /// The direction a wander is currently heading, so it drifts rather than jitters.
    wander_angle: f32,

    /// Which way the current dart is going, and how much of it is left.
    buzz_angle: f32,
    buzz_remaining: f32,

    /// How long the tick being run is.
    ///
    /// Held rather than threaded through, because one behaviour in nine hundred needs it and giving
    /// every one of them an argument for it would be paying everywhere for a single caller.
    elapsed_ms: u32,

    /// The deadline a `timed_random` transition drew on entering this state.
    ///
    /// Drawn once rather than rolled every tick, because rolling would make the transition fire at
    /// a random moment near the minimum rather than at a random moment in the range.
    deadline_ms: u32,

    /// Damage taken since this state was entered, for transitions that react to being hurt.
    damage_in_state: i32,

    /// How far around a circle a swirl or a pacing movement has travelled.
    phase: f32,

    /// The locked heading of a charge, and how long is left of it. A charge that re-aimed every
    /// tick would be a follow.
    charge: Option<(f32, u32)>,

    /// Which taunt line comes next, so a boss works through what it has to say rather than
    /// repeating one line.
    said: u32,

    /// Which child of a `sequence` acts next.
    step: u32,

    /// How long the entity has been standing still, for the transition that watches for it.
    still_for_ms: u32,

    /// Where it was last tick, which is how standing still is noticed at all.
    was_at: Option<(f32, f32)>,

    /// Scratch for the ancestry walk, reused so a tick allocates nothing.
    chain: Vec<usize>,
}

impl Mind {
    /// A fresh mind, starting in the program's entry state.
    pub fn new(program: &Program, seed: u32) -> Mind {
        let mut mind = Mind {
            current: program
                .state(program.root)
                .map(|state| state.entry)
                .unwrap_or(program.root),
            in_state_ms: 0,
            cooldowns: vec![0; program.slots],
            children: 0,
            // Zero is a fixed point of the generator below, so it is never a valid seed.
            seed: if seed == 0 { 0x9e37_79b9 } else { seed },
            wander_angle: 0.0,
            buzz_angle: 0.0,
            buzz_remaining: 0.0,
            elapsed_ms: 0,
            deadline_ms: 0,
            damage_in_state: 0,
            phase: 0.0,
            charge: None,
            said: 0,
            step: 0,
            still_for_ms: 0,
            was_at: None,
            chain: Vec::new(),
        };

        let landing = mind.current;
        mind.arm_cooldowns(program, landing);
        mind
    }

    /// Puts the mind into a state from outside, as an order does.
    ///
    /// The same as a transition firing, so the state is entered properly: its innermost child is
    /// landed on and its cooldowns are cleared. An order that only set the index would leave a
    /// boss's minions in a state whose behaviours were all mid-cooldown from the last one.
    pub fn force_into(&mut self, program: &Program, target: usize) {
        self.enter(program, target);
    }

    /// Whether the enemy is in `target` or in a state nested inside it.
    ///
    /// Entering a state lands on its innermost child, so a mind ordered into an outer state is
    /// never sitting on that index itself. Anything asking "is it already there?" has to walk up.
    pub fn is_within(&mut self, program: &Program, target: usize) -> bool {
        program.ancestry(self.current, &mut self.chain);
        self.chain.contains(&target)
    }

    /// Which state the enemy is in.
    pub fn state(&self) -> usize {
        self.current
    }

    /// How long it has been there.
    pub fn in_state_ms(&self) -> u32 {
        self.in_state_ms
    }

    /// The name of the current state, for logs and tests.
    pub fn state_name<'a>(&self, program: &'a Program) -> &'a str {
        program
            .state(self.current)
            .map(|state| state.name.as_str())
            .unwrap_or("")
    }

    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed % 10_000) as f32 / 10_000.0
    }

    /// Moves to a state, entering its innermost child and clearing its cooldowns.
    fn enter(&mut self, program: &Program, target: usize) {
        let landing = program
            .state(target)
            .map(|state| state.entry)
            .unwrap_or(target);

        self.current = landing;
        self.in_state_ms = 0;
        self.damage_in_state = 0;
        self.charge = None;

        // Drawn on entry, so a group that entered together does not leave together.
        self.deadline_ms = 0;

        self.arm_cooldowns(program, landing);
    }

    /// Sets every cooldown in a state's ancestry to what that behaviour starts with.
    ///
    /// A behaviour that was mid-cooldown when its state was left should not still be waiting when
    /// the state is entered again, or a boss re-entering an attack phase stands there for the
    /// remainder of a cooldown it started minutes ago.
    ///
    /// Armed to the behaviour's own starting value rather than to zero, because a shot's is its
    /// offset: that is what turns a volley written as a dozen shoots into a sequence instead of one
    /// simultaneous burst. Called on the first state as well as on every later one, since a boss
    /// that opens with a staggered volley never transitions before firing it.
    fn arm_cooldowns(&mut self, program: &Program, landing: usize) {
        program.ancestry(landing, &mut self.chain);
        for index in &self.chain {
            let Some(state) = program.state(*index) else {
                continue;
            };
            let mut slot = state.slot_base;
            for behaviour in &state.behaviours {
                if slot < self.cooldowns.len() {
                    self.cooldowns[slot] = behaviour.entry_cooldown_ms();
                }
                slot += behaviour.slots();
            }
        }
    }

    /// Advances one tick, appending whatever the enemy wants to do.
    pub fn tick(
        &mut self,
        program: &Program,
        senses: &Senses,
        elapsed_ms: u32,
        out: &mut Vec<Action>,
    ) {
        out.clear();
        self.elapsed_ms = elapsed_ms;
        self.in_state_ms = self.in_state_ms.saturating_add(elapsed_ms);
        self.damage_in_state = self.damage_in_state.saturating_add(senses.damage_taken);

        // Movement is measured rather than asked about: a behaviour that wanted to move and was
        // stopped by a wall has not moved, and that is what the transition is watching for.
        const STILL: f32 = 0.01;
        match self.was_at {
            Some((x, y)) if (senses.x - x).abs() < STILL && (senses.y - y).abs() < STILL => {
                self.still_for_ms = self.still_for_ms.saturating_add(elapsed_ms);
            }
            _ => self.still_for_ms = 0,
        }
        self.was_at = Some((senses.x, senses.y));

        if let Some((_, left)) = &mut self.charge {
            *left = left.saturating_sub(elapsed_ms);
            if *left == 0 {
                self.charge = None;
            }
        }

        for slot in self.cooldowns.iter_mut() {
            *slot = slot.saturating_sub(elapsed_ms);
        }

        // Transitions first, innermost outwards, first match wins.
        program.ancestry(self.current, &mut self.chain);
        let chain = std::mem::take(&mut self.chain);

        let mut moved_to = None;
        'outer: for index in &chain {
            let Some(state) = program.state(*index) else {
                continue;
            };
            for transition in &state.transitions {
                if self.fires(program, &transition.condition, senses) {
                    moved_to = Some(transition.target);
                    break 'outer;
                }
            }
        }
        self.chain = chain;

        if let Some(target) = moved_to {
            self.enter(program, target);
        }

        // Then behaviours, again innermost outwards.
        program.ancestry(self.current, &mut self.chain);
        let chain = std::mem::take(&mut self.chain);

        let mut has_moved = false;
        for index in &chain {
            let Some(state) = program.state(*index) else {
                continue;
            };
            let mut slot = state.slot_base;
            for primitive in &state.behaviours {
                slot += self.run(program, primitive, slot, senses, &mut has_moved, out);
            }
        }
        self.chain = chain;
    }

    fn fires(&mut self, program: &Program, condition: &Condition, senses: &Senses) -> bool {
        match condition {
            Condition::Timed { after_ms } => self.in_state_ms >= *after_ms,

            Condition::TimedRandom { min_ms, max_ms } => {
                if self.deadline_ms == 0 {
                    let span = max_ms.saturating_sub(*min_ms);
                    self.deadline_ms = min_ms + (self.random() * span as f32) as u32;
                    // A range of zero would otherwise redraw every tick and never be held.
                    self.deadline_ms = self.deadline_ms.max(1);
                }
                self.in_state_ms >= self.deadline_ms
            }

            Condition::DamageTaken { amount } => self.damage_in_state >= *amount,

            Condition::PlayerSaid {
                word,
                within,
                exact_case,
            } => senses.said.iter().any(|(distance, text)| {
                if within.is_some_and(|near| *distance > near) {
                    return false;
                }

                // A whole word rather than a substring: the original matches a regular expression,
                // and every use in the content is a plain word. "Red" should answer to somebody
                // saying "red" and not to somebody saying "prepared".
                text.split(|c: char| !c.is_alphanumeric()).any(|held| {
                    if *exact_case {
                        held == word
                    } else {
                        held.eq_ignore_ascii_case(word)
                    }
                })
            }),

            Condition::NotMoving { after_ms } => self.still_for_ms >= *after_ms,

            Condition::EntityWithin { kind, radius } => program
                .kind_of(*kind)
                .is_some_and(|kind| senses.any_within(kind, *radius)),

            // Every one of them must be absent. A name the host does not know counts as absent
            // rather than as present: a boss whose guardian was removed from the content should
            // wake up rather than wait forever.
            Condition::NoneWithin { kinds, radius } => kinds.iter().all(|name| {
                program
                    .kind_of(*name)
                    .is_none_or(|kind| !senses.any_within(kind, *radius))
            }),

            Condition::PlayerWithin { radius, see_invis } => {
                let nearest = if *see_invis {
                    senses.nearest_player_hiding
                } else {
                    senses.nearest_player
                };

                nearest.is_some_and(|player| player.distance <= *radius)
            }

            Condition::NoPlayerWithin { radius } => senses
                .nearest_player
                .is_none_or(|player| player.distance > *radius),

            Condition::HpBelow { fraction } => senses.health_fraction() <= *fraction,

            // Reported at compile time; here it simply never fires.
            Condition::Unsupported { .. } => false,
        }
    }

    /// Runs one primitive. Returns how many cooldown slots it consumed.
    fn run(
        &mut self,
        program: &Program,
        primitive: &Primitive,
        slot: usize,
        senses: &Senses,
        has_moved: &mut bool,
        out: &mut Vec<Action>,
    ) -> usize {
        match primitive {
            Primitive::Shoot {
                count,
                spread,
                fixed_angle,
                cooldown_ms,
                cooldown_offset_ms: _,
                projectile,
                acquire_range,
                default_angle,
                angle_offset,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }

                // A fixed angle fires regardless of who is about. Otherwise it aims at whoever is
                // nearest *and within its own range*, falling back to the default angle when the
                // content gives one and holding fire when it does not.
                let aimed = match fixed_angle {
                    Some(degrees) => Some(degrees.to_radians()),
                    None => senses
                        .nearest_player
                        .filter(|player| player.distance <= *acquire_range)
                        .map(|player| (player.y - senses.y).atan2(player.x - senses.x))
                        .or_else(|| default_angle.map(f32::to_radians)),
                };
                let Some(angle) = aimed else {
                    return 1;
                };

                out.push(Action::Shoot {
                    angle: angle + angle_offset.to_radians(),
                    count: *count,
                    spread: *spread,
                    projectile: *projectile,
                });

                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = *cooldown_ms;
                }
                1
            }

            Primitive::Wander { speed } => {
                if *has_moved {
                    return 1;
                }
                // Drift rather than jitter: the heading turns a little each tick instead of being
                // redrawn, which reads as an animal wandering rather than one having a fit.
                let turn = (self.random() - 0.5) * 0.6;
                self.wander_angle += turn;

                out.push(Action::Move {
                    angle: self.wander_angle,
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::Buzz {
                speed,
                distance,
                cooldown_ms,
            } => {
                if *has_moved {
                    return 1;
                }

                // Waiting between darts is what makes this buzzing rather than walking. The wait is
                // held in the same cooldown slot the behaviour would use to fire, since a dart and a
                // shot are both "this behaviour acted".
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }

                // A new heading only once the last dart has been spent, so it travels in straight
                // short bursts rather than shivering on the spot.
                if self.buzz_remaining <= 0.0 {
                    // One of the eight compass directions, as the original draws it: two integers
                    // in minus one to one, never both zero.
                    let eighth = (self.random() * 8.0) as u32 % 8;
                    self.buzz_angle = eighth as f32 * std::f32::consts::FRAC_PI_4;
                    self.buzz_remaining = *distance;

                    if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                        *cooldown = *cooldown_ms;
                    }
                }

                self.buzz_remaining -= *speed * (self.elapsed_ms as f32 / 1000.0);

                out.push(Action::Move {
                    angle: self.buzz_angle,
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::Follow {
                speed,
                acquire_range,
                range,
            } => {
                if *has_moved {
                    return 1;
                }
                let Some(player) = senses.nearest_player else {
                    return 1;
                };
                if player.distance > *acquire_range || player.distance <= *range {
                    return 1;
                }

                out.push(Action::Move {
                    angle: (player.y - senses.y).atan2(player.x - senses.x),
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::Orbit {
                speed,
                radius,
                acquire_range,
                target,
            } => {
                if *has_moved {
                    return 1;
                }

                // A named target is circled instead of the nearest player, which is what the
                // content asks for in 137 of its 167 orbits: minions ring their boss rather than
                // ringing whoever walked in.
                let centre = match target {
                    Some(name) => {
                        let kinds = program.kinds_of(*name);
                        senses
                            .nearby
                            .iter()
                            .filter(|other| kinds.contains(&other.kind))
                            .filter(|other| other.distance <= *acquire_range)
                            .min_by(|a, b| a.distance.total_cmp(&b.distance))
                            .map(|other| (other.x, other.y, other.distance))
                    }
                    None => senses
                        .nearest_player
                        .filter(|player| player.distance <= *acquire_range)
                        .map(|player| (player.x, player.y, player.distance)),
                };
                let Some((centre_x, centre_y, distance)) = centre else {
                    return 1;
                };

                // Tangential, corrected toward the intended radius so it spirals into the ring
                // rather than orbiting at whatever distance it happened to arrive at.
                let toward = (centre_y - senses.y).atan2(centre_x - senses.x);
                let drift = (distance - *radius).clamp(-1.0, 1.0);
                let angle = toward + std::f32::consts::FRAC_PI_2 - drift * 0.6;

                out.push(Action::Move {
                    angle,
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::StayBack { speed, distance } => {
                if *has_moved {
                    return 1;
                }
                let Some(player) = senses.nearest_player else {
                    return 1;
                };
                if player.distance >= *distance {
                    return 1;
                }

                out.push(Action::Move {
                    angle: (senses.y - player.y).atan2(senses.x - player.x),
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::StayCloseToSpawn { speed, range } => {
                if *has_moved {
                    return 1;
                }
                let (dx, dy) = (senses.spawn_x - senses.x, senses.spawn_y - senses.y);
                if (dx * dx + dy * dy).sqrt() <= *range {
                    return 1;
                }

                out.push(Action::Move {
                    angle: dy.atan2(dx),
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::HealSelf {
                amount,
                cooldown_ms,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }
                if senses.hp >= senses.max_hp {
                    return 1;
                }

                out.push(Action::Heal { amount: *amount });
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = *cooldown_ms;
                }
                1
            }

            Primitive::Spawn {
                child,
                max_children,
                cooldown_ms,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }
                if self.children >= *max_children {
                    return 1;
                }

                out.push(Action::Spawn {
                    child: *child,
                    count: 1,
                    offset_x: 0.0,
                    offset_y: 0.0,
                    state: None,
                    delay_ms: 0,
                });
                self.children += 1;

                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = *cooldown_ms;
                }
                1
            }

            Primitive::Suicide => {
                out.push(Action::Vanish);
                1
            }

            Primitive::Prioritize(children) => {
                // The first child that produces anything wins, and the rest are skipped. That is
                // what makes an enemy look decided: it chases if it can, otherwise keeps its
                // distance, otherwise wanders.
                let mut used = 1;
                let before = out.len();

                for child in children {
                    if out.len() > before {
                        // Already satisfied; still account for the slots so indices stay aligned.
                        used += child.slots();
                        continue;
                    }
                    used += self.run(program, child, slot + used, senses, has_moved, out);
                }
                used
            }

            Primitive::Reproduce {
                child,
                density_radius,
                density_max,
                cooldown_ms,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }

                // Counted from what is actually standing there rather than from what this entity
                // remembers making. Killing the children lets it make more, and its own death does
                // not leak a count that nothing will ever decrement.
                // Every kind the name holds counts toward the crowd. A spawner told to make
                // dwarves makes one of several, and counting only the first would let it fill the
                // room with the other two.
                let kinds = program.kinds_of(*child);
                if !kinds.is_empty() {
                    let crowd = senses
                        .nearby
                        .iter()
                        .filter(|other| {
                            kinds.contains(&other.kind) && other.distance <= *density_radius
                        })
                        .count();
                    if crowd >= *density_max as usize {
                        return 1;
                    }
                }

                out.push(Action::Spawn {
                    child: *child,
                    count: 1,
                    offset_x: 0.0,
                    offset_y: 0.0,
                    state: None,
                    delay_ms: 0,
                });
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = *cooldown_ms;
                }
                1
            }

            Primitive::TossObject {
                child,
                radius,
                fixed_angle,
                cooldown_ms,
                warning_ms,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }

                // Thrown toward whoever is nearest unless the content fixed the angle. With nobody
                // in sight there is nothing to aim at, and dropping it underfoot is not what the
                // behaviour is for.
                let angle = match fixed_angle {
                    Some(degrees) => degrees.to_radians(),
                    None => match senses.nearest_player {
                        Some(player) => (player.y - senses.y).atan2(player.x - senses.x),
                        None => return 1,
                    },
                };

                // A ring rather than a point: a radius the content gives is how far out it lands.
                let reach = *radius * (0.4 + 0.6 * self.random());
                out.push(Action::Spawn {
                    child: *child,
                    count: 1,
                    offset_x: angle.cos() * reach,
                    offset_y: angle.sin() * reach,
                    state: None,
                    delay_ms: *warning_ms,
                });

                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = *cooldown_ms;
                }
                1
            }

            Primitive::Grenade {
                radius,
                damage,
                range,
                cooldown_ms,
                effect,
                effect_ms,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }
                let Some(player) = senses.nearest_player else {
                    return 1;
                };
                if player.distance > *range {
                    return 1;
                }

                // Thrown where the player is now. Leading them would make it unavoidable, which is
                // the difference between a hard attack and one nobody can play around.
                out.push(Action::Grenade {
                    offset_x: player.x - senses.x,
                    offset_y: player.y - senses.y,
                    radius: *radius,
                    damage: *damage,
                    effect: *effect,
                    effect_ms: *effect_ms,
                });

                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = *cooldown_ms;
                }
                1
            }

            Primitive::Decay { after_ms } => {
                if self.in_state_ms >= *after_ms {
                    out.push(Action::Vanish);
                }
                1
            }

            Primitive::ConditionalEffect {
                effect,
                duration_ms,
                target,
                radius,
            } => {
                // Re-applied every tick rather than once on entry. A duration of zero means "while
                // this state lasts", and the only way to express that to a world that expires
                // effects on a timer is to keep renewing it.
                out.push(Action::Effect {
                    effect: *effect,
                    duration_ms: if *duration_ms == 0 {
                        RENEWAL_MS
                    } else {
                        *duration_ms
                    },
                    radius: *radius,
                    target: *target,
                });
                1
            }

            Primitive::SetAltTexture { index } => {
                out.push(Action::Texture { index: *index });
                1
            }

            Primitive::ChangeSize { rate, target } => {
                out.push(Action::Resize {
                    rate: *rate,
                    target: *target,
                });
                1
            }

            Primitive::Taunt {
                lines,
                probability,
                cooldown_ms,
                broadcast,
            } => {
                if lines.is_empty() || self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }
                if self.random() > *probability {
                    // Still put it on cooldown, so a low probability means "rarely" rather than
                    // "on most ticks, eventually".
                    if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                        *cooldown = *cooldown_ms;
                    }
                    return 1;
                }

                let line = lines[self.said as usize % lines.len()].clone();
                self.said = self.said.wrapping_add(1);

                out.push(Action::Say {
                    text: line,
                    broadcast: *broadcast,
                });
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = *cooldown_ms;
                }
                1
            }

            Primitive::Order {
                radius,
                kind,
                state,
                once,
            } => {
                // A standing order is re-sent every tick, as the original does, so that an entity
                // arriving in range afterwards is caught at once. Re-sending is harmless because
                // the receiving side leaves alone anything already in the ordered state.
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }

                out.push(Action::Order {
                    radius: *radius,
                    kind: *kind,
                    state: state.clone(),
                });
                if *once
                    && let Some(cooldown) = self.cooldowns.get_mut(slot)
                {
                    // A once-only order is held for longer than any state lasts, so entering the
                    // state again is what gives it a second time rather than waiting it out.
                    *cooldown = u32::MAX;
                }
                1
            }

            Primitive::Flash {
                colour,
                period_ms,
                repeats,
            } => {
                // Once on entering the state. Restarting the blink every tick would leave it
                // permanently on its first frame.
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }
                out.push(Action::Flash {
                    colour: *colour,
                    period_ms: *period_ms,
                    repeats: *repeats,
                });
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = u32::MAX;
                }
                1
            }

            Primitive::RemoveEffect { effect } => {
                out.push(Action::RemoveEffect { effect: *effect });
                1
            }

            Primitive::ScaleHealth {
                per_player,
                maximum_extra,
                radius,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }
                out.push(Action::ScaleHealth {
                    per_player: *per_player,
                    maximum_extra: *maximum_extra,
                    radius: *radius,
                });
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = SCALE_INTERVAL_MS;
                }
                1
            }

            Primitive::Sequence { children } => {
                // One child per turn, advancing only when the current one actually does something.
                // Advancing regardless would step through a pattern while the enemy stood idle.
                let mut used = 1;
                let before = out.len();
                let turn = self.step as usize;

                for (index, child) in children.iter().enumerate() {
                    if index == turn % children.len().max(1) {
                        used += self.run(program, child, slot + used, senses, has_moved, out);
                    } else {
                        used += child.slots();
                    }
                }

                if out.len() > before {
                    self.step = self.step.wrapping_add(1);
                }
                used
            }

            Primitive::Transform { into } => {
                out.push(Action::Transform { into: *into });
                1
            }

            Primitive::Protect {
                speed,
                protectee,
                acquire_range,
                protect_range,
                reprotect_range,
            } => {
                if *has_moved {
                    return 1;
                }
                let Some(kind) = program.kind_of(*protectee) else {
                    return 1;
                };
                let Some(ward) = senses.nearest_of(kind, *acquire_range) else {
                    return 1;
                };

                // Close enough is close enough. Without the second, smaller radius it would jitter
                // in and out at exactly the protection distance.
                if ward.distance <= *reprotect_range {
                    return 1;
                }
                if ward.distance <= *protect_range && self.charge.is_none() {
                    return 1;
                }

                out.push(Action::Move {
                    angle: (ward.y - senses.y).atan2(ward.x - senses.x),
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::HealOthers {
                radius,
                amount,
                kind,
                players,
                cooldown_ms,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }

                // Nothing to heal is not a reason to spend the cooldown.
                //
                // A name that is a group holds several kinds and any of them counts, which is what
                // healing a group means: a crystal heals every other crystal there is.
                let wanted = kind.map(|name| program.kinds_of(name));
                let anyone = senses.nearby.iter().any(|other| {
                    other.distance <= *radius
                        && other.player == *players
                        && wanted.is_none_or(|kinds| kinds.contains(&other.kind))
                        && other.hp < other.max_hp
                });
                if !anyone {
                    return 1;
                }

                out.push(Action::HealOthers {
                    radius: *radius,
                    amount: *amount,
                    kind: *kind,
                    players: *players,
                });
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = *cooldown_ms;
                }
                1
            }

            Primitive::MoveTo { x, y, speed } => {
                if *has_moved {
                    return 1;
                }
                let (dx, dy) = (x - senses.x, y - senses.y);
                if (dx * dx + dy * dy).sqrt() <= ARRIVED {
                    return 1;
                }

                out.push(Action::Move {
                    angle: dy.atan2(dx),
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::MoveLine { speed, angle } => {
                if *has_moved {
                    return 1;
                }
                out.push(Action::Move {
                    angle: angle.to_radians(),
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::BackAndForth { speed, distance } => {
                if *has_moved {
                    return 1;
                }
                // Measured from the spawn rather than from wherever it drifted to, so a long fight
                // does not walk the pacing across the room.
                let travelled = senses.x - senses.spawn_x;
                if travelled.abs() >= *distance {
                    self.phase = if travelled > 0.0 {
                        std::f32::consts::PI
                    } else {
                        0.0
                    };
                }

                out.push(Action::Move {
                    angle: self.phase,
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::Charge {
                speed,
                range,
                cooldown_ms,
            } => {
                if *has_moved {
                    return 1;
                }

                // Mid-charge it keeps the heading it committed to. Re-aiming every tick would make
                // this a fast follow, and a charge is dangerous precisely because it can be dodged.
                if let Some((angle, _)) = self.charge {
                    out.push(Action::Move {
                        angle,
                        speed: *speed,
                    });
                    *has_moved = true;
                    return 1;
                }

                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }
                let Some(player) = senses.nearest_player else {
                    return 1;
                };
                if player.distance > *range {
                    return 1;
                }

                let angle = (player.y - senses.y).atan2(player.x - senses.x);
                self.charge = Some((angle, CHARGE_MS));
                out.push(Action::Move {
                    angle,
                    speed: *speed,
                });
                *has_moved = true;

                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = (*cooldown_ms).max(CHARGE_MS);
                }
                1
            }

            Primitive::Swirl {
                speed,
                radius,
                targeted,
            } => {
                if *has_moved {
                    return 1;
                }

                let (centre_x, centre_y) = if *targeted {
                    match senses.nearest_player {
                        Some(player) => (player.x, player.y),
                        None => (senses.spawn_x, senses.spawn_y),
                    }
                } else {
                    (senses.spawn_x, senses.spawn_y)
                };

                let (dx, dy) = (senses.x - centre_x, senses.y - centre_y);
                let out_by = (dx * dx + dy * dy).sqrt() - *radius;

                // Tangential, pulled toward the ring, so it spirals in rather than orbiting at
                // whatever distance it happened to start from.
                let around = dy.atan2(dx) + std::f32::consts::FRAC_PI_2;
                let correction = out_by.clamp(-1.0, 1.0) * 0.8;

                out.push(Action::Move {
                    angle: around - correction * dy.atan2(dx).signum(),
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::ReturnToSpawn { speed, tolerance } => {
                if *has_moved {
                    return 1;
                }
                let (dx, dy) = (senses.spawn_x - senses.x, senses.spawn_y - senses.y);
                if (dx * dx + dy * dy).sqrt() <= *tolerance {
                    return 1;
                }

                out.push(Action::Move {
                    angle: dy.atan2(dx),
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::StayAbove { speed, altitude } => {
                if *has_moved {
                    return 1;
                }
                let Some(player) = senses.nearest_player else {
                    return 1;
                };
                if player.distance >= *altitude {
                    return 1;
                }

                out.push(Action::Move {
                    angle: (senses.y - player.y).atan2(senses.x - player.x),
                    speed: *speed,
                });
                *has_moved = true;
                1
            }

            Primitive::NoExperience => {
                out.push(Action::NoExperience);
                1
            }

            Primitive::RemoveNearby { radius, kind } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }
                out.push(Action::RemoveNearby {
                    radius: *radius,
                    kind: *kind,
                });
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = ORDER_INTERVAL_MS;
                }
                1
            }

            Primitive::ApplySetpiece { name } => {
                // Once, on entering the state, as the original does. A setpiece redrawn every tick
                // would rebuild the room around whoever walked into it.
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }
                out.push(Action::Setpiece { name: name.clone() });
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = u32::MAX;
                }
                1
            }

            Primitive::GroundTransform {
                tile,
                radius,
                cooldown_ms,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }
                out.push(Action::Ground {
                    tile: *tile,
                    radius: *radius,
                });
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    // Ground changes are expensive and repeating one changes nothing, so a
                    // behaviour that gave no interval gets a generous one rather than none.
                    *cooldown = (*cooldown_ms).max(GROUND_INTERVAL_MS);
                }
                1
            }

            // Nothing happens during a tick. The world reads these when the entity dies.
            Primitive::OnDeath(_) => 1,

            Primitive::Every {
                period_ms,
                children,
            } => {
                let mut used = 1;
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    // The children are skipped, but their slots still have to be accounted for or
                    // every later behaviour would read someone else's cooldown.
                    return 1 + children.iter().map(Primitive::slots).sum::<usize>();
                }

                for child in children {
                    used += self.run(program, child, slot + used, senses, has_moved, out);
                }
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = *period_ms;
                }
                used
            }

            Primitive::When {
                condition,
                children,
            } => {
                if !self.fires(program, condition, senses) {
                    return 1 + children.iter().map(Primitive::slots).sum::<usize>();
                }

                let mut used = 1;
                for child in children {
                    used += self.run(program, child, slot + used, senses, has_moved, out);
                }
                used
            }

            Primitive::Unsupported { .. } => 1,
        }
    }
}

#[cfg(test)]
mod speech {
    use super::*;
    use crate::program::{Condition, Senses};

    fn heard<'a>(said: &'a [(f32, &'a str)]) -> Senses<'a> {
        Senses {
            x: 0.0,
            y: 0.0,
            hp: 100,
            max_hp: 100,
            spawn_x: 0.0,
            spawn_y: 0.0,
            nearest_player: None,
            nearest_player_hiding: None,
            nearby: &[],
            said,
            damage_taken: 0,
        }
    }

    fn listens(word: &str, within: Option<f32>, exact_case: bool) -> Condition {
        Condition::PlayerSaid {
            word: word.to_string(),
            within,
            exact_case,
        }
    }

    fn hears(condition: &Condition, said: &[(f32, &str)]) -> bool {
        let program = crate::Program::default();
        let mut mind = Mind::new(&program, 1);
        mind.fires(&program, condition, &heard(said))
    }

    #[test]
    fn the_word_is_heard_when_it_is_said() {
        assert!(hears(&listens("Red", None, false), &[(1.0, "Red")]));
    }

    #[test]
    fn a_word_inside_another_word_is_not_the_word() {
        // "Red" should answer to somebody saying red and not to somebody saying prepared.
        assert!(!hears(&listens("Red", None, false), &[(1.0, "prepared")]));
        assert!(!hears(&listens("Red", None, false), &[(1.0, "Fred")]));
    }

    #[test]
    fn the_word_is_found_in_a_sentence() {
        assert!(hears(
            &listens("Red", None, false),
            &[(1.0, "I think we should do Red first")]
        ));
    }

    #[test]
    fn capitals_do_not_matter_unless_they_are_asked_to() {
        assert!(hears(&listens("Red", None, false), &[(1.0, "red")]));
        assert!(!hears(&listens("Red", None, true), &[(1.0, "red")]));
    }

    #[test]
    fn somebody_too_far_away_is_not_heard() {
        assert!(hears(&listens("Red", Some(10.0), false), &[(5.0, "Red")]));
        assert!(!hears(&listens("Red", Some(10.0), false), &[(50.0, "Red")]));
    }

    #[test]
    fn silence_is_not_the_word() {
        assert!(!hears(&listens("Red", None, false), &[]));
    }

    #[test]
    fn one_speaker_saying_it_is_enough() {
        assert!(hears(
            &listens("Red", None, false),
            &[(1.0, "no"), (2.0, "Red"), (3.0, "maybe")]
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::compile;
    use crate::parse::parse;

    fn program(source: &str) -> Program {
        let parsed = parse(source).expect("should parse");
        let (programs, diagnostics) = compile(&parsed);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        programs.programs.into_iter().next().expect("one program")
    }

    fn alone() -> Senses<'static> {
        Senses {
            x: 10.0,
            y: 10.0,
            hp: 100,
            max_hp: 100,
            spawn_x: 10.0,
            spawn_y: 10.0,
            nearest_player: None,
            nearest_player_hiding: None,
            nearby: &[],
            said: &[],
            damage_taken: 0,
        }
    }

    fn with_player_at(x: f32, y: f32) -> Senses<'static> {
        let mut senses = alone();
        let (dx, dy) = (x - senses.x, y - senses.y);
        let there = Some(Nearby {
            x,
            y,
            distance: (dx * dx + dy * dy).sqrt(),
        });

        senses.nearest_player = there;
        senses.nearest_player_hiding = there;
        senses
    }

    /// A player standing there who an enemy cannot normally see.
    fn with_hidden_player_at(x: f32, y: f32) -> Senses<'static> {
        let mut senses = with_player_at(x, y);
        senses.nearest_player = None;
        senses
    }

    #[test]
    fn an_enemy_does_not_notice_somebody_hiding() {
        let program = program(
            r#"enemy "X" {
                state idle { on player_within(dist: 10) -> awake }
                state awake { }
            }"#,
        );

        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &with_hidden_player_at(12.0, 10.0), 50, &mut out);
        assert_eq!(mind.state_name(&program), "idle");
    }

    #[test]
    fn an_enemy_written_to_see_the_invisible_notices_them() {
        // Ten enemies in the game are written this way and mean it: a Candyland enemy that runs
        // from you is meant to run whether or not you are hiding, and a sprite that teleports away
        // is meant to escape an invisible pursuer.
        let program = program(
            r#"enemy "X" {
                state idle { on player_within(dist: 10, see_invis: true) -> awake }
                state awake { }
            }"#,
        );

        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &with_hidden_player_at(12.0, 10.0), 50, &mut out);
        assert_eq!(mind.state_name(&program), "awake");
    }

    #[test]
    fn a_timed_transition_moves_between_states() {
        let program = program(
            r#"enemy "X" {
                 state a { on timed(400ms) -> b }
                 state b { on timed(400ms) -> a }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        assert_eq!(mind.state_name(&program), "a");

        // 350ms is not yet 400.
        for _ in 0..7 {
            mind.tick(&program, &alone(), 50, &mut out);
        }
        assert_eq!(mind.state_name(&program), "a");

        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(mind.state_name(&program), "b");
        assert_eq!(mind.in_state_ms(), 0, "the clock restarts on entry");
    }

    #[test]
    fn a_player_coming_into_range_wakes_the_enemy() {
        let program = program(
            r#"enemy "X" {
                 state idle { on player_within(8) -> angry }
                 state angry { on no_player_within(12) -> idle }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &with_player_at(30.0, 10.0), 50, &mut out);
        assert_eq!(mind.state_name(&program), "idle", "too far to notice");

        mind.tick(&program, &with_player_at(15.0, 10.0), 50, &mut out);
        assert_eq!(mind.state_name(&program), "angry");

        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(
            mind.state_name(&program),
            "idle",
            "and settles when they leave"
        );
    }

    #[test]
    fn shooting_respects_its_cooldown() {
        let program = program(
            r#"enemy "X" {
                 state a { shoot(count: 3, shoot_angle: 10, cooldown: 500ms) }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &with_player_at(14.0, 10.0), 50, &mut out);
        assert_eq!(out.len(), 1, "fires immediately");
        match &out[0] {
            Action::Shoot { count, spread, .. } => {
                assert_eq!(*count, 3);
                assert_eq!(*spread, 10.0);
            }
            other => panic!("expected a shot, got {other:?}"),
        }

        // Nine more ticks of 50ms is 450ms, still inside the cooldown.
        for _ in 0..9 {
            mind.tick(&program, &with_player_at(14.0, 10.0), 50, &mut out);
            assert!(out.is_empty(), "should still be reloading");
        }

        mind.tick(&program, &with_player_at(14.0, 10.0), 50, &mut out);
        assert_eq!(out.len(), 1, "and fires again once ready");
    }

    #[test]
    fn an_unaimed_shot_holds_fire_with_nobody_in_sight() {
        let program = program(r#"enemy "X" { state a { shoot(count: 1, cooldown: 100ms) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &alone(), 50, &mut out);
        assert!(
            out.is_empty(),
            "shooting at nothing is worse than not shooting"
        );

        mind.tick(&program, &with_player_at(12.0, 10.0), 200, &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_fixed_angle_fires_regardless_of_who_is_about() {
        let program = program(
            r#"enemy "X" { state a { shoot(count: 1, fixed_angle: 90, cooldown: 100ms) } }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            Action::Shoot { angle, .. } => {
                assert!(
                    (angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
                    "90 degrees"
                );
            }
            other => panic!("expected a shot, got {other:?}"),
        }
    }

    #[test]
    fn follow_moves_toward_a_player_and_stops_at_its_range() {
        let program = program(r#"enemy "X" { state a { follow(1.0, 20, 3) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &with_player_at(20.0, 10.0), 50, &mut out);
        match &out[0] {
            Action::Move { angle, speed } => {
                assert!(angle.abs() < 1e-5, "due east");
                assert_eq!(*speed, 1.0);
            }
            other => panic!("expected a move, got {other:?}"),
        }

        // Already inside the range it wants to keep.
        mind.tick(&program, &with_player_at(12.0, 10.0), 50, &mut out);
        assert!(out.is_empty(), "close enough, so it stops");
    }

    #[test]
    fn stay_back_retreats_from_someone_too_close() {
        let program = program(r#"enemy "X" { state a { stay_back(0.8, 6) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &with_player_at(12.0, 10.0), 50, &mut out);
        match &out[0] {
            Action::Move { angle, .. } => {
                assert!(
                    (angle.abs() - std::f32::consts::PI).abs() < 1e-4,
                    "due west, away from the player"
                );
            }
            other => panic!("expected a move, got {other:?}"),
        }
    }

    #[test]
    fn prioritise_runs_the_first_child_that_acts_and_no_others() {
        let program = program(
            r#"enemy "X" {
                 state a {
                   prioritize {
                     follow(1.0, 20, 3)
                     wander(0.4)
                   }
                 }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        // A player in range: follow acts, wander does not.
        mind.tick(&program, &with_player_at(20.0, 10.0), 50, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            Action::Move { speed, .. } => {
                assert_eq!(*speed, 1.0, "the follow speed, not the wander")
            }
            other => panic!("expected a move, got {other:?}"),
        }

        // Nobody about: follow declines, so wander takes over.
        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            Action::Move { speed, .. } => assert_eq!(*speed, 0.4, "the wander speed"),
            other => panic!("expected a move, got {other:?}"),
        }
    }

    #[test]
    fn only_one_movement_takes_effect_per_tick() {
        // Two competing movers in one state. Applying both would produce a direction neither asked
        // for, so the innermost wins and the other is skipped.
        let program = program(
            r#"enemy "X" {
                 state a {
                   follow(1.0, 20, 1)
                   wander(0.4)
                 }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &with_player_at(20.0, 10.0), 50, &mut out);
        let moves = out
            .iter()
            .filter(|action| matches!(action, Action::Move { .. }))
            .count();
        assert_eq!(moves, 1);
    }

    #[test]
    fn behaviours_are_inherited_from_enclosing_states() {
        // The point of nesting: a movement pattern that persists across attack phases.
        let program = program(
            r#"enemy "X" {
                 state fight {
                   wander(0.4)
                   state phase1 { shoot(count: 1, fixed_angle: 0, cooldown: 100ms) }
                 }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        assert_eq!(
            mind.state_name(&program),
            "phase1",
            "entry descends to the leaf"
        );

        mind.tick(&program, &alone(), 50, &mut out);
        assert!(
            out.iter().any(|a| matches!(a, Action::Shoot { .. })),
            "the leaf shoots"
        );
        assert!(
            out.iter().any(|a| matches!(a, Action::Move { .. })),
            "the parent still wanders"
        );
    }

    #[test]
    fn an_inner_transition_beats_an_outer_one() {
        let program = program(
            r#"enemy "X" {
                 state fight {
                   on timed(50ms) -> other
                   state phase1 { on timed(50ms) -> phase2 }
                   state phase2 { }
                 }
                 state other { }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(
            mind.state_name(&program),
            "phase2",
            "the innermost transition wins when both are ready"
        );
    }

    #[test]
    fn re_entering_a_state_clears_its_cooldowns() {
        // Otherwise a boss returning to an attack phase stands there waiting out a cooldown it
        // started before it left.
        let program = program(
            r#"enemy "X" {
                 state a {
                   shoot(count: 1, fixed_angle: 0, cooldown: 10s)
                   on timed(100ms) -> b
                 }
                 state b { on timed(100ms) -> a }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        // Two seconds of bouncing between a and b, every entry to a well inside the ten-second
        // cooldown. Without the reset there would be exactly one shot in the whole run.
        let mut shots = 0;
        for _ in 0..40 {
            mind.tick(&program, &alone(), 50, &mut out);
            shots += out
                .iter()
                .filter(|action| matches!(action, Action::Shoot { .. }))
                .count();
        }

        assert!(
            shots >= 8,
            "expected a shot on each re-entry, got {shots} in two seconds"
        );
    }

    #[test]
    fn spawning_stops_at_the_limit() {
        let program =
            program(r#"enemy "X" { state a { spawn("Slime", max_children: 2, cooldown: 50ms) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let mut spawned = 0;
        for _ in 0..20 {
            mind.tick(&program, &alone(), 100, &mut out);
            spawned += out
                .iter()
                .filter(|action| matches!(action, Action::Spawn { .. }))
                .count();
        }

        assert_eq!(spawned, 2, "the limit is a limit");
        assert_eq!(mind.children, 2);
    }

    #[test]
    fn health_thresholds_fire_when_hurt() {
        let program = program(
            r#"enemy "X" {
                 state healthy { on hp_below(0.5) -> desperate }
                 state desperate { heal_self(amount: 50, cooldown: 1s) }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(mind.state_name(&program), "healthy");

        let mut hurt = alone();
        hurt.hp = 40;
        mind.tick(&program, &hurt, 50, &mut out);
        assert_eq!(mind.state_name(&program), "desperate");
        assert!(out.iter().any(|a| matches!(a, Action::Heal { .. })));
    }

    #[test]
    fn a_wander_drifts_rather_than_jitters() {
        let program = program(r#"enemy "X" { state a { wander(0.4) } }"#);
        let mut mind = Mind::new(&program, 12345);
        let mut out = Vec::new();

        let mut angles = Vec::new();
        for _ in 0..40 {
            mind.tick(&program, &alone(), 50, &mut out);
            if let Some(Action::Move { angle, .. }) = out.first() {
                angles.push(*angle);
            }
        }

        assert_eq!(angles.len(), 40);
        for pair in angles.windows(2) {
            assert!(
                (pair[1] - pair[0]).abs() < 0.35,
                "a wander should turn gradually, not teleport its heading"
            );
        }
    }

    #[test]
    fn an_unsupported_behaviour_does_nothing_and_does_not_disturb_its_neighbours() {
        let parsed = parse(
            r#"enemy "X" {
                 state a {
                   dance_the_tarantella(3)
                   shoot(count: 1, fixed_angle: 0, cooldown: 100ms)
                 }
               }"#,
        )
        .unwrap();
        let (programs, diagnostics) = compile(&parsed);
        assert_eq!(diagnostics.len(), 1, "reported once");

        let program = &programs.programs[0];
        let mut mind = Mind::new(program, 1);
        let mut out = Vec::new();

        mind.tick(program, &alone(), 50, &mut out);
        assert_eq!(out.len(), 1, "the shoot beside it still runs");
    }

    // -- the behaviours added to cover the game's content --------------------------------------

    /// The first behaviour of a named state.
    ///
    /// State zero is the unnamed root that everything hangs under, so indexing it finds nothing.
    fn first_behaviour<'a>(program: &'a Program, state: &str) -> &'a Primitive {
        let index = program.state_named(state).expect("no such state");
        &program.states[index].behaviours[0]
    }

    /// Senses with one neighbour of a given kind at a distance.
    fn beside(kind: u16, distance: f32) -> Vec<Neighbour> {
        vec![Neighbour {
            kind,
            id: 7,
            x: 10.0 + distance,
            y: 10.0,
            distance,
            hp: 50,
            max_hp: 100,
            player: false,
        }]
    }

    fn seeing<'a>(nearby: &'a [Neighbour]) -> Senses<'a> {
        Senses { nearby, ..alone() }
    }

    /// Resolves every name in a program to a number, so conditions that name entities can fire.
    fn resolved(source: &str, known: &[(&str, u16)]) -> Program {
        let mut program = program(source);
        program.resolve(|name| {
            known
                .iter()
                .filter(|(held, _)| *held == name)
                .map(|(_, kind)| *kind)
                .collect()
        });
        program
    }

    #[test]
    fn a_conditional_effect_is_renewed_rather_than_applied_once() {
        // The world expires effects on a timer, so an effect meant to last as long as a state has
        // to be renewed. Applying it once would have every boss lose its invulnerability a quarter
        // of a second into the phase that grants it.
        let program = program(r#"enemy "X" { state a { conditional_effect(invulnerable) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        for tick in 0..5 {
            mind.tick(&program, &alone(), 50, &mut out);
            assert!(
                matches!(out.as_slice(), [Action::Effect { effect: 24, .. }]),
                "tick {tick} did not renew the effect: {out:?}"
            );
        }
    }

    #[test]
    fn a_permanent_effect_and_a_timed_one_are_told_apart() {
        let program = program(
            r#"enemy "X" {
                 state a { conditional_effect(armored, perm: true) }
                 state b { conditional_effect(slowed, 5) }
               }"#,
        );
        let mut out = Vec::new();

        let mut mind = Mind::new(&program, 1);
        mind.tick(&program, &alone(), 50, &mut out);
        let Action::Effect {
            effect,
            duration_ms,
            ..
        } = out[0]
        else {
            panic!("expected an effect, got {out:?}")
        };
        assert_eq!(effect, 25, "armored");
        assert!(
            duration_ms > 0 && duration_ms < 1000,
            "renewed, not forever"
        );
    }

    #[test]
    fn an_orbit_circles_what_it_was_told_to_circle() {
        // The content names something to orbit in 137 of its 167 uses. Circling the nearest player
        // instead turns a ring the player moves around into a ring that follows them.
        let mut program = program(r#"enemy "X" { state a { orbit(1, 4, 20, target: "King") } }"#);
        const KING: u16 = 7;
        program.resolve(|name| if name == "King" { vec![KING] } else { Vec::new() });

        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        // The king stands to the west, a player to the east, and the orbit should ignore the
        // player entirely.
        let neighbours = [Neighbour {
            kind: KING,
            id: 1,
            x: 4.0,
            y: 10.0,
            distance: 6.0,
            hp: 100,
            max_hp: 100,
            player: false,
        }];
        let mut senses = with_player_at(16.0, 10.0);
        senses.nearby = &neighbours;

        mind.tick(&program, &senses, 50, &mut out);

        let Some(Action::Move { angle, .. }) = out.first() else {
            panic!("expected a move, got {out:?}");
        };

        // Tangential to the king, which is west of us, so the heading has a westward component
        // rather than the eastward one a player-following orbit would produce.
        assert!(
            angle.cos() < 0.5,
            "should be circling the king to the west, not the player to the east"
        );
    }

    #[test]
    fn a_volley_written_as_several_shoots_fires_in_sequence() {
        // The idiom this exists for: a dozen shoots with the same enormous cooldown and offsets a
        // fifth of a second apart, each firing once. Without the offset all of them fire on the
        // first frame and then nothing happens again, which is not a slightly wrong boss.
        let program = program(
            r#"enemy "X" { state a {
                 shoot(20, count: 1, cooldown: 100000)
                 shoot(20, count: 1, cooldown: 100000, cooldown_offset: 200)
                 shoot(20, count: 1, cooldown: 100000, cooldown_offset: 400)
               } }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        // A player stands well inside every shot's range.
        let seen = with_player_at(11.0, 10.0);

        mind.tick(&program, &seen, 50, &mut out);
        assert_eq!(out.len(), 1, "only the unoffset one fires immediately");

        let mut fired = 0;
        for _ in 0..4 {
            out.clear();
            mind.tick(&program, &seen, 50, &mut out);
            fired += out.len();
        }
        assert_eq!(fired, 1, "the second at 200ms");

        for _ in 0..4 {
            out.clear();
            mind.tick(&program, &seen, 50, &mut out);
            fired += out.len();
        }
        assert_eq!(fired, 2, "and the third at 400ms, none of them again");
    }

    #[test]
    fn a_shot_does_not_reach_past_its_own_range() {
        // Every shoot in the content passes a radius and it used to be dropped, so an enemy with a
        // four-tile attack fired at anything inside the twenty-tile sense radius.
        let program = program(r#"enemy "X" { state a { shoot(4, count: 1, cooldown: 100) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let far = with_player_at(20.0, 10.0);
        mind.tick(&program, &far, 50, &mut out);
        assert!(out.is_empty(), "ten tiles away is out of a four-tile range");

        let near = with_player_at(13.0, 10.0);
        mind.tick(&program, &near, 50, &mut out);
        assert_eq!(out.len(), 1, "three tiles away is inside it");
    }

    #[test]
    fn an_order_stands_for_as_long_as_the_state_does() {
        // The original re-sends every tick so that an entity arriving in range afterwards is caught
        // at once. What stops the targets being held at the start of the ordered state is the
        // receiving side, which leaves alone anything already in it.
        let program = program(r#"enemy "X" { state a { order(10, "Minion", "attack") } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(out.len(), 1, "ordered on the first tick");

        let mut given = 0;
        for _ in 0..40 {
            mind.tick(&program, &alone(), 50, &mut out);
            given += out.len();
        }
        assert_eq!(given, 40, "one order per tick, for as long as the state lasts");
    }

    #[test]
    fn a_once_only_order_is_given_once_per_entry() {
        let program = program(r#"enemy "X" { state a { order_once(10, "Minion", "attack") } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(out.len(), 1, "ordered on entering a");

        let mut given = 0;
        for _ in 0..40 {
            mind.tick(&program, &alone(), 50, &mut out);
            given += out.len();
        }
        assert_eq!(given, 0, "not again while still in a");

        mind.force_into(&program, program.state_named("a").expect("a"));
        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(out.len(), 1, "and once more on re-entering a");
    }

    #[test]
    fn a_mind_ordered_into_an_outer_state_counts_as_being_in_it() {
        // Entering a state lands on its innermost child, so an entity ordered into `outer` is
        // sitting on `inner`. A caller asking "is it already there?" must get yes, or a standing
        // order would restart the state — and every cooldown in it — on every repeat.
        let program = program(
            r#"enemy "X" {
                state idle { }
                state outer { state inner { wander(1) } }
            }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let outer = program.state_named("outer").expect("outer");
        let inner = program.state_named("inner").expect("inner");

        mind.force_into(&program, outer);
        assert_eq!(mind.state(), inner, "lands on the innermost child");
        assert!(mind.is_within(&program, outer), "still counts as in outer");
        assert!(mind.is_within(&program, inner));
        assert!(!mind.is_within(&program, program.state_named("idle").expect("idle")));
    }

    #[test]
    fn a_charge_holds_its_heading_instead_of_following() {
        // A charge that re-aimed every tick would be a fast follow, and the whole point of one is
        // that it can be dodged.
        let program = program(r#"enemy "X" { state a { charge(2, 20, cooldown: 5000) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &with_player_at(20.0, 10.0), 50, &mut out);
        let Action::Move { angle: first, .. } = out[0] else {
            panic!("expected a move, got {out:?}")
        };

        // The player runs to the other side. A follow would turn; a charge does not.
        mind.tick(&program, &with_player_at(10.0, 20.0), 50, &mut out);
        let Action::Move { angle: second, .. } = out[0] else {
            panic!("expected a move, got {out:?}")
        };
        assert_eq!(first, second, "the charge should not have re-aimed");
    }

    #[test]
    fn a_charge_stops_and_does_not_start_again_at_once() {
        let program = program(r#"enemy "X" { state a { charge(2, 20, cooldown: 5000) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let mut moved = 0;
        for _ in 0..20 {
            mind.tick(&program, &with_player_at(20.0, 10.0), 100, &mut out);
            moved += out.len();
        }
        assert!(
            moved > 0 && moved < 20,
            "should charge then stop, not run forever: {moved} of 20 ticks"
        );
    }

    #[test]
    fn a_decay_vanishes_when_its_time_is_up_and_not_before() {
        let program = program(r#"enemy "X" { state a { decay(1000) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        for _ in 0..19 {
            mind.tick(&program, &alone(), 50, &mut out);
            assert!(out.is_empty(), "still alive");
        }
        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(out, vec![Action::Vanish]);
    }

    #[test]
    fn a_decay_written_without_a_time_does_not_vanish_immediately() {
        // The transpiler writes `decay()` and `decay(0)` for the same thing, and reading the zero
        // as "now" would make every summon that uses it die on the tick it was made.
        for written in [r#"decay()"#, r#"decay(0)"#] {
            let program = program(&format!(r#"enemy "X" {{ state a {{ {written} }} }}"#));
            let mut mind = Mind::new(&program, 1);
            let mut out = Vec::new();

            mind.tick(&program, &alone(), 50, &mut out);
            assert!(out.is_empty(), "{written} vanished at once");
        }
    }

    #[test]
    fn a_taunt_works_through_its_lines_rather_than_repeating_one() {
        let program = program(
            r#"enemy "X" { state a { taunt("first", "second", "third", cooldown: 100ms) } }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let mut said = Vec::new();
        for _ in 0..30 {
            mind.tick(&program, &alone(), 60, &mut out);
            for action in &out {
                if let Action::Say { text, .. } = action {
                    said.push(text.to_string());
                }
            }
        }

        assert!(said.len() >= 3, "should have said several things: {said:?}");
        assert_eq!(&said[..3], &["first", "second", "third"]);
    }

    #[test]
    fn a_reproduce_stops_when_its_own_kind_is_already_crowded() {
        let program = resolved(
            r#"enemy "X" { state a { reproduce("Spawn", 5, 2, cooldown: 0) } }"#,
            &[("Spawn", 900)],
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(out.len(), 1, "nothing about, so it reproduces");

        // Two of its own kind already standing there is the limit.
        let crowd = vec![beside(900, 1.0)[0], beside(900, 2.0)[0]];
        mind.tick(&program, &seeing(&crowd), 50, &mut out);
        assert!(out.is_empty(), "should have held off: {out:?}");

        // A different kind does not count toward the limit.
        let strangers = vec![beside(901, 1.0)[0], beside(901, 2.0)[0]];
        mind.tick(&program, &seeing(&strangers), 50, &mut out);
        assert_eq!(out.len(), 1, "another kind should not crowd it out");
    }

    #[test]
    fn an_entity_condition_fires_only_for_the_kind_it_names() {
        let program = resolved(
            r#"enemy "X" {
                 state a { on entity_exists("Guard", 10) -> b }
                 state b { }
               }"#,
            &[("Guard", 900)],
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &seeing(&beside(901, 5.0)), 50, &mut out);
        assert_eq!(mind.state_name(&program), "a", "a different kind");

        mind.tick(&program, &seeing(&beside(900, 5.0)), 50, &mut out);
        assert_eq!(mind.state_name(&program), "b");
    }

    #[test]
    fn an_entity_condition_respects_its_radius() {
        let program = resolved(
            r#"enemy "X" {
                 state a { on entity_exists("Guard", 10) -> b }
                 state b { }
               }"#,
            &[("Guard", 900)],
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &seeing(&beside(900, 50.0)), 50, &mut out);
        assert_eq!(mind.state_name(&program), "a", "too far away");

        mind.tick(&program, &seeing(&beside(900, 9.0)), 50, &mut out);
        assert_eq!(mind.state_name(&program), "b");
    }

    #[test]
    fn a_boss_waits_for_every_guardian_not_just_the_first() {
        // This is what the condition is for: a boss sealed until all of its guardians are dead.
        // Firing on the first absence would open every such fight after one kill.
        let program = resolved(
            r#"enemy "Boss" {
                 state sealed { on entities_not_exists(100, "Left", "Right") -> awake }
                 state awake { }
               }"#,
            &[("Left", 900), ("Right", 901)],
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let both = vec![beside(900, 5.0)[0], beside(901, 5.0)[0]];
        mind.tick(&program, &seeing(&both), 50, &mut out);
        assert_eq!(mind.state_name(&program), "sealed");

        mind.tick(&program, &seeing(&beside(901, 5.0)), 50, &mut out);
        assert_eq!(mind.state_name(&program), "sealed", "one still standing");

        mind.tick(&program, &seeing(&[]), 50, &mut out);
        assert_eq!(mind.state_name(&program), "awake");
    }

    #[test]
    fn an_unresolved_guardian_counts_as_absent() {
        // A boss whose guardian was renamed out of the content should wake up rather than wait
        // forever in a room nobody can finish.
        let mut program = program(
            r#"enemy "Boss" {
                 state sealed { on entities_not_exists(100, "Ghost") -> awake }
                 state awake { }
               }"#,
        );
        program.resolve(|_| Vec::new());

        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();
        mind.tick(&program, &seeing(&beside(900, 1.0)), 50, &mut out);

        assert_eq!(mind.state_name(&program), "awake");
    }

    #[test]
    fn a_damage_condition_counts_only_what_was_taken_in_the_state() {
        let program = program(
            r#"enemy "X" {
                 state a { on damage_taken(100) -> b }
                 state b { on damage_taken(100) -> c }
                 state c { }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let hit = |amount| Senses {
            damage_taken: amount,
            ..alone()
        };

        mind.tick(&program, &hit(60), 50, &mut out);
        assert_eq!(mind.state_name(&program), "a");
        mind.tick(&program, &hit(60), 50, &mut out);
        assert_eq!(mind.state_name(&program), "b", "a hundred and twenty taken");

        // The count restarts on entering the new state, so the overflow does not carry.
        mind.tick(&program, &hit(0), 50, &mut out);
        assert_eq!(
            mind.state_name(&program),
            "b",
            "the tally should have reset"
        );
    }

    #[test]
    fn a_random_wait_lands_inside_its_range() {
        let program = program(
            r#"enemy "X" {
                 state a { on timed_random(2000, 1) -> b }
                 state b { }
               }"#,
        );

        let mut waits = Vec::new();
        for seed in 1..12 {
            let mut mind = Mind::new(&program, seed);
            let mut out = Vec::new();
            let mut waited = 0;

            while mind.state_name(&program) == "a" && waited < 5_000 {
                mind.tick(&program, &alone(), 50, &mut out);
                waited += 50;
            }
            waits.push(waited);
        }

        assert!(
            waits.iter().all(|held| *held <= 2_050),
            "never longer than the range: {waits:?}"
        );
        assert!(
            waits.iter().collect::<std::collections::HashSet<_>>().len() > 1,
            "different seeds should wait different times: {waits:?}"
        );
    }

    #[test]
    fn an_unrandomised_wait_is_exactly_its_time() {
        let program = program(
            r#"enemy "X" {
                 state a { on timed_random(1000, 0) -> b }
                 state b { }
               }"#,
        );
        let mut mind = Mind::new(&program, 5);
        let mut out = Vec::new();

        for _ in 0..19 {
            mind.tick(&program, &alone(), 50, &mut out);
            assert_eq!(mind.state_name(&program), "a");
        }
        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(mind.state_name(&program), "b");
    }

    #[test]
    fn a_timed_group_runs_its_children_on_a_period() {
        let program = program(
            r#"enemy "X" {
                 state a {
                   timed(500) {
                     shoot(count: 1, fixed_angle: 0, cooldown: 0)
                   }
                 }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let mut fired = 0;
        for _ in 0..20 {
            mind.tick(&program, &alone(), 100, &mut out);
            fired += out.len();
        }

        // Two seconds at half a second each.
        assert!(
            (4..=5).contains(&fired),
            "fired {fired} times in two seconds"
        );
    }

    #[test]
    fn healing_others_does_not_spend_its_cooldown_on_nobody() {
        let program = resolved(
            r#"enemy "X" { state a { heal_group(10, "Ally", 100, cooldown: 1000) } }"#,
            &[("Ally", 900)],
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &alone(), 50, &mut out);
        assert!(out.is_empty(), "nobody to heal");

        // An ally arrives hurt, and is healed at once rather than after a cooldown it never spent.
        mind.tick(&program, &seeing(&beside(900, 5.0)), 50, &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
    }

    #[test]
    fn healing_skips_anyone_already_at_full_health() {
        let program = resolved(
            r#"enemy "X" { state a { heal_group(10, "Ally", 100, cooldown: 1000) } }"#,
            &[("Ally", 900)],
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let mut whole = beside(900, 5.0);
        whole[0].hp = whole[0].max_hp;

        mind.tick(&program, &seeing(&whole), 50, &mut out);
        assert!(out.is_empty(), "nothing needed healing: {out:?}");
    }

    #[test]
    fn protecting_something_moves_toward_it_only_when_it_is_far() {
        let program = resolved(
            r#"enemy "X" {
                 state a { protect(1, "Ward", acquire_range: 20, protection_range: 5,
                                   reprotect_range: 3) }
               }"#,
            &[("Ward", 900)],
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &seeing(&beside(900, 2.0)), 50, &mut out);
        assert!(out.is_empty(), "close enough already");

        mind.tick(&program, &seeing(&beside(900, 12.0)), 50, &mut out);
        assert!(
            matches!(out.as_slice(), [Action::Move { .. }]),
            "should close the distance: {out:?}"
        );

        mind.tick(&program, &seeing(&beside(900, 40.0)), 50, &mut out);
        assert!(out.is_empty(), "too far to even see it");
    }

    #[test]
    fn a_move_to_stops_when_it_arrives() {
        let program = program(r#"enemy "X" { state a { move_to(speed: 1, x: 20, y: 10) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &alone(), 50, &mut out);
        assert!(matches!(out.as_slice(), [Action::Move { .. }]));

        // Standing on the target: nothing more to do, and no jitter around it.
        let arrived = Senses {
            x: 20.0,
            y: 10.0,
            ..alone()
        };
        mind.tick(&program, &arrived, 50, &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn the_two_spellings_of_move_to_read_their_arguments_the_right_way_round() {
        // `MoveTo(speed, x, y)` and `MoveTo2(x, y, speed)` are the same behaviour with the
        // arguments reversed. Reading one with the other's order sends the enemy to the speed.
        let first = program(r#"enemy "X" { state a { move_to(1, 20, 30) } }"#);
        let second = program(r#"enemy "X" { state a { move_to2(20, 30, 1) } }"#);

        let of = |program: &Program| match &first_behaviour(program, "a") {
            Primitive::MoveTo { x, y, speed } => (*x, *y, *speed),
            other => panic!("expected a move_to, got {other:?}"),
        };

        assert_eq!(of(&first), (20.0, 30.0, 1.0));
        assert_eq!(of(&second), of(&first));
    }

    #[test]
    fn removing_entities_is_not_suicide() {
        // `RemoveEntity(dist, children)` removes other entities. Reading it as a suicide would
        // have every boss that tidies up its summons kill itself instead.
        let program = program(r#"enemy "X" { state a { remove_entity(9999, "manager") } }"#);

        let found = first_behaviour(&program, "a");
        assert!(
            matches!(found, Primitive::RemoveNearby { .. }),
            "got {found:?}"
        );
    }

    #[test]
    fn a_death_effect_does_nothing_while_the_entity_lives() {
        let program = program(r#"enemy "X" { state a { drop_portal_on_death("Somewhere", 1) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        for _ in 0..20 {
            mind.tick(&program, &alone(), 50, &mut out);
            assert!(out.is_empty(), "nothing happens until it dies: {out:?}");
        }

        assert!(
            first_behaviour(&program, "a").death_effect().is_some(),
            "but the effect is there to be read when it does"
        );
    }

    #[test]
    fn resolving_reports_the_names_the_host_does_not_have() {
        let mut program =
            program(r#"enemy "X" { state a { order(10, "Real", "go") spawn("Missing") } }"#);
        let missing = program.resolve(|name| {
            if name == "Real" {
                vec![900]
            } else {
                Vec::new()
            }
        });

        assert_eq!(missing, vec!["Missing"]);
    }

    #[test]
    fn a_mind_is_deterministic() {
        let program = program(r#"enemy "X" { state a { wander(0.4) } }"#);

        let run = |seed| {
            let mut mind = Mind::new(&program, seed);
            let mut out = Vec::new();
            let mut angles = Vec::new();
            for _ in 0..20 {
                mind.tick(&program, &alone(), 50, &mut out);
                if let Some(Action::Move { angle, .. }) = out.first() {
                    angles.push(format!("{angle:.6}"));
                }
            }
            angles
        };

        assert_eq!(run(7), run(7), "the same seed must replay identically");
        assert_ne!(run(7), run(8), "and different seeds must differ");
    }
}
