using System;
using System.IO;
using Hendra.Account;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// The tallies blob, against the format the server writes.
/// </summary>
/// <remarks>
/// The bytes are built here the way <c>common/FameStats.Write</c> builds them -- an identity byte
/// then a network-order int -- because that is the only contract between the two, and getting the
/// byte order wrong reads Shots of 33554432 instead of 2 without failing anywhere.
/// </remarks>
public class CharacterStatsTests
{
    private static string Blob(params (int Id, int Value)[] entries)
    {
        using var stream = new MemoryStream();

        foreach (var (id, value) in entries)
        {
            stream.WriteByte((byte)id);
            stream.WriteByte((byte)(value >> 24));
            stream.WriteByte((byte)(value >> 16));
            stream.WriteByte((byte)(value >> 8));
            stream.WriteByte((byte)value);
        }

        return Convert.ToBase64String(stream.ToArray());
    }

    [Fact]
    public void ReadsNetworkOrderValues()
    {
        var stats = CharacterStats.Parse(Blob((0, 1655926)));

        Assert.Single(stats.Statistics);
        Assert.Equal(0, stats.Statistics[0].Id);
        Assert.Equal(1655926, stats.Statistics[0].Value);
    }

    /// <summary>The two tabs are one blob: the dungeons are a run of ids inside the statistics.</summary>
    [Fact]
    public void SplitsDungeonsFromStatistics()
    {
        var stats = CharacterStats.Parse(Blob((0, 10), (13, 3), (6, 40), (21, 1)));

        Assert.Equal(2, stats.Statistics.Count);
        Assert.Equal(2, stats.Dungeons.Count);

        Assert.Equal("Pirate Caves", CharacterStats.Label(13));

        // The dungeons tab reads its rows out of this table rather than out of id order, and a
        // tally the blob never carried answers zero rather than throwing.
        Assert.Contains(13, CharacterStats.DungeonsByName);
        Assert.DoesNotContain(0, CharacterStats.DungeonsByName);
        Assert.Equal(3, stats.Value(13));
        Assert.Equal(0, stats.Value(99));
    }

    [Fact]
    public void KeepsServerOrder()
    {
        var stats = CharacterStats.Parse(Blob((6, 1), (0, 2), (8, 3)));

        Assert.Equal(new[] { 6, 0, 8 }, new[] { stats.Statistics[0].Id, stats.Statistics[1].Id, stats.Statistics[2].Id });
    }

    /// <summary>A newer server may add tallies; an older client should show the ones it knows.</summary>
    [Fact]
    public void KeepsUnknownIdsWithAFallbackLabel()
    {
        var stats = CharacterStats.Parse(Blob((99, 7)));

        Assert.Single(stats.Statistics);
        Assert.Equal("Statistic 99", CharacterStats.Label(99));
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("not base64 at all !!")]
    public void SurvivesRubbish(string input)
    {
        var stats = CharacterStats.Parse(input);

        Assert.Empty(stats.Statistics);
        Assert.Empty(stats.Dungeons);
    }

    /// <summary>A truncated record is dropped rather than throwing the panel away with it.</summary>
    [Fact]
    public void IgnoresATrailingPartialRecord()
    {
        string whole = Blob((0, 5));
        byte[] bytes = Convert.FromBase64String(whole);
        Array.Resize(ref bytes, bytes.Length + 3);

        var stats = CharacterStats.Parse(Convert.ToBase64String(bytes));

        Assert.Single(stats.Statistics);
        Assert.Equal(5, stats.Statistics[0].Value);
    }

    [Fact]
    public void PicksOutMinutesActive()
    {
        Assert.Equal(742, CharacterStats.Parse(Blob((20, 742))).MinutesActive);
    }
}
