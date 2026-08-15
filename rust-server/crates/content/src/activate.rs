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

        /// Only the largest of these counts, rather than each adding a halved share.
        ///
        /// The content's `noStack`, which `AEStatBoostSelf` and `AEStatBoostAura` both pass
        /// straight to `ActivateBoost.Push` (`Player.UseItem.cs:1071`, `:1103`). A stacking boost
        /// goes on a pile where each one below the top is worth half as much again
        /// (`ActivateBoost.cs:22-23`); a non-stacking one joins a list of which only the head is
        /// read (`:25`), so two people holding the same aura get one of it.
        no_stack: bool,
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
    ///
    /// Carries nothing, because nothing about the volley is written on the activation: `AEShoot`
    /// (`Player.UseItem.cs:1119-1128`) reads the count and the gap off the item itself, and all
    /// twenty-three `Shoot` activations in the content are the bare `<Activate>Shoot</Activate>`
    /// with no attributes at all.
    Shoot,

    /// Fires a ring outward from the aimed point.
    ///
    /// Carries nothing for the same reason [`Effect::Shoot`] does. `AEBulletNova` writes twenty
    /// into a fixed array and steps `i * 2PI / 20` (`Player.UseItem.cs:1146-1153`); no attribute of
    /// the activation is read, and all twelve in the content are the bare element.
    BulletNova,

    /// Creates an object at the aimed position.
    Create { child: String },

    /// Moves the user to the aimed position.
    Teleport { max_distance: f32 },

    /// Damages everything in a circle at the aimed position.
    ///
    /// `DazeBlast` alone. The original names it in `ActivateEffects` and then never dispatches it,
    /// so `Activate` drops it into `default` and logs "not implemented"
    /// (`Player.UseItem.cs:361-364`). One item in the shipped content carries one, and this side
    /// gives it the blast its name and its `radius`/`totalDamage` attributes describe rather than
    /// leaving that item inert.
    Blast {
        radius: f32,
        damage: i32,
        effect: Option<ConditionEffect>,
        effect_ms: u32,
    },

    /// Freezes every enemy in a circle at the aimed point, then makes them briefly unfreezable.
    ///
    /// `StasisBlast` (`Player.UseItem.cs:800-841`). It deals no damage at all and its circle is
    /// three tiles whatever the content writes — the handler passes the literal `3` to `AOE` and
    /// reads no `radius` — so the only number on the activation is how long the hold lasts.
    StasisBlast { duration_ms: u32 },

    /// A ball lobbed at the aimed point that poisons what it lands among.
    ///
    /// `PoisonGrenade` (`Player.UseItem.cs:675-701`). The damage is not dealt where the ball lands;
    /// it is spread over `duration_ms` and paid out a second at a time by `PoisonEnemy` (`:1292`).
    PoisonGrenade {
        radius: f32,
        total_damage: i32,
        duration_ms: u32,
    },

    /// The same ball, healing the players it lands among instead.
    ///
    /// `HealingGrenade` (`Player.UseItem.cs:1218-1244`), which differs from the poison above in
    /// the side it catches and in paying its `totalDamage` out as health.
    HealingGrenade {
        radius: f32,
        total_heal: i32,
        duration_ms: u32,
    },

    /// A stance the first press enters and the second press spends.
    ///
    /// `AEShurikenAbility` (`Player.UseItem.cs:566-581`), which is not a blast of any kind: the
    /// first use hangs `NinjaSpeedy` on the player and stops, and the second fires the item's own
    /// volley for a second helping of magic and drops the stance again.
    ShurikenAbility,

    /// Sets a base stat outright, rather than adding to it.
    ///
    /// `AEFixedStat` (`Player.UseItem.cs:645-649`): a bare assignment with no ceiling and no
    /// overflow into a boost, unlike [`Effect::IncrementStat`] beside it.
    FixedStat { stat: u8, amount: i32 },

    /// Drains health from those hit and gives it to the user.
    VampireBlast { radius: f32, damage: i32, heal: i32 },

    /// A bolt that picks one enemy in the direction aimed and jumps from it to the next.
    ///
    /// Not a blast: it never touches the ground between its targets, and which enemies it reaches
    /// depends on how they are spaced rather than on how close they are to the cursor
    /// (`Player.UseItem.cs:703-788`).
    Lightning {
        damage: i32,

        /// How many bodies the bolt visits in all, including the first.
        max_targets: u32,

        effect: Option<ConditionEffect>,
        effect_ms: u32,
    },

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

    /// Turns a locked door standing nearby into the one it leads to.
    ///
    /// `AEUnlockPortal` (`Player.UseItem.cs:433-510`), which is a swap in the room rather than a
    /// permission on the account: it finds the nearest portal named by `lockedName` within three
    /// tiles, takes it out of the world, stands the portal that leads to `dungeonName` in its
    /// place, and announces it to everyone there.
    UnlockPortal {
        /// The world the door will lead to, whose own definition names the portal to build.
        dungeon: String,

        /// The object id of the locked door to look for.
        locked: String,
    },

    /// A condition effect laid over an area, with everything about it in the attributes.
    ///
    /// `GenericActivate`, which despite the name is fully specified: it applies `condEffect` for a
    /// duration over a range, centred on the caster or on the aimed point, to players or to
    /// enemies. Entities in stasis or invincible are skipped, as `AEGenericActivate` skips them.
    GenericArea {
        effect: Option<ConditionEffect>,
        duration_ms: u32,
        range: f32,

        /// Whether it lands on players rather than on enemies.
        targets_players: bool,

        /// Whether it is centred on the aimed point rather than on the caster.
        centred_on_aim: bool,
    },

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

    /// A vault chest, which in the original is a message and nothing else.
    ///
    /// `UnlockSlot` (`Player.UseItem.cs:541-544`) sends "New vault chest unlocked successfully."
    /// and unlocks no slot at all: capacity is bought through the vault panel, so the item is a
    /// receipt for something that already happened. Kept as it is written, because an item that
    /// silently did nothing and an item that says it worked are different bugs.
    VaultSlot,

    /// A box whose contents the content decides.
    LootBox,

    /// A dye whose colour is chosen when it is opened.
    MysteryDye,
}

/// The cosmetic changes, which differ only in what they set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    /// Recolours the wearer, as a dye item's `<Activate>Dye</Activate>` asks.
    ///
    /// Which layer it paints and what colour is not in the activation at all: `AEDye` reads the
    /// item's own `Tex1` and `Tex2` and copies whichever is non-zero onto the player
    /// (`Player.UseItem.cs:583-589`). So this variant names only the act, and whoever holds the
    /// item turns it into one of the two below.
    Dye,

    /// Paints the cloth layer, the original's `Texture1`.
    DyeCloth,

    /// Paints the accessory layer, the original's `Texture2`.
    DyeAccessory,

    /// Grants a character skin.
    Skin,

    /// Grants a pet skin.
    PetSkin,
}

/// Scales a value by the user's wisdom, as `Player.UseWisMod` does.
///
/// Below thirty wisdom nothing happens at all. Above it a value grows by `wisdom / 150` of itself,
/// so seventy-five wisdom is half again and a hundred and fifty is double. The rounding is the
/// original's: `offset` is the number of decimal places kept, zero for amounts and one for ranges,
/// and the result is truncated to whole units unless a tenth survives the floor.
pub fn use_wis_mod(value: f32, wisdom: i32, offset: i32) -> f32 {
    if wisdom < 30 {
        return value;
    }

    let scale = 10f64.powi(offset);
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let value = value as f64;

    let grown = (value * wisdom as f64 / 150.0) + (value * sign);
    let floored = (grown * scale).floor() / scale;

    if floored - (floored as i64 as f64) * sign >= sign / scale {
        ((floored * 10.0) as i64) as f32 / 10.0
    } else {
        floored as i64 as f32
    }
}

impl Effect {
    /// The same effect with its amounts and ranges grown by the user's wisdom.
    ///
    /// Applied only where the content sets `useWisMod`, and then only by the five activations that
    /// read the flag at all. `AEHeal` (`Player.UseItem.cs:983`), `AEMagic` (`:948`), `AEMagicNova`
    /// (`:934`), `AEStatBoostSelf` (`:1102`) and `AEClearConditionEffectAura` (`:1020`) all take
    /// `eff.Amount` and `eff.Range` raw, so a flag on one of those is written down and never read.
    /// Growing them anyway would make a nova restore more magic than the original ever does.
    pub fn scaled_by_wisdom(self, wisdom: i32) -> Effect {
        let amount = |value: i32| use_wis_mod(value as f32, wisdom, 0) as i32;
        let range = |value: f32| use_wis_mod(value, wisdom, 1);
        let time = |value: u32| use_wis_mod(value as f32 / 1000.0, wisdom, 1).max(0.0) * 1000.0;

        match self {
            Effect::HealNova {
                amount: a,
                range: r,
            } => Effect::HealNova {
                amount: amount(a),
                range: range(r),
            },
            Effect::GenericArea {
                effect,
                duration_ms,
                range: r,
                targets_players,
                centred_on_aim,
            } => Effect::GenericArea {
                effect,
                duration_ms: time(duration_ms) as u32,
                range: range(r),
                targets_players,
                centred_on_aim,
            },
            // An aura only. `AEStatBoostSelf` never reads the flag, so a self-boost keeps the
            // numbers the content wrote whatever the wearer's wisdom is.
            Effect::StatBoost {
                stat,
                amount: a,
                duration_ms,
                range: Some(r),
                no_stack,
            } => Effect::StatBoost {
                stat,
                amount: amount(a),
                duration_ms: time(duration_ms) as u32,
                range: Some(range(r)),
                no_stack,
            },
            Effect::ConditionSelf {
                effect,
                duration_ms,
            } => Effect::ConditionSelf {
                effect,
                duration_ms: time(duration_ms) as u32,
            },
            Effect::ConditionAura {
                effect,
                duration_ms,
                range: r,
            } => Effect::ConditionAura {
                effect,
                duration_ms: time(duration_ms) as u32,
                range: range(r),
            },

            // Everything else has nothing wisdom is defined to scale.
            other => other,
        }
    }

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
                    no_stack: desc.flag("noStack"),
                },
                None => unsupported(desc),
            },
            "FixedStat" => match stat() {
                Some(stat) => Effect::FixedStat {
                    stat,
                    amount: amount(),
                },
                None => unsupported(desc),
            },
            "StatBoostAura" => match stat() {
                Some(stat) => Effect::StatBoost {
                    stat,
                    amount: amount(),
                    duration_ms: duration(),
                    range: Some(range()),
                    no_stack: desc.flag("noStack"),
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

            "Shoot" => Effect::Shoot,
            "BulletNova" => Effect::BulletNova,

            "Create" => Effect::Create {
                child: text(desc, "id").unwrap_or_default().to_string(),
            },
            "Teleport" => Effect::Teleport {
                max_distance: number(desc, "maxDistance", 20.0) as f32,
            },

            // Four names that read alike and are four unrelated handlers in the original. Reading
            // them as one blast gave the stasis blast a radius the original ignores, turned a
            // grenade's damage-over-time into a single hit, and made a ninja's stance an explosion.
            "StasisBlast" => Effect::StasisBlast {
                duration_ms: duration_ms(desc, "duration", 0.0),
            },
            "PoisonGrenade" => Effect::PoisonGrenade {
                radius: number(desc, "radius", 0.0) as f32,
                total_damage: number(desc, "totalDamage", 0.0) as i32,
                duration_ms: duration_ms(desc, "duration", 0.0),
            },
            "HealingGrenade" => Effect::HealingGrenade {
                radius: number(desc, "radius", 0.0) as f32,
                total_heal: number(desc, "totalDamage", 0.0) as i32,
                duration_ms: duration_ms(desc, "duration", 0.0),
            },
            "ShurikenAbility" => Effect::ShurikenAbility,
            "DazeBlast" => Effect::Blast {
                radius: number(desc, "radius", 0.0) as f32,
                damage: number(desc, "totalDamage", number(desc, "damage", 0.0)) as i32,
                effect: Some(ConditionEffect::Dazed),
                effect_ms: duration_ms(desc, "duration", 3.0),
            },

            // A scepter's bolt names no radius at all: every scepter in the content declares
            // `totalDamage` and `maxTargets` and nothing else
            // (`EmbeddedData_EquipCXML.dat:7300`). What it hits is decided by where the enemies
            // are standing, not by a circle.
            "Lightning" => Effect::Lightning {
                damage: number(desc, "totalDamage", number(desc, "damage", 0.0)) as i32,
                max_targets: number(desc, "maxTargets", 1.0).max(1.0) as u32,
                effect: text(desc, "condEffect").and_then(named_condition),
                effect_ms: (number(desc, "effectDuration", 0.0) * 1000.0) as u32,
            },
            "VampireBlast" => Effect::VampireBlast {
                radius: number(desc, "radius", 3.0) as f32,
                damage: number(desc, "totalDamage", 0.0) as i32,
                heal: number(desc, "heal", 0.0) as i32,
            },

            // `Pet` and `PermaPet` name the creature with `objectId`; `CreatePet` names nothing at
            // all. `id` is read as a fallback because the original's `ActivateEffect` keeps both
            // attributes (`XmlDescriptors.cs:414-415`, `:429-430`) and neither is spelled the same
            // way twice across the file set.
            "CreatePet" | "Pet" | "PermaPet" => Effect::Pet {
                name: text(desc, "objectId")
                    .or_else(|| text(desc, "id"))
                    .map(str::to_string),
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
            // Two names, and neither of them is `id`: the one item in the game that carries one
            // writes `dungeonName="Wine Cellar" lockedName="Locked Wine Cellar Portal"`. Reading
            // `id` left both empty, which is a key that matched no door.
            "UnlockPortal" => Effect::UnlockPortal {
                dungeon: text(desc, "dungeonName").unwrap_or_default().to_string(),
                locked: text(desc, "lockedName").unwrap_or_default().to_string(),
            },
            "LootBox" => Effect::Unlock {
                kind: Unlock::LootBox,
                value: text(desc, "id").unwrap_or_default().to_string(),
            },
            "UnlockSlot" => Effect::Unlock {
                kind: Unlock::VaultSlot,
                value: String::new(),
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
            "GenericActivate" => Effect::GenericArea {
                effect: text(desc, "condEffect").and_then(named_condition),
                duration_ms: duration(),
                range: range(),
                targets_players: text(desc, "target") == Some("player"),
                centred_on_aim: text(desc, "center") == Some("mouse"),
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

/// Which of the eleven stats an activate names, or `None` if it names none of them.
///
/// A written number is in the content's own numbering and goes through
/// [`Stat::from_content_number`]; a written name is matched directly, because a few files spell the
/// stat out. The two are different alphabets for the same things: `stat="21"` and
/// `stat="Defense"` both mean defence, and `stat="3"` means max magic rather than the defence its
/// digit would suggest here. Only the eight a class declares are ever spelled out in words.
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
    fn wisdom_grows_an_ability_only_once_it_is_worth_having() {
        // Below thirty it does nothing at all, which is what makes the stat a threshold rather
        // than a slope.
        assert_eq!(use_wis_mod(100.0, 0, 0), 100.0);
        assert_eq!(use_wis_mod(100.0, 29, 0), 100.0);

        // At seventy-five it is half again, and at a hundred and fifty it doubles.
        assert_eq!(use_wis_mod(100.0, 75, 0), 150.0);
        assert_eq!(use_wis_mod(100.0, 150, 0), 200.0);
    }

    #[test]
    fn a_heal_nova_grows_in_both_what_it_heals_and_how_far() {
        let nova = Effect::of(&desc(
            "HealNova",
            &[("amount", "100"), ("range", "4"), ("useWisMod", "true")],
        ));

        let Effect::HealNova { amount, range } = nova.scaled_by_wisdom(75) else {
            panic!("expected a heal nova");
        };
        assert_eq!(amount, 150);
        assert!(range > 4.0, "the aura widens as well: {range}");
    }

    #[test]
    fn a_heal_reads_its_amount() {
        assert_eq!(
            Effect::of(&desc("Heal", &[("amount", "100")])),
            Effect::Heal { amount: 100 }
        );
    }

    #[test]
    fn wisdom_reaches_only_the_five_activations_that_read_the_flag() {
        // The flag is on the activation, but five of the handlers never look at it: `AEHeal`,
        // `AEMagic`, `AEMagicNova`, `AEStatBoostSelf` and `AEClearConditionEffectAura` all take
        // the written numbers raw. Scaling them anyway is a heal the original never gives.
        let heal = Effect::of(&desc("Heal", &[("amount", "100"), ("useWisMod", "true")]));
        let magic = Effect::of(&desc("Magic", &[("amount", "100"), ("useWisMod", "true")]));
        let nova = Effect::of(&desc(
            "MagicNova",
            &[("amount", "100"), ("range", "4"), ("useWisMod", "true")],
        ));
        let boost = Effect::of(&desc(
            "StatBoostSelf",
            &[("stat", "22"), ("amount", "20"), ("useWisMod", "true")],
        ));
        let cleanse = Effect::of(&desc(
            "ClearConditionEffectAura",
            &[("range", "4"), ("useWisMod", "true")],
        ));

        for effect in [heal, magic, nova, boost, cleanse] {
            assert_eq!(
                effect.clone().scaled_by_wisdom(150),
                effect,
                "wisdom should leave this one alone"
            );
        }
    }

    #[test]
    fn a_stat_aura_says_whether_only_the_largest_of_it_counts() {
        // The six healing auras in the content are `noStack`, and every other aura is not. Reading
        // it off the range rather than off the attribute makes the wrong half of them halve.
        let priest = Effect::of(&desc(
            "StatBoostAura",
            &[
                ("stat", "0"),
                ("amount", "25"),
                ("range", "4.5"),
                ("noStack", "true"),
            ],
        ));
        let ordinary = Effect::of(&desc(
            "StatBoostAura",
            &[("stat", "22"), ("amount", "30"), ("range", "6")],
        ));

        assert!(matches!(
            priest,
            Effect::StatBoost {
                no_stack: true,
                ..
            }
        ));
        assert!(matches!(
            ordinary,
            Effect::StatBoost {
                no_stack: false,
                ..
            }
        ));
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
    fn the_four_names_that_read_alike_are_four_different_things() {
        // `Activate` sends each of these to a handler of its own, and three of the four are not
        // blasts at all: a poison grenade is a fuse and a damage-over-time, a stasis blast freezes
        // for no damage in a circle it never measures, and a shuriken ability is a stance.
        // Collapsing them lost the telegraph, the immunity and the second press.
        assert_eq!(
            Effect::of(&desc(
                "PoisonGrenade",
                &[("radius", "3"), ("totalDamage", "250"), ("duration", "5.5")]
            )),
            Effect::PoisonGrenade {
                radius: 3.0,
                total_damage: 250,
                duration_ms: 5_500,
            }
        );
        assert_eq!(
            Effect::of(&desc("StasisBlast", &[("duration", "4.5")])),
            Effect::StasisBlast {
                duration_ms: 4_500
            }
        );
        assert_eq!(
            Effect::of(&desc("ShurikenAbility", &[])),
            Effect::ShurikenAbility
        );
    }

    #[test]
    fn a_stasis_blast_reads_no_radius_because_the_original_reads_none() {
        // `StasisBlast` hands `AOE` the literal 3 (`Player.UseItem.cs:812`). A radius written on
        // one of these is a number the original never looks at, and none of the ten in the content
        // writes one.
        assert_eq!(
            Effect::of(&desc("StasisBlast", &[("radius", "9"), ("duration", "3")])),
            Effect::StasisBlast {
                duration_ms: 3_000
            }
        );
    }

    #[test]
    fn a_portal_key_reads_the_two_names_the_one_item_in_the_game_writes() {
        // The Wine Cellar Incantation, verbatim from `EmbeddedData_EquipCXML.xml:8741`. It carries
        // no `id` at all, so reading one gave a key that named no dungeon and no door: the account
        // row was written under an empty string and nothing in the room ever changed.
        let key = Effect::of(&desc(
            "UnlockPortal",
            &[
                ("dungeonName", "Wine Cellar"),
                ("lockedName", "Locked Wine Cellar Portal"),
            ],
        ));

        assert_eq!(
            key,
            Effect::UnlockPortal {
                dungeon: "Wine Cellar".to_string(),
                locked: "Locked Wine Cellar Portal".to_string(),
            }
        );
    }

    #[test]
    fn a_daze_blast_is_the_one_of_the_four_that_really_is_a_blast() {
        assert!(matches!(
            Effect::of(&desc("DazeBlast", &[("radius", "2.0"), ("totalDamage", "25")])),
            Effect::Blast {
                radius: 2.0,
                damage: 25,
                effect: Some(ConditionEffect::Dazed),
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
    fn a_pet_is_named_by_the_attribute_the_content_actually_writes() {
        // Every `Pet` and `PermaPet` activation in the shipped files spells the creature with
        // `objectId`, and not one of them uses `id`. Reading `id` left all eighty-seven pet and egg
        // items summoning nothing at all, silently.
        let drake = Effect::of(&desc("Pet", &[("objectId", "Blue Drake")]));
        assert_eq!(
            drake,
            Effect::Pet {
                name: Some("Blue Drake".to_string()),
                permanent: false
            }
        );

        let rock = Effect::of(&desc(
            "Pet",
            &[
                ("hideEffect", "true"),
                ("cooldown", "20"),
                ("objectId", "Pet Rock"),
            ],
        ));
        assert_eq!(
            rock,
            Effect::Pet {
                name: Some("Pet Rock".to_string()),
                permanent: false
            }
        );

        // `CreatePet` names nothing, and stays nothing.
        assert_eq!(
            Effect::of(&desc("CreatePet", &[])),
            Effect::Pet {
                name: None,
                permanent: false
            }
        );
    }

    #[test]
    fn a_shoot_activation_carries_nothing_because_the_item_carries_it_all() {
        // All twenty-three in the content are the bare element. The count and the arc live on the
        // item, which is where `AEShoot` reads them from.
        assert_eq!(Effect::of(&desc("Shoot", &[])), Effect::Shoot);
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
    fn a_generic_activate_is_an_area_effect_rather_than_an_unknown_id() {
        // The name suggests a dispatch on an id and it is nothing of the kind: AEGenericActivate
        // lays a condition over an area, and every argument it needs is in the attributes. Reading
        // it as an unknown id left twenty-six items saying "nothing happens".
        let effect = Effect::of(&desc(
            "GenericActivate",
            &[
                ("condEffect", "Damaging"),
                ("duration", "5"),
                ("range", "6"),
                ("target", "player"),
                ("center", "player"),
            ],
        ));

        assert_eq!(
            effect,
            Effect::GenericArea {
                effect: Some(ConditionEffect::Damaging),
                duration_ms: 5000,
                range: 6.0,
                targets_players: true,
                centred_on_aim: false,
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
            "FixedStat",
            "HealingGrenade",
            "UnlockSlot",
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
