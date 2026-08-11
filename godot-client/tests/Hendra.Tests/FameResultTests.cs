using Hendra.Account;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// Parsing the fame tally.
///
/// The response is the server's own <c>Fame.ToXml</c>. The bonus list is the part worth pinning
/// down: it is a flat run of sibling elements carrying their value as text and their wording as an
/// attribute, which is easy to read as the other way round.
/// </summary>
public sealed class FameResultTests
{
    /// <summary>Trimmed from a real response, keeping the shape of every part that is read.</summary>
    private const string Response = """
        <Fame>
          <Char id="1">
            <ObjectType>782</ObjectType>
            <Level>12</Level>
            <Exp>34500</Exp>
            <CurrentFame>118</CurrentFame>
            <Account>
              <Name>Tester</Name>
            </Account>
          </Char>
          <BaseFame>118</BaseFame>
          <TotalFame>164</TotalFame>
          <Shots>690</Shots>
          <ShotsThatDamage>345</ShotsThatDamage>
          <SpecialAbilityUses>4</SpecialAbilityUses>
          <TilesUncovered>79187</TilesUncovered>
          <Teleports>2</Teleports>
          <PotionsDrunk>7</PotionsDrunk>
          <MonsterKills>210</MonsterKills>
          <MonsterAssists>33</MonsterAssists>
          <GodKills>5</GodKills>
          <GodAssists>1</GodAssists>
          <CubeKills>0</CubeKills>
          <OryxKills>0</OryxKills>
          <QuestsCompleted>3</QuestsCompleted>
          <LevelUpAssists>0</LevelUpAssists>
          <MinutesActive>41</MinutesActive>
          <Bonus id="first_born" desc="First death of any of your characters">20</Bonus>
          <Bonus id="ancient_curse" desc="Killed an ancient curse">6</Bonus>
          <CreatedOn>1754899200</CreatedOn>
          <KilledBy>Sprite God</KilledBy>
        </Fame>
        """;

    [Fact]
    public void ReadsTheHeadlineFigures()
    {
        var fame = FameResult.Parse(Response);

        Assert.Equal("Tester", fame.CharacterName);
        Assert.Equal("Sprite God", fame.KilledBy);
        Assert.Equal(12, fame.Level);
        Assert.Equal(34500, fame.Experience);
        Assert.Equal(118, fame.BaseFame);
        Assert.Equal(164, fame.TotalFame);
    }

    /// <summary>
    /// The class comes back as a plain decimal object type, even though the same ids appear in
    /// hexadecimal everywhere in the game's own XML — the server writes it straight out of a
    /// <c>ushort</c>.
    /// </summary>
    [Fact]
    public void ReadsTheClassAsDecimal()
    {
        Assert.Equal(782, FameResult.Parse(Response).ObjectType);
    }

    [Fact]
    public void ReadsBonusesInOrder()
    {
        var fame = FameResult.Parse(Response);

        Assert.Equal(2, fame.Bonuses.Count);
        Assert.Equal("first_born", fame.Bonuses[0].Id);
        Assert.Equal("First death of any of your characters", fame.Bonuses[0].Description);
        Assert.Equal(20, fame.Bonuses[0].Fame);
        Assert.Equal(6, fame.Bonuses[1].Fame);
    }

    [Fact]
    public void ReadsTheTallies()
    {
        var fame = FameResult.Parse(Response);

        Assert.Equal(690, fame.Shots);
        Assert.Equal(210, fame.MonsterKills);
        Assert.Equal(79187, fame.TilesUncovered);
        Assert.Equal(0.5f, fame.Accuracy, 3);
    }

    /// <summary>A character that never fired has no accuracy rather than a division by zero.</summary>
    [Fact]
    public void AccuracyIsZeroWhenNothingWasFired()
    {
        Assert.Equal(0f, FameResult.Parse("<Fame><Shots>0</Shots></Fame>").Accuracy);
    }

    /// <summary>A response missing everything optional still parses.</summary>
    [Fact]
    public void ToleratesAMinimalResponse()
    {
        var fame = FameResult.Parse("<Fame />");

        Assert.Equal(string.Empty, fame.CharacterName);
        Assert.Empty(fame.Bonuses);
        Assert.Equal(0, fame.TotalFame);
    }
}
