//! What an item does when it is used.
//!
//! An `ActivateDesc` is a name and a bag of attributes, read straight from the content files. This
//! turns one into an [`Effect`], which is a closed set the simulation can act on, the same shape
//! the behaviour crate compiles its primitives into.
//!
//! # What is not decided here
//!
//! Nothing reads the world. An effect says "heal this much" or "fire this many"; where the shot
//! goes and who it reaches is the simulation's to work out, exactly as it is for a behaviour.
//!
//! # Where the numbers come from
//!
//! `Player.UseItem` in the original server. Attribute names follow the content files rather than
//! the C# fields, because the files are what has to be read.

use crate::desc::ActivateDesc;
use crate::effect::ConditionEffect;
use crate::player::Stat;

/// One thing using an item does.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Restores health to the user.
    Heal { amount: i32 },

    /// Restores magic to the user.
    Magic { amount: i32 },

    /// Restores health to everyone in range.
    HealNova { amount: i32, range: f32 },

    /// Restores magic to everyone in range.
    MagicNova { amount: i32, range: f32 },

    /// Raises one of the user's stats permanently.
    ///
    /// The most used activate in the game by a wide margin: every potion is one of these.
    IncrementStat { stat: u8, amount: i32 },

    /// Raises a stat for a time.
    StatBoost {
        stat: u8,
        amount: i32,
        duration_ms: u32,

        /// Everyone in range rather than only the user.
        range: Option<f32>,
    },

    /// Applies a condition effect to the user.
    ConditionSelf {
        effect: ConditionEffect,
        duration_ms: u32,
    },

    /// Applies a condition effect to everyone in range.
    ConditionAura {
        effect: ConditionEffect,
        duration_ms: u32,
        range: f32,
    },

    /// Removes harmful conditions.
    Cleanse {
        /// Everyone in range rather than only the user.
        range: Option<f32>,
    },

    /// Fires a spread of the item's own projectile.
    Shoot { count: u32, spread: f32 },

    /// Fires a ring outward.
    BulletNova { count: u32 },

    /// Creates an object at the aimed position.
    Create { child: String },

    /// Moves the user to the aimed position.
    Teleport { max_distance: f32 },

    /// Damages everything in a circle at the aimed position.
    Blast {
        radius: f32,
        damage: i32,
        effect: Option<ConditionEffect>,
        effect_ms: u32,
    },

    /// Drains health from those hit and gives it to the user.
    VampireBlast { radius: f32, damage: i32, heal: i32 },

    /// Changes what the user looks like. Cosmetic, so the server only records it.
    Appearance { kind: Appearance, value: u32 },

    /// Summons a pet, which is a companion entity of its own.
    Pet {
        /// Which pet, when the content names one.
        name: Option<String>,

        /// Kept beyond the world it was summoned in.
        permanent: bool,
    },

    /// Leaves something behind that acts on its own: a decoy that draws fire, or a trap that arms.
    Placed {
        kind: Placed,
        duration_ms: u32,
        radius: f32,
        damage: i32,
        effect: Option<ConditionEffect>,
    },

    /// Adds to a currency the account holds.
    Currency { kind: Currency, amount: i32 },

    /// Multiplies something the account earns, for a time.
    Boost {
        kind: Boost,
        duration_ms: u32,
        multiplier: f32,
    },

    /// Grants a permanent capability: a backpack, a class, a portal.
    Unlock { kind: Unlock, value: String },

    /// Opens a way somewhere.
    Portal { name: String, duration_ms: u32 },

    /// Something whose whole behaviour is written in the content rather than in the name.
    ///
    /// The original dispatches these on an id and does something different for each. Carried so
    /// the id reaches the simulation, which is the only place that can know what it means.
    Generic { id: String },

    /// Something the content asks for that this runtime does not implement.
    ///
    /// Kept rather than rejected so one unimplemented activate costs that activate and not the
    /// item, and it is reported by name at load.
    Unsupported { name: String },
}

/// What a placed effect leaves behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placed {
    /// Draws fire, and is destroyed by it.
    Decoy,

    /// Arms where it lands and fires when something comes near.
    Trap,
}

/// The things an account accumulates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    Fame,
    Token,
}

/// The things that can be multiplied for a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boost {
    Experience,
    LootDrop,
    LootTier,
}

/// The permanent capabilities an item can grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unlock {
    /// A second row of carried slots.
    Backpack,

    /// A class, bypassing its levelling requirement.
    Class,

    /// A destination.
    Portal,

    /// A box whose contents the content decides.
    LootBox,

    /// A dye whose colour is chosen when it is opened.
    MysteryDye,
}

/// The cosmetic changes, which differ only in what they set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    /// Recolours the item's cloth.
    Dye,

    /// Grants a character skin.
    Skin,

    /// Grants a pet skin.
    PetSkin,
}

impl Effect {
    /// Whether this is something the runtime knows how to carry out.
    pub fn is_supported(&self) -> bool {
        !matches!(self, Effect::Unsupported { .. })
    }

    /// The name the content used, for reporting.
    pub fn unsupported_name(&self) -> Option<&str> {
        match self {
            Effect::Unsupported { name } => Some(name),
            _ => None,
        }
    }

    /// Reads one activate.
    pub fn of(desc: &ActivateDesc) -> Effect {
        let amount = || number(desc, "amount", 0.0) as i32;
        let range = || number(desc, "range", 3.0) as f32;
        let duration = || duration_ms(desc, "duration", 0.0);
        let stat = || stat_index(text(desc, "stat").unwrap_or_default());

        match desc.name.as_str() {
            "Heal" => Effect::Heal { amount: amount() },
            "Magic" => Effect::Magic { amount: amount() },
            "HealNova" => Effect::HealNova {
                amount: amount(),
                range: range(),
            },
            "MagicNova" => Effect::MagicNova {
                amount: amount(),
                range: range(),
            },

            // A stat these do not recognise is reported rather than guessed at. Guessing is what
            // made every potion in the game raise max health: an unmatched number fell through to
            // stat zero, which is a plausible-looking answer and the wrong one.
            "IncrementStat" => match stat() {
                Some(stat) => Effect::IncrementStat {
                    stat,
                    amount: amount(),
                },
                None => unsupported(desc),
            },
            "StatBoostSelf" => match stat() {
                Some(stat) => Effect::StatBoost {
                    stat,
                    amount: amount(),
                    duration_ms: duration(),
                    range: None,
                },
                None => unsupported(desc),
            },
            "StatBoostAura" => match stat() {
                Some(stat) => Effect::StatBoost {
                    stat,
                    amount: amount(),
                    duration_ms: duration(),
                    range: Some(range()),
                },
                None => unsupported(desc),
            },

            "ConditionEffectSelf" => Effect::ConditionSelf {
                effect: condition(desc),
                duration_ms: duration(),
            },
            "ConditionEffectAura" => Effect::ConditionAura {
                effect: condition(desc),
                duration_ms: duration(),
                range: range(),
            },

            "RemoveNegativeConditions" | "ClearConditionEffectAura" => Effect::Cleanse {
                range: Some(range()),
            },
            "RemoveNegativeConditionsSelf" | "ClearConditionEffectSelf" => {
                Effect::Cleanse { range: None }
            }

            "Shoot" => Effect::Shoot {
                count: number(desc, "numShots", 1.0).max(1.0) as u32,
                spread: number(desc, "arcGap", 0.0) as f32,
            },
            "BulletNova" => Effect::BulletNova {
                count: number(desc, "numShots", 8.0).max(1.0) as u32,
            },

            "Create" => Effect::Create {
                child: text(desc, "id").unwrap_or_default().to_string(),
            },
            "Teleport" => Effect::Teleport {
                max_distance: number(desc, "maxDistance", 20.0) as f32,
            },

            // The blasts differ in what they leave behind rather than in what they do.
            "PoisonGrenade" | "StasisBlast" | "DazeBlast" | "Lightning" | "ShurikenAbility" => {
                Effect::Blast {
                    radius: number(desc, "radius", 3.0) as f32,
                    damage: number(desc, "totalDamage", number(desc, "damage", 0.0)) as i32,
                    effect: blast_condition(&desc.name),
                    effect_ms: duration_ms(desc, "duration", 3.0),
                }
            }
            "VampireBlast" => Effect::VampireBlast {
                radius: number(desc, "radius", 3.0) as f32,
                damage: number(desc, "totalDamage", 0.0) as i32,
                heal: number(desc, "heal", 0.0) as i32,
            },

            "CreatePet" | "Pet" | "PermaPet" => Effect::Pet {
                name: text(desc, "id").map(str::to_string),
                permanent: desc.name == "PermaPet",
            },

            "Decoy" => Effect::Placed {
                kind: Placed::Decoy,
                duration_ms: duration_ms(desc, "duration", 3.0),
                radius: number(desc, "distance", 0.0) as f32,
                damage: 0,
                effect: None,
            },
            "Trap" => Effect::Placed {
                kind: Placed::Trap,
                duration_ms: duration_ms(desc, "duration", 10.0),
                radius: number(desc, "radius", 4.0) as f32,
                damage: number(desc, "totalDamage", 0.0) as i32,
                effect: text(desc, "condEffect").and_then(named_condition),
            },

            "Fame" => Effect::Currency {
                kind: Currency::Fame,
                amount: amount(),
            },
            "Token" => Effect::Currency {
                kind: Currency::Token,
                amount: number(desc, "amount", 1.0) as i32,
            },

            "XPBoost" => Effect::Boost {
                kind: Boost::Experience,
                duration_ms: duration_ms(desc, "duration", 0.0),
                multiplier: number(desc, "amount", 2.0) as f32,
            },
            "LDBoost" => Effect::Boost {
                kind: Boost::LootDrop,
                duration_ms: duration_ms(desc, "duration", 0.0),
                multiplier: number(desc, "amount", 2.0) as f32,
            },
            "LTBoost" => Effect::Boost {
                kind: Boost::LootTier,
                duration_ms: duration_ms(desc, "duration", 0.0),
                multiplier: number(desc, "amount", 2.0) as f32,
            },

            "Backpack" => Effect::Unlock {
                kind: Unlock::Backpack,
                value: String::new(),
            },
            "Unlock" => Effect::Unlock {
                kind: Unlock::Class,
                value: text(desc, "id").unwrap_or_default().to_string(),
            },
            "UnlockPortal" => Effect::Unlock {
                kind: Unlock::Portal,
                value: text(desc, "id").unwrap_or_default().to_string(),
            },
            "LootBox" => Effect::Unlock {
                kind: Unlock::LootBox,
                value: text(desc, "id").unwrap_or_default().to_string(),
            },
            "MysteryDyes" => Effect::Unlock {
                kind: Unlock::MysteryDye,
                value: String::new(),
            },

            "MysteryPortal" => Effect::Portal {
                name: text(desc, "id").unwrap_or_default().to_string(),
                duration_ms: duration_ms(desc, "timeoutMS", 30.0),
            },

            // Dispatched on an id in the original, so the id is what has to travel.
            "GenericActivate" => Effect::Generic {
                id: text(desc, "id").unwrap_or_default().to_string(),
            },

            "Dye" => Effect::Appearance {
                kind: Appearance::Dye,
                value: number(desc, "id", 0.0).max(0.0) as u32,
            },
            "UnlockSkin" => Effect::Appearance {
                kind: Appearance::Skin,
                value: number(desc, "id", 0.0).max(0.0) as u32,
            },
            "PetSkin" => Effect::Appearance {
                kind: Appearance::PetSkin,
                value: number(desc, "id", 0.0).max(0.0) as u32,
            },

            other => Effect::Unsupported {
                name: other.to_string(),
            },
        }
    }
}

/// An attribute as a number, accepting the hex the content sometimes writes.
fn number(desc: &ActivateDesc, name: &str, fallback: f64) -> f64 {
    desc.args
        .iter()
        .find(|(key, _)| key == name)
        .and_then(|(_, value)| crate::xml::parse_float(value))
        .unwrap_or(fallback)
}

fn text<'a>(desc: &'a ActivateDesc, name: &str) -> Option<&'a str> {
    desc.args
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// A duration, which the content writes in seconds for activates and milliseconds elsewhere.
fn duration_ms(desc: &ActivateDesc, name: &str, fallback: f64) -> u32 {
    let written = number(desc, name, fallback);
    if written > 0.0 && written < 1000.0 {
        (written * 1000.0) as u32
    } else {
        written.max(0.0) as u32
    }
}

fn condition(desc: &ActivateDesc) -> ConditionEffect {
    text(desc, "effect")
        .and_then(named_condition)
        .unwrap_or(ConditionEffect::Dead)
}

/// What each blast leaves on those it catches.
fn blast_condition(name: &str) -> Option<ConditionEffect> {
    match name {
        "PoisonGrenade" => Some(ConditionEffect::Bleeding),
        "StasisBlast" => Some(ConditionEffect::Stasis),
        "DazeBlast" => Some(ConditionEffect::Dazed),
        _ => None,
    }
}

/// A condition effect written by name, matched the way the files spell them.
fn named_condition(written: &str) -> Option<ConditionEffect> {
    let tidy: String = written
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();

    (0u16..64)
        .filter_map(ConditionEffect::from_index)
        .find(|effect| {
            let name: String = format!("{effect:?}")
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .map(|c| c.to_ascii_lowercase())
                .collect();
            name == tidy
        })
}

/// An activate whose arguments this runtime could not make sense of.
fn unsupported(desc: &ActivateDesc) -> Effect {
    Effect::Unsupported {
        name: desc.name.clone(),
    }
}

/// Which of the eight stats an activate names, or `None` if it names none of them.
///
/// A written number is in the content's own numbering and goes through
/// [`Stat::from_content_number`]; a written name is matched directly, because a few files spell the
/// stat out. The two are different alphabets for the same eight things: `stat="21"` and
/// `stat="Defense"` both mean defence, and `stat="3"` means max magic rather than the defence its
/// digit would suggest here.
fn stat_index(written: &str) -> Option<u8> {
    let tidy: String = written
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();

    if let Ok(number) = tidy.parse::<i64>() {
        return Stat::from_content_number(number).map(|stat| stat.index() as u8);
    }

    Some(match tidy.as_str() {
        "maxhitpoints" | "hp" => 0,
        "maxmagicpoints" | "mp" => 1,
        "attack" => 2,
        "defense" | "defence" => 3,
        "speed" => 4,
        "dexterity" => 5,
        "hpregen" | "vitality" => 6,
        "mpregen" | "wisdom" => 7,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(name: &str, args: &[(&str, &str)]) -> ActivateDesc {
        ActivateDesc {
            name: name.to_string(),
            args: args
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn a_potion_raises_the_stat_it_names() {
        // The most used activate in the game: every potion is one of these.
        let effect = Effect::of(&desc("IncrementStat", &[("stat", "0"), ("amount", "5")]));

        assert_eq!(effect, Effect::IncrementStat { stat: 0, amount: 5 });
    }

    #[test]
    fn a_stat_can_be_named_by_word_or_by_number() {
        // The files use both, and the two are different alphabets: attack is `20` written as a
        // number and `Attack` written as a word. Reading one as the other raises the wrong stat.
        for written in ["20", "Attack", "attack"] {
            let effect = Effect::of(&desc(
                "IncrementStat",
                &[("stat", written), ("amount", "1")],
            ));
            assert_eq!(
                effect,
                Effect::IncrementStat { stat: 2, amount: 1 },
                "{written}"
            );
        }
    }

    #[test]
    fn every_potion_in_the_game_raises_what_it_says_on_the_bottle() {
        // The whole table, because getting one row wrong is how twenty-one of the game's
        // twenty-four potions came to raise max health. The left column is what the content
        // writes; the right is the position in the eight-stat array.
        for (written, expected) in [
            ("0", 0),  // Life
            ("3", 1),  // Mana
            ("20", 2), // Attack
            ("21", 3), // Defense
            ("22", 4), // Speed
            ("26", 6), // Vitality
            ("27", 7), // Wisdom
            ("28", 5), // Dexterity
        ] {
            let effect = Effect::of(&desc(
                "IncrementStat",
                &[("stat", written), ("amount", "1")],
            ));
            assert_eq!(
                effect,
                Effect::IncrementStat {
                    stat: expected,
                    amount: 1
                },
                "stat={written}"
            );
        }
    }

    #[test]
    fn a_stat_number_outside_the_table_is_reported_rather_than_guessed() {
        // Falling through to stat zero is exactly the bug this replaced: it turns an unreadable
        // potion into a max-health potion, which looks like it worked.
        let effect = Effect::of(&desc("IncrementStat", &[("stat", "99"), ("amount", "1")]));

        assert_eq!(effect.unsupported_name(), Some("IncrementStat"));
    }

    #[test]
    fn wisdom_and_vitality_are_the_names_the_files_use_for_the_regens() {
        assert_eq!(stat_index("Vitality"), Some(6));
        assert_eq!(stat_index("HpRegen"), Some(6));
        assert_eq!(stat_index("Wisdom"), Some(7));
        assert_eq!(stat_index("MpRegen"), Some(7));
    }

    #[test]
    fn a_heal_reads_its_amount() {
        assert_eq!(
            Effect::of(&desc("Heal", &[("amount", "100")])),
            Effect::Heal { amount: 100 }
        );
    }

    #[test]
    fn an_aura_carries_a_range_and_a_self_effect_does_not() {
        let aura = Effect::of(&desc(
            "StatBoostAura",
            &[("stat", "22"), ("amount", "20"), ("range", "6")],
        ));
        let alone = Effect::of(&desc("StatBoostSelf", &[("stat", "22"), ("amount", "20")]));

        assert!(matches!(aura, Effect::StatBoost { range: Some(_), .. }));
        assert!(matches!(alone, Effect::StatBoost { range: None, .. }));
    }

    #[test]
    fn a_duration_in_seconds_becomes_milliseconds() {
        // Activates write seconds where everything else writes milliseconds, and reading five as
        // five milliseconds makes every temporary effect invisible.
        let effect = Effect::of(&desc(
            "ConditionEffectSelf",
            &[("effect", "Speedy"), ("duration", "5")],
        ));

        assert_eq!(
            effect,
            Effect::ConditionSelf {
                effect: ConditionEffect::Speedy,
                duration_ms: 5_000,
            }
        );
    }

    #[test]
    fn a_condition_is_matched_however_the_files_spell_it() {
        for written in ["Speedy", "speedy", "SPEEDY"] {
            let effect = Effect::of(&desc("ConditionEffectSelf", &[("effect", written)]));
            assert!(
                matches!(
                    effect,
                    Effect::ConditionSelf {
                        effect: ConditionEffect::Speedy,
                        ..
                    }
                ),
                "{written}"
            );
        }
    }

    #[test]
    fn each_blast_leaves_what_it_is_named_for() {
        let poison = Effect::of(&desc("PoisonGrenade", &[("radius", "3")]));
        let stasis = Effect::of(&desc("StasisBlast", &[("radius", "3")]));

        assert!(matches!(
            poison,
            Effect::Blast {
                effect: Some(ConditionEffect::Bleeding),
                ..
            }
        ));
        assert!(matches!(
            stasis,
            Effect::Blast {
                effect: Some(ConditionEffect::Stasis),
                ..
            }
        ));
    }

    #[test]
    fn the_three_pet_activates_differ_only_in_whether_the_pet_is_kept() {
        let summoned = Effect::of(&desc("CreatePet", &[("id", "Fire Sprite")]));
        let permanent = Effect::of(&desc("PermaPet", &[("id", "Fire Sprite")]));

        assert_eq!(
            summoned,
            Effect::Pet {
                name: Some("Fire Sprite".to_string()),
                permanent: false
            }
        );
        assert!(matches!(
            permanent,
            Effect::Pet {
                permanent: true,
                ..
            }
        ));
    }

    #[test]
    fn a_trap_carries_what_it_does_and_a_decoy_does_not() {
        let trap = Effect::of(&desc(
            "Trap",
            &[
                ("radius", "4"),
                ("totalDamage", "200"),
                ("condEffect", "Slowed"),
            ],
        ));
        let decoy = Effect::of(&desc("Decoy", &[("duration", "3")]));

        assert!(matches!(
            trap,
            Effect::Placed {
                kind: Placed::Trap,
                damage: 200,
                effect: Some(ConditionEffect::Slowed),
                ..
            }
        ));
        assert!(matches!(
            decoy,
            Effect::Placed {
                kind: Placed::Decoy,
                damage: 0,
                effect: None,
                ..
            }
        ));
    }

    #[test]
    fn the_boosts_are_told_apart_by_what_they_multiply() {
        for (name, expected) in [
            ("XPBoost", Boost::Experience),
            ("LDBoost", Boost::LootDrop),
            ("LTBoost", Boost::LootTier),
        ] {
            let effect = Effect::of(&desc(name, &[("duration", "3600")]));
            assert!(
                matches!(effect, Effect::Boost { kind, .. } if kind == expected),
                "{name}"
            );
        }
    }

    #[test]
    fn a_generic_activate_carries_the_id_the_simulation_needs() {
        // The original dispatches these on the id and does something different for each, so
        // dropping it would leave twenty-six items doing nothing with no way to tell why.
        let effect = Effect::of(&desc("GenericActivate", &[("id", "Nexus Amulet")]));

        assert_eq!(
            effect,
            Effect::Generic {
                id: "Nexus Amulet".to_string()
            }
        );
    }

    #[test]
    fn every_kind_the_content_uses_is_read_as_something() {
        // The list the coverage example measures. A kind added to the content without being added
        // here would show up as an item that silently does nothing.
        let kinds = [
            "IncrementStat",
            "Dye",
            "UnlockSkin",
            "CreatePet",
            "ConditionEffectSelf",
            "Create",
            "Heal",
            "GenericActivate",
            "StatBoostSelf",
            "Pet",
            "Shoot",
            "Magic",
            "ConditionEffectAura",
            "Token",
            "BulletNova",
            "HealNova",
            "PoisonGrenade",
            "VampireBlast",
            "Decoy",
            "Lightning",
            "Trap",
            "Teleport",
            "StatBoostAura",
            "StasisBlast",
            "ShurikenAbility",
            "MagicNova",
            "Fame",
            "PermaPet",
            "PetSkin",
            "MysteryPortal",
            "XPBoost",
            "ClearConditionEffectAura",
            "ClearConditionEffectSelf",
            "LDBoost",
            "LTBoost",
            "RemoveNegativeConditions",
            "Unlock",
            "Backpack",
            "DazeBlast",
            "LootBox",
            "MysteryDyes",
            "RemoveNegativeConditionsSelf",
            "UnlockPortal",
        ];

        for kind in kinds {
            // A stat is supplied because the three that take one now refuse an activate that
            // names no stat, rather than silently choosing the first.
            let effect = Effect::of(&desc(kind, &[("stat", "20")]));
            assert!(effect.is_supported(), "{kind} is not read as anything");
        }
    }
    #[test]
    fn something_this_runtime_does_not_know_is_kept_and_named() {
        // One unimplemented activate should cost that activate, not the item that carries it.
        let effect = Effect::of(&desc("SummonTheMoon", &[]));

        assert!(!effect.is_supported());
        assert_eq!(effect.unsupported_name(), Some("SummonTheMoon"));
    }

    #[test]
    fn an_activate_with_no_attributes_at_all_still_reads() {
        // A half-written content file should cost its own numbers, not the load.
        assert_eq!(Effect::of(&desc("Heal", &[])), Effect::Heal { amount: 0 });
    }
}
