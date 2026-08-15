using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Threading.Tasks;
using System.Xml.Linq;
using Hendra.Account;

namespace Hendra.UI;

/// <summary>
/// Everything the characters panel and the create-character page draw, fetched in one go.
/// </summary>
/// <remarks>
/// <para>
/// <c>/char/list</c> answers with the living characters, the account and the server list.
/// It does <em>not</em> answer with the dead ones: the server builds that list from
/// <c>GetAliveCharacters</c>. The account's deaths reach the client only as news items, one per
/// death, carrying a <c>fame:&lt;charId&gt;</c> link. Those links are what the graveyard is built
/// from, and <c>/char/fame</c> is asked for each so the row shows the class, level and fame the
/// server recorded rather than a sentence parsed out of a headline.
/// </para>
/// <para>
/// The server keeps ten deaths, so the graveyard is the ten most recent and not the whole history.
/// </para>
/// </remarks>
public sealed class CharacterRoster
{
    /// <summary>
    /// The fame each class quest asks for.
    /// </summary>
    /// <remarks>
    /// The original's ladder. This server records no class quests of its own — the database has a
    /// best-fame-per-class column but never writes it into any response — so a quest's progress is
    /// worked out here from the fame of the characters the account can actually be seen to have.
    /// </remarks>
    public static readonly int[] ClassQuestGoals = { 20, 500, 1500, 5000, 15000 };

    public AccountInfo Account { get; private set; } = new();
    public List<ServerInfo> Servers { get; } = new();
    public List<CharacterInfo> Alive { get; } = new();
    public List<CharacterInfo> Graveyard { get; } = new();

    public int MaxCharacters { get; private set; }
    public int NextCharacterId { get; private set; }

    /// <summary>Best level reached with each class, straight out of the list's own table.</summary>
    public Dictionary<ushort, int> BestLevel { get; } = new();

    /// <summary>Highest fame seen on a living character of each class.</summary>
    public Dictionary<ushort, int> HighestAliveFame { get; } = new();

    /// <summary>The same across the graveyard as well, which is what a class quest counts.</summary>
    public Dictionary<ushort, int> BestFame { get; } = new();

    /// <summary>Whether the server answered the graveyard requests at all.</summary>
    public bool GraveyardReadable { get; private set; } = true;

    /// <summary>How many of the whole ladder are still open, over every class the game has.</summary>
    public int ClassQuestsOutstanding(IEnumerable<ushort> classes)
    {
        int outstanding = 0;
        foreach (ushort type in classes)
        {
            int best = BestFame.GetValueOrDefault(type);
            outstanding += ClassQuestGoals.Count(goal => best < goal);
        }

        return outstanding;
    }

    /// <summary>How many rungs of the ladder this class has climbed, out of five.</summary>
    public int ClassQuestStars(ushort type)
    {
        int best = BestFame.GetValueOrDefault(type);
        return ClassQuestGoals.Count(goal => best >= goal);
    }

    /// <summary>The fame the next class quest wants, or zero once every one is done.</summary>
    public int NextClassQuestGoal(ushort type)
    {
        int best = BestFame.GetValueOrDefault(type);
        return ClassQuestGoals.FirstOrDefault(goal => best < goal);
    }

    /// <summary>The line under a character's name: its class quest, or its fame once they are done.</summary>
    public string FameLine(CharacterInfo character)
    {
        int goal = NextClassQuestGoal(character.ObjectType);
        return goal > 0
            ? $"Class Quest: {character.CurrentFame:N0} of {goal:N0} Fame"
            : $"{character.CurrentFame:N0} Fame";
    }

    /// <summary>Fetches and assembles the whole roster, graveyard included.</summary>
    public static async Task<CharacterRoster> FetchAsync(string appServerUrl, string guid, string password)
    {
        using var client = new AppEngineClient(appServerUrl);
        string xml = await client.PostAsync("/char/list", new Dictionary<string, string>
        {
            ["guid"] = guid,
            ["password"] = password,
        });

        return await FromListAsync(appServerUrl, xml);
    }

    /// <summary>
    /// Builds a roster from a character list already in hand.
    /// </summary>
    /// <remarks>
    /// The sign-in page has fetched one by the time it can show a panel, and asking again would be a
    /// second round trip for an answer that has not had time to change.
    /// </remarks>
    public static async Task<CharacterRoster> FromListAsync(string appServerUrl, string xml)
    {
        var roster = new CharacterRoster();
        var parsed = CharListResult.Parse(xml);

        roster.Account = parsed.Account;
        roster.MaxCharacters = parsed.MaxCharacters;
        roster.NextCharacterId = parsed.NextCharacterId;
        roster.Servers.AddRange(parsed.Servers);
        roster.Alive.AddRange(parsed.Characters.Where(c => !c.Dead));

        var root = XDocument.Parse(xml).Root;
        roster.ReadBestLevels(root);
        await roster.ReadGraveyardAsync(appServerUrl, root);
        roster.Tally();
        return roster;
    }

    /// <summary>
    /// The account's own record of what it has done with each class.
    /// </summary>
    /// <remarks>
    /// Two tables carry it and they overlap. <c>MaxClassLevelList</c> is the level table, keyed by a
    /// decimal object type; <c>Account/Stats</c> is the fuller one, keyed by a hexadecimal type and
    /// carrying the best fame as well — which is the number the class quests are actually counted
    /// against, so it is read rather than guessed at from the characters that happen to be alive.
    /// </remarks>
    private void ReadBestLevels(XElement root)
    {
        foreach (var entry in root?.Element("MaxClassLevelList")?.Elements("MaxClassLevel")
                             ?? Enumerable.Empty<XElement>())
        {
            if (ushort.TryParse(entry.Attribute("classType")?.Value, out ushort type) &&
                int.TryParse(entry.Attribute("maxLevel")?.Value, out int level))
                BestLevel[type] = level;
        }

        foreach (var entry in root?.Element("Account")?.Element("Stats")?.Elements("ClassStats")
                              ?? Enumerable.Empty<XElement>())
        {
            string id = entry.Attribute("objectType")?.Value;
            if (id == null)
                continue;

            if (id.StartsWith("0x", StringComparison.OrdinalIgnoreCase))
                id = id.Substring(2);

            if (!ushort.TryParse(id, NumberStyles.HexNumber, CultureInfo.InvariantCulture, out ushort type))
                continue;

            if (int.TryParse(entry.Element("BestLevel")?.Value, out int level))
                BestLevel[type] = Math.Max(BestLevel.GetValueOrDefault(type), level);

            if (int.TryParse(entry.Element("BestFame")?.Value, out int fame))
                BestFame[type] = Math.Max(BestFame.GetValueOrDefault(type), fame);
        }
    }

    /// <summary>
    /// Turns the account's death notices into characters.
    /// </summary>
    /// <remarks>
    /// The notices are mixed in with the server's global news, so they are picked out by their link
    /// rather than by position. A death whose detail request fails is dropped instead of being shown
    /// as a row of blanks; every one failing is what marks the graveyard unreadable.
    /// </remarks>
    private async Task ReadGraveyardAsync(string appServerUrl, XElement root)
    {
        var charIds = (root?.Element("News")?.Elements("Item") ?? Enumerable.Empty<XElement>())
            .Select(item => item.Element("Link")?.Value)
            .Where(link => link != null && link.StartsWith("fame:", StringComparison.Ordinal))
            .Select(link => link.Substring("fame:".Length))
            .Select(id => int.TryParse(id, NumberStyles.Integer, CultureInfo.InvariantCulture, out int value)
                ? value
                : -1)
            .Where(id => id >= 0)
            .Distinct()
            .ToList();

        if (charIds.Count == 0 || string.IsNullOrEmpty(Account.AccountId))
            return;

        using var client = new AppEngineClient(appServerUrl);
        int failures = 0;

        foreach (int charId in charIds)
        {
            try
            {
                string xml = await client.PostAsync("/char/fame", new Dictionary<string, string>
                {
                    ["accountId"] = Account.AccountId,
                    ["charId"] = charId.ToString(CultureInfo.InvariantCulture),
                });

                var character = ParseDead(xml);
                if (character != null)
                    Graveyard.Add(character);
            }
            catch (Exception)
            {
                failures++;
            }
        }

        GraveyardReadable = failures < charIds.Count;
    }

    /// <summary>
    /// One dead character, out of a <c>/char/fame</c> reply.
    /// </summary>
    /// <remarks>
    /// The reply wraps the same <c>Char</c> element the list uses inside a <c>Fame</c> element, and
    /// adds the total the death was banked at — which is the number the graveyard shows, since it
    /// includes the bonuses the character only earned by dying.
    /// </remarks>
    private static CharacterInfo ParseDead(string xml)
    {
        var fame = XDocument.Parse(xml).Root;
        var element = fame?.Element("Char");
        if (element == null)
            return null;

        var character = CharListResult.Parse(new XElement("Chars", element).ToString())
            .Characters.FirstOrDefault();

        if (character == null)
            return null;

        character.Dead = true;

        if (int.TryParse(fame.Element("TotalFame")?.Value, NumberStyles.Integer,
                CultureInfo.InvariantCulture, out int total) && total > 0)
            character.CurrentFame = total;

        return character;
    }

    /// <summary>Rolls the two lists up into the per-class bests the class quests are read from.</summary>
    private void Tally()
    {
        foreach (var character in Alive)
        {
            ushort type = character.ObjectType;
            HighestAliveFame[type] = Math.Max(HighestAliveFame.GetValueOrDefault(type), character.CurrentFame);
            BestFame[type] = Math.Max(BestFame.GetValueOrDefault(type), character.CurrentFame);
            BestLevel[type] = Math.Max(BestLevel.GetValueOrDefault(type), character.Level);
        }

        foreach (var character in Graveyard)
        {
            ushort type = character.ObjectType;
            BestFame[type] = Math.Max(BestFame.GetValueOrDefault(type), character.CurrentFame);
            BestLevel[type] = Math.Max(BestLevel.GetValueOrDefault(type), character.Level);
        }
    }
}
