using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Xml.Linq;

namespace Hendra.Account;

/// <summary>One world server the player can connect to.</summary>
public sealed class ServerInfo
{
    public string Name = string.Empty;
    public string Address = string.Empty;
    public double Latitude;
    public double Longitude;

    /// <summary>Fraction of capacity in use. At or above 1 the server is full.</summary>
    public double Usage;

    public bool AdminOnly;

    /// <summary>
    /// The world server port. Fixed rather than carried in the XML — the original never read one
    /// from the server list either.
    /// </summary>
    public int Port = 2050;

    public bool IsFull => Usage >= 1.0;
    public bool IsCrowded => Usage >= 0.66;

    /// <summary>Lower sorts first when choosing automatically.</summary>
    public int Priority => AdminOnly ? 2 : IsCrowded ? 1 : 0;

    public override string ToString() => $"{Name} ({Address}:{Port})";
}

/// <summary>One of the account's characters.</summary>
public sealed class CharacterInfo
{
    public int CharacterId;
    public ushort ObjectType;
    public int Level;
    public int Experience;
    public int CurrentFame;
    public int MaxHitPoints;
    public int HitPoints;
    public int MaxMagicPoints;
    public int MagicPoints;
    public int Attack;
    public int Defense;
    public int Speed;
    public int Dexterity;
    public int Vitality;
    public int Wisdom;
    public int Texture1;
    public int Texture2;
    public int Skin;
    public bool Dead;
    public bool HasBackpack;
    public int[] Equipment = Array.Empty<int>();

    /// <summary>
    /// When the character was rolled, or null on a server that does not say.
    /// </summary>
    /// <remarks>
    /// The field has always been in the server's database and its character model; it was simply
    /// never written into the list's XML. Null here means an older server, and the panel leaves the
    /// line out rather than inventing a date.
    /// </remarks>
    public DateTime? CreatedAt;

    /// <summary>The character's tallies, live. See <see cref="CharacterStats"/>.</summary>
    public CharacterStats Stats = CharacterStats.Parse(null);
}

/// <summary>The account itself.</summary>
public sealed class AccountInfo
{
    public string AccountId = string.Empty;
    public string Name = string.Empty;
    public int Credits;
    public int Fame;
    public int Rank;
    public bool NameChosen;
    public bool Admin;
    public bool VerifiedEmail;
    public string GuildName = string.Empty;
    public int GuildRank;
}

/// <summary>
/// Everything <c>/char/list</c> returns: the characters, the account, and — despite the endpoint's
/// name — the server list.
/// </summary>
/// <remarks>
/// That the server list arrives here rather than from anything called "servers" is worth knowing
/// before going looking for it.
/// </remarks>
public sealed class CharListResult
{
    public int NextCharacterId;
    public int MaxCharacters;
    public readonly List<CharacterInfo> Characters = new();
    public readonly List<ServerInfo> Servers = new();
    public AccountInfo Account = new();

    public static CharListResult Parse(string xml)
    {
        var root = XDocument.Parse(xml).Root
                   ?? throw new FormatException("Character list response has no root element.");

        var result = new CharListResult
        {
            NextCharacterId = Int(root.Attribute("nextCharId")?.Value),
            MaxCharacters = Int(root.Attribute("maxNumChars")?.Value),
        };

        foreach (var element in root.Elements("Char"))
            result.Characters.Add(ParseCharacter(element));

        var account = root.Element("Account");
        if (account != null)
            result.Account = ParseAccount(account);

        var servers = root.Element("Servers");
        if (servers != null)
        {
            foreach (var element in servers.Elements("Server"))
                result.Servers.Add(ParseServer(element));
        }

        return result;
    }

    /// <summary>
    /// Picks a server automatically: the least loaded, nearest one that is not full.
    /// </summary>
    /// <param name="preferredName">An exact name match wins outright, whatever its load.</param>
    /// <param name="latitude">The player's own position, used to break ties by distance.</param>
    /// <param name="longitude">See <paramref name="latitude"/>.</param>
    public ServerInfo ChooseServer(string preferredName = null, double latitude = 0, double longitude = 0)
    {
        if (Servers.Count == 0)
            return null;

        if (!string.IsNullOrEmpty(preferredName))
        {
            var preferred = Servers.FirstOrDefault(s =>
                string.Equals(s.Name, preferredName, StringComparison.OrdinalIgnoreCase));
            if (preferred != null)
                return preferred;
        }

        var candidates = Servers.Where(s => !s.IsFull && !s.AdminOnly).ToList();
        if (candidates.Count == 0)
            candidates = Servers;

        return candidates
            .OrderBy(s => s.Priority)
            .ThenBy(s => DistanceSquared(s, latitude, longitude))
            .First();
    }

    private static double DistanceSquared(ServerInfo server, double latitude, double longitude)
    {
        double dLat = server.Latitude - latitude;
        double dLon = server.Longitude - longitude;
        return dLat * dLat + dLon * dLon;
    }

    private static CharacterInfo ParseCharacter(XElement e) => new()
    {
        CharacterId = Int(e.Attribute("id")?.Value),
        ObjectType = (ushort)Int(Text(e, "ObjectType")),
        Level = Int(Text(e, "Level")),
        Experience = Int(Text(e, "Exp")),
        CurrentFame = Int(Text(e, "CurrentFame")),
        MaxHitPoints = Int(Text(e, "MaxHitPoints")),
        HitPoints = Int(Text(e, "HitPoints")),
        MaxMagicPoints = Int(Text(e, "MaxMagicPoints")),
        MagicPoints = Int(Text(e, "MagicPoints")),
        Attack = Int(Text(e, "Attack")),
        Defense = Int(Text(e, "Defense")),
        Speed = Int(Text(e, "Speed")),
        Dexterity = Int(Text(e, "Dexterity")),
        Vitality = Int(Text(e, "HpRegen")),
        Wisdom = Int(Text(e, "MpRegen")),
        Texture1 = Int(Text(e, "Tex1")),
        Texture2 = Int(Text(e, "Tex2")),
        Skin = Int(Text(e, "Texture")),
        Dead = Bool(Text(e, "Dead")),
        HasBackpack = Text(e, "HasBackpack") == "1",
        Equipment = SplitInts(Text(e, "Equipment")),
        CreatedAt = Timestamp(Text(e, "CreateTime")),
        Stats = CharacterStats.Parse(Text(e, "PCStats")),
    };

    /// <summary>
    /// A round-trip timestamp, or null if the server sent none or sent a default one.
    /// </summary>
    /// <remarks>
    /// Characters rolled before the server wrote the field carry the zero date, and "Created on
    /// 1 January 0001" is worse than no line at all.
    /// </remarks>
    private static DateTime? Timestamp(string value)
    {
        if (string.IsNullOrWhiteSpace(value))
            return null;

        if (!DateTime.TryParse(value, CultureInfo.InvariantCulture,
                DateTimeStyles.RoundtripKind, out var parsed))
            return null;

        return parsed.Year < 2000 ? null : parsed;
    }

    private static AccountInfo ParseAccount(XElement e) => new()
    {
        AccountId = Text(e, "AccountId") ?? string.Empty,
        Name = Text(e, "Name") ?? string.Empty,
        Credits = Int(Text(e, "Credits")),
        Fame = Int(Text(e, "Fame")),
        Rank = Int(Text(e, "Rank")),

        // Presence flags. The server writes `NameChosen ? new XElement("NameChosen", "") : null`,
        // so the element is absent when false and *empty* when true -- reading its content would
        // get "" and call every one of them false.
        NameChosen = e.Element("NameChosen") != null,
        Admin = e.Element("Admin") != null,
        VerifiedEmail = e.Element("VerifiedEmail") != null,

        GuildName = Text(e.Element("Guild"), "Name") ?? string.Empty,
        GuildRank = Int(Text(e.Element("Guild"), "Rank")),
    };

    private static ServerInfo ParseServer(XElement e) => new()
    {
        Name = Text(e, "Name") ?? string.Empty,
        Address = Text(e, "DNS") ?? string.Empty,
        Latitude = Double(Text(e, "Lat")),
        Longitude = Double(Text(e, "Long")),
        Usage = Double(Text(e, "Usage")),
        AdminOnly = e.Element("AdminOnly") != null && Bool(Text(e, "AdminOnly")),
    };

    private static string Text(XElement parent, string name)
    {
        string value = parent?.Element(name)?.Value;
        return string.IsNullOrWhiteSpace(value) ? null : value.Trim();
    }

    private static int Int(string text) =>
        int.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out int value) ? value : 0;

    private static double Double(string text) =>
        double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out double value) ? value : 0;

    private static bool Bool(string text) =>
        text != null && (text.Equals("true", StringComparison.OrdinalIgnoreCase) || text == "1");


    private static int[] SplitInts(string text) =>
        string.IsNullOrEmpty(text)
            ? Array.Empty<int>()
            : text.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
                .Select(Int)
                .ToArray();
}
