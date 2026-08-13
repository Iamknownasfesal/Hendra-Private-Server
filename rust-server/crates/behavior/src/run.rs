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
//! At most one movement takes effect per tick. Several behaviours may want to move — a `follow`
//! inside a state that also wanders — and applying both would produce a diagonal neither asked
//! for. The innermost one wins, which is the same precedence transitions use.

use crate::program::*;

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

    /// Scratch for the ancestry walk, reused so a tick allocates nothing.
    chain: Vec<usize>,
}

impl Mind {
    /// A fresh mind, starting in the program's entry state.
    pub fn new(program: &Program, seed: u32) -> Mind {
        Mind {
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
            chain: Vec::new(),
        }
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

        // A behaviour that was mid-cooldown when its state was left should not still be waiting
        // when the state is entered again — otherwise a boss re-entering an attack phase stands
        // there doing nothing for the remainder of a cooldown it started minutes ago.
        program.ancestry(landing, &mut self.chain);
        for index in &self.chain {
            let Some(state) = program.state(*index) else {
                continue;
            };
            let needed: usize = state.behaviours.iter().map(Primitive::slots).sum();
            for slot in state.slot_base..(state.slot_base + needed).min(self.cooldowns.len()) {
                self.cooldowns[slot] = 0;
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
        self.in_state_ms = self.in_state_ms.saturating_add(elapsed_ms);

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
                if self.fires(&transition.condition, senses) {
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
                slot += self.run(primitive, slot, senses, &mut has_moved, out);
            }
        }
        self.chain = chain;
    }

    fn fires(&self, condition: &Condition, senses: &Senses) -> bool {
        match condition {
            Condition::Timed { after_ms } => self.in_state_ms >= *after_ms,

            Condition::PlayerWithin { radius } => senses
                .nearest_player
                .is_some_and(|player| player.distance <= *radius),

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
                projectile,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }

                // Without a fixed angle it aims at whoever is nearest, and with nobody in sight it
                // holds fire rather than shooting at the origin.
                let angle = match fixed_angle {
                    Some(degrees) => degrees.to_radians(),
                    None => match senses.nearest_player {
                        Some(player) => (player.y - senses.y).atan2(player.x - senses.x),
                        None => return 1,
                    },
                };

                out.push(Action::Shoot {
                    angle,
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
            } => {
                if *has_moved {
                    return 1;
                }
                let Some(player) = senses.nearest_player else {
                    return 1;
                };
                if player.distance > *acquire_range {
                    return 1;
                }

                // Tangential, corrected toward the intended radius so it spirals into the ring
                // rather than orbiting at whatever distance it happened to arrive at.
                let toward = (player.y - senses.y).atan2(player.x - senses.x);
                let drift = (player.distance - *radius).clamp(-1.0, 1.0);
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
                    child: child.clone(),
                    count: 1,
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
                    used += self.run(child, slot + used, senses, has_moved, out);
                }
                used
            }

            Primitive::Unsupported { .. } => 1,
        }
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

    fn alone() -> Senses {
        Senses {
            x: 10.0,
            y: 10.0,
            hp: 100,
            max_hp: 100,
            spawn_x: 10.0,
            spawn_y: 10.0,
            nearest_player: None,
        }
    }

    fn with_player_at(x: f32, y: f32) -> Senses {
        let mut senses = alone();
        let (dx, dy) = (x - senses.x, y - senses.y);
        senses.nearest_player = Some(Nearby {
            x,
            y,
            distance: (dx * dx + dy * dy).sqrt(),
        });
        senses
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

        // Nine more ticks of 50ms is 450ms — still inside the cooldown.
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
                   taunt("you shall not pass")
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
