using System;

namespace Hendra.Data;

/// <summary>
/// Status effects, as a single 64-bit mask mirroring wServer's <c>ConditionEffects</c>.
/// Bit <c>i</c> corresponds to <see cref="ConditionEffectIndex"/> <c>i</c>.
/// </summary>
/// <remarks>
/// The AS3 client modelled this as two separate 32-bit words with their own id numbering, offset by
/// one from the server's (it reserved id 0 for "Nothing"), and every predicate had to know which
/// word its bit lived in. One flags enum matching the server is both simpler and removes a whole
/// class of "checked the wrong batch" bugs. See <see cref="ConditionEffectsCodec"/> for the
/// two-field wire encoding.
/// </remarks>
[Flags]
public enum ConditionEffects : ulong
{
    None = 0,

    Dead = 1UL << 0,
    Quiet = 1UL << 1,
    Weak = 1UL << 2,
    Slowed = 1UL << 3,
    Sick = 1UL << 4,
    Dazed = 1UL << 5,
    Stunned = 1UL << 6,
    Blind = 1UL << 7,
    Hallucinating = 1UL << 8,
    Drunk = 1UL << 9,
    Confused = 1UL << 10,
    StunImmune = 1UL << 11,
    Invisible = 1UL << 12,
    Paralyzed = 1UL << 13,
    Speedy = 1UL << 14,
    Bleeding = 1UL << 15,
    ArmorBreakImmune = 1UL << 16,
    Healing = 1UL << 17,
    Damaging = 1UL << 18,
    Berserk = 1UL << 19,
    Paused = 1UL << 20,
    Stasis = 1UL << 21,
    StasisImmune = 1UL << 22,
    Invincible = 1UL << 23,
    Invulnerable = 1UL << 24,
    Armored = 1UL << 25,
    ArmorBroken = 1UL << 26,
    Hexed = 1UL << 27,
    NinjaSpeedy = 1UL << 28,
    Unstable = 1UL << 29,
    Darkness = 1UL << 30,
    SlowedImmune = 1UL << 31,
    DazedImmune = 1UL << 32,
    ParalyzeImmune = 1UL << 33,
    Petrify = 1UL << 34,
    PetrifyImmune = 1UL << 35,
    PetDisable = 1UL << 36,
    Curse = 1UL << 37,
    CurseImmune = 1UL << 38,
    HpBoost = 1UL << 39,
    MpBoost = 1UL << 40,
    AttBoost = 1UL << 41,
    DefBoost = 1UL << 42,
    SpdBoost = 1UL << 43,
    DexBoost = 1UL << 44,
    VitBoost = 1UL << 45,
    WisBoost = 1UL << 46,
    Hidden = 1UL << 47,
    Muted = 1UL << 48,
    PartyVision = 1UL << 49,
    XMasVision = 1UL << 50,

    /// <summary>Effects that make the screen post-process. Drives the blur / desaturate passes.</summary>
    MapFilterMask = Drunk | Blind | Paused,

    /// <summary>An entity carrying any of these cannot be hit by a projectile.</summary>
    ProjectileNoHitMask = Invincible | Stasis | Paused,
}

/// <summary>
/// Effect identifiers as they appear in XML and in the Damage / Aoe packets, mirroring wServer's
/// <c>ConditionEffectIndex</c>. The value is the bit position in <see cref="ConditionEffects"/>.
/// </summary>
public enum ConditionEffectIndex
{
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

    /// <summary>
    /// Client-only pseudo-effect. Never appears on the wire; the AS3 client used it to tag damage
    /// text originating from a damaging tile so it renders in the armour-piercing colour.
    /// </summary>
    GroundDamage = 99,
}

/// <summary>
/// Converts between the 64-bit mask and the split pair of int stats the protocol carries.
/// </summary>
/// <remarks>
/// The server splits its ulong as <c>Effects = (int)value</c> and
/// <c>Effects2 = (int)((ulong)value &gt;&gt; 31)</c> (wServer/realm/Entity.cs). Note the shift is 31,
/// not 32, so bit 31 (SlowedImmune) is deliberately present in *both* fields. Reassembling with a
/// 32-bit shift would misplace every effect from SlowedImmune upward.
/// </remarks>
public static class ConditionEffectsCodec
{
    /// <summary>Rebuilds the mask from the Effects (stat 30) and Effects2 (stat 82) values.</summary>
    public static ConditionEffects Combine(int effects, int effects2)
    {
        // Cast through uint so the int sign bit does not sign-extend across the whole ulong.
        ulong low = (uint)effects;
        ulong high = (ulong)(uint)effects2 << 31;
        return (ConditionEffects)(low | high);
    }

    /// <summary>Replaces the low half of <paramref name="current"/> from a new Effects value.</summary>
    public static ConditionEffects WithEffects(ConditionEffects current, int effects)
    {
        ulong preserved = (ulong)current & ~0xFFFFFFFFUL;
        return (ConditionEffects)(preserved | (uint)effects);
    }

    /// <summary>Replaces the high half of <paramref name="current"/> from a new Effects2 value.</summary>
    public static ConditionEffects WithEffects2(ConditionEffects current, int effects2)
    {
        // Bits 0..30 of the mask are carried only by Effects; everything from bit 31 up is
        // re-derived from Effects2.
        ulong preserved = (ulong)current & 0x7FFFFFFFUL;
        return (ConditionEffects)(preserved | ((ulong)(uint)effects2 << 31));
    }

    public static bool Has(this ConditionEffects effects, ConditionEffects flag) =>
        (effects & flag) != 0;

    public static ConditionEffects ToFlag(this ConditionEffectIndex index) =>
        (ConditionEffects)(1UL << (int)index);
}
