using System.Collections.Generic;
using Hendra.Data;

namespace Hendra.Render;

/// <summary>
/// Which icon stands for each status effect, and where it comes from.
/// </summary>
/// <remarks>
/// <para>
/// The original's <c>ConditionEffect.effects_</c> table: every effect that shows an icon names one
/// or more sprite indices into the <c>lofiInterface2</c> sheet, and an effect with several cycles
/// through them. The ones with no icon are the ones you are not meant to see — immunities, the
/// invisible flag, being dead.
/// </para>
/// <para>
/// The animation rate is the original's too: the frame is <c>time / 500</c>, so a multi-frame icon
/// steps twice a second, and it steps in lockstep across every entity on screen because it is
/// driven by the clock rather than by anything per-entity.
/// </para>
/// </remarks>
public static class ConditionIcons
{
    /// <summary>The sheet the original takes them from.</summary>
    public const string Sheet = "lofiInterface2";

    /// <summary>How long each frame of a multi-frame icon is held.</summary>
    public const int FrameMs = 500;

    /// <summary>
    /// The effects worth showing, in the order the original lists them.
    /// </summary>
    /// <remarks>
    /// Order matters only in that it decides the order icons appear in the row, and keeping the
    /// original's means a stack of effects reads the same way it does there.
    /// </remarks>
    private static readonly (ConditionEffects Effect, int[] Frames)[] Table =
    {
        (ConditionEffects.Quiet, new[] { 32 }),
        (ConditionEffects.Weak, new[] { 34, 35, 36, 37 }),
        (ConditionEffects.Slowed, new[] { 1 }),
        (ConditionEffects.Sick, new[] { 39 }),
        (ConditionEffects.Dazed, new[] { 44 }),
        (ConditionEffects.Stunned, new[] { 45 }),
        (ConditionEffects.Blind, new[] { 41 }),
        (ConditionEffects.Hallucinating, new[] { 42 }),
        (ConditionEffects.Drunk, new[] { 43 }),
        (ConditionEffects.Confused, new[] { 2 }),
        (ConditionEffects.Paralyzed, new[] { 53, 54 }),
        (ConditionEffects.Speedy, new[] { 0 }),
        (ConditionEffects.Bleeding, new[] { 46 }),
        (ConditionEffects.Healing, new[] { 47 }),
        (ConditionEffects.Damaging, new[] { 49 }),
        (ConditionEffects.Berserk, new[] { 50 }),
        (ConditionEffects.Invulnerable, new[] { 17 }),
        (ConditionEffects.Armored, new[] { 16 }),
        (ConditionEffects.ArmorBroken, new[] { 55 }),
        (ConditionEffects.Hexed, new[] { 42 }),
        (ConditionEffects.NinjaSpeedy, new[] { 0 }),
        (ConditionEffects.Unstable, new[] { 56 }),
        (ConditionEffects.Darkness, new[] { 57 }),
        (ConditionEffects.Curse, new[] { 58 }),
    };

    /// <summary>
    /// The sprite indices to show for an entity's current effects.
    /// </summary>
    /// <param name="conditions">Everything currently affecting the entity.</param>
    /// <param name="nowMs">The frame clock, which drives multi-frame icons.</param>
    /// <param name="into">Filled with one index per effect worth showing. Cleared first.</param>
    public static void Collect(ConditionEffects conditions, int nowMs, List<int> into)
    {
        into.Clear();
        if (conditions == ConditionEffects.None)
            return;

        int frame = nowMs / FrameMs;

        foreach (var (effect, frames) in Table)
        {
            if ((conditions & effect) == 0)
                continue;

            // Modulo of a negative would index backwards off the array; the clock starts at zero
            // and only climbs, but the guard costs nothing.
            into.Add(frames[(frame % frames.Length + frames.Length) % frames.Length]);
        }
    }
}
