using Hendra.Account;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// Parsing the guild roll, and the rank ladder it is sorted by.
///
/// The ranks are not consecutive — they go up in tens — so anything that treats them as an index
/// or adds one to promote lands between two real ranks and the server refuses it silently.
/// </summary>
public sealed class GuildResultTests
{
    private const string Response = """
        <Guild id="7" name="Test Guild">
          <TotalFame>4200</TotalFame>
          <CurrentFame>1200</CurrentFame>
          <HallType>Guild Hall 2</HallType>
          <Member>
            <Name>Founder Person</Name>
            <Rank>40</Rank>
            <Fame>900</Fame>
            <LastSeen>-1</LastSeen>
          </Member>
          <Member>
            <Name>Officer Person</Name>
            <Rank>20</Rank>
            <Fame>200</Fame>
            <LastSeen>1754899200</LastSeen>
          </Member>
          <Member>
            <Name>New Person</Name>
            <Rank>0</Rank>
            <Fame>0</Fame>
            <LastSeen>1754899000</LastSeen>
          </Member>
        </Guild>
        """;

    [Fact]
    public void ReadsTheGuildAndItsRoll()
    {
        var guild = GuildResult.Parse(Response);

        Assert.Equal(7, guild.Id);
        Assert.Equal("Test Guild", guild.Name);
        Assert.Equal(1200, guild.CurrentFame);
        Assert.Equal("Guild Hall 2", guild.HallType);
        Assert.Equal(3, guild.Members.Count);
        Assert.Equal("Founder Person", guild.Members[0].Name);
        Assert.Equal(900, guild.Members[0].Fame);
    }

    /// <summary>
    /// A negative last-seen means the account is connected, not that it was seen before the epoch.
    /// </summary>
    [Fact]
    public void NegativeLastSeenMeansOnline()
    {
        var guild = GuildResult.Parse(Response);

        Assert.True(guild.Members[0].IsOnline);
        Assert.False(guild.Members[1].IsOnline);
    }

    [Fact]
    public void FindsYourOwnRankByName()
    {
        var guild = GuildResult.Parse(Response);

        Assert.Equal(20, guild.RankOf("Officer Person"));
        Assert.Equal(20, guild.RankOf("officer person"));
        Assert.Equal(-1, guild.RankOf("Somebody Else"));
    }

    /// <summary>Ranks go up in tens, and anything above the top name still reads as that name.</summary>
    [Theory]
    [InlineData(0, "Initiate")]
    [InlineData(5, "Initiate")]
    [InlineData(10, "Member")]
    [InlineData(20, "Officer")]
    [InlineData(30, "Leader")]
    [InlineData(40, "Founder")]
    [InlineData(99, "Founder")]
    public void NamesTheRanks(int rank, string expected)
    {
        Assert.Equal(expected, GuildRank.Name(rank));
    }

    /// <summary>Officers and above may manage; members and initiates may not.</summary>
    [Theory]
    [InlineData(0, false)]
    [InlineData(10, false)]
    [InlineData(20, true)]
    [InlineData(40, true)]
    public void OnlyOfficersManage(int rank, bool expected)
    {
        Assert.Equal(expected, GuildRank.CanManage(rank));
    }

    /// <summary>The ladder is ordered, so stepping along it lands on real ranks.</summary>
    [Fact]
    public void TheLadderIsInOrder()
    {
        var ladder = GuildRank.Assignable;

        for (int i = 1; i < ladder.Length; i++)
            Assert.True(ladder[i] > ladder[i - 1], "the rank ladder is not ascending");

        Assert.Equal(GuildRank.Initiate, ladder[0]);
        Assert.Equal(GuildRank.Leader, ladder[^1]);
    }

    /// <summary>A guild with nobody in it, and a response missing everything optional, both parse.</summary>
    [Fact]
    public void ToleratesAMinimalResponse()
    {
        var guild = GuildResult.Parse("<Guild />");

        Assert.Empty(guild.Members);
        Assert.Equal(string.Empty, guild.Name);
        Assert.Equal(-1, guild.RankOf("anyone"));
    }
}
