using System;
using System.Collections.Generic;
using System.Globalization;
using System.Xml.Linq;

namespace Hendra.Account;

/// <summary>One fame bonus and what it was awarded for.</summary>
public readonly struct FameBonus
{
    /// <summary>The bonus's key, e.g. <c>ancient_curse</c>.</summary>
    public readonly string Id;

    /// <summary>The server's own prose for it, ready to show.</summary>
    public readonly string Description;

    public readonly int Fame;

    public FameBonus(string id, string description, int fame)
    {
        Id = id;
        Description = description;
        Fame = fame;
    }
}

/// <summary>
/// What a character did with its life: the tally <c>/char/fame</c> returns once it is dead.
/// </summary>
/// <remarks>
/// <para>
/// The endpoint answers <c>&lt;Error&gt;Character not dead&lt;/Error&gt;</c> for a living character,
/// so it is only ever asked after a Death packet — which is also where the account and character ids
/// come from. The client never otherwise knows its own numeric account id when it has connected
/// straight to a world rather than through the character list.
/// </para>
/// <para>
/// The bonuses are computed and worded server-side, so they are shown as sent rather than being
/// re-derived here. That also means a server that adds a bonus needs no client change.
/// </para>
/// </remarks>
public sealed class FameResult
{
    public string CharacterName = string.Empty;
    public string KilledBy = string.Empty;
    public ushort ObjectType;
    public int Level;
    public int Experience;

    /// <summary>Fame the character had banked before the bonuses were applied.</summary>
    public int BaseFame;

    /// <summary>What the death was finally worth.</summary>
    public int TotalFame;

    public readonly List<FameBonus> Bonuses = new();

    // The tallies worth showing. The response carries about thirty; these are the ones the original
    // put on the death screen, in its order.
    public int Shots;
    public int ShotsThatDamage;
    public int MonsterKills;
    public int MonsterAssists;
    public int GodKills;
    public int GodAssists;
    public int OryxKills;
    public int CubeKills;
    public int QuestsCompleted;
    public int SpecialAbilityUses;
    public int PotionsDrunk;
    public int Teleports;
    public int TilesUncovered;
    public int LevelUpAssists;
    public int MinutesActive;

    /// <summary>Accuracy, as a fraction. Zero when nothing was fired.</summary>
    public float Accuracy => Shots == 0 ? 0f : ShotsThatDamage / (float)Shots;

    public static FameResult Parse(string xml)
    {
        var root = XDocument.Parse(xml).Root
                   ?? throw new FormatException("Fame response has no root element.");

        var result = new FameResult
        {
            KilledBy = Text(root, "KilledBy") ?? string.Empty,
            BaseFame = Int(Text(root, "BaseFame")),
            TotalFame = Int(Text(root, "TotalFame")),

            Shots = Int(Text(root, "Shots")),
            ShotsThatDamage = Int(Text(root, "ShotsThatDamage")),
            MonsterKills = Int(Text(root, "MonsterKills")),
            MonsterAssists = Int(Text(root, "MonsterAssists")),
            GodKills = Int(Text(root, "GodKills")),
            GodAssists = Int(Text(root, "GodAssists")),
            OryxKills = Int(Text(root, "OryxKills")),
            CubeKills = Int(Text(root, "CubeKills")),
            QuestsCompleted = Int(Text(root, "QuestsCompleted")),
            SpecialAbilityUses = Int(Text(root, "SpecialAbilityUses")),
            PotionsDrunk = Int(Text(root, "PotionsDrunk")),
            Teleports = Int(Text(root, "Teleports")),
            TilesUncovered = Int(Text(root, "TilesUncovered")),
            LevelUpAssists = Int(Text(root, "LevelUpAssists")),
            MinutesActive = Int(Text(root, "MinutesActive")),
        };

        // The character element is the same shape /char/list returns, with the account name grafted
        // on. Only three fields off it are wanted here.
        var character = root.Element("Char");
        if (character != null)
        {
            result.ObjectType = (ushort)Int(Text(character, "ObjectType"));
            result.Level = Int(Text(character, "Level"));
            result.Experience = Int(Text(character, "Exp"));
            result.CharacterName = Text(character.Element("Account"), "Name") ?? string.Empty;
        }

        foreach (var bonus in root.Elements("Bonus"))
        {
            result.Bonuses.Add(new FameBonus(
                bonus.Attribute("id")?.Value ?? string.Empty,
                bonus.Attribute("desc")?.Value ?? string.Empty,
                Int(bonus.Value)));
        }

        return result;
    }

    private static string Text(XElement parent, string name)
    {
        string value = parent?.Element(name)?.Value;
        return string.IsNullOrWhiteSpace(value) ? null : value.Trim();
    }

    private static int Int(string text) =>
        int.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out int value) ? value : 0;
}
