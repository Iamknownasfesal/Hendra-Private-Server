using System;
using System.Collections.Generic;
using System.Globalization;
using System.Xml.Linq;

namespace Hendra.Account;

/// <summary>The guild ranks the server recognises, and what they are allowed to do.</summary>
public static class GuildRank
{
    public const int Initiate = 0;
    public const int Member = 10;
    public const int Officer = 20;
    public const int Leader = 30;
    public const int Founder = 40;

    /// <summary>The rank's name, as the game shows it.</summary>
    public static string Name(int rank) => rank switch
    {
        >= Founder => "Founder",
        >= Leader => "Leader",
        >= Officer => "Officer",
        >= Member => "Member",
        _ => "Initiate",
    };

    /// <summary>
    /// Whether someone of this rank may invite, remove and promote.
    /// </summary>
    /// <remarks>
    /// Officer and above. The server checks this too and answers a refusal with a message, so this
    /// only decides whether the buttons are worth offering.
    /// </remarks>
    public static bool CanManage(int rank) => rank >= Officer;

    /// <summary>The ranks a manager can set someone else to, lowest first.</summary>
    public static readonly int[] Assignable = { Initiate, Member, Officer, Leader };
}

/// <summary>One person in a guild.</summary>
public readonly struct GuildMemberInfo
{
    public readonly string Name;
    public readonly int Rank;
    public readonly int Fame;

    /// <summary>When they were last seen, as a Unix timestamp. Negative means never.</summary>
    public readonly long LastSeen;

    public GuildMemberInfo(string name, int rank, int fame, long lastSeen)
    {
        Name = name;
        Rank = rank;
        Fame = fame;
        LastSeen = lastSeen;
    }

    /// <summary>Whether the server considers this member to be online right now.</summary>
    /// <remarks>The server stores -1 while an account is connected rather than a time.</remarks>
    public bool IsOnline => LastSeen < 0;
}

/// <summary>
/// A guild and its roll, from <c>/guild/listMembers</c>.
/// </summary>
/// <remarks>
/// The roll is already sorted by the server — rank first, then fame, then name — so it is shown in
/// the order it arrives rather than sorted again here. Re-sorting would only be a chance to
/// disagree with the leaderboard the server shows elsewhere.
/// </remarks>
public sealed class GuildResult
{
    public int Id;
    public string Name = string.Empty;
    public int CurrentFame;
    public int TotalFame;
    public string HallType = string.Empty;

    public readonly List<GuildMemberInfo> Members = new();

    public static GuildResult Parse(string xml)
    {
        var root = XDocument.Parse(xml).Root
                   ?? throw new FormatException("Guild response has no root element.");

        var result = new GuildResult
        {
            Id = Int(root.Attribute("id")?.Value),
            Name = root.Attribute("name")?.Value ?? string.Empty,
            CurrentFame = Int(Text(root, "CurrentFame")),
            TotalFame = Int(Text(root, "TotalFame")),
            HallType = Text(root, "HallType") ?? string.Empty,
        };

        foreach (var member in root.Elements("Member"))
        {
            result.Members.Add(new GuildMemberInfo(
                Text(member, "Name") ?? string.Empty,
                Int(Text(member, "Rank")),
                Int(Text(member, "Fame")),
                Long(Text(member, "LastSeen"))));
        }

        return result;
    }

    /// <summary>This account's own rank in the guild, or -1 if the name is not on the roll.</summary>
    public int RankOf(string accountName)
    {
        foreach (var member in Members)
        {
            if (string.Equals(member.Name, accountName, StringComparison.OrdinalIgnoreCase))
                return member.Rank;
        }

        return -1;
    }

    private static string Text(XElement parent, string name)
    {
        string value = parent?.Element(name)?.Value;
        return string.IsNullOrWhiteSpace(value) ? null : value.Trim();
    }

    private static int Int(string text) =>
        int.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out int value) ? value : 0;

    private static long Long(string text) =>
        long.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out long value) ? value : 0;
}
