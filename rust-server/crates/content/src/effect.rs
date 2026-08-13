//! Condition effects: the status flags an entity can carry.
//!
//! The numbering is protocol-visible: the client keeps its own copy in `src/Data/ConditionEffects.cs`
//! and the two must agree, because effects travel as a bitmask keyed by these indices. Adding one
//! means adding it at the end, never renumbering.

use std::fmt;

/// A status effect, by its wire index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u16)]
pub enum ConditionEffect {
    Dead = 0,
    Quiet = 1,
    Weak = 2,
    Slowed = 3,
    Sick = 4,
    Dazed = 5,
    Stunned = 6,
    Blind = 7,
    Hallucinating = 8,
    Drunk = 9,
    Confused = 10,
    StunImmune = 11,
    Invisible = 12,
    Paralyzed = 13,
    Speedy = 14,
    Bleeding = 15,
    ArmorBreakImmune = 16,
    Healing = 17,
    Damaging = 18,
    Berserk = 19,
    Paused = 20,
    Stasis = 21,
    StasisImmune = 22,
    Invincible = 23,
    Invulnerable = 24,
    Armored = 25,
    ArmorBroken = 26,
    Hexed = 27,
    NinjaSpeedy = 28,
    Unstable = 29,
    Darkness = 30,
    SlowedImmune = 31,
    DazedImmune = 32,
    ParalyzeImmune = 33,
    Petrify = 34,
    PetrifyImmune = 35,
    PetDisable = 36,
    Curse = 37,
    CurseImmune = 38,
    HpBoost = 39,
    MpBoost = 40,
    AttBoost = 41,
    DefBoost = 42,
    SpdBoost = 43,
    DexBoost = 44,
    VitBoost = 45,
    WisBoost = 46,
    Hidden = 47,
    Muted = 48,
    PartyVision = 49,
    XMasVision = 50,
}

/// How many effects the game currently defines. Indices `0..CONDITION_EFFECT_COUNT` are in use;
/// everything above is room for content of our own.
pub const CONDITION_EFFECT_COUNT: usize = 51;

/// How many effects a [`ConditionSet`] can hold.
///
/// Vanilla uses 51 of these, so a `u64` set would have left thirteen for our own content, enough
/// to run out of, and widening it after the client ships is a protocol break. A `u128` is still a
/// single value the compiler keeps in two registers, so set operations remain two instructions and
/// the room stops being a concern.
pub const CONDITION_EFFECT_CAPACITY: usize = u128::BITS as usize;

// Make overflowing the mask a build failure rather than a bit that silently stops being set.
const _: () = assert!(CONDITION_EFFECT_COUNT <= CONDITION_EFFECT_CAPACITY);

impl ConditionEffect {
    /// Every effect, in wire order.
    pub const ALL: [ConditionEffect; CONDITION_EFFECT_COUNT] = {
        use ConditionEffect::*;
        [
            Dead,
            Quiet,
            Weak,
            Slowed,
            Sick,
            Dazed,
            Stunned,
            Blind,
            Hallucinating,
            Drunk,
            Confused,
            StunImmune,
            Invisible,
            Paralyzed,
            Speedy,
            Bleeding,
            ArmorBreakImmune,
            Healing,
            Damaging,
            Berserk,
            Paused,
            Stasis,
            StasisImmune,
            Invincible,
            Invulnerable,
            Armored,
            ArmorBroken,
            Hexed,
            NinjaSpeedy,
            Unstable,
            Darkness,
            SlowedImmune,
            DazedImmune,
            ParalyzeImmune,
            Petrify,
            PetrifyImmune,
            PetDisable,
            Curse,
            CurseImmune,
            HpBoost,
            MpBoost,
            AttBoost,
            DefBoost,
            SpdBoost,
            DexBoost,
            VitBoost,
            WisBoost,
            Hidden,
            Muted,
            PartyVision,
            XMasVision,
        ]
    };

    pub fn from_index(index: u16) -> Option<ConditionEffect> {
        Self::ALL.get(index as usize).copied()
    }

    pub fn index(self) -> u16 {
        self as u16
    }

    /// The spelling used in the content XML.
    ///
    /// The files are inconsistent about spacing and case (`"Armor Broken"`, `"ArmorBroken"`,
    /// `"armor broken"` all appear), so matching ignores both.
    pub fn from_name(name: &str) -> Option<ConditionEffect> {
        let wanted: String = name
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
            .flat_map(char::to_lowercase)
            .collect();

        Self::ALL.into_iter().find(|effect| {
            let candidate: String = effect.name().chars().flat_map(char::to_lowercase).collect();
            candidate == wanted
        })
    }

    /// The canonical name, matching the client's enum.
    pub fn name(self) -> &'static str {
        use ConditionEffect::*;
        match self {
            Dead => "Dead",
            Quiet => "Quiet",
            Weak => "Weak",
            Slowed => "Slowed",
            Sick => "Sick",
            Dazed => "Dazed",
            Stunned => "Stunned",
            Blind => "Blind",
            Hallucinating => "Hallucinating",
            Drunk => "Drunk",
            Confused => "Confused",
            StunImmune => "StunImmune",
            Invisible => "Invisible",
            Paralyzed => "Paralyzed",
            Speedy => "Speedy",
            Bleeding => "Bleeding",
            ArmorBreakImmune => "ArmorBreakImmune",
            Healing => "Healing",
            Damaging => "Damaging",
            Berserk => "Berserk",
            Paused => "Paused",
            Stasis => "Stasis",
            StasisImmune => "StasisImmune",
            Invincible => "Invincible",
            Invulnerable => "Invulnerable",
            Armored => "Armored",
            ArmorBroken => "ArmorBroken",
            Hexed => "Hexed",
            NinjaSpeedy => "NinjaSpeedy",
            Unstable => "Unstable",
            Darkness => "Darkness",
            SlowedImmune => "SlowedImmune",
            DazedImmune => "DazedImmune",
            ParalyzeImmune => "ParalyzeImmune",
            Petrify => "Petrify",
            PetrifyImmune => "PetrifyImmune",
            PetDisable => "PetDisable",
            Curse => "Curse",
            CurseImmune => "CurseImmune",
            HpBoost => "HPBoost",
            MpBoost => "MPBoost",
            AttBoost => "AttBoost",
            DefBoost => "DefBoost",
            SpdBoost => "SpdBoost",
            DexBoost => "DexBoost",
            VitBoost => "VitBoost",
            WisBoost => "WisBoost",
            Hidden => "Hidden",
            Muted => "Muted",
            PartyVision => "PartyVision",
            XMasVision => "XMasVision",
        }
    }
}

impl fmt::Display for ConditionEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A set of condition effects, as the bitmask the wire carries.
///
/// Effects are read and written every tick for every visible entity, so this is a plain integer
/// rather than a collection. A set operation is a couple of instructions and copying it is free.
///
/// On the wire this is never sent as sixteen fixed bytes. Almost every entity carries no effects
/// at all, so it is written as a leading byte count followed by only the non-zero bytes, and it is
/// only written when it changed since the last tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ConditionSet(pub u128);

impl ConditionSet {
    pub const EMPTY: ConditionSet = ConditionSet(0);

    pub fn contains(self, effect: ConditionEffect) -> bool {
        self.0 & (1u128 << effect.index()) != 0
    }

    pub fn insert(&mut self, effect: ConditionEffect) {
        self.0 |= 1u128 << effect.index();
    }

    pub fn remove(&mut self, effect: ConditionEffect) {
        self.0 &= !(1u128 << effect.index());
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether any effect in `other` is also in this set.
    pub fn intersects(self, other: ConditionSet) -> bool {
        self.0 & other.0 != 0
    }

    /// Effects in this set but not in `other`: what turned on since the last tick.
    pub fn gained(self, other: ConditionSet) -> ConditionSet {
        ConditionSet(self.0 & !other.0)
    }

    /// How many effects are set. Used by the encoder to decide whether to write anything at all.
    pub fn len(self) -> u32 {
        self.0.count_ones()
    }

    pub fn iter(self) -> impl Iterator<Item = ConditionEffect> {
        ConditionEffect::ALL
            .into_iter()
            .filter(move |&effect| self.contains(effect))
    }
}

impl FromIterator<ConditionEffect> for ConditionSet {
    fn from_iter<I: IntoIterator<Item = ConditionEffect>>(iter: I) -> Self {
        let mut set = ConditionSet::EMPTY;
        for effect in iter {
            set.insert(effect);
        }
        set
    }
}

/// A condition effect as declared on a projectile or item, with how long it lasts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppliedEffect {
    pub effect: ConditionEffect,
    pub duration_ms: i32,
    pub range: f32,

    /// Declared `target="1"`: applied by a pet to its owner rather than by the shooter to what it
    /// hits.
    pub targets_owner: bool,
}

impl AppliedEffect {
    pub fn parse(node: &crate::xml::Node) -> Option<AppliedEffect> {
        let name = if node.text.is_empty() {
            node.attr("effect")?
        } else {
            node.text.as_str()
        };

        Some(AppliedEffect {
            effect: ConditionEffect::from_name(name)?,
            // Written in seconds as a float; the simulation counts milliseconds.
            duration_ms: node
                .attr_float("duration")
                .map(|seconds| (seconds * 1000.0) as i32)
                .unwrap_or(0),
            range: node.attr_float("range").unwrap_or(0.0) as f32,
            targets_owner: node.attr_int("target").unwrap_or(0) == 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_match_their_position() {
        for (index, effect) in ConditionEffect::ALL.into_iter().enumerate() {
            assert_eq!(effect.index() as usize, index, "{effect} is misnumbered");
        }
    }

    #[test]
    fn names_round_trip_through_the_spellings_the_content_uses() {
        assert_eq!(
            ConditionEffect::from_name("Armor Broken"),
            Some(ConditionEffect::ArmorBroken)
        );
        assert_eq!(
            ConditionEffect::from_name("armorbroken"),
            Some(ConditionEffect::ArmorBroken)
        );
        assert_eq!(
            ConditionEffect::from_name("HPBoost"),
            Some(ConditionEffect::HpBoost)
        );
        assert_eq!(ConditionEffect::from_name("Nonsense"), None);
    }

    #[test]
    fn the_mask_is_wide_enough_for_every_effect() {
        let mut set = ConditionSet::EMPTY;
        for effect in ConditionEffect::ALL {
            set.insert(effect);
        }
        assert_eq!(set.iter().count(), CONDITION_EFFECT_COUNT);

        set.remove(ConditionEffect::Dead);
        assert!(!set.contains(ConditionEffect::Dead));
        assert!(set.contains(ConditionEffect::XMasVision));
    }
}
