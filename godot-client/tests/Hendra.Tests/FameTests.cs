using Hendra.UI;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// Stars, on the original's FameUtil numbers.
///
/// A character earns one star per fame threshold it passes, up to five, and an account's rating is
/// the sum across every class — so it measures how widely you have played rather than how far you
/// have taken one character.
/// </summary>
public sealed class FameTests
{
    [Theory]
    [InlineData(0, 0)]
    [InlineData(19, 0)]
    [InlineData(20, 1)]
    [InlineData(149, 1)]
    [InlineData(150, 2)]
    [InlineData(400, 3)]
    [InlineData(800, 4)]
    [InlineData(2000, 5)]
    public void EachThresholdIsAStar(int fame, int expected)
    {
        Assert.Equal(expected, Fame.Stars(fame));
    }

    /// <summary>Five is the cap for one character however much fame it piles up.</summary>
    [Fact]
    public void FiveIsTheMost()
    {
        Assert.Equal(5, Fame.Stars(1_000_000));
    }

    /// <summary>
    /// The colour steps every time the total passes another multiple of the class count, which is
    /// what makes the last colour mean "has done this with everything".
    /// </summary>
    [Fact]
    public void TheColourStepsWithEveryClassworth()
    {
        const int Classes = 14;

        Assert.Equal(Fame.Colour(0, Classes), Fame.Colour(13, Classes));
        Assert.NotEqual(Fame.Colour(13, Classes), Fame.Colour(14, Classes));
        Assert.NotEqual(Fame.Colour(14, Classes), Fame.Colour(28, Classes));
    }

    /// <summary>Past the last band it stays at the top colour rather than running off the end.</summary>
    [Fact]
    public void ItStopsAtTheTopColour()
    {
        const int Classes = 14;

        Assert.Equal(Fame.Colour(70, Classes), Fame.Colour(9999, Classes));
    }

    [Fact]
    public void AdministratorsHaveTheirOwn()
    {
        Assert.NotEqual(Fame.Colour(70, 14), Fame.Colour(70, 14, isAdmin: true));
    }
}
