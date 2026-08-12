using System;
using System.Collections.Generic;
using System.Globalization;
using Hendra.Data;
using Hendra.Resources;

namespace Hendra.UI;

/// <summary>
/// What an item's tooltip says, as lines. <see cref="ItemTooltipPanel"/> decides how they look.
/// </summary>
/// <remarks>
/// <para>
/// Kept apart from the panel so the wording can be exercised without an engine: the panel is a
/// Control and needs a scene tree, these are strings and need nothing.
/// </para>
/// <para>
/// The phrasing is the original's, from the same language keys — "Damage: {damage}",
/// "Range: {range}", "Shots: {numShots}", "MP Cost: {cost}", "On Equip:". Each falls back to the
/// English the table would have supplied, since it is fetched over HTTP and is not there at once.
/// </para>
/// </remarks>
public static class ItemTooltip
{
    /// <summary>A line, and whether it is a value or a heading over other lines.</summary>
    public readonly struct Line
    {
        public readonly string Text;
        public readonly bool IsHeading;

        public Line(string text, bool isHeading = false)
        {
            Text = text;
            IsHeading = isHeading;
        }
    }

    private static readonly Dictionary<int, string> StatNames = new()
    {
        [(int)StatsType.MaxHp] = "Max HP",
        [(int)StatsType.MaxMp] = "Max MP",
        [(int)StatsType.Attack] = "Attack",
        [(int)StatsType.Defense] = "Defense",
        [(int)StatsType.Speed] = "Speed",
        [(int)StatsType.Dexterity] = "Dexterity",
        [(int)StatsType.Vitality] = "Vitality",
        [(int)StatsType.Wisdom] = "Wisdom",
    };

    /// <summary>
    /// Everything the item does, in the order the original lists it.
    /// </summary>
    /// <remarks>
    /// What appears is decided by what the item is — a weapon by its shot, an ability by its cost,
    /// armour by what it adds. Listing every field for every item would bury the one line that
    /// matters under a dozen reading "0".
    /// </remarks>
    public static List<Line> Effects(ObjectDesc desc, Text.StringMap strings = null)
    {
        var lines = new List<Line>();
        if (desc == null)
            return lines;

        var shot = FirstProjectile(desc);
        if (shot != null)
        {
            string damage = shot.MinDamage == shot.MaxDamage
                ? shot.MinDamage.ToString(CultureInfo.InvariantCulture)
                : $"{shot.MinDamage} - {shot.MaxDamage}";

            lines.Add(new Line(Fill(strings, "EquipmentToolTip.damage", "Damage: {damage}", "damage", damage)));

            // The original's own arithmetic: speed times lifetime over ten thousand, in tiles.
            float range = shot.Speed * shot.LifetimeMs / 10000f;
            lines.Add(new Line(Fill(strings, "EquipmentToolTip.range", "Range: {range}", "range", Round(range))));

            if (shot.MultiHit)
                lines.Add(new Line(Get(strings, "GeneralProjectileComparison.multiHit", "Shots hit multiple targets")));

            if (shot.ArmorPiercing)
                lines.Add(new Line(Get(strings, "GeneralProjectileComparison.armorPiercing", "Ignores defense of target")));

            if (shot.PassesCover)
                lines.Add(new Line(Get(strings, "GeneralProjectileComparison.passesCover", "Shots pass through obstacles")));
        }

        if (desc.NumProjectiles > 1)
        {
            lines.Add(new Line(Fill(strings, "EquipmentToolTip.shots", "Shots: {numShots}", "numShots",
                desc.NumProjectiles.ToString(CultureInfo.InvariantCulture))));
        }

        if (MathF.Abs(desc.RateOfFire - 1f) > 0.005f)
        {
            lines.Add(new Line(Fill(strings, "EquipmentToolTip.rateOfFire", "Rate of Fire: {data}", "data",
                $"{(int)MathF.Round(desc.RateOfFire * 100f)}%")));
        }

        if (desc.MpCost > 0)
        {
            lines.Add(new Line(Fill(strings, "EquipmentToolTip.mpCost", "MP Cost: {cost}", "cost",
                desc.MpCost.ToString(CultureInfo.InvariantCulture))));
        }

        if (desc.EquipBonuses.Length == 0)
            return lines;

        lines.Add(new Line(Get(strings, "EquipmentToolTip.onEquip", "On Equip:"), isHeading: true));

        foreach (var (stat, amount) in desc.EquipBonuses)
        {
            string name = StatNames.TryGetValue(stat, out string known) ? known : $"Stat {stat}";
            lines.Add(new Line($"  {(amount >= 0 ? "+" : string.Empty)}{amount} {name}"));
        }

        return lines;
    }

    /// <summary>What stops you using it: whether it binds, and whether it is spent.</summary>
    public static List<string> Restrictions(ObjectDesc desc, Text.StringMap strings = null)
    {
        var lines = new List<string>();
        if (desc == null)
            return lines;

        if (desc.Consumable)
            lines.Add(Get(strings, "EquipmentToolTip.consumedWithUse", "Consumed with use"));

        if (desc.Soulbound)
            lines.Add(Get(strings, "Item.Soulbound", "Soulbound"));

        return lines;
    }

    /// <summary>
    /// The tag at the top right.
    /// </summary>
    /// <remarks>
    /// The original's TierUtil: the tier if the item has one, UT if it has none, and nothing at all
    /// for consumables, treasure and pet food, which have no notion of quality.
    /// </remarks>
    public static string TierTag(ObjectDesc desc)
    {
        if (desc == null || desc.Consumable)
            return null;

        return desc.Tier >= 0 ? $"T{desc.Tier}" : "UT";
    }

    /// <summary>
    /// The shot an item fires, or null if it fires none.
    /// </summary>
    /// <remarks>
    /// Items are keyed by bullet type because a few fire more than one kind, but every weapon in
    /// this data set has exactly one and the first is the one to describe.
    /// </remarks>
    public static ProjectileDesc FirstProjectile(ObjectDesc desc)
    {
        if (desc?.Projectiles == null)
            return null;

        foreach (var shot in desc.Projectiles.Values)
            return shot;

        return null;
    }

    private static string Fill(Text.StringMap strings, string key, string fallback, string token, string value) =>
        Get(strings, key, fallback).Replace("{" + token + "}", value);

    private static string Get(Text.StringMap strings, string key, string fallback) =>
        strings != null && strings.Has(key) ? strings.Get(key) : fallback;

    private static string Round(float value) => value.ToString("0.#", CultureInfo.InvariantCulture);
}
