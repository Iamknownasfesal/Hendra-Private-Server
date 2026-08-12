using System;
using System.Collections.Generic;

namespace Hendra.Account;

/// <summary>
/// A living character's tallies, as the character list carries them.
/// </summary>
/// <remarks>
/// <para>
/// The server keeps these in one blob per character -- <c>common/FameStats.cs</c> -- and writes it
/// base64 into the <c>PCStats</c> element of the character list. It is the same set the death
/// screen shows, except that this one is readable while the character is still alive, which
/// <c>/char/fame</c> is not: that endpoint answers "Character not dead" and nothing else.
/// </para>
/// <para>
/// The format is a stream of records: one byte of identity, then a big-endian int. Unknown ids are
/// skipped rather than treated as an error, because the server may add tallies and an older client
/// should show the ones it knows rather than nothing at all.
/// </para>
/// </remarks>
public sealed class CharacterStats
{
    /// <summary>One tally, in the order the server wrote it.</summary>
    public readonly struct Entry
    {
        public readonly int Id;
        public readonly int Value;

        public Entry(int id, int value)
        {
            Id = id;
            Value = value;
        }
    }

    private readonly List<Entry> _entries = new();

    /// <summary>Every tally that is not a dungeon, in server order.</summary>
    public IReadOnlyList<Entry> Statistics => _statistics;

    /// <summary>Every dungeon's completion count, in server order.</summary>
    public IReadOnlyList<Entry> Dungeons => _dungeons;

    private readonly List<Entry> _statistics = new();
    private readonly List<Entry> _dungeons = new();

    /// <summary>How many minutes the character has been played, which the header shows.</summary>
    public int MinutesActive { get; private set; }

    /// <summary>
    /// Reads the blob. Never throws: a truncated or unfamiliar one yields what could be read.
    /// </summary>
    /// <remarks>
    /// Deliberately forgiving. This arrives over HTTP from a server that may be a different build,
    /// and the character panel refusing to open because one trailing byte was odd would be a worse
    /// outcome than a panel missing its last row.
    /// </remarks>
    public static CharacterStats Parse(string base64)
    {
        var stats = new CharacterStats();
        if (string.IsNullOrWhiteSpace(base64))
            return stats;

        byte[] bytes;
        try
        {
            bytes = Convert.FromBase64String(base64.Trim());
        }
        catch (FormatException)
        {
            return stats;
        }

        // Five bytes a record: the identity and a big-endian int, which is what the server's NReader
        // writes -- network order, not the machine's.
        for (int at = 0; at + 4 < bytes.Length; at += 5)
        {
            int id = bytes[at];
            int value = bytes[at + 1] << 24 | bytes[at + 2] << 16 | bytes[at + 3] << 8 | bytes[at + 4];

            stats._entries.Add(new Entry(id, value));

            if (id == MinutesActiveId)
                stats.MinutesActive = value;

            if (IsDungeon(id))
                stats._dungeons.Add(new Entry(id, value));
            else
                stats._statistics.Add(new Entry(id, value));
        }

        return stats;
    }

    /// <summary>Everything the blob carried, dungeons included, in the order it carried it.</summary>
    public IReadOnlyList<Entry> All => _entries;

    private const int MinutesActiveId = 20;

    /// <summary>
    /// Which ids are dungeon completions rather than statistics.
    /// </summary>
    /// <remarks>
    /// The server's own numbering, and the reason the two tabs can be filled from one blob. It is a
    /// range test rather than a table because the server allocated them as two contiguous runs.
    /// </remarks>
    private static bool IsDungeon(int id) => id is >= 13 and <= 18 or >= 21 and <= 24;

    /// <summary>
    /// What each id is called, and which tier a dungeon belongs to.
    /// </summary>
    /// <remarks>
    /// The names are the server's field names turned into words. A real localisation table keyed by
    /// id is what the brief asks for and what should replace this the moment the language file
    /// carries these keys -- <see cref="Label"/> is the single place to change.
    /// </remarks>
    private static readonly (string Label, string Tier)[] Names =
    {
        ("Shots Fired", null),
        ("Shots That Damage", null),
        ("Abilities Used", null),
        ("Tiles Uncovered", null),
        ("Teleports", null),
        ("Potions Drunk", null),
        ("Monster Kills", null),
        ("Monster Assists", null),
        ("God Kills", null),
        ("God Assists", null),
        ("Cube Kills", null),
        ("Oryx Kills", null),
        ("Quests Completed", null),
        ("Pirate Caves", "Low"),
        ("Undead Lairs", "Mid"),
        ("Abyss of Demons", "Mid"),
        ("Snake Pits", "Low"),
        ("Spider Dens", "Low"),
        ("Sprite Worlds", "High"),
        ("Level Up Assists", null),
        ("Minutes Active", null),
        ("Tombs of the Ancients", "High"),
        ("Ocean Trenches", "Mid"),
        ("Deadwater Docks", "Low"),
        ("Manors of the Immortals", "High"),
    };

    /// <summary>The words for a tally, or its bare id if this build does not know it.</summary>
    public static string Label(int id) =>
        id >= 0 && id < Names.Length ? Names[id].Label : $"Statistic {id}";

    /// <summary>Which tier a dungeon sits in, for the group headers. Null for a statistic.</summary>
    public static string Tier(int id) =>
        id >= 0 && id < Names.Length ? Names[id].Tier : null;
}
