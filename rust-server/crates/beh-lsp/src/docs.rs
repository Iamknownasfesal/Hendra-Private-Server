//! What every behaviour, transition and loot entry means, and what it does with its arguments.
//!
//! The compiler in `hendra-behavior` reads arguments by name, falling back to a position taken from
//! the C# constructor the content was converted from. That table lives in a `match` and is invisible
//! from a content file, so it is restated here in a form an editor can show: the name the compiler
//! reads, where it reads it from, what happens when it is missing, and what the number means.
//!
//! `ignored` is the other half, and the more useful one. The content is full of arguments written
//! with a name the compiler never looks for — `timed(time: 50)` when the compiler reads `after` —
//! and those are silently replaced by a default. Listing them is what lets an editor say so.

/// What a name is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Something an enemy does.
    Behaviour,
    /// A behaviour holding other behaviours in a block.
    Group,
    /// A condition on `on … ->`.
    Condition,
    /// An entry in a `loot` table.
    Loot,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Behaviour => "behaviour",
            Kind::Group => "group behaviour",
            Kind::Condition => "transition",
            Kind::Loot => "loot entry",
        }
    }
}

/// One argument the compiler reads.
pub struct Param {
    pub name: &'static str,
    /// Where it is read from when written without a name. `None` when only the name works.
    pub position: Option<usize>,
    /// What kind of value it wants, in the content's own vocabulary.
    pub value: &'static str,
    /// What is used when it is not written at all.
    pub default: &'static str,
    pub doc: &'static str,
}

/// An argument the content writes that the compiler does not read.
pub struct Ignored {
    pub name: &'static str,
    /// Why writing it has no effect.
    pub why: &'static str,
}

pub struct Entry {
    pub name: &'static str,
    /// Other spellings that compile to the same thing.
    pub aliases: &'static [&'static str],
    pub kind: Kind,
    pub summary: &'static str,
    pub params: &'static [Param],
    pub ignored: &'static [Ignored],
    /// Said when the runtime does not do this at all.
    pub note: Option<&'static str>,
}

impl Entry {
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|param| param.name == name)
    }

    pub fn ignored(&self, name: &str) -> Option<&Ignored> {
        self.ignored.iter().find(|entry| entry.name == name)
    }

    /// `shoot(radius, count, …)`, for the first line of a tooltip.
    pub fn signature(&self) -> String {
        let arguments: Vec<&str> = self.params.iter().map(|param| param.name).collect();
        format!("{}({})", self.name, arguments.join(", "))
    }
}

/// Looks a name up as a behaviour, a group, or a loot entry.
pub fn behaviour(name: &str) -> Option<&'static Entry> {
    find(BEHAVIOURS, name)
}

pub fn condition(name: &str) -> Option<&'static Entry> {
    find(CONDITIONS, name)
}

pub fn loot(name: &str) -> Option<&'static Entry> {
    find(LOOT, name)
}

/// The entry for a name in whichever table fits where it was written.
pub fn lookup(name: &str, kind: Kind) -> Option<&'static Entry> {
    match kind {
        Kind::Condition => condition(name),
        Kind::Loot => loot(name),
        _ => behaviour(name),
    }
}

fn find(table: &'static [Entry], name: &str) -> Option<&'static Entry> {
    table
        .iter()
        .find(|entry| entry.name == name || entry.aliases.contains(&name))
}

/// The closest name in a table, for a misspelling.
pub fn nearest(table: &'static [Entry], name: &str) -> Option<&'static str> {
    table
        .iter()
        .flat_map(|entry| std::iter::once(entry.name).chain(entry.aliases.iter().copied()))
        .map(|candidate| (candidate, distance(name, candidate)))
        .filter(|(_, distance)| *distance <= 3)
        .min_by_key(|(_, distance)| *distance)
        .map(|(candidate, _)| candidate)
}

/// The closest argument name of an entry, for a misspelling.
pub fn nearest_param(entry: &'static Entry, name: &str) -> Option<&'static str> {
    entry
        .params
        .iter()
        .map(|param| (param.name, distance(name, param.name)))
        .filter(|(_, distance)| *distance <= 3)
        .min_by_key(|(_, distance)| *distance)
        .map(|(candidate, _)| candidate)
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

/// The condition effects, by the names the content writes and the index each one has.
pub const EFFECTS: &[(&str, u8)] = &[
    ("dead", 0),
    ("quiet", 1),
    ("weak", 2),
    ("slowed", 3),
    ("sick", 4),
    ("dazed", 5),
    ("stunned", 6),
    ("blind", 7),
    ("hallucinating", 8),
    ("drunk", 9),
    ("confused", 10),
    ("stunimmune", 11),
    ("invisible", 12),
    ("paralyzed", 13),
    ("speedy", 14),
    ("bleeding", 15),
    ("armorbreakimmune", 16),
    ("healing", 17),
    ("damaging", 18),
    ("berserk", 19),
    ("paused", 20),
    ("stasis", 21),
    ("stasisimmune", 22),
    ("invincible", 23),
    ("invulnerable", 24),
    ("armored", 25),
    ("armorbroken", 26),
    ("hexed", 27),
    ("ninjaspeedy", 28),
    ("unstable", 29),
    ("darkness", 30),
    ("slowedimmune", 31),
    ("dazedimmune", 32),
    ("paralyzeimmune", 33),
    ("petrified", 34),
    ("petrifyimmune", 35),
    ("petdisable", 36),
    ("cursed", 37),
    ("curseimmune", 38),
    ("hpboost", 39),
    ("nothing", 0),
];

pub fn effect(name: &str) -> Option<u8> {
    let wanted = name.to_ascii_lowercase();
    EFFECTS
        .iter()
        .find(|(known, _)| *known == wanted)
        .map(|(_, index)| *index)
}

const SPEED: &str = "tiles a second, near enough";

// -- behaviours ---------------------------------------------------------------------------------

pub const BEHAVIOURS: &[Entry] = &[
    Entry {
        name: "shoot",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Fires at the nearest player it can see. The commonest behaviour in the game by a \
                  factor of ten: everything that fights does it.",
        params: &[
            Param { name: "radius", position: Some(0), value: "tiles", default: "20", doc: "How far away a player can be and still be shot at." },
            Param { name: "count", position: Some(1), value: "number", default: "1", doc: "Projectiles per shot. More than one spreads them over `shoot_angle`." },
            Param { name: "shoot_angle", position: Some(2), value: "degrees", default: "0", doc: "The gap between projectiles in a burst." },
            Param { name: "projectile", position: Some(3), value: "number", default: "0", doc: "Which of the enemy's projectiles to fire, by index in its object data." },
            Param { name: "fixed_angle", position: None, value: "degrees", default: "aims at the target", doc: "Fires this direction regardless of where anyone is standing." },
            Param { name: "angle_offset", position: Some(6), value: "degrees", default: "0", doc: "Turns the whole burst by this much." },
            Param { name: "default_angle", position: None, value: "degrees", default: "aims at the target", doc: "The direction to fire in when there is nobody to aim at." },
            Param { name: "predictive", position: None, value: "0 to 1", default: "0", doc: "How much to lead a moving target. 1 aims where they will be." },
            Param { name: "cooldown_offset", position: Some(9), value: "ms", default: "0", doc: "Waits this long before the first shot, so several shooters do not fire in step." },
            Param { name: "cooldown", position: Some(10), value: "ms", default: "1000", doc: "The wait between shots." },
        ],
        ignored: &[
            Ignored { name: "rotate_angle", why: "the runtime does not turn a burst between shots" },
        ],
        note: None,
    },
    Entry {
        name: "wander",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Drifts in a direction, changing its mind now and then. What an enemy does when \
                  nobody is nearby.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "0.4", doc: "How fast it drifts." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "buzz",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Darts a short distance in a random direction, waits, and darts again.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "2", doc: "How fast each dart is." },
            Param { name: "dist", position: Some(1), value: "tiles", default: "0.5", doc: "How far each dart goes." },
            Param { name: "cooldown", position: Some(2), value: "ms", default: "0", doc: "The wait between darts." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "follow",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Walks towards the nearest player and stops at `range`. Pair it with `shoot` and \
                  the enemy chases you down.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "1", doc: "How fast it walks." },
            Param { name: "acquire_range", position: Some(1), value: "tiles", default: "10", doc: "How far away it notices a player." },
            Param { name: "range", position: Some(2), value: "tiles", default: "6", doc: "How close it gets before stopping." },
            Param { name: "duration", position: Some(3), value: "ms", default: "as long as the state lasts", doc: "How long one chase lasts before it rests." },
            Param { name: "cooldown", position: Some(4), value: "ms", default: "0", doc: "The rest between chases." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "orbit",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Circles a player, or a named entity, at a fixed distance.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "1", doc: "How fast it goes round." },
            Param { name: "radius", position: Some(1), value: "tiles", default: "4", doc: "How far out it circles." },
            Param { name: "acquire_range", position: Some(2), value: "tiles", default: "10", doc: "How far away it looks for what to circle." },
            Param { name: "target", position: Some(3), value: "an object name", default: "the nearest player", doc: "What to circle. Guards circle their boss this way." },
        ],
        ignored: &[
            Ignored { name: "speed_variance", why: "the runtime gives every orbiter the same speed" },
            Ignored { name: "radius_variance", why: "the runtime gives every orbiter the same radius" },
            Ignored { name: "orbit_clockwise", why: "the runtime picks the direction itself" },
        ],
        note: None,
    },
    Entry {
        name: "stay_back",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Backs away from the nearest player to keep a distance. A kiter.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "1", doc: "How fast it retreats." },
            Param { name: "distance", position: Some(1), value: "tiles", default: "8", doc: "The distance it tries to hold." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "stay_close_to_spawn",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Walks back towards where it was created when it strays too far. Usually written \
                  above a `wander` inside a `prioritize`, which is what keeps a room's enemies in \
                  their room.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "1", doc: "How fast it returns." },
            Param { name: "range", position: Some(1), value: "tiles", default: "5", doc: "How far it may stray before returning." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "stay_above",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Moves off ground lower than a height, which is how flying enemies stay off the \
                  floor of a room.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "1", doc: "How fast it moves away." },
            Param { name: "altitude", position: Some(1), value: "number", default: "5", doc: "The ground height it will not go below." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "heal_self",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Heals itself on a timer.",
        params: &[
            Param { name: "amount", position: Some(0), value: "hit points", default: "100", doc: "How much each heal restores." },
            Param { name: "cooldown", position: Some(1), value: "ms", default: "1000", doc: "The wait between heals." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "heal_group",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Heals nearby members of a group. The argument order is not the same as \
                  `heal_entity`: the cooldown comes before the amount.",
        params: &[
            Param { name: "range", position: Some(0), value: "tiles", default: "10", doc: "How far the healing reaches." },
            Param { name: "group", position: Some(1), value: "a group name", default: "anything nearby", doc: "Which group is healed." },
            Param { name: "cooldown", position: Some(2), value: "ms", default: "1000", doc: "The wait between heals." },
            Param { name: "heal_amount", position: Some(3), value: "hit points", default: "100", doc: "How much each heal restores." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "heal_entity",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Heals a named entity nearby.",
        params: &[
            Param { name: "range", position: Some(0), value: "tiles", default: "10", doc: "How far the healing reaches." },
            Param { name: "name", position: Some(1), value: "an object name", default: "anything nearby", doc: "What is healed." },
            Param { name: "heal_amount", position: Some(2), value: "hit points", default: "100", doc: "How much each heal restores." },
            Param { name: "cooldown", position: Some(3), value: "ms", default: "1000", doc: "The wait between heals." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "heal_player",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Heals nearby players.",
        params: &[
            Param { name: "range", position: Some(0), value: "tiles", default: "10", doc: "How far the healing reaches." },
            Param { name: "cooldown", position: Some(1), value: "ms", default: "1000", doc: "The wait between heals." },
            Param { name: "heal_amount", position: Some(2), value: "hit points", default: "100", doc: "How much each heal restores." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "spawn",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Creates children up to a limit and keeps replacing the ones that die. The count \
                  is of children this enemy has made, not of what is standing nearby.",
        params: &[
            Param { name: "children", position: Some(0), value: "an object name", default: "nothing", doc: "What it creates." },
            Param { name: "max_children", position: Some(1), value: "number", default: "5", doc: "How many it will have alive at once." },
            Param { name: "initial_spawn", position: Some(2), value: "0 to 1", default: "0.5", doc: "The share of `max_children` created at once on entering the state." },
            Param { name: "cooldown", position: Some(3), value: "ms", default: "1000", doc: "The wait between replacements." },
            Param { name: "gives_no_xp", position: None, value: "true or false", default: "true", doc: "Whether killing the children is worth nothing, which is what stops a spawner being an experience farm." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "spawn_group",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "The same as `spawn`, but what it creates is a named group rather than one object.",
        params: &[
            Param { name: "group", position: Some(0), value: "a group name", default: "nothing", doc: "The group to draw children from." },
            Param { name: "max_children", position: Some(1), value: "number", default: "5", doc: "How many it will have alive at once." },
            Param { name: "initial_spawn", position: Some(2), value: "0 to 1", default: "0.5", doc: "The share created at once on entering the state." },
            Param { name: "cooldown", position: Some(3), value: "ms", default: "1000", doc: "The wait between replacements." },
            Param { name: "gives_no_xp", position: None, value: "true or false", default: "true", doc: "Whether the children are worth nothing." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "reproduce",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Creates more of something while there are fewer than a limit standing nearby. \
                  Unlike `spawn` it counts what is around it, so a group of these will not \
                  overpopulate a room together.",
        params: &[
            Param { name: "children", position: Some(0), value: "an object name", default: "nothing", doc: "What it creates." },
            Param { name: "density_radius", position: Some(1), value: "tiles", default: "10", doc: "How far out it counts its own kind." },
            Param { name: "density_max", position: Some(2), value: "number", default: "5", doc: "How many may stand within that radius before it stops." },
            Param { name: "cooldown", position: Some(3), value: "ms", default: "1000", doc: "The wait between births." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "reproduce_children",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "`reproduce` with the arguments the other way round: the numbers first and the \
                  name last. Counts within 15 tiles, which the C# form had no way to say.",
        params: &[
            Param { name: "max_children", position: Some(0), value: "number", default: "5", doc: "How many may stand nearby before it stops." },
            Param { name: "initial_spawn", position: Some(1), value: "0 to 1", default: "unused", doc: "Read by the C# and ignored here." },
            Param { name: "cooldown", position: Some(2), value: "ms", default: "1000", doc: "The wait between births." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "toss_object",
        aliases: &["invisi_toss"],
        kind: Kind::Behaviour,
        summary: "Throws something to a spot on the ground, which lands after a warning of 1500ms. \
                  `invisi_toss` is the same throw without the thrower being seen doing it.",
        params: &[
            Param { name: "child", position: Some(0), value: "an object name", default: "nothing", doc: "What is thrown." },
            Param { name: "range", position: Some(1), value: "tiles", default: "5", doc: "How far it throws." },
            Param { name: "angle", position: None, value: "degrees", default: "throws at a player", doc: "Throws this direction regardless of where anyone is." },
            Param { name: "cooldown", position: Some(3), value: "ms", default: "1000", doc: "The wait between throws." },
        ],
        ignored: &[
            Ignored { name: "min_range", why: "the runtime throws at one distance" },
            Ignored { name: "max_range", why: "the runtime throws at one distance; write `range`" },
            Ignored { name: "min_angle", why: "the runtime throws at one angle" },
            Ignored { name: "max_angle", why: "the runtime throws at one angle; write `angle`" },
            Ignored { name: "toss_invis", why: "write `invisi_toss` instead, which is the same behaviour" },
            Ignored { name: "cooldown_offset", why: "the runtime does not stagger the first throw" },
        ],
        note: None,
    },
    Entry {
        name: "grenade",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Lobs a blast that damages everyone within a radius where it lands.",
        params: &[
            Param { name: "radius", position: Some(0), value: "tiles", default: "2", doc: "How wide the blast is." },
            Param { name: "damage", position: Some(1), value: "hit points", default: "100", doc: "What it does to everyone caught." },
            Param { name: "range", position: Some(2), value: "tiles", default: "5", doc: "How far it throws." },
            Param { name: "fixed_angle", position: None, value: "degrees", default: "throws at a player", doc: "Throws this direction regardless of where anyone is." },
            Param { name: "cooldown", position: Some(4), value: "ms", default: "1000", doc: "The wait between throws." },
            Param { name: "effect", position: None, value: "an effect name", default: "none", doc: "A condition effect applied to everyone caught." },
            Param { name: "effect_duration", position: None, value: "seconds or ms", default: "0", doc: "How long that effect lasts." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "suicide",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Removes itself at once, without dropping loot or counting as a kill. How a \
                  minion cleans itself up at the end of a phase.",
        params: &[],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "remove_entity",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Removes *other* entities nearby, not itself. What a boss uses to tidy up its \
                  summons between phases.",
        params: &[
            Param { name: "dist", position: Some(0), value: "tiles", default: "10", doc: "How far out it reaches." },
            Param { name: "children", position: Some(1), value: "an object name", default: "anything nearby", doc: "What is removed." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "decay",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Removes itself after a wait. A written zero means the argument was left out, \
                  which is ten seconds.",
        params: &[
            Param { name: "time", position: Some(0), value: "ms", default: "10000", doc: "How long it lasts." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "conditional_effect",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Applies a condition effect for as long as the state lasts, or for a time. \
                  `conditional_effect(invulnerable)` is how a boss makes itself untouchable during \
                  a phase.",
        params: &[
            Param { name: "effect", position: Some(0), value: "an effect name or number", default: "nothing", doc: "Which effect to apply." },
            Param { name: "duration", position: Some(1), value: "seconds or ms", default: "as long as the state lasts", doc: "How long it lasts. A value under 60 is read as seconds." },
            Param { name: "perm", position: None, value: "anything", default: "not written", doc: "Written on its own to mean the effect lasts as long as the state does." },
            Param { name: "range", position: Some(2), value: "tiles", default: "0", doc: "How far the effect reaches when it is not on itself." },
            Param { name: "target", position: Some(3), value: "players, enemies or itself", default: "itself", doc: "Who is affected." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "remove_conditional_effect",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Takes a condition effect off again.",
        params: &[
            Param { name: "effect", position: Some(0), value: "an effect name or number", default: "nothing", doc: "Which effect to remove." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "set_alt_texture",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Switches the sprite the client draws. How a boss changes appearance between \
                  phases without becoming a different enemy.",
        params: &[
            Param { name: "index", position: Some(0), value: "number", default: "0", doc: "Which alternate texture from the object's data, where 0 is its own." },
        ],
        ignored: &[
            Ignored { name: "min_value", why: "the runtime sets one texture rather than cycling through a range" },
            Ignored { name: "max_value", why: "the runtime sets one texture rather than cycling through a range" },
            Ignored { name: "loop", why: "the runtime does not animate the texture" },
            Ignored { name: "cooldown", why: "the runtime does not animate the texture" },
        ],
        note: None,
    },
    Entry {
        name: "flash",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Flashes a colour a number of times. The telegraph before something dangerous: \
                  every enemy that explodes flashes first.",
        params: &[
            Param { name: "color", position: Some(0), value: "0xRRGGBB as a number", default: "0", doc: "The colour to flash." },
            Param { name: "flash_period", position: Some(1), value: "seconds", default: "0.5", doc: "How long one flash takes. Seconds here, unlike almost everything else." },
            Param { name: "flash_repeats", position: Some(2), value: "number", default: "1", doc: "How many times it flashes." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "scale_h_p",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Raises maximum health for every player nearby, so a boss written for a crowd is \
                  not trivial when two people find it. A distance of zero means the room.",
        params: &[
            Param { name: "amount_per_player", position: Some(0), value: "hit points", default: "0", doc: "Health added for each player counted." },
            Param { name: "max_additional", position: Some(1), value: "hit points", default: "0", doc: "The most that can be added altogether." },
            Param { name: "heal_after_max", position: Some(2), value: "true or false", default: "unused", doc: "Read by the C# and ignored here." },
            Param { name: "dist", position: Some(3), value: "tiles", default: "20", doc: "How far out players are counted." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "change_size",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Grows or shrinks towards a size, where 100 is normal.",
        params: &[
            Param { name: "rate", position: Some(0), value: "size a second", default: "0", doc: "How fast it changes. Negative shrinks." },
            Param { name: "target", position: Some(1), value: "number", default: "100", doc: "The size it stops at." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "taunt",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Says something. Every text argument is a line it might say; `{PLAYER}` in a line \
                  is replaced with whoever it is talking to.",
        params: &[
            Param { name: "probability", position: None, value: "0 to 1, or a percentage", default: "1", doc: "How likely it is to say anything when the cooldown is up." },
            Param { name: "cooldown", position: None, value: "ms", default: "5000", doc: "The wait between lines." },
            Param { name: "broadcast", position: None, value: "1 or 0", default: "0", doc: "Whether the whole world hears it rather than the room." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "order",
        aliases: &["order_once"],
        kind: Kind::Behaviour,
        summary: "Puts nearby entities into a named state. How a boss starts a phase in its \
                  minions. `order_once` does it a single time rather than repeatedly.",
        params: &[
            Param { name: "range", position: Some(0), value: "tiles", default: "10", doc: "How far the order carries." },
            Param { name: "children", position: Some(1), value: "an object name", default: "anything nearby", doc: "Who is ordered." },
            Param { name: "target_state", position: Some(2), value: "a state name", default: "nothing", doc: "The state they are put into. It is a state of theirs, not of this enemy." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "transform",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Becomes a different object, keeping its position. The second half of a boss \
                  fight is often a different enemy entirely.",
        params: &[
            Param { name: "target", position: Some(0), value: "an object name", default: "nothing", doc: "What it becomes." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "protect",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Stays between a named entity and everyone else, returning to it when pushed away.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "1", doc: "How fast it moves." },
            Param { name: "protectee", position: Some(1), value: "an object name", default: "nothing", doc: "What it guards." },
            Param { name: "acquire_range", position: Some(2), value: "tiles", default: "10", doc: "How far away it will look for what it guards." },
            Param { name: "protection_range", position: Some(3), value: "tiles", default: "4", doc: "How close it stays." },
            Param { name: "reprotect_range", position: Some(4), value: "tiles", default: "2", doc: "How far it may drift before returning." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "move_to",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Walks to a spot relative to where it was created, once.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "1", doc: "How fast it walks." },
            Param { name: "x", position: Some(1), value: "tiles", default: "0", doc: "How far east of its spawn." },
            Param { name: "y", position: Some(2), value: "tiles", default: "0", doc: "How far south of its spawn." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "move_to2",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "`move_to` with the arguments the other way round: the position first, the speed \
                  last.",
        params: &[
            Param { name: "x", position: Some(0), value: "tiles", default: "0", doc: "How far east of its spawn." },
            Param { name: "y", position: Some(1), value: "tiles", default: "0", doc: "How far south of its spawn." },
            Param { name: "speed", position: Some(2), value: SPEED, default: "2", doc: "How fast it walks." },
        ],
        ignored: &[
            Ignored { name: "once", why: "the runtime always walks there once" },
            Ignored { name: "is_map_position", why: "the runtime reads the position as relative to the spawn" },
        ],
        note: None,
    },
    Entry {
        name: "move_line",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Walks in a straight line in a fixed direction, whatever is in the way.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "1", doc: "How fast it walks." },
            Param { name: "direction", position: Some(1), value: "degrees", default: "0", doc: "Which way it goes." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "back_and_forth",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Paces a fixed distance one way, then the other. A patrol.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "1", doc: "How fast it paces." },
            Param { name: "distance", position: Some(1), value: "tiles", default: "5", doc: "How far each leg is." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "charge",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Rushes at where a player is standing, then rests.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "1", doc: "How fast the rush is." },
            Param { name: "range", position: Some(1), value: "tiles", default: "10", doc: "How far away it will start one." },
            Param { name: "cooldown", position: Some(2), value: "ms", default: "1000", doc: "The rest between rushes." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "swirl",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Spirals around a point, either where it stands or where a player is.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "1", doc: "How fast it spirals." },
            Param { name: "radius", position: Some(1), value: "tiles", default: "5", doc: "How wide the spiral is." },
            Param { name: "targeted", position: Some(2), value: "1 or 0", default: "0", doc: "Whether it spirals around a player rather than where it started." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "return_to_spawn",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Walks back to where it was created and stops there.",
        params: &[
            Param { name: "speed", position: Some(0), value: SPEED, default: "1", doc: "How fast it returns." },
            Param { name: "return_within_radius", position: Some(1), value: "tiles", default: "0.5", doc: "How close counts as arrived." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "set_no_x_p",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Makes killing this worth no experience, for a summon that would otherwise be farmed.",
        params: &[],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "remove_tile_object",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Removes a named thing standing on the ground nearby, such as a wall or a gate.",
        params: &[
            Param { name: "target", position: Some(0), value: "an object name", default: "anything nearby", doc: "What is removed." },
            Param { name: "radius", position: Some(1), value: "tiles", default: "1", doc: "How far out it reaches." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "apply_setpiece",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Draws one of the structures a realm is built from, where the enemy is standing.",
        params: &[
            Param { name: "name", position: Some(0), value: "a setpiece name", default: "nothing", doc: "Which structure to draw." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "ground_transform",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Changes the ground around it to another tile.",
        params: &[
            Param { name: "tile", position: Some(0), value: "a tile name", default: "nothing", doc: "What the ground becomes." },
            Param { name: "radius", position: Some(1), value: "tiles", default: "1", doc: "How much ground changes." },
            Param { name: "cooldown", position: Some(2), value: "ms", default: "0", doc: "The wait between changes." },
        ],
        ignored: &[
            Ignored { name: "relative_x", why: "the runtime changes the ground around the enemy" },
            Ignored { name: "relative_y", why: "the runtime changes the ground around the enemy" },
            Ignored { name: "persist", why: "the runtime always leaves the changed ground behind" },
        ],
        note: None,
    },
    Entry {
        name: "replace_tile",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Changes the ground, naming what is there and what it becomes. It is the *second* \
                  name that the ground turns into.",
        params: &[
            Param { name: "range", position: Some(0), value: "tiles", default: "1", doc: "How much ground changes." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "transform_on_death",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Leaves something else behind when it dies. The first half of a two-part boss \
                  ends here.",
        params: &[
            Param { name: "target", position: Some(0), value: "an object name", default: "nothing", doc: "What appears in its place." },
        ],
        ignored: &[
            Ignored { name: "min", why: "the runtime leaves exactly one behind" },
            Ignored { name: "max", why: "the runtime leaves exactly one behind" },
            Ignored { name: "probability", why: "the runtime always transforms" },
        ],
        note: None,
    },
    Entry {
        name: "drop_portal_on_death",
        aliases: &["realm_portal_drop"],
        kind: Kind::Behaviour,
        summary: "Opens a portal where it died, which closes after a while. How a dungeon is \
                  entered at all.",
        params: &[
            Param { name: "target", position: Some(0), value: "a portal name", default: "nothing", doc: "Which portal opens." },
            Param { name: "probability", position: Some(1), value: "0 to 1, or a percentage", default: "1", doc: "How likely it is to open." },
            Param { name: "timeout", position: Some(2), value: "ms", default: "30000", doc: "How long the portal stays open." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "change_ground_on_death",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Changes the ground around it when it dies. Naming two tiles means the second is \
                  what the ground becomes.",
        params: &[
            Param { name: "radius", position: Some(0), value: "tiles", default: "1", doc: "How much ground changes." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "remove_object_on_death",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Removes named things nearby when it dies, such as the walls of the room it was \
                  guarding.",
        params: &[
            Param { name: "target", position: Some(0), value: "an object name", default: "anything nearby", doc: "What is removed." },
            Param { name: "radius", position: Some(1), value: "tiles", default: "10", doc: "How far out it reaches." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "order_on_death",
        aliases: &[],
        kind: Kind::Behaviour,
        summary: "Puts nearby entities into a named state when it dies.",
        params: &[
            Param { name: "range", position: Some(0), value: "tiles", default: "10", doc: "How far the order carries." },
            Param { name: "children", position: Some(1), value: "an object name", default: "anything nearby", doc: "Who is ordered." },
            Param { name: "target_state", position: Some(2), value: "a state name", default: "nothing", doc: "The state they are put into, which is one of theirs." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "transfer_damage_on_death",
        aliases: &["copy_damage_on_death"],
        kind: Kind::Behaviour,
        summary: "Gives the damage it took to something else when it dies, so that hitting a \
                  shield counts against the boss behind it.",
        params: &[
            Param { name: "target", position: Some(0), value: "an object name", default: "anything nearby", doc: "What receives the damage." },
            Param { name: "radius", position: Some(1), value: "tiles", default: "50", doc: "How far out it looks for it." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "prioritize",
        aliases: &[],
        kind: Kind::Group,
        summary: "Runs the first behaviour inside it that has something to do, and nothing else. \
                  `stay_close_to_spawn` above a `wander` is the pattern: wander, unless too far \
                  from home.",
        params: &[],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "sequence",
        aliases: &[],
        kind: Kind::Group,
        summary: "Runs the behaviours inside it one after another, each until it finishes.",
        params: &[],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "timed",
        aliases: &[],
        kind: Kind::Group,
        summary: "Runs what is inside it once every period. Not the same word as the `timed` on a \
                  transition, which is a wait rather than a repeat.",
        params: &[
            Param { name: "period", position: Some(0), value: "ms", default: "1000", doc: "How often the block runs." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "if",
        aliases: &[],
        kind: Kind::Group,
        summary: "Runs what is inside it while a condition holds.",
        params: &[],
        ignored: &[],
        note: Some(
            "The runtime compiles this to a condition it never satisfies, so nothing inside it \
             runs. No converted content uses it.",
        ),
    },
    Entry {
        name: "on_death_behavior",
        aliases: &[],
        kind: Kind::Group,
        summary: "Holds behaviours meant to run when the enemy dies.",
        params: &[],
        ignored: &[],
        note: Some(
            "The runtime runs what is inside it immediately rather than at death. No converted \
             content uses it.",
        ),
    },
];

// -- transitions --------------------------------------------------------------------------------

pub const CONDITIONS: &[Entry] = &[
    Entry {
        name: "timed",
        aliases: &[],
        kind: Kind::Condition,
        summary: "Fires once the state has been running for a while. The commonest way a fight \
                  moves from one phase to the next.",
        params: &[
            Param { name: "after", position: Some(0), value: "ms", default: "1000", doc: "How long the state runs before this fires." },
        ],
        ignored: &[
            Ignored { name: "time", why: "the compiler reads this as `after`, and the value written here is thrown away for the default of 1000ms" },
            Ignored { name: "randomized", why: "write `timed_random` instead, which is the transition that varies its wait" },
            Ignored { name: "target_state", why: "the state to move to is what follows `->`" },
        ],
        note: None,
    },
    Entry {
        name: "timed_random",
        aliases: &[],
        kind: Kind::Condition,
        summary: "Fires after a wait drawn between zero and a time, so several copies of an enemy \
                  do not change phase together.",
        params: &[
            Param { name: "time", position: Some(0), value: "ms", default: "1000", doc: "The longest it will wait." },
            Param { name: "randomized", position: Some(1), value: "1 or 0", default: "0", doc: "Whether the wait varies at all. Without it the wait is exactly `time`." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "player_within",
        aliases: &[],
        kind: Kind::Condition,
        summary: "Fires when a player comes within a distance. How a boss wakes up.",
        params: &[
            Param { name: "radius", position: Some(0), value: "tiles", default: "10", doc: "How close a player has to come." },
            Param { name: "see_invis", position: Some(1), value: "true or false", default: "false", doc: "Whether invisible players count." },
        ],
        ignored: &[
            Ignored { name: "dist", why: "the compiler reads this as `radius`, and the value written here is thrown away for the default of 10 tiles" },
        ],
        note: None,
    },
    Entry {
        name: "no_player_within",
        aliases: &[],
        kind: Kind::Condition,
        summary: "Fires when there is nobody within a distance. How a boss goes back to sleep.",
        params: &[
            Param { name: "radius", position: Some(0), value: "tiles", default: "10", doc: "The distance that has to be empty." },
        ],
        ignored: &[
            Ignored { name: "dist", why: "the compiler reads this as `radius`, and the value written here is thrown away for the default of 10 tiles" },
        ],
        note: None,
    },
    Entry {
        name: "hp_below",
        aliases: &[],
        kind: Kind::Condition,
        summary: "Fires when health falls below a share of the maximum. Written either as a \
                  fraction or as a percentage; both are understood.",
        params: &[
            Param { name: "fraction", position: Some(0), value: "0 to 1, or a percentage", default: "0.5", doc: "The share of health this fires at." },
        ],
        ignored: &[
            Ignored { name: "threshold", why: "the compiler reads this as `fraction`, and the value written here is thrown away for the default of half health" },
            Ignored { name: "target_state", why: "the state to move to is what follows `->`" },
        ],
        note: None,
    },
    Entry {
        name: "entity_exists",
        aliases: &["entity_count_greater_than"],
        kind: Kind::Condition,
        summary: "Fires while a named entity is standing nearby.",
        params: &[
            Param { name: "radius", position: Some(1), value: "tiles", default: "10", doc: "How far out it looks." },
        ],
        ignored: &[
            Ignored { name: "target", why: "the name is read from the entry's text arguments rather than by this name" },
            Ignored { name: "dist", why: "the compiler reads this as `radius`, and the value written here is thrown away for the default of 10 tiles" },
        ],
        note: None,
    },
    Entry {
        name: "entity_not_exists",
        aliases: &[],
        kind: Kind::Condition,
        summary: "Fires when no named entity is left nearby. How a room's door opens once its \
                  guards are dead.",
        params: &[
            Param { name: "radius", position: Some(1), value: "tiles", default: "10", doc: "How far out it looks." },
        ],
        ignored: &[
            Ignored { name: "target", why: "the names are read from the entry's text arguments rather than by this name" },
            Ignored { name: "dist", why: "the compiler reads this as `radius`, and the value written here is thrown away for the default of 10 tiles" },
        ],
        note: None,
    },
    Entry {
        name: "entities_not_exists",
        aliases: &[],
        kind: Kind::Condition,
        summary: "The same as `entity_not_exists` for several names at once. The radius comes \
                  first here and the names after it, which is the opposite way round from the \
                  singular form.",
        params: &[
            Param { name: "radius", position: Some(0), value: "tiles", default: "10", doc: "How far out it looks." },
        ],
        ignored: &[
            Ignored { name: "dist", why: "the compiler reads this as `radius`, and the value written here is thrown away for the default of 10 tiles" },
        ],
        note: None,
    },
    Entry {
        name: "damage_taken",
        aliases: &[],
        kind: Kind::Condition,
        summary: "Fires once the enemy has taken an amount of damage in this state.",
        params: &[
            Param { name: "amount", position: Some(0), value: "hit points", default: "1", doc: "How much damage it takes to fire." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "player_text",
        aliases: &[],
        kind: Kind::Condition,
        summary: "Fires when a player says a word nearby. How a puzzle boss is answered.",
        params: &[
            Param { name: "word", position: Some(0), value: "text", default: "nothing", doc: "What has to be said." },
            Param { name: "within", position: Some(1), value: "tiles", default: "anywhere in the room", doc: "How close the speaker has to be." },
            Param { name: "ignore_case", position: Some(3), value: "true or false", default: "true", doc: "Whether capitals matter." },
        ],
        ignored: &[],
        note: None,
    },
    Entry {
        name: "not_moving",
        aliases: &[],
        kind: Kind::Condition,
        summary: "Fires when the enemy has stood still for a while.",
        params: &[
            Param { name: "delay", position: Some(0), value: "ms", default: "250", doc: "How long it has to be still." },
        ],
        ignored: &[],
        note: None,
    },
];

// -- loot ---------------------------------------------------------------------------------------

pub const LOOT: &[Entry] = &[
    Entry {
        name: "item",
        aliases: &[],
        kind: Kind::Loot,
        summary: "One named item that can drop.",
        params: &[
            Param { name: "name", position: Some(0), value: "an item name", default: "the entry is dropped", doc: "Which item." },
            Param { name: "chance", position: Some(1), value: "0 to 1", default: "0", doc: "How likely it is to drop." },
            Param { name: "num_required", position: None, value: "number", default: "0", doc: "How many of it have to drop together." },
        ],
        ignored: &[
            Ignored { name: "item", why: "the compiler reads the name as `name`, and an entry it cannot find a name for is dropped from the table entirely" },
            Ignored { name: "probability", why: "the compiler reads the chance as `chance`, so the entry never drops" },
        ],
        note: None,
    },
    Entry {
        name: "tier",
        aliases: &[],
        kind: Kind::Loot,
        summary: "A random item of a tier and kind, rather than a named one.",
        params: &[
            Param { name: "tier", position: Some(0), value: "number", default: "0", doc: "Which tier of item." },
            Param { name: "kind", position: Some(1), value: "weapon, ability, armor, ring or any", default: "any", doc: "What sort of item." },
            Param { name: "chance", position: Some(2), value: "0 to 1", default: "0", doc: "How likely it is to drop." },
            Param { name: "num_required", position: None, value: "number", default: "0", doc: "How many of it have to drop together." },
        ],
        ignored: &[
            Ignored { name: "type", why: "the compiler reads the sort of item as `kind`, so any tier of item may drop" },
            Ignored { name: "probability", why: "the compiler reads the chance as `chance`, so the entry never drops" },
        ],
        note: None,
    },
    Entry {
        name: "threshold",
        aliases: &[],
        kind: Kind::Loot,
        summary: "A rule about who may have what is inside it: only players who did at least this \
                  share of the damage. Soulbound loot, in the content's own words.",
        params: &[
            Param { name: "threshold", position: Some(0), value: "0 to 1", default: "0", doc: "The share of damage a player has to have done." },
        ],
        ignored: &[],
        note: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_the_compiler_knows_is_documented() {
        // The compiler's own list, which is what it suggests against for a misspelling.
        for name in [
            "shoot", "wander", "buzz", "follow", "orbit", "stay_back", "stay_close_to_spawn",
            "stay_above", "heal_self", "heal_group", "heal_entity", "heal_player", "spawn",
            "spawn_group", "reproduce", "reproduce_children", "toss_object", "invisi_toss",
            "grenade", "suicide", "remove_entity", "decay", "conditional_effect",
            "remove_conditional_effect", "set_alt_texture", "flash", "scale_h_p", "change_size",
            "taunt", "order", "order_once", "transform", "protect", "move_to", "move_to2",
            "move_line", "back_and_forth", "charge", "swirl", "return_to_spawn", "set_no_x_p",
            "remove_tile_object", "apply_setpiece", "ground_transform", "replace_tile",
            "transform_on_death", "drop_portal_on_death", "realm_portal_drop",
            "change_ground_on_death", "remove_object_on_death", "order_on_death",
            "transfer_damage_on_death", "copy_damage_on_death", "prioritize", "sequence", "timed",
            "if", "on_death_behavior",
        ] {
            assert!(behaviour(name).is_some(), "`{name}` has no entry");
        }

        for name in [
            "timed", "timed_random", "player_within", "no_player_within", "hp_below",
            "entity_exists", "entity_count_greater_than", "entity_not_exists",
            "entities_not_exists", "damage_taken", "player_text", "not_moving",
        ] {
            assert!(condition(name).is_some(), "`{name}` has no transition entry");
        }
    }

    #[test]
    fn an_argument_is_documented_once() {
        for entry in BEHAVIOURS.iter().chain(CONDITIONS).chain(LOOT) {
            for param in entry.params {
                assert!(
                    entry.ignored(param.name).is_none(),
                    "`{}` lists `{}` as both read and ignored",
                    entry.name,
                    param.name
                );
            }
        }
    }

    #[test]
    fn a_misspelling_finds_its_neighbour() {
        assert_eq!(nearest(BEHAVIOURS, "shoo"), Some("shoot"));
        assert_eq!(nearest(CONDITIONS, "hp_less"), Some("hp_below"));
        assert_eq!(nearest(BEHAVIOURS, "completely_different_thing"), None);
    }
}
