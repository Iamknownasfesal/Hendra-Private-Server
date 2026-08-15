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

/// How far one leg of a wander carries before another direction is drawn, in tiles.
///
/// `600 / 1000f` (`logic/behaviors/Wander.cs:47`), spent by distance travelled rather than by time,
/// so a slow wanderer holds its direction for longer than a fast one.
const WANDER_LEG: f32 = 0.6;

/// A transition countdown that has never been reached, which the original spells as a null in its
/// state storage: the first tick that reaches it draws the starting value.
const UNSTARTED: u32 = u32::MAX;

/// How often health is rescaled to the crowd.
///
/// Not every tick: the count changes as people walk in and out, and rescaling constantly would
/// make a boss's health bar jitter throughout the fight.
const SCALE_INTERVAL_MS: u32 = 2000;

/// What one behaviour drew for itself when its state was entered.
///
/// The original keeps this in the per-entity state storage, which is why two enemies running the
/// same orbit circle at different speeds and different distances.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Drawn {
    speed: f32,
    radius: f32,

    /// Which way round, as the plus or minus one the original multiplies by.
    direction: f32,
}

/// What a movement behaviour's speed argument is worth on this tick.
///
/// Nought while the host is paralysed, and nought for ever afterwards for every enemy of the type,
/// because the original zeroes a field it shares between them all and never writes it back — see
/// [`zero_speed_on_paralysis`]. Zero here still moves: the host turns it into 0.74 tiles a second
/// through [`tiles_per_second`].
///
/// Called before any early return, because that is where the original writes: ahead of the target
/// search, and ahead of the check that gives way to a movement further in.
fn speed_after_paralysis(program: &Program, slot: usize, senses: &Senses, speed: f32) -> f32 {
    if zero_speed_on_paralysis(program, slot, senses.paralyzed) || senses.paralyzed {
        0.0
    } else {
        speed
    }
}

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

    /// The direction a wander is currently heading, and how far is left of that leg.
    wander_angle: f32,
    wander_remaining: f32,

    /// Which way the current dart is going, and how much of it is left.
    buzz_angle: f32,
    buzz_remaining: f32,

    /// How long the tick being run is.
    ///
    /// Held rather than threaded through, because one behaviour in nine hundred needs it and giving
    /// every one of them an argument for it would be paying everywhere for a single caller.
    ///
    /// This is the caller's tick, and it is what movement is measured in: a mind driven twenty times
    /// a second covers the same ground per second as one driven six times a second, in smaller
    /// steps. Everything a behaviour *decides* — when to shoot, when to spawn, when to give up on a
    /// state — is measured in [`grain_ms`](Self::grain_ms) instead, because deciding is what the
    /// original does on a clock of its own.
    elapsed_ms: u32,

    /// Time banked toward the original's next 166 ms logic tick.
    ///
    /// Every countdown a behaviour owns moves in whole ticks of the original's clock and never in
    /// fractions of one, so a mind driven on a faster tick banks what it is given and spends it a
    /// whole tick at a time. Without this, a `cooldown: 500` shoot fires every 500 ms here against
    /// every 830 ms there — see [`on_the_original_clock`].
    grain_ms: u32,

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

    /// Where it was standing when it entered the state it is in.
    ///
    /// What the offsets in `move_to2` are measured from, worked out on the first tick of the state
    /// rather than as the state is entered, because entering can happen from an order that has no
    /// view of where the entity is.
    entered_at: Option<(f32, f32)>,

    /// Per-slot countdowns for behaviours that run in spells rather than on a cooldown alone.
    timers: Vec<u32>,

    /// What each slot's cooldown becomes when its spell ends.
    rest_after_spell: Vec<u32>,

    /// Whether the opening batch of children has been made in this state.
    spawned_opening: bool,

    /// How far through a run of sprites each slot has stepped, cleared when the state is entered.
    ///
    /// Empty until something animates, which is five behaviours in the whole of the content.
    stepped: Vec<u32>,

    /// What each slot drew when its state was entered, for the behaviours that vary per entity.
    ///
    /// Empty until something draws, and cleared on entering a state so the next entry draws again.
    draws: Vec<Option<Drawn>>,

    /// Which `follow` slots have stopped beside the target they were chasing.
    ///
    /// The original's `Resting` state (`logic/behaviors/Follow.cs:103-119`). A chase gives up at
    /// `range` and does not start again until the target is a whole tile further off than that, so
    /// which of the two the behaviour is in has to be remembered rather than worked out from the
    /// distance alone. Without it an enemy standing at exactly its leash chases on one tick and
    /// stops on the next, for as long as the target stands still.
    ///
    /// Never cleared on entering a state, which is where the original keeps it: `Follow` overrides
    /// neither `OnStateEntry` nor `OnStateExit`, so its state storage outlives the state that ran it
    /// (`logic/Behavior.cs:27-58`).
    ///
    /// Empty until something follows, so an enemy that never chases pays nothing for this.
    resting: Vec<bool>,

    /// What is left of each `timed` group's cycle, and [`UNSTARTED`] for one not entered yet.
    ///
    /// Its own store rather than a cooldown, because a cooldown is spent by the passing of time and
    /// this is spent by the group being *run*: the original decrements it inside `Timed.TickCore`
    /// (`logic/behaviors/Timed.cs:31-45`), which a `sequence` only reaches for the child whose turn
    /// it is. Ticking every group every tick had a two-part sequence hand over after a single tick,
    /// because the part waiting its turn ran its period down while it waited.
    ///
    /// Empty until something is timed, so an enemy with no `timed` pays nothing for this.
    every: Vec<u32>,

    /// Whether the group that last ran finished a cycle on this tick.
    ///
    /// The original's `CycleStatus` (`logic/CycleBehavior.cs`), kept only for the one place that
    /// reads it here: a `sequence` steps on when its current child reports `Completed`, and a
    /// `timed` child reports that when its period runs out rather than when it acts.
    completed: bool,

    /// The tick of the original's clock the current tick is spending, which is nought on a call
    /// that only banked time. Held so a group can spend it more than once, as `Timed` does.
    ticked_ms: u32,

    /// What is left of each timed transition's countdown, and [`UNSTARTED`] for one never reached.
    ///
    /// Kept per transition and never cleared, which is where the original keeps it
    /// (`logic/Transition.cs:24-35`): the countdown belongs to the transition object and
    /// `Entity.SwitchTo` does not touch the state storage. Leaving a state part-way through a
    /// twenty-second timer and coming back to it therefore resumes at whatever was left, so the
    /// second visit to a phase is shorter than the first. Restarting it instead — which is what
    /// measuring from a per-state clock does — gives every visit the full length.
    timed_transitions: Vec<u32>,

    /// Whether a state has been switched into whose entry work has not been done yet.
    ///
    /// `Entity.SwitchTo` moves the state and sets `_stateEntry`, then returns; the entry itself
    /// runs at the top of the *next* `TickState` (`realm/Entity.cs:236-269`). Doing both at the
    /// moment of the switch would put the new state a whole tick ahead of where the original has
    /// it, and would spend the tick a transition fires on the new state instead of the old one.
    owes_entry: bool,

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
            wander_remaining: 0.0,
            buzz_angle: 0.0,
            buzz_remaining: 0.0,
            elapsed_ms: 0,
            grain_ms: 0,
            deadline_ms: 0,
            damage_in_state: 0,
            phase: 0.0,
            charge: None,
            said: 0,
            step: 0,
            still_for_ms: 0,
            was_at: None,
            entered_at: None,
            every: Vec::new(),
            completed: false,
            ticked_ms: 0,
            timed_transitions: vec![UNSTARTED; program.transition_slots],
            owes_entry: false,
            chain: Vec::new(),
            timers: vec![0; program.slots],
            rest_after_spell: vec![0; program.slots],
            spawned_opening: false,
            resting: Vec::new(),
            stepped: Vec::new(),
            draws: Vec::new(),
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
    ///
    /// `Order` reaches for the same `Entity.SwitchTo` a transition does
    /// (`logic/behaviors/Order.cs:38`), so an ordered minion also owes its entry until its own next
    /// tick rather than being entered inside the tick of whoever ordered it.
    pub fn force_into(&mut self, program: &Program, target: usize) {
        self.switch_to(program, target);
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

    /// How far through its run of sprites this slot has stepped.
    fn stepped(&self, slot: usize) -> u32 {
        self.stepped.get(slot).copied().unwrap_or(0)
    }

    /// Moves a slot on to the next sprite of its run.
    fn step(&mut self, slot: usize) {
        if self.stepped.len() <= slot {
            self.stepped.resize(slot + 1, 0);
        }
        self.stepped[slot] += 1;
    }

    /// What this slot drew on entering the state, if it has drawn yet.
    fn drawn(&self, slot: usize) -> Option<Drawn> {
        self.draws.get(slot).copied().flatten()
    }

    /// Remembers a draw for as long as the state lasts.
    fn draw(&mut self, slot: usize, drawn: Drawn) -> Drawn {
        if self.draws.len() <= slot {
            self.draws.resize(slot + 1, None);
        }
        self.draws[slot] = Some(drawn);
        drawn
    }

    /// Whether a `follow` slot is standing beside its target rather than chasing it.
    fn resting(&self, slot: usize) -> bool {
        self.resting.get(slot).copied().unwrap_or(false)
    }

    /// Records whether a `follow` slot has stopped, allocating only for one that has.
    fn rest(&mut self, slot: usize, stopped: bool) {
        if self.resting.len() <= slot {
            if !stopped {
                return;
            }
            self.resting.resize(slot + 1, false);
        }
        self.resting[slot] = stopped;
    }

    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed % 10_000) as f32 / 10_000.0
    }

    /// Moves to a state, landing on its innermost child, and owes it an entry.
    ///
    /// Only the move happens here. `Entity.SwitchTo` (`realm/Entity.cs:236-244`) sets the state and
    /// a flag and does nothing else; everything the new state needs doing to it waits for the top of
    /// the next tick, which is [`enter`](Self::enter).
    fn switch_to(&mut self, program: &Program, target: usize) {
        self.current = program
            .state(target)
            .map(|state| state.entry)
            .unwrap_or(target);
        self.owes_entry = true;
    }

    /// Does what entering the current state calls for: fresh cooldowns and nothing carried over.
    ///
    /// The original's `OnStateEntry` pass, which runs at the top of the tick after the switch and
    /// not on the tick of the switch itself (`realm/Entity.cs:257-269`).
    fn enter(&mut self, program: &Program) {
        let landing = self.current;

        self.in_state_ms = 0;
        self.damage_in_state = 0;
        self.charge = None;
        self.spawned_opening = false;
        self.timers.iter_mut().for_each(|timer| *timer = 0);
        self.rest_after_spell.iter_mut().for_each(|rest| *rest = 0);

        // Drawn afresh on every entry, which is where the original draws them.
        self.draws.iter_mut().for_each(|drawn| *drawn = None);
        self.stepped.iter_mut().for_each(|step| *step = 0);

        // `Timed.OnStateEntry` puts the whole period back on the clock, and the wrappers pass entry
        // down to their children, so a timed group nested in a sequence is reset too.
        self.every.iter_mut().for_each(|left| *left = UNSTARTED);

        // Filled in on the first tick of the new state, by whoever is looking at the world then.
        self.entered_at = None;

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

        // A state switched into last tick is entered now, before anything else, which is where the
        // original does it (`realm/Entity.cs:257-269`).
        if self.owes_entry {
            self.owes_entry = false;
            self.enter(program);
        }

        self.damage_in_state = self.damage_in_state.saturating_add(senses.damage_taken);

        // What the caller has given us since the original's clock last moved, spent a whole 166 ms
        // tick at a time. A caller ticking faster than the original banks the remainder rather than
        // dropping it, so six of our ticks are one of theirs and nothing drifts.
        self.grain_ms += elapsed_ms;
        let ticked_ms = self.grain_ms / LOGIC_TICK_MS * LOGIC_TICK_MS;
        self.grain_ms -= ticked_ms;
        self.ticked_ms = ticked_ms;

        self.in_state_ms = self.in_state_ms.saturating_add(ticked_ms);

        // Movement is measured rather than asked about: a behaviour that wanted to move and was
        // stopped by a wall has not moved, and that is what the transition is watching for.
        const STILL: f32 = 0.01;
        match self.was_at {
            Some((x, y)) if (senses.x - x).abs() < STILL && (senses.y - y).abs() < STILL => {
                self.still_for_ms = self.still_for_ms.saturating_add(ticked_ms);
            }
            _ => self.still_for_ms = 0,
        }
        self.was_at = Some((senses.x, senses.y));

        if let Some((_, left)) = &mut self.charge {
            *left = left.saturating_sub(ticked_ms);
            if *left == 0 {
                self.charge = None;
            }
        }

        for slot in self.cooldowns.iter_mut() {
            *slot = slot.saturating_sub(ticked_ms);
        }

        // A spell that has run its length hands over to the rest that follows it. The rest is the
        // behaviour's own cooldown, which is why it is armed here rather than where the spell runs.
        for index in 0..self.timers.len() {
            if self.timers[index] == 0 {
                continue;
            }
            self.timers[index] = self.timers[index].saturating_sub(ticked_ms);
            if self.timers[index] == 0 {
                self.cooldowns[index] = on_the_original_clock(self.rest_after_spell[index]);
            }
        }

        // Transitions first, innermost outwards, first match wins.
        //
        // The state this tick belongs to, whatever a transition does to it further down. The
        // original walks the chain it started the tick on and keeps ticking it after a transition
        // has fired (`realm/Entity.cs:271-292`), so a transition costs a tick before the new state
        // says anything: the phase change is announced on one tick and acted on from the next.
        let origin = self.current;
        program.ancestry(origin, &mut self.chain);
        let chain = std::mem::take(&mut self.chain);

        let mut moved_to = None;
        'outer: for index in &chain {
            let Some(state) = program.state(*index) else {
                continue;
            };
            for transition in &state.transitions {
                // The two conditions that count down rather than look at the world keep the count
                // on the transition itself, so they are stepped here where the transition is in
                // hand rather than inside `fires`, which only sees the condition.
                let fired = match &transition.condition {
                    Condition::Timed { after_ms } => {
                        self.counts_down(transition.slot, *after_ms, *after_ms, ticked_ms)
                    }

                    Condition::TimedRandom { min_ms, max_ms } => {
                        // Randomised draws its first wait from nought to the time and every later
                        // one from the time itself (`TimedRandomTransition.cs:22-38`). Drawn only
                        // where the original draws, so a mind that never reaches this transition
                        // rolls nothing and stays deterministic.
                        let unstarted = self.timed_transitions.get(transition.slot)
                            == Some(&UNSTARTED)
                            && min_ms != max_ms;
                        let first = if unstarted {
                            (self.random() * *max_ms as f32) as u32
                        } else {
                            *max_ms
                        };
                        self.counts_down(transition.slot, first, *max_ms, ticked_ms)
                    }

                    other => self.fires(program, other, senses),
                };

                if fired {
                    moved_to = Some(transition.target);
                    break 'outer;
                }
            }
        }
        self.chain = chain;

        if let Some(target) = moved_to {
            self.switch_to(program, target);
        }

        // Where the state began, for the behaviours that measure from it. Taken here rather than
        // inside `enter`, which is also called from an order and has no view of the world.
        if self.entered_at.is_none() {
            self.entered_at = Some((senses.x, senses.y));
        }

        // Then behaviours, again innermost outwards, and still the state the tick began in.
        program.ancestry(origin, &mut self.chain);
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

    /// Steps one transition's own countdown by a tick, and says whether it has run out.
    ///
    /// `TimedTransition.TickCore` (`logic/transitions/TimedTransition.cs:26-34`) fires on the tick
    /// that *finds* the counter at or below zero and re-arms it there, rather than on the tick that
    /// empties it, so a wait written as `t` takes `ceil(t / 166) + 1` ticks. Stepped only on a tick
    /// of the original's clock, because `TickState` runs on no other.
    ///
    /// The counter survives leaving the state, which is the point of keeping it here rather than
    /// measuring against how long the state has been current.
    fn counts_down(&mut self, slot: usize, first_ms: u32, again_ms: u32, ticked_ms: u32) -> bool {
        if ticked_ms == 0 {
            return false;
        }
        let Some(left) = self.timed_transitions.get_mut(slot) else {
            return false;
        };

        if *left == UNSTARTED {
            *left = first_ms;
        }
        if *left == 0 {
            *left = again_ms;
            return true;
        }

        *left = left.saturating_sub(ticked_ms);
        false
    }

    fn fires(&mut self, program: &Program, condition: &Condition, senses: &Senses) -> bool {
        match condition {
            Condition::Timed { after_ms } => self.in_state_ms >= on_the_original_clock(*after_ms),

            Condition::TimedRandom { min_ms, max_ms } => {
                if self.deadline_ms == 0 {
                    let span = max_ms.saturating_sub(*min_ms);
                    self.deadline_ms = min_ms + (self.random() * span as f32) as u32;
                    // A range of zero would otherwise redraw every tick and never be held.
                    self.deadline_ms = self.deadline_ms.max(1);
                }
                self.in_state_ms >= on_the_original_clock(self.deadline_ms)
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

            Condition::NotMoving { after_ms } => {
                self.still_for_ms >= on_the_original_clock(*after_ms)
            }

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
                predictive,
                rotate_angle,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }

                // Held before anything is aimed or rolled, exactly where `Shoot.cs:132` holds it:
                // inside the branch that has already found the cooldown spent, and returning
                // without writing a new one. The volley is refused, not deferred.
                if senses.stunned {
                    return 1;
                }

                let target = senses
                    .nearest_player
                    .filter(|player| player.distance <= *acquire_range);

                // A fixed angle fires regardless of who is about. Otherwise it aims at whoever is
                // nearest *and within its own range*, falling back to the default angle when the
                // content gives one and holding fire when it does not.
                let aimed = match (fixed_angle, target) {
                    (Some(degrees), _) => Some(degrees.to_radians()),

                    // Leading a moving target, which is the whole of `Shoot.Predict`: where the
                    // target will be if it keeps going, four samples of its own travel ahead of
                    // where it is. Rolled per shot, so a shot written at a half leads every other
                    // time and looks like an enemy that sometimes reads you and sometimes does not.
                    (None, Some(player)) => {
                        Some(if *predictive != 0.0 && *predictive > self.random() {
                            let ahead_x = player.x + PREDICT_STEPS * (player.x - player.past_x);
                            let ahead_y = player.y + PREDICT_STEPS * (player.y - player.past_y);
                            (ahead_y - senses.y).atan2(ahead_x - senses.x)
                        } else {
                            (player.y - senses.y).atan2(player.x - senses.x)
                        })
                    }

                    (None, None) => default_angle.map(f32::to_radians),
                };
                let Some(angle) = aimed else {
                    return 1;
                };

                // The spread turns a little further with every volley this behaviour has fired,
                // which is what sweeps a fixed pattern around the room. The count belongs to the
                // behaviour rather than to the enemy, so a type with several copies alive shares
                // one sweep between them — see [`next_rotation`].
                //
                // Asked for only by the shots that turn. The original advances the count for every
                // volley, but it is read only through this multiplication, so for a shot with no
                // rotation the number cannot be observed.
                let rotations = if *rotate_angle != 0.0 {
                    next_rotation(program, slot) as f32
                } else {
                    0.0
                };
                let turned =
                    angle + angle_offset.to_radians() + rotate_angle.to_radians() * rotations;

                // Halved and rounded up while dazed: `(int)Math.Ceiling(_count / 2.0)`, a double
                // division, so three shots become two rather than one.
                let count = if senses.dazed {
                    count.div_ceil(2)
                } else {
                    *count
                };

                out.push(Action::Shoot {
                    angle: turned,
                    count,
                    spread: *spread,
                    projectile: *projectile,
                });

                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = on_the_original_clock(*cooldown_ms);
                }
                1
            }

            Primitive::Wander { speed } => {
                let speed = speed_after_paralysis(program, slot, senses, *speed);
                if *has_moved {
                    return 1;
                }

                // Legs of six tenths of a tile, each in one of the four diagonals, which is what
                // `Wander.cs:45` draws: two independent signs, normalised. Not a heading that
                // drifts — the original never moves a wanderer along an axis, and the change of
                // direction is a step rather than a curve.
                if self.wander_remaining <= 0.0 {
                    let east = self.random() < 0.5;
                    let south = self.random() < 0.5;
                    self.wander_angle = match (east, south) {
                        (true, true) => std::f32::consts::FRAC_PI_4,
                        (true, false) => -std::f32::consts::FRAC_PI_4,
                        (false, true) => 3.0 * std::f32::consts::FRAC_PI_4,
                        (false, false) => -3.0 * std::f32::consts::FRAC_PI_4,
                    };
                    self.wander_remaining = WANDER_LEG;
                }

                self.wander_remaining -=
                    tiles_per_second(speed, false) * (self.elapsed_ms as f32 / 1000.0);

                out.push(Action::Move {
                    angle: self.wander_angle,
                    speed,
                });
                *has_moved = true;
                1
            }

            Primitive::Buzz {
                speed,
                distance,
                cooldown_ms,
            } => {
                let speed = speed_after_paralysis(program, slot, senses, *speed);
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
                        *cooldown = on_the_original_clock(*cooldown_ms);
                    }
                }

                // Spent by how far it actually travelled, which is what `Buzz.cs:66` subtracts, so
                // a dart covers its distance rather than a fraction of it.
                self.buzz_remaining -=
                    tiles_per_second(speed, false) * (self.elapsed_ms as f32 / 1000.0);

                out.push(Action::Move {
                    angle: self.buzz_angle,
                    speed,
                });
                *has_moved = true;
                1
            }

            Primitive::Follow {
                speed,
                acquire_range,
                range,
                duration_ms,
                cooldown_ms,
            } => {
                let speed = speed_after_paralysis(program, slot, senses, *speed);
                if *has_moved {
                    return 1;
                }

                // With a duration the original follows in spells: it chases for that long, then
                // stands for the cooldown, then chases again. Without one it simply follows, which
                // is what the great majority of uses ask for.
                if *duration_ms > 0 {
                    if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                        return 1;
                    }
                    if self.timers.get(slot).copied().unwrap_or(0) == 0
                        && let Some(timer) = self.timers.get_mut(slot)
                    {
                        *timer = on_the_original_clock(*duration_ms);
                        self.rest_after_spell[slot] = *cooldown_ms;
                    }
                }

                // Nobody within reach puts the behaviour back to `DontKnowWhere`, which is neither
                // of the two states the deadband is between: the next target found is chased from
                // `range` rather than from `range + 1` (`Follow.cs:70-75` and `:104-110`).
                let Some(player) = senses.nearest_player else {
                    self.rest(slot, false);
                    return 1;
                };
                if player.distance > *acquire_range {
                    self.rest(slot, false);
                    return 1;
                }

                // A chase stops at `range` and does not start again until the target is a whole
                // tile beyond it. `Acquired` hands over to `Resting` the moment the distance is no
                // longer greater than `range` (`Follow.cs:87-101`), and `Resting` hands back
                // only once it passes `range + 1` (`Follow.cs:113`). Chasing again at `range` instead
                // has an enemy on the edge of its leash step in and out of pursuit every tick.
                let leash = if self.resting(slot) {
                    *range + 1.0
                } else {
                    *range
                };
                if player.distance <= leash {
                    self.rest(slot, true);
                    return 1;
                }
                self.rest(slot, false);

                // The chase is aimed a little off centre, as the original aims it: each axis of the
                // difference vector has `Random.Next(-2, 2) / 2f` subtracted from it before the
                // vector is normalised (`Follow.cs:90-91`). The draw excludes its upper bound, so it
                // is one of −2, −1, 0 and 1, and subtracting it shifts the aim by +1, +0.5, 0 or
                // −0.5 tiles — a set that is deliberately not symmetric. Speed is untouched; only
                // the heading moves, and that is what keeps a pack of chasers spread out instead of
                // collapsing into one stack on the way in.
                let east = ((self.random() * 4.0) as i32 - 2) as f32 / 2.0;
                let south = ((self.random() * 4.0) as i32 - 2) as f32 / 2.0;
                let dx = player.x - senses.x - east;
                let dy = player.y - senses.y - south;

                out.push(Action::Move {
                    angle: dy.atan2(dx),
                    speed,
                });
                *has_moved = true;
                1
            }

            Primitive::Orbit {
                speed,
                radius,
                acquire_range,
                target,
                speed_variance,
                radius_variance,
                clockwise,
            } => {
                // Zeroed like everything else, but read only where the original reads it: at the
                // draw below, which happens on entering a state. `Orbit.TickCore` writes the field
                // and then circles on the copy `OnStateEntry` took (`Orbit.cs:77`), so the tick
                // that cripples an orbit is not the tick it slows down on — the next state entry
                // is. The variance is untouched, so a crippled orbit draws in ±10% of the speed it
                // used to have and can even circle backwards.
                let base_speed = if zero_speed_on_paralysis(program, slot, senses.paralyzed) {
                    0.0
                } else {
                    *speed
                };

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
                            .map(|other| (other.x, other.y))
                    }
                    None => senses
                        .nearest_player
                        .filter(|player| player.distance <= *acquire_range)
                        .map(|player| (player.x, player.y)),
                };
                let Some((centre_x, centre_y)) = centre else {
                    return 1;
                };

                // Drawn once on entering the state, as `Orbit.OnStateEntry` does, so a ring of
                // minions spreads into a band at slightly different speeds and radii rather than
                // moving as one rigid object. Without it every crystal in a ring sits at exactly
                // the same distance and the ring is a wheel.
                let drawn = match self.drawn(slot) {
                    Some(drawn) => drawn,
                    None => {
                        let speed = base_speed + speed_variance * (self.random() * 2.0 - 1.0);
                        let radius = radius + radius_variance * (self.random() * 2.0 - 1.0);
                        let direction = match clockwise {
                            Some(true) => 1.0,
                            Some(false) => -1.0,
                            // The content spells this `null` for the two orbits that mean "either
                            // way", and the draw is per entity, so a pair splits.
                            None => {
                                if self.random() < 0.5 {
                                    1.0
                                } else {
                                    -1.0
                                }
                            }
                        };
                        self.draw(
                            slot,
                            Drawn {
                                speed,
                                radius,
                                direction,
                            },
                        )
                    }
                };

                // The original steers at a point on the ring rather than along the tangent: it
                // takes the bearing from the centre to itself, advances that bearing by however far
                // it can travel this tick, and walks at the point that lands on. That is what makes
                // an orbit close on its radius instead of spiralling toward it.
                let bearing = if senses.y == centre_y && senses.x == centre_x {
                    // Standing exactly on top of what it means to circle, which has no bearing at
                    // all. The original jitters by up to a tile to break the tie.
                    (self.random() * 2.0 - 1.0).atan2(self.random() * 2.0 - 1.0)
                } else {
                    (senses.y - centre_y).atan2(senses.x - centre_x)
                };

                // A ring of nothing has no angular speed to divide by. The original divides anyway
                // and moves the entity to a position that is not a number; refusing to circle is
                // the one place this does something the original does not, and nothing in the
                // content asks for it.
                if drawn.radius <= 0.0 {
                    return 1;
                }

                let travel = tiles_per_second(drawn.speed, false);
                let swept = bearing
                    + drawn.direction * (travel / drawn.radius) * (self.elapsed_ms as f32 / 1000.0);

                let (to_x, to_y) = (
                    centre_x + swept.cos() * drawn.radius,
                    centre_y + swept.sin() * drawn.radius,
                );

                out.push(Action::Move {
                    angle: (to_y - senses.y).atan2(to_x - senses.x),
                    speed: drawn.speed,
                });
                *has_moved = true;
                1
            }

            Primitive::StayBack { speed, distance } => {
                let speed = speed_after_paralysis(program, slot, senses, *speed);
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
                    speed,
                });
                *has_moved = true;
                1
            }

            Primitive::StayCloseToSpawn { speed, range } => {
                let speed = speed_after_paralysis(program, slot, senses, *speed);
                if *has_moved {
                    return 1;
                }
                let (dx, dy) = (senses.spawn_x - senses.x, senses.spawn_y - senses.y);
                if (dx * dx + dy * dy).sqrt() <= *range {
                    return 1;
                }

                out.push(Action::Move {
                    angle: dy.atan2(dx),
                    speed,
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
                    *cooldown = on_the_original_clock(*cooldown_ms);
                }
                1
            }

            Primitive::Spawn {
                child,
                max_children,
                initial_spawn,
                cooldown_ms,
                gives_no_xp,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }
                if self.children >= *max_children {
                    return 1;
                }

                // The original makes its opening batch as the state is entered — a fraction of the
                // maximum, truncated — and one at a time on the cooldown after that. A spawner that
                // trickled from nothing would take its whole fight to fill a room the content
                // expects to be full when the fight starts.
                let count = if self.spawned_opening {
                    1
                } else {
                    self.spawned_opening = true;
                    ((*max_children as f32 * *initial_spawn) as u32).max(1)
                }
                .min(*max_children - self.children);

                out.push(Action::Spawn {
                    child: *child,
                    count,
                    offset_x: 0.0,
                    offset_y: 0.0,
                    state: None,
                    delay_ms: 0,
                    gives_no_xp: *gives_no_xp,
                });
                self.children += count;

                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = on_the_original_clock(*cooldown_ms);
                }
                1
            }

            Primitive::Suicide => {
                out.push(Action::Vanish { dies: true });
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
                    gives_no_xp: true,
                    state: None,
                    delay_ms: 0,
                });
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = on_the_original_clock(*cooldown_ms);
                }
                1
            }

            Primitive::TossObject {
                child,
                radius,
                fixed_angle,
                cooldown_ms,
                cooldown_offset_ms: _,
                min_range,
                max_range,
                warning_ms,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }

                // The range is also how far away a target may be: `GetNearestEntity(_range, null)`.
                let player = senses
                    .nearest_player
                    .filter(|player| player.distance <= *radius);

                // With nobody in reach and no fixed bearing there is nothing to aim at, and
                // dropping it underfoot is not what the behaviour is for.
                if player.is_none() && fixed_angle.is_none() {
                    return 1;
                }

                // How far out it lands. A pair of bounds turns the fixed distance into a roll,
                // which is what makes a minefield a field rather than a ring.
                let reach = match (min_range, max_range) {
                    (Some(least), Some(most)) => least + self.random() * (most - least),
                    _ => *radius,
                };

                // A fixed bearing throws its own distance in that direction; otherwise it lands on
                // the target rather than at the thrower's range along the line to them.
                let (offset_x, offset_y) = match (fixed_angle, player) {
                    (Some(degrees), _) => {
                        let angle = degrees.to_radians();
                        (angle.cos() * reach, angle.sin() * reach)
                    }
                    (None, Some(player)) => (player.x - senses.x, player.y - senses.y),
                    (None, None) => return 1,
                };

                out.push(Action::Spawn {
                    child: *child,
                    count: 1,
                    // A thrown thing is worth what it is worth. `TossObject` sets terrain and the
                    // summoned mark on its child and never writes `GivesNoXp` at all
                    // (`TossObject.cs:170-199`), unlike `Spawn` and `Reproduce`, which both do.
                    gives_no_xp: false,
                    offset_x,
                    offset_y,
                    state: None,
                    delay_ms: *warning_ms,
                });

                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = on_the_original_clock(*cooldown_ms);
                }
                1
            }

            Primitive::Grenade {
                radius,
                damage,
                range,
                fixed_angle,
                cooldown_ms,
                effect,
                effect_ms,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }

                // `Grenade.cs:50` returns rather than rolling a new cooldown, so the throw is held
                // for as long as the stun lasts and comes the moment it ends.
                if senses.stunned {
                    return 1;
                }

                // A fixed angle throws at its own range in that direction, with or without anyone
                // about. Otherwise it lands where the player is now: leading them would make it
                // unavoidable, which is the difference between a hard attack and one nobody can
                // play around.
                let (offset_x, offset_y) = match fixed_angle {
                    Some(degrees) => {
                        let angle = degrees.to_radians();
                        (range * angle.cos(), range * angle.sin())
                    }
                    None => {
                        let Some(player) = senses.nearest_player else {
                            return 1;
                        };
                        if player.distance > *range {
                            return 1;
                        }
                        (player.x - senses.x, player.y - senses.y)
                    }
                };

                out.push(Action::Grenade {
                    offset_x,
                    offset_y,
                    radius: *radius,
                    damage: *damage,
                    effect: *effect,
                    effect_ms: *effect_ms,
                });

                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = on_the_original_clock(*cooldown_ms);
                }
                1
            }

            Primitive::Decay { after_ms } => {
                if self.in_state_ms >= on_the_original_clock(*after_ms) {
                    out.push(Action::Vanish { dies: false });
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

            Primitive::SetAltTexture {
                index,
                last,
                step_ms,
                looping,
            } => {
                // Without a last sprite there is no run: the entity wears the one it was given and
                // that is the end of it, which is what two hundred and fifty of the uses want.
                let Some(last) = last else {
                    out.push(Action::Texture { index: *index });
                    return 1;
                };

                let span = (last.saturating_sub(*index) as u32) + 1;
                let stepped = self.stepped(slot);
                let at = if *looping {
                    stepped % span
                } else {
                    stepped.min(span - 1)
                };

                out.push(Action::Texture {
                    index: index.saturating_add(at as u8),
                });

                // Held at the last sprite once the run is over, unless it loops.
                let more = *looping || at + 1 < span;
                if more && self.cooldowns.get(slot).copied().unwrap_or(0) == 0 {
                    self.step(slot);
                    if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                        *cooldown = on_the_original_clock(*step_ms);
                    }
                }
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
                        *cooldown = on_the_original_clock(*cooldown_ms);
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
                    *cooldown = on_the_original_clock(*cooldown_ms);
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
                if *once && let Some(cooldown) = self.cooldowns.get_mut(slot) {
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
                    *cooldown = on_the_original_clock(SCALE_INTERVAL_MS);
                }
                1
            }

            Primitive::Sequence { children } => {
                // One child per turn, advancing only when the current one actually does something.
                // Advancing regardless would step through a pattern while the enemy stood idle.
                let mut used = 1;
                let before = out.len();
                let turn = self.step as usize;
                let mut timed = false;

                for (index, child) in children.iter().enumerate() {
                    if index == turn % children.len().max(1) {
                        timed = matches!(child, Primitive::Every { .. });
                        used += self.run(program, child, slot + used, senses, has_moved, out);
                    } else {
                        used += child.slots();
                    }
                }

                // A `timed` child acts on every tick and finishes only when its period runs out,
                // so acting is not what moves the sequence on: `Sequence.TickCore` steps when the
                // child's `CycleStatus` says `Completed` (`logic/behaviors/Sequence.cs:30-38`), and
                // for a `timed` that is once a period. Reading "it did something" instead would run
                // a whole rotation of phases in as many ticks.
                let advanced = if timed {
                    self.completed
                } else {
                    out.len() > before
                };

                if advanced {
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
                let speed = speed_after_paralysis(program, slot, senses, *speed);
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
                    speed,
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
                    *cooldown = on_the_original_clock(*cooldown_ms);
                }
                1
            }

            Primitive::MoveTo {
                // The original writes `speed = 0` here too, but reads `_speed` when it moves
                // (`MoveTo.cs:26 against :30`), so the write is dead and this behaviour is one of the four
                // paralysis never cripples.
                x,
                y,
                speed,
                relative,
            } => {
                if *has_moved {
                    return 1;
                }

                // An offset is measured from where the enemy stood when the state began, which is
                // where `MoveTo2.OnStateEntry` works its target out.
                let (from_x, from_y) = match relative {
                    true => self.entered_at.unwrap_or((senses.x, senses.y)),
                    false => (0.0, 0.0),
                };
                let (x, y) = (from_x + x, from_y + y);

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
                // The original writes `speed = 0` here too, but reads `_speed` when it moves
                // (`MoveLine.cs:24 against :28`), so the write is dead and this behaviour is one of the four
                // paralysis never cripples.
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
                let speed = speed_after_paralysis(program, slot, senses, *speed);
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
                    speed,
                });
                *has_moved = true;
                1
            }

            Primitive::Charge {
                // The original writes `speed = 0` here too, but reads `_speed` when it moves
                // (`Charge.cs:42 against :70`), so the write is dead and this behaviour is one of the four
                // paralysis never cripples.
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
                    *cooldown = on_the_original_clock((*cooldown_ms).max(CHARGE_MS));
                }
                1
            }

            Primitive::Swirl {
                speed,
                radius,
                targeted,
            } => {
                let speed = speed_after_paralysis(program, slot, senses, *speed);
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
                    speed,
                });
                *has_moved = true;
                1
            }

            Primitive::ReturnToSpawn { speed, tolerance } => {
                // The original writes `speed = 0` here too, but reads `_speed` when it moves
                // (`ReturnToSpawn.cs:26 against :34`), so the write is dead and this behaviour is one of the four
                // paralysis never cripples.
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
                let speed = speed_after_paralysis(program, slot, senses, *speed);
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
                    speed,
                });
                *has_moved = true;
                1
            }

            Primitive::NoExperience => {
                out.push(Action::NoExperience);
                1
            }

            Primitive::RemoveNearby { radius, kind, dies } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }
                out.push(Action::RemoveNearby {
                    radius: *radius,
                    kind: *kind,
                    dies: *dies,
                });
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    *cooldown = on_the_original_clock(ORDER_INTERVAL_MS);
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
                offset,
            } => {
                if self.cooldowns.get(slot).copied().unwrap_or(0) > 0 {
                    return 1;
                }

                // A pair of offsets means one square, that far away, rather than a circle here.
                let (offset_x, offset_y) = offset.unwrap_or((0.0, 0.0));
                out.push(Action::Ground {
                    tile: *tile,
                    radius: if offset.is_some() { 0.0 } else { *radius },
                    offset_x,
                    offset_y,
                });
                if let Some(cooldown) = self.cooldowns.get_mut(slot) {
                    // Ground changes are expensive and repeating one changes nothing, so a
                    // behaviour that gave no interval gets a generous one rather than none.
                    *cooldown = on_the_original_clock((*cooldown_ms).max(GROUND_INTERVAL_MS));
                }
                1
            }

            // Nothing happens during a tick. The world reads these when the entity dies.
            Primitive::OnDeath(_) => 1,

            Primitive::Every {
                period_ms,
                children,
            } => {
                // `Timed.TickCore` (`logic/behaviors/Timed.cs:31-45`) ticks every child on every
                // tick and never skips one. The period is not a gate on them: it is the length of a
                // cycle, and all it decides is when the group reports `Completed` to a `sequence`
                // stepping through it. Gating the children on it instead turned a boss written to
                // chase for two seconds and then pace for one into a boss that took a single step
                // every two seconds.
                if self.every.len() <= slot {
                    self.every.resize(slot + 1, UNSTARTED);
                }
                if self.every[slot] == UNSTARTED {
                    self.every[slot] = *period_ms;
                }

                let steps = children.len().max(1);
                let mut finished = false;

                if self.ticked_ms > 0 {
                    // Once per child, because the original decrements inside its loop over them, so
                    // a group of three spends its period three times as fast. And only the last
                    // child's decrement can leave the status Completed, because the next one round
                    // the loop sets it back to InProgress before decrementing again.
                    for _ in 0..steps {
                        self.every[slot] = self.every[slot].saturating_sub(self.ticked_ms);

                        finished = self.every[slot] == 0;
                        if finished {
                            self.every[slot] = *period_ms;
                        }
                    }
                }

                let mut used = 1;
                for child in children {
                    used += self.run(program, child, slot + used, senses, has_moved, out);
                }

                // After the children, whose own runs would otherwise overwrite it.
                self.completed = finished;
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
            stunned: false,
            dazed: false,
            paralyzed: false,
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
            stunned: false,
            dazed: false,
            paralyzed: false,
        }
    }

    fn with_player_at(x: f32, y: f32) -> Senses<'static> {
        let mut senses = alone();
        let (dx, dy) = (x - senses.x, y - senses.y);
        let there = Some(Nearby::at(x, y, (dx * dx + dy * dy).sqrt()));

        senses.nearest_player = there;
        senses.nearest_player_hiding = there;
        senses
    }

    /// A player standing there, having come from somewhere else one position sample ago.
    fn with_player_moving(x: f32, y: f32, from: (f32, f32)) -> Senses<'static> {
        let mut senses = with_player_at(x, y);
        if let Some(player) = senses.nearest_player.as_mut() {
            player.past_x = from.0;
            player.past_y = from.1;
        }
        senses.nearest_player_hiding = senses.nearest_player;
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
    fn a_thrown_thing_is_worth_what_it_is_worth() {
        // `TossObject` sets its child's terrain and summoned mark and never touches `GivesNoXp`
        // (`TossObject.cs:170-199`), unlike `Spawn`, whose argument for it defaults to true
        // (`Spawn.cs:46`), and `Reproduce`, which hard-codes it (`Reproduce.cs:97`). Three hundred
        // and eighty-nine throws in the content were being handed the spawner's rule instead.
        let program = program(
            r#"enemy "X" {
                state a { toss_object("Egg", 5, 90) }
            }"#,
        );

        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();
        mind.tick(&program, &with_player_at(12.0, 10.0), 50, &mut out);

        let thrown = out
            .iter()
            .find_map(|action| match action {
                Action::Spawn { gives_no_xp, .. } => Some(*gives_no_xp),
                _ => None,
            })
            .expect("a throw");

        assert!(!thrown, "a thrown egg is killable and worth killing");
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

        // `on timed(400ms)` is not four hundred milliseconds on the original: the countdown moves
        // in whole 166 ms logic ticks and the tick that empties it does not fire, so the switch
        // lands on the fourth tick, at 664 ms. Thirteen of our ticks is 650 ms of banked time,
        // which is three of the original's ticks and not yet four.
        for _ in 0..13 {
            mind.tick(&program, &alone(), 50, &mut out);
        }
        assert_eq!(
            mind.state_name(&program),
            "a",
            "three of the original's ticks is not four"
        );

        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(mind.state_name(&program), "b");

        // Still the old state's clock: the switch happened on this tick and the entry that clears
        // it is owed until the next one, which is where `Entity.TickState` does it.
        assert_ne!(mind.in_state_ms(), 0);

        mind.tick(&program, &alone(), 50, &mut out);
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

        // A `cooldown: 500ms` shoot does not fire twice a second on the original. Five hundred
        // milliseconds is four of its 166 ms ticks once rounded up, the tick that fires does not
        // decrement, and so the next shot lands five ticks later, at 830 ms. Sixteen more of our
        // ticks is 800 ms of banked time, which is four of the original's ticks and not yet five.
        for _ in 0..15 {
            mind.tick(&program, &with_player_at(14.0, 10.0), 50, &mut out);
            assert!(out.is_empty(), "should still be reloading");
        }

        mind.tick(&program, &with_player_at(14.0, 10.0), 50, &mut out);
        assert_eq!(out.len(), 1, "and fires again once ready");
    }

    /// When each shot lands, in milliseconds from the first tick, for a mind driven at `step_ms`.
    fn shot_times(source: &str, step_ms: u32, for_ms: u32) -> Vec<u32> {
        let program = program(source);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let mut at = Vec::new();
        let mut elapsed = 0;
        while elapsed < for_ms {
            mind.tick(&program, &with_player_at(12.0, 10.0), step_ms, &mut out);
            if out.iter().any(|a| matches!(a, Action::Shoot { .. })) {
                at.push(elapsed);
            }
            elapsed += step_ms;
        }
        at
    }

    #[test]
    fn a_cooldown_keeps_the_original_period_whatever_the_tick_is() {
        // The period a countdown really has on the original: rounded up to a whole 166 ms logic
        // tick, plus the tick that reads it as spent without acting on it
        // (`logic/behaviors/Shoot.cs:206-215`).
        //
        // Reading the written number as the period is what a 50 ms tick does if nothing stops it,
        // and it makes every timed enemy in the game act between a third and two thirds faster
        // than it does on the original — 500 ms between shots instead of 830, 1000 instead of 1328.
        for (written, period) in [(300u32, 498u32), (500, 830), (1000, 1328), (1200, 1494)] {
            let source = format!(
                r#"enemy "X" {{ state a {{ shoot(20, count: 1, cooldown: {written}ms) }} }}"#
            );

            // Driven at our rate and at the original's, which must agree with each other and with
            // the original's arithmetic.
            for step in [50u32, 166] {
                let at = shot_times(&source, step, period * 6);
                assert!(
                    at.len() >= 5,
                    "expected several shots for {written}ms at a {step}ms tick, got {at:?}"
                );

                for pair in at.windows(2) {
                    let gap = pair[1] - pair[0];
                    assert!(
                        gap.abs_diff(period) < step,
                        "a cooldown of {written}ms fires every {period}ms on the original, \
                         but a {step}ms tick put {gap}ms between two shots: {at:?}"
                    );
                }
            }
        }
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

    /// The angle of the one shot a tick produced.
    fn shot_angle(out: &[Action]) -> f32 {
        match out.first() {
            Some(Action::Shoot { angle, .. }) => *angle,
            other => panic!("expected a shot, got {other:?}"),
        }
    }

    #[test]
    fn a_leading_shot_aims_four_samples_ahead_of_a_moving_target() {
        // `Shoot.Predict`: `targetX = target.X + 4 * (target.X - history.X)`, and the same for Y,
        // then the angle to that point. A shot at a probability of one leads every time.
        let program =
            program(r#"enemy "X" { state a { shoot(count: 1, predictive: 1, cooldown: 100ms) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        // Standing at (10, 10), a player at (16, 10) who was at (14, 10) one sample ago: two tiles
        // of travel, so the shot is aimed four of those ahead, at (24, 10) — still due east.
        mind.tick(
            &program,
            &with_player_moving(16.0, 10.0, (14.0, 10.0)),
            50,
            &mut out,
        );
        assert!(shot_angle(&out).abs() < 1e-5, "due east, along the travel");

        // The same travel across the enemy's line of sight, where leading is visible as an angle.
        // The player is at (16, 10) having come from (16, 8): the aim point is (16, 18), which is
        // atan2(8, 6) from the enemy at (10, 10) rather than the atan2(0, 6) of aiming at them.
        let mut mind = Mind::new(&program, 1);
        mind.tick(
            &program,
            &with_player_moving(16.0, 10.0, (16.0, 8.0)),
            50,
            &mut out,
        );

        let led = shot_angle(&out);
        assert!(
            (led - 8.0f32.atan2(6.0)).abs() < 1e-5,
            "aimed at the lead point, not at the player: {led}"
        );
        assert!(led > 0.5, "the aim has to be well off the direct line");
    }

    #[test]
    fn a_shot_that_does_not_lead_aims_where_the_target_stands() {
        // The same target and the same travel, with the probability at zero: `_predictive != 0` is
        // the first half of the original's test, so a shot written without one never leads.
        let program = program(r#"enemy "X" { state a { shoot(count: 1, cooldown: 100ms) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(
            &program,
            &with_player_moving(16.0, 10.0, (16.0, 8.0)),
            50,
            &mut out,
        );
        assert!(shot_angle(&out).abs() < 1e-5, "straight at the player");
    }

    #[test]
    fn a_shot_leads_as_often_as_the_content_asks_and_no_more() {
        // `_predictive > Random.NextDouble()`, rolled per shot: a half leads about half the time,
        // which is what makes such an enemy feel like it sometimes reads you and sometimes does not.
        let program = program(
            r#"enemy "X" { state a { shoot(count: 1, predictive: 0.5, cooldown: 100ms) } }"#,
        );
        let mut mind = Mind::new(&program, 7);
        let mut out = Vec::new();

        // A `cooldown: 100ms` shoot is one shot every 332 ms on the original's clock, so seventy
        // seconds of ticks is a little over two hundred shots to sample.
        let mut led = 0;
        for _ in 0..700 {
            mind.tick(
                &program,
                &with_player_moving(16.0, 10.0, (16.0, 8.0)),
                100,
                &mut out,
            );
            if !out.is_empty() && shot_angle(&out).abs() > 1e-5 {
                led += 1;
            }
        }

        assert!(
            (60..=140).contains(&led),
            "a half should lead about half of two hundred shots, not {led}"
        );
    }

    #[test]
    fn a_target_with_no_history_is_led_from_the_origin() {
        // The original's history is a fresh `Position[256]`, never primed, so a player who has been
        // in the world for less than two samples reads back `(0, 0)` and is led toward five times
        // their own position. It lasts two thirds of a second and it is what the original does.
        let program =
            program(r#"enemy "X" { state a { shoot(count: 1, predictive: 1, cooldown: 100ms) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        // A player at (16, 12) with an empty history: the aim point is (80, 60), and from (10, 10)
        // that is atan2(50, 70) rather than the atan2(2, 6) of aiming at them.
        mind.tick(&program, &with_player_at(16.0, 12.0), 50, &mut out);
        assert!(
            (shot_angle(&out) - 50.0f32.atan2(70.0)).abs() < 1e-5,
            "an unsampled target is led from the origin"
        );
    }

    #[test]
    fn a_spread_turns_a_little_further_with_every_volley() {
        // `a += _rotateAngle * _rotateCount; _rotateCount++`, which is what sweeps a fixed pattern
        // around the room. The count is never reset, so the sweep carries on across states.
        //
        // Its own enemy name, because the count is kept per program and slot for the whole process
        // rather than per enemy: a second test that shared the name and rotated at the same slot
        // would start this one part-way through its sweep.
        let program = program(
            r#"enemy "Sweeper" {
                 state a { shoot(count: 1, fixed_angle: 0, rotate_angle: 10, cooldown: 100ms) }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        // Gathered as they come rather than one a tick: the volleys are 332 ms apart on the
        // original's clock, whatever the caller's tick is.
        let mut angles = Vec::new();
        for _ in 0..20 {
            mind.tick(&program, &alone(), 100, &mut out);
            if !out.is_empty() {
                angles.push(shot_angle(&out));
            }
        }
        assert!(angles.len() >= 4, "expected four volleys, got {angles:?}");
        angles.truncate(4);

        for (fired, angle) in angles.iter().enumerate() {
            let wanted = (10.0 * fired as f32).to_radians();
            assert!(
                (angle - wanted).abs() < 1e-5,
                "volley {fired} should be {wanted} radians round, not {angle}"
            );
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
                // Eastward, but not exactly: the original knocks the chase vector off by up to a
                // tile on each axis before normalising it, which at ten tiles is six degrees.
                assert!(angle.abs() < 1.0f32.atan2(9.0), "roughly due east, got {angle}");
                assert_eq!(*speed, 1.0);
            }
            other => panic!("expected a move, got {other:?}"),
        }

        // Already inside the range it wants to keep.
        mind.tick(&program, &with_player_at(12.0, 10.0), 50, &mut out);
        assert!(out.is_empty(), "close enough, so it stops");
    }

    #[test]
    fn follow_aims_a_little_off_centre_rather_than_dead_straight() {
        // `Follow.cs:90-91` subtracts `Random.Next(-2, 2) / 2f` from each axis of the chase vector
        // before normalising it. The draw excludes its upper bound, so it is one of −2, −1, 0 and
        // 1, and subtracting it shifts the aim by +1, +0.5, 0 or −0.5 tiles. Aiming dead straight
        // instead lets a pack of chasers stack into a single body on the way in.
        let program = program(r#"enemy "X" { state a { follow(1.0, 20, 3) } }"#);
        let mut mind = Mind::new(&program, 7);
        let mut out = Vec::new();

        // Ten tiles due east and never moving, so every difference between headings is the wobble.
        let senses = with_player_at(20.0, 10.0);

        // Every heading the four offsets on each axis can produce, from the same standing start.
        // Thirteen rather than sixteen: with no offset northwards or southwards the heading is due
        // east whatever the other axis did.
        let offsets = [1.0f32, 0.5, 0.0, -0.5];
        let mut possible: Vec<f32> = Vec::new();
        for east in offsets {
            for south in offsets {
                let angle = south.atan2(10.0 + east);
                if !possible.iter().any(|had| (had - angle).abs() < 1e-6) {
                    possible.push(angle);
                }
            }
        }
        assert_eq!(possible.len(), 13);

        let mut seen: Vec<f32> = Vec::new();
        for _ in 0..400 {
            mind.tick(&program, &senses, 50, &mut out);
            let Some(Action::Move { angle, .. }) = out.first() else {
                panic!("a follow ten tiles out should move");
            };
            assert!(
                possible.iter().any(|wanted| (wanted - angle).abs() < 1e-6),
                "heading {angle} is not one the offsets can produce"
            );
            if !seen.iter().any(|had| (had - angle).abs() < 1e-6) {
                seen.push(*angle);
            }
        }

        assert_eq!(
            seen.len(),
            possible.len(),
            "every heading the offsets allow should turn up: {seen:?}"
        );

        // The asymmetry, which is the part instinct would get wrong: the northward offset reaches a
        // whole tile and the southward one only half of it.
        let north = 1.0f32.atan2(10.0);
        let south = (-1.0f32).atan2(10.0);
        assert!(
            seen.iter().any(|angle| (angle - north).abs() < 1e-6),
            "a whole tile of northward offset should appear"
        );
        assert!(
            !seen.iter().any(|angle| (angle - south).abs() < 1e-6),
            "a whole tile of southward offset cannot be drawn"
        );
    }

    #[test]
    fn follow_rests_until_its_target_is_a_tile_past_its_range() {
        // `Follow` stops at `range` and re-acquires only past `range + 1` (`Follow.cs:87-118`).
        // Chasing again the moment the distance exceeds `range` makes an enemy on the edge of its
        // leash start and stop on alternate ticks.
        let program = program(r#"enemy "X" { state a { follow(1.0, 20, 3) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        mind.tick(&program, &with_player_at(15.0, 10.0), 50, &mut out);
        assert!(!out.is_empty(), "five tiles out, so it chases");

        mind.tick(&program, &with_player_at(13.0, 10.0), 50, &mut out);
        assert!(out.is_empty(), "at its range, so it stops");

        mind.tick(&program, &with_player_at(13.5, 10.0), 50, &mut out);
        assert!(
            out.is_empty(),
            "half a tile past its range is inside the deadband, so it stays put"
        );

        mind.tick(&program, &with_player_at(14.5, 10.0), 50, &mut out);
        assert!(!out.is_empty(), "a tile and a half past its range, so it chases again");

        mind.tick(&program, &with_player_at(13.5, 10.0), 50, &mut out);
        assert!(
            !out.is_empty(),
            "having re-acquired, the deadband is gone until it stops again"
        );
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

        // Both are ready on the same tick of the original's clock — 50 ms rounds up to one of its
        // ticks and the tick that empties the countdown does not fire, so both come due at 332 ms.
        for _ in 0..7 {
            mind.tick(&program, &alone(), 50, &mut out);
        }
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

        // Six seconds of bouncing between a and b, every entry to a well inside the ten-second
        // cooldown. A `timed(100ms)` is 332 ms of the original's clock, so a round trip is two
        // thirds of a second and there are nine entries to a in the run. Without the reset there
        // would be exactly one shot in the whole of it.
        let mut shots = 0;
        for _ in 0..120 {
            mind.tick(&program, &alone(), 50, &mut out);
            shots += out
                .iter()
                .filter(|action| matches!(action, Action::Shoot { .. }))
                .count();
        }

        assert!(
            shots >= 8,
            "expected a shot on each re-entry, got {shots} in six seconds"
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
        assert!(
            !out.iter().any(|a| matches!(a, Action::Heal { .. })),
            "the tick a transition fires is spent in the state being left"
        );

        mind.tick(&program, &hurt, 50, &mut out);
        assert!(out.iter().any(|a| matches!(a, Action::Heal { .. })));
    }

    #[test]
    fn the_tick_a_transition_fires_is_spent_in_the_state_being_left() {
        // `Entity.TickState` (`realm/Entity.cs:271-292`) keeps walking the chain it started the
        // tick on after a transition has fired, and the new state's `OnStateEntry` waits for the
        // top of the next tick. Entering and running the new state in the same tick puts every
        // phase change in the game a tick early and skips the last act of the phase being left.
        let program = program(
            r#"enemy "X" {
                 state before { conditional_effect(invulnerable) on timed(1ms) -> after }
                 state after { conditional_effect(paralyzed) }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let acting = |out: &Vec<Action>| match out.first() {
            Some(Action::Effect { effect, .. }) => *effect,
            other => panic!("expected an effect, got {other:?}"),
        };

        // `on timed(1ms)` is two of the original's ticks, so the first tick stays put.
        mind.tick(&program, &alone(), 166, &mut out);
        assert_eq!(mind.state_name(&program), "before");
        let leaving = acting(&out);

        // The second fires the transition, and still acts for the state it is leaving.
        mind.tick(&program, &alone(), 166, &mut out);
        assert_eq!(mind.state_name(&program), "after");
        assert_eq!(
            acting(&out),
            leaving,
            "the new state ran a tick before the original would have entered it"
        );

        mind.tick(&program, &alone(), 166, &mut out);
        assert_ne!(acting(&out), leaving, "and from the next tick it is in it");
    }

    #[test]
    fn a_wander_walks_diagonals_in_legs_of_six_tenths_of_a_tile() {
        // `Wander.TickCore`: a direction of two independent signs, normalised, redrawn once the leg
        // has been walked off. Never along an axis, and never a curve.
        let program = program(r#"enemy "X" { state a { wander(0.4) } }"#);
        let mut mind = Mind::new(&program, 12345);
        let mut out = Vec::new();

        let mut angles = Vec::new();
        for _ in 0..80 {
            mind.tick(&program, &alone(), 50, &mut out);
            if let Some(Action::Move { angle, .. }) = out.first() {
                angles.push(*angle);
            }
        }

        assert_eq!(angles.len(), 80);

        let diagonals = [
            std::f32::consts::FRAC_PI_4,
            -std::f32::consts::FRAC_PI_4,
            3.0 * std::f32::consts::FRAC_PI_4,
            -3.0 * std::f32::consts::FRAC_PI_4,
        ];
        for angle in &angles {
            assert!(
                diagonals.iter().any(|corner| (corner - angle).abs() < 1e-5),
                "a wander only ever walks a diagonal, not {angle}"
            );
        }

        // At 0.4 the enemy covers 2.96 tiles a second, so a leg of six tenths lasts four ticks and
        // a little. Anything much shorter is a heading being redrawn every tick.
        let changes = angles.windows(2).filter(|pair| pair[0] != pair[1]).count();
        assert!(
            (10..=20).contains(&changes),
            "eighty ticks should hold about sixteen legs, not {changes} changes"
        );
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
        program.resolve(|name| {
            if name == "King" {
                vec![KING]
            } else {
                Vec::new()
            }
        });

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
    fn an_animated_texture_steps_through_its_run_and_stops_at_the_end() {
        // `SetAltTexture(minValue, maxValue, cooldown, loop)`: one sprite per cooldown from the
        // first to the last, held there unless it loops.
        let program = program(
            r#"enemy "X" { state a { set_alt_texture(1, 3, cooldown: 200, loop: false) } }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let mut drawn = Vec::new();
        for _ in 0..20 {
            mind.tick(&program, &alone(), 100, &mut out);
            match out.first() {
                Some(Action::Texture { index }) => drawn.push(*index),
                other => panic!("expected a texture, got {other:?}"),
            }
        }

        assert_eq!(drawn[0], 1, "starts at the first sprite");
        assert_eq!(drawn[19], 3, "and holds at the last");
        assert!(
            drawn.windows(2).all(|pair| pair[1] >= pair[0]),
            "a run that does not loop never goes back: {drawn:?}"
        );

        // Two hundred milliseconds a sprite over three sprites, so the run takes four tenths of a
        // second and cannot be over inside two ticks of a hundred.
        let reached = drawn.iter().position(|index| *index == 3).unwrap();
        assert!(reached >= 4, "stepped through the run in {reached} ticks");
        assert_eq!(drawn[1], 1, "the first sprite is held for its own step");
    }

    #[test]
    fn a_single_texture_is_set_and_left_alone() {
        // Two hundred and fifty of the content's uses are this: one sprite, no run.
        let program = program(r#"enemy "X" { state a { set_alt_texture(2) } }"#);
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        for _ in 0..5 {
            mind.tick(&program, &alone(), 100, &mut out);
            assert_eq!(out.first(), Some(&Action::Texture { index: 2 }));
        }
    }

    #[test]
    fn an_orbit_settles_onto_its_radius_and_keeps_going_round() {
        // `Orbit.TickCore` steers at a point on the ring rather than along the tangent, so an enemy
        // that starts well inside or outside its radius closes on it and then holds it.
        let program = program(r#"enemy "X" { state a { orbit(1, 4, 20) } }"#);
        let mut mind = Mind::new(&program, 3);
        let mut out = Vec::new();

        // Standing on top of the thing it circles is the degenerate case the original jitters out
        // of; start it a little away and let it walk.
        let (mut x, mut y) = (11.0f32, 10.0f32);
        let mut swept = 0.0f32;
        let mut was = (y - 10.0).atan2(x - 10.0);

        for _ in 0..200 {
            let mut senses = with_player_at(10.0, 10.0);
            senses.x = x;
            senses.y = y;
            senses.nearest_player = Some(Nearby::at(
                10.0,
                10.0,
                ((x - 10.0).powi(2) + (y - 10.0).powi(2)).sqrt(),
            ));

            mind.tick(&program, &senses, 50, &mut out);
            let Some(Action::Move { angle, speed }) = out.first() else {
                panic!("expected a move, got {out:?}");
            };

            let step = tiles_per_second(*speed, false) * 0.05;
            x += angle.cos() * step;
            y += angle.sin() * step;

            let bearing = (y - 10.0).atan2(x - 10.0);
            let mut turned = bearing - was;
            while turned > std::f32::consts::PI {
                turned -= 2.0 * std::f32::consts::PI;
            }
            while turned < -std::f32::consts::PI {
                turned += 2.0 * std::f32::consts::PI;
            }
            swept += turned;
            was = bearing;
        }

        let radius = ((x - 10.0).powi(2) + (y - 10.0).powi(2)).sqrt();
        assert!(
            (radius - 4.0).abs() < 1.0,
            "should have settled near its radius of four, not {radius}"
        );

        // Anticlockwise, which is the original's default: `orbitClockwise` defaults to false, and
        // a direction of minus one takes the bearing backwards.
        assert!(
            swept < -2.0,
            "should have gone round the other way, sweeping {swept} radians"
        );
    }

    #[test]
    fn two_orbits_of_the_same_ring_do_not_move_as_one() {
        // `Orbit.OnStateEntry` draws a speed and a radius per entity, varying by a tenth of the
        // speed unless the script says otherwise. Without it a ring of minions is a rigid wheel.
        let program = program(r#"enemy "X" { state a { orbit(1, 4, 20) } }"#);
        let mut out = Vec::new();

        let speed_of = |seed: u32, out: &mut Vec<Action>| {
            let mut mind = Mind::new(&program, seed);
            let mut senses = with_player_at(10.0, 10.0);
            senses.x = 14.0;
            senses.y = 10.0;
            senses.nearest_player = Some(Nearby::at(10.0, 10.0, 4.0));
            mind.tick(&program, &senses, 50, out);
            match out.first() {
                Some(Action::Move { speed, .. }) => *speed,
                other => panic!("expected a move, got {other:?}"),
            }
        };

        let first = speed_of(1, &mut out);
        let second = speed_of(99, &mut out);

        assert_ne!(first, second, "two entities drew the same speed");
        for drawn in [first, second] {
            assert!(
                (drawn - 1.0).abs() <= 0.1001,
                "a tenth of the speed either way, not {drawn}"
            );
        }
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

        // An offset is a countdown like any other, so it too is rounded up to a whole logic tick
        // and then checked once more before it is spent: 200 ms is the original's third tick, at
        // 332 ms, and 400 ms is its fourth, at 498 ms.
        let mut fired = 0;
        for _ in 0..5 {
            out.clear();
            mind.tick(&program, &seen, 50, &mut out);
            fired += out.len();
        }
        assert_eq!(fired, 0, "neither offset one has come due yet");

        out.clear();
        mind.tick(&program, &seen, 50, &mut out);
        fired += out.len();
        assert_eq!(fired, 1, "the second at 332ms");

        for _ in 0..2 {
            out.clear();
            mind.tick(&program, &seen, 50, &mut out);
            fired += out.len();
        }
        assert_eq!(fired, 1, "and nothing in between");

        out.clear();
        mind.tick(&program, &seen, 50, &mut out);
        fired += out.len();
        assert_eq!(fired, 2, "and the third at 498ms, none of them again");
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
        assert_eq!(
            given, 40,
            "one order per tick, for as long as the state lasts"
        );
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

        // `Decay.cs:27` counts down in whole logic ticks and vanishes on the tick after the one
        // that empties the counter, so a decay written as a second lasts 1328 ms. Twenty-six of our
        // ticks is 1300 ms, which banks seven of the original's ticks and not yet eight.
        for _ in 0..26 {
            mind.tick(&program, &alone(), 50, &mut out);
            assert!(out.is_empty(), "still alive");
        }
        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(out, vec![Action::Vanish { dies: false }]);
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

        // A tick long enough to carry one of the original's, since even a cooldown written as zero
        // is one logic tick of rest there.
        mind.tick(&program, &alone(), 200, &mut out);
        assert_eq!(out.len(), 1, "nothing about, so it reproduces");

        // Two of its own kind already standing there is the limit.
        let crowd = vec![beside(900, 1.0)[0], beside(900, 2.0)[0]];
        mind.tick(&program, &seeing(&crowd), 200, &mut out);
        assert!(out.is_empty(), "should have held off: {out:?}");

        // A different kind does not count toward the limit.
        let strangers = vec![beside(901, 1.0)[0], beside(901, 2.0)[0]];
        mind.tick(&program, &seeing(&strangers), 200, &mut out);
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
    fn a_timed_transition_resumes_where_it_left_off() {
        // `Transition.Tick` keys the countdown on the transition object and `Entity.SwitchTo` never
        // clears the state storage (`logic/Transition.cs:24-35`, `realm/Entity.cs:236-244`), so a
        // state left half way through its timer picks up from there when it is entered again. The
        // second visit is shorter than the first, and for a boss cycling through phases that is the
        // difference between a fixed rotation and one that speeds up.
        let program = program(
            r#"enemy "X" {
                 state a { on timed(996ms) -> b }
                 state b { on hp_below(0.9) -> a }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let ticks_to_leave_a = |mind: &mut Mind, out: &mut Vec<Action>| {
            let mut ticks = 0;
            while mind.state_name(&program) == "a" && ticks < 40 {
                mind.tick(&program, &alone(), LOGIC_TICK_MS, out);
                ticks += 1;
            }
            // Out of `a`, and out of `b` again on the next hurt tick.
            let mut hurt = alone();
            hurt.hp = 50;
            mind.tick(&program, &hurt, LOGIC_TICK_MS, out);
            mind.tick(&program, &hurt, LOGIC_TICK_MS, out);
            ticks
        };

        let first = ticks_to_leave_a(&mut mind, &mut out);
        let second = ticks_to_leave_a(&mut mind, &mut out);

        assert_eq!(
            first, 7,
            "996 ms is six ticks of counting and one of firing"
        );
        assert!(
            second < first,
            "the second visit should resume the countdown, not restart it: {first} then {second}"
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

        // The draw is inside the range, but the countdown that spends it is not: it moves in whole
        // 166 ms ticks and the tick that finds it empty is one more again, so a draw of just under
        // two seconds is thirteen ticks plus one, or 2324 ms, before the switch.
        let longest = on_the_original_clock(2_000) + 50;
        assert!(
            waits.iter().all(|held| *held <= longest),
            "never longer than the range spent on the original's clock: {waits:?}"
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

        // A second on the original's clock is 1328 ms: seven ticks to empty the countdown and one
        // more to act on it. Twenty-six of our ticks banks seven of theirs.
        for _ in 0..26 {
            mind.tick(&program, &alone(), 50, &mut out);
            assert_eq!(mind.state_name(&program), "a");
        }
        mind.tick(&program, &alone(), 50, &mut out);
        assert_eq!(mind.state_name(&program), "b");
    }

    #[test]
    fn a_timed_group_runs_its_children_every_tick_and_not_on_its_period() {
        // `Timed.TickCore` (`logic/behaviors/Timed.cs:31-45`) ticks its children unconditionally;
        // the period only decides when it reports a finished cycle. Gating them on it made a shot
        // written to fire as fast as it could fire once every period instead.
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
        for _ in 0..40 {
            mind.tick(&program, &alone(), 100, &mut out);
            fired += out.len();
        }

        // Four seconds is twenty-four of the original's ticks, and a cooldown of nothing is still
        // one tick between shots. Half a second's period does not come into it.
        assert!(
            (24..=25).contains(&fired),
            "fired {fired} times in four seconds, which is a period rather than a tick"
        );
    }

    #[test]
    fn a_sequence_holds_each_timed_child_for_its_whole_period() {
        // What the period is actually for. `Sequence` steps on when its child reports `Completed`
        // (`logic/behaviors/Sequence.cs:30-38`), and a `timed` reports that once a period, so a
        // boss written to do one thing for a while and then another does exactly that.
        let program = program(
            r#"enemy "X" {
                 state a {
                   sequence() {
                     timed(996) { conditional_effect(invulnerable) }
                     timed(996) { conditional_effect(paralyzed) }
                   }
                 }
               }"#,
        );
        let mut mind = Mind::new(&program, 1);
        let mut out = Vec::new();

        let mut seen = Vec::new();
        for _ in 0..14 {
            mind.tick(&program, &alone(), LOGIC_TICK_MS, &mut out);
            match out.first() {
                Some(Action::Effect { effect, .. }) => seen.push(*effect),
                other => panic!("expected an effect every tick, got {other:?}"),
            }
        }

        // `Timed` acts and decrements on the same tick, so 996 ms is six ticks, not the seven a
        // transition of the same length would take.
        let first = seen[0];
        assert_eq!(
            seen.iter().take_while(|effect| **effect == first).count(),
            6,
            "the first child should hold for its whole period: {seen:?}"
        );
        assert!(
            seen[6..12].iter().all(|effect| *effect != first),
            "and then hand over to the second for as long: {seen:?}"
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
            Primitive::MoveTo { x, y, speed, .. } => (*x, *y, *speed),
            other => panic!("expected a move_to, got {other:?}"),
        };

        assert_eq!(of(&first), (20.0, 30.0, 1.0));
        assert_eq!(of(&second), of(&first));
    }

    #[test]
    fn a_move_to2_walks_an_offset_and_a_move_to_walks_to_a_point() {
        // Twenty-five of the twenty-nine `move_to2`s in the content are offsets of a few tiles.
        // Read as map coordinates they send the enemy to the corner of the world.
        let relative = program(r#"enemy "X" { state a { move_to2(-4, 0, 1) } }"#);
        let absolute =
            program(r#"enemy "X" { state a { move_to2(4, 4, 1, is_map_position: true) } }"#);

        let heading = |program: &Program| {
            let mut mind = Mind::new(program, 1);
            let mut out = Vec::new();
            mind.tick(program, &alone(), 50, &mut out);
            match out.first() {
                Some(Action::Move { angle, .. }) => *angle,
                other => panic!("expected a move, got {other:?}"),
            }
        };

        // Standing at (10, 10): four tiles west is due west, and the map point (4, 4) is
        // north-west of it.
        assert!(
            (heading(&relative).abs() - std::f32::consts::PI).abs() < 1e-4,
            "an offset of minus four should walk west"
        );
        assert!(
            (heading(&absolute) - (-6.0f32).atan2(-6.0)).abs() < 1e-4,
            "a map position should walk at the point itself"
        );
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

/// The state the original keeps on a behaviour object rather than on the enemy running it.
///
/// Paralysis crippling a whole species for the rest of the process, and the volley count that turns
/// a sweeping spread. Every program here has a name of its own, because what is under test is a
/// table keyed by program name that nothing ever clears — two tests sharing a name would be two
/// tests sharing one latch and one sweep.
#[cfg(test)]
mod shared_behaviour_state {
    use super::*;
    use crate::compile::compile;
    use crate::parse::parse;

    fn program(source: &str) -> Program {
        let parsed = parse(source).expect("should parse");
        let (programs, diagnostics) = compile(&parsed);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        programs.programs.into_iter().next().expect("one program")
    }

    fn senses(paralyzed: bool) -> Senses<'static> {
        Senses {
            x: 10.0,
            y: 10.0,
            hp: 100,
            max_hp: 100,
            spawn_x: 10.0,
            spawn_y: 10.0,
            nearest_player: Some(Nearby::at(20.0, 10.0, 10.0)),
            nearest_player_hiding: Some(Nearby::at(20.0, 10.0, 10.0)),
            nearby: &[],
            said: &[],
            damage_taken: 0,
            stunned: false,
            dazed: false,
            paralyzed,
        }
    }

    /// The speed the first movement of one tick asks for.
    fn moved_at(mind: &mut Mind, program: &Program, paralyzed: bool) -> Option<f32> {
        let mut out = Vec::new();
        mind.tick(program, &senses(paralyzed), 50, &mut out);
        out.iter().find_map(|action| match action {
            Action::Move { speed, .. } => Some(*speed),
            _ => None,
        })
    }

    #[test]
    fn one_paralysed_enemy_cripples_every_enemy_of_its_type_for_good() {
        // `Follow.cs:51` writes `speed = 0` into a field of the behaviour object, and
        // `BehaviorDb.cs:68-88` gives every enemy of a type the same behaviour objects. One tick of
        // paralysis on one of them is therefore the whole species, permanently.
        let program = program(r#"enemy "Paralysis Victim" { state a { follow(0.75, 20, 1) } }"#);

        let mut victim = Mind::new(&program, 1);
        let mut bystander = Mind::new(&program, 2);

        assert_eq!(
            moved_at(&mut bystander, &program, false),
            Some(0.75),
            "the bystander should start at the speed the content asked for"
        );

        // One tick, and only for the one that was hit.
        assert_eq!(moved_at(&mut victim, &program, true), Some(0.0));

        // The effect is over for everybody, and the bystander was never touched by it.
        assert_eq!(
            moved_at(&mut victim, &program, false),
            Some(0.0),
            "the victim recovered a speed the original never restores"
        );
        assert_eq!(
            moved_at(&mut bystander, &program, false),
            Some(0.0),
            "an enemy that was never paralysed kept its speed"
        );

        // And nought is not still: the constant term of `Utils.GetSpeed` survives it.
        assert_eq!(tiles_per_second(0.0, false), 0.74);
        assert!((tiles_per_second(0.75, false) - 4.9025).abs() < 1e-4);
    }

    #[test]
    fn a_crippled_behaviour_follows_the_program_into_another_copy_of_it() {
        // Each world holds its own clone of the compiled programs, where the original holds one
        // `State` tree shared by every world in the process (`realm/Entity.cs:236-245`). The latch
        // has to survive the copy or the crippling would stop at the edge of a world.
        let source = r#"enemy "Paralysis Traveller" { state a { wander(1.0) } }"#;
        let here = program(source);
        let elsewhere = program(source);

        let mut mind = Mind::new(&here, 1);
        assert_eq!(moved_at(&mut mind, &here, true), Some(0.0));

        let mut abroad = Mind::new(&elsewhere, 9);
        assert_eq!(
            moved_at(&mut abroad, &elsewhere, false),
            Some(0.0),
            "a separately compiled copy of the same enemy escaped the crippling"
        );
    }

    #[test]
    fn only_the_behaviour_that_was_ticked_is_crippled() {
        // The write happens in `TickCore`, so a behaviour in a state the enemy was not in when it
        // was paralysed keeps its speed until it is ticked while paralysed itself.
        let program = program(
            r#"enemy "Paralysis Neighbour" {
                state a { follow(1.0, 20, 1) }
                state b { follow(2.0, 20, 1) }
            }"#,
        );

        let mut mind = Mind::new(&program, 1);
        assert_eq!(moved_at(&mut mind, &program, true), Some(0.0));

        // A fresh enemy of the same type, driven into the state that was never ticked while
        // paralysed.
        let mut other = Mind::new(&program, 2);
        let target = program.state_named("b").expect("state b");
        other.force_into(&program, target);
        assert_eq!(
            moved_at(&mut other, &program, false),
            Some(2.0),
            "a behaviour that was never ticked while paralysed lost its speed"
        );
    }

    #[test]
    fn every_enemy_of_a_type_shares_one_sweep() {
        // `_rotateCount` is a field of the `Shoot` object (`Shoot.cs:147`), and the object belongs
        // to the type rather than to the enemy. Two of a kind firing on the same tick therefore take
        // consecutive numbers instead of the same one: the pair fan out a step apart and the pattern
        // advances twice per round.
        //
        // The Hermit God fight is where a player meets this. `reproduce("Whirlpool", 3, 1)` puts up
        // to three whirlpools in the water at once and each runs
        // `shoot(0, 1, fixed_angle: 0, rotate_angle: 30, cooldown: 400)`, so the three sweep as a
        // fan thirty degrees apart at ninety degrees a volley — not as one beam at thirty.
        let program = program(
            r#"enemy "Paralysis Whirlpool" {
                state a { shoot(count: 1, fixed_angle: 0, rotate_angle: 30, cooldown: 100ms) }
            }"#,
        );

        let mut first = Mind::new(&program, 1);
        let mut second = Mind::new(&program, 2);
        let mut third = Mind::new(&program, 3);

        let volley = |mind: &mut Mind, program: &Program| {
            let mut out = Vec::new();
            for _ in 0..8 {
                mind.tick(program, &senses(false), 100, &mut out);
                if let Some(Action::Shoot { angle, .. }) = out.first() {
                    return Some(angle.to_degrees());
                }
            }
            None
        };

        let angles = [
            volley(&mut first, &program),
            volley(&mut second, &program),
            volley(&mut third, &program),
        ];

        for (fired, angle) in angles.iter().enumerate() {
            let wanted = 30.0 * fired as f32;
            let got = angle.expect("each should have fired once");
            assert!(
                (got - wanted).abs() < 1e-3,
                "the {fired}th of three whirlpools fired at {got} rather than {wanted}"
            );
        }
    }

    #[test]
    fn a_sweep_survives_the_state_it_was_fired_from_and_the_copy_it_was_compiled_into() {
        // Nothing resets `_rotateCount`: `Shoot.OnStateEntry` writes only the cooldown into the
        // per-entity state (`Shoot.cs:57-59`) and the class has no `OnStateExit`. A boss leaving an
        // attack phase and coming back to it carries on sweeping from where it stopped, and so does
        // the next copy of that boss the server spawns, in whatever world it spawns in.
        let source = r#"enemy "Paralysis Lighthouse" {
            state a { shoot(count: 1, fixed_angle: 0, rotate_angle: 30, cooldown: 100ms) }
        }"#;
        let here = program(source);
        let elsewhere = program(source);

        let volley = |mind: &mut Mind, program: &Program| {
            let mut out = Vec::new();
            for _ in 0..8 {
                mind.tick(program, &senses(false), 100, &mut out);
                if let Some(Action::Shoot { angle, .. }) = out.first() {
                    return angle.to_degrees();
                }
            }
            panic!("expected a volley");
        };

        let mut mind = Mind::new(&here, 1);
        assert!((volley(&mut mind, &here) - 0.0).abs() < 1e-3);

        // Out of the state and back into it, which arms the cooldown afresh but not the sweep.
        let target = here.state_named("a").expect("state a");
        mind.force_into(&here, target);
        assert!(
            (volley(&mut mind, &here) - 30.0).abs() < 1e-3,
            "re-entering the state restarted the sweep"
        );

        // And a separately compiled copy of the same enemy, as another world holds.
        let mut abroad = Mind::new(&elsewhere, 9);
        assert!(
            (volley(&mut abroad, &elsewhere) - 60.0).abs() < 1e-3,
            "a copy of the program in another world had its own sweep"
        );
    }

    #[test]
    fn the_four_behaviours_whose_write_is_dead_are_untouched() {
        // `Charge`, `MoveLine`, `MoveTo` and `ReturnToSpawn` each declare both `speed` and
        // `_speed`, zero the first and move on the second, so their paralysis clause does nothing
        // at all.
        let program =
            program(r#"enemy "Paralysis Immune" { state a { move_line(1.5, direction: 0) } }"#);

        let mut mind = Mind::new(&program, 1);
        assert_eq!(moved_at(&mut mind, &program, true), Some(1.5));
        assert_eq!(
            moved_at(&mut mind, &program, false),
            Some(1.5),
            "a dead write in the original became a live one here"
        );
    }

    #[test]
    fn an_orbit_keeps_circling_until_the_next_time_it_enters_its_state() {
        // `Orbit.TickCore` zeroes the field but circles on the copy `OnStateEntry` took
        // (`Orbit.cs:50` against `:77`), so the crippling waits for the next entry. The drawn speed
        // is then nought plus the variance, which the constructor worked out from the speed it used
        // to have and which nothing zeroes.
        let program = program(
            r#"enemy "Paralysis Circler" {
                state a { orbit(1.0, 3.0, 20, speed_variance: 0.0) }
            }"#,
        );

        let mut mind = Mind::new(&program, 1);
        assert_eq!(
            moved_at(&mut mind, &program, true),
            Some(1.0),
            "the tick that cripples an orbit should still circle at full speed"
        );
        assert_eq!(
            moved_at(&mut mind, &program, false),
            Some(1.0),
            "and so should every tick until the state is entered again"
        );

        // Entering the state again redraws, and the draw now starts from nought.
        let target = program.state_named("a").expect("state a");
        mind.force_into(&program, target);
        assert_eq!(
            moved_at(&mut mind, &program, false),
            Some(0.0),
            "the orbit redrew its speed and did not pick up the crippling"
        );

        // A fresh enemy of the same type draws from nought on its very first entry.
        let mut hatchling = Mind::new(&program, 4);
        assert_eq!(moved_at(&mut hatchling, &program, false), Some(0.0));
    }
}
