using System.Collections.Generic;
using System.Globalization;
using Hendra.Data;
using Hendra.Resources;

namespace Hendra.UI;

/// <summary>
/// The text shown when the pointer rests on an item.
/// </summary>
/// <remarks>
/// <para>
/// The original draws a panel here rather than a string: a title bar in the tier's colour, the slot
/// it goes in, a table of what it does, the flavour line, and a footer of who may use it. This is
/// the same information as text, which Godot's tooltip already knows how to place and size.
/// </para>
/// <para>
/// What it says is decided by what the item is. A weapon is described by its damage and rate of
/// fire, an ability by what it costs, a piece of armour by what it adds — listing every field for
/// every item would bury the one line that matters under a dozen that read "0".
/// </para>
/// </remarks>
public static class ItemTooltip
{
    /// <summary>The slot numbers, as the XML's SlotType uses them.</summary>
    private static readonly Dictionary<int, string> SlotNames = new()
    {
        [1] = "Sword", [2] = "Dagger", [3] = "Bow", [4] = "Tome", [5] = "Shield",
        [6] = "Leather armour", [7] = "Heavy armour", [8] = "Wand", [9] = "Ring",
        [10] = "Potion", [11] = "Spell", [12] = "Seal", [13] = "Cloak", [14] = "Robe",
        [15] = "Quiver", [16] = "Helm", [17] = "Staff", [18] = "Poison", [19] = "Skull",
        [20] = "Trap", [21] = "Orb", [22] = "Prism", [23] = "Scepter", [24] = "Katana",
        [25] = "Shuriken",
    };

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

    /// <summary>Describes an item, or returns null if there is nothing to describe.</summary>
    public static string Describe(ObjectDesc desc)
    {
        if (desc == null)
            return null;

        var lines = new List<string>();

        string name = desc.DisplayId ?? desc.Id ?? "Unknown item";
        lines.Add(desc.Tier >= 0 ? $"{name}   T{desc.Tier}" : name);

        string slot = SlotName(desc.SlotType);
        if (slot != null)
            lines.Add(slot);

        // A weapon: what it hits for, and how often. Rate of fire is a multiplier on the player's
        // own, so it is shown as the percentage the original shows rather than the raw number.
        var shot = FirstProjectile(desc);
        if (shot != null)
        {
            lines.Add(shot.MinDamage == shot.MaxDamage
                ? $"Damage: {shot.MinDamage}"
                : $"Damage: {shot.MinDamage}–{shot.MaxDamage}");

            if (desc.NumProjectiles > 1)
                lines.Add($"Shots: {desc.NumProjectiles}");

            if (!Near(desc.RateOfFire, 1f))
                lines.Add($"Rate of fire: {Percent(desc.RateOfFire)}");

            if (shot.Speed > 0 && shot.LifetimeMs > 0)
                lines.Add($"Range: {Round(shot.Speed / 10f * (shot.LifetimeMs / 1000f))}");
        }

        foreach (var (stat, amount) in desc.EquipBonuses)
        {
            string statName = StatNames.TryGetValue(stat, out string known) ? known : $"Stat {stat}";
            lines.Add($"{(amount >= 0 ? "+" : string.Empty)}{amount} {statName}");
        }

        if (desc.MpCost > 0)
            lines.Add($"MP cost: {desc.MpCost}");

        if (desc.CooldownMs > 0)
            lines.Add($"Cooldown: {Round(desc.CooldownMs / 1000f)}s");

        if (desc.Consumable)
            lines.Add("Consumed on use");

        if (desc.Soulbound)
            lines.Add("Soulbound");

        if (!string.IsNullOrWhiteSpace(desc.Description))
        {
            lines.Add(string.Empty);
            lines.Add(desc.Description);
        }

        if (desc.FeedPower > 0)
        {
            lines.Add(string.Empty);
            lines.Add($"Feed power: {desc.FeedPower}");
        }

        return string.Join("\n", lines);
    }

    /// <summary>
    /// The shot an item fires, or null if it fires none.
    /// </summary>
    /// <remarks>
    /// Items are keyed by bullet type because a few fire more than one kind, but every weapon in
    /// this data set has exactly one and the first is the one to describe.
    /// </remarks>
    private static ProjectileDesc FirstProjectile(ObjectDesc desc)
    {
        if (desc.Projectiles == null)
            return null;

        foreach (var shot in desc.Projectiles.Values)
            return shot;

        return null;
    }

    private static string SlotName(int slotType) =>
        SlotNames.TryGetValue(slotType, out string name) ? name : null;

    private static bool Near(float value, float target) => System.MathF.Abs(value - target) < 0.005f;

    private static string Percent(float multiplier) =>
        $"{Godot.Mathf.RoundToInt(multiplier * 100f).ToString(CultureInfo.InvariantCulture)}%";

    private static string Round(float value) =>
        value.ToString("0.#", CultureInfo.InvariantCulture);
}
