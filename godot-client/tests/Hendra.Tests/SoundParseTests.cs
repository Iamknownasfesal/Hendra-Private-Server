using System.Linq;
using Hendra.Resources;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// The sounds an object declares.
///
/// Almost every one is a bare &lt;Sound&gt; — a weapon's firing sound, which is sound zero. Only a
/// handful of objects carry several and number them, for the PlaySound packet to choose between.
/// Requiring the number threw away every weapon sound in the game.
/// </summary>
public sealed class SoundParseTests
{
    private static ObjectDesc Parse(string body)
    {
        var data = new GameData();
        data.AddObjects($"<Objects><Object type=\"0x1\" id=\"Thing\">{body}</Object></Objects>");
        return data.GetObject(1);
    }

    [Fact]
    public void ABareSoundIsSoundZero()
    {
        var desc = Parse("<Sound>weapon/poor_quality_bow</Sound>");

        Assert.NotNull(desc.Sounds);
        Assert.Equal("weapon/poor_quality_bow", desc.Sounds[0]);
    }

    [Fact]
    public void NumberedSoundsKeepTheirNumbers()
    {
        var desc = Parse("<Sound id=\"0\">first</Sound><Sound id=\"3\">fourth</Sound>");

        Assert.Equal("first", desc.Sounds[0]);
        Assert.Equal("fourth", desc.Sounds[3]);
    }

    /// <summary>An empty element names no sound, and should not be stored as one.</summary>
    [Fact]
    public void AnEmptySoundIsNotASound()
    {
        var desc = Parse("<Sound></Sound>");

        Assert.True(desc.Sounds == null || !desc.Sounds.ContainsKey(0));
    }

    [Fact]
    public void NoSoundElementLeavesNone()
    {
        Assert.Null(Parse("<Class>Equipment</Class>").Sounds);
    }
}
