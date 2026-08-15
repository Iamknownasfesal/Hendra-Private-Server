using Godot;
using Hendra.UI;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// The HUD's arithmetic, at every resolution the brief names.
/// </summary>
/// <remarks>
/// Two things go wrong with a corner-anchored interface, and neither shows up on the machine it was
/// written on. Clusters grow into each other at a window shape nobody opened, and the overlay eats
/// clicks that were meant for the world. Both are properties of the numbers rather than of the
/// drawing, which is why the numbers live in a struct with no engine in it and are checked here.
/// </remarks>
public class HudLayoutTests
{
    /// <summary>The window sizes the brief's acceptance criteria name.</summary>
    public static TheoryData<int, int> Resolutions => new()
    {
        { 1280, 720 },
        { 1920, 1080 },
        { 2560, 1440 },
        { 3440, 1440 },
    };

    [Theory]
    [MemberData(nameof(Resolutions))]
    public void ClustersDoNotOverlap(int width, int height)
    {
        var layout = new HudLayout(HudLayout.SpaceFor(new Vector2(width, height)));

        var clusters = new System.Collections.Generic.List<(string Name, Rect2 Rect, bool Interactive)>(
            layout.Clusters());

        for (int i = 0; i < clusters.Count; i++)
        {
            for (int j = i + 1; j < clusters.Count; j++)
            {
                Assert.False(
                    clusters[i].Rect.Intersects(clusters[j].Rect),
                    $"{clusters[i].Name} overlaps {clusters[j].Name} at {width}x{height}");
            }
        }
    }

    [Theory]
    [MemberData(nameof(Resolutions))]
    public void ClustersStayOnScreen(int width, int height)
    {
        var space = HudLayout.SpaceFor(new Vector2(width, height));
        var layout = new HudLayout(space);

        foreach (var (name, rect, _) in layout.Clusters())
        {
            Assert.True(rect.Position.X >= 0f && rect.Position.Y >= 0f,
                $"{name} starts off screen at {width}x{height}: {rect.Position}");

            Assert.True(rect.End.X <= space.X + 0.01f && rect.End.Y <= space.Y + 0.01f,
                $"{name} runs off the far edge at {width}x{height}: {rect.End} of {space}");
        }
    }

    /// <summary>
    /// The passthrough rule: the gaps between clusters belong to the world.
    /// </summary>
    /// <remarks>
    /// The single most common bug in an interface built this way, and the one that is hardest to
    /// notice -- a swallowed click looks like the character ignoring an order rather than like a
    /// panel doing something wrong.
    /// </remarks>
    [Theory]
    [MemberData(nameof(Resolutions))]
    public void GapsBetweenClustersReachTheWorld(int width, int height)
    {
        var space = HudLayout.SpaceFor(new Vector2(width, height));
        var layout = new HudLayout(space);

        var gaps = new (string Where, Vector2 At)[]
        {
            ("the middle of the screen", space / 2f),
            ("under the player card", new Vector2(HudLayout.Margin + 40f, layout.PlayerCard.End.Y + 120f)),
            ("right of the player card", new Vector2(layout.PlayerCard.End.X + 60f, layout.PlayerCard.Position.Y)),
            ("right of the chat", new Vector2(layout.Chat.End.X + 20f, space.Y - 40f)),
            ("under the quest tracker", new Vector2(layout.Quest.Position.X + 40f, layout.Quest.End.Y + 60f)),
            ("left of the column", new Vector2(layout.ColumnLeft - 30f, 120f)),
        };

        foreach (var (where, at) in gaps)
        {
            Assert.False(layout.HitsCluster(at),
                $"a click {where} at {width}x{height} is swallowed by the interface");
        }
    }

    /// <summary>The numbers only mean anything if the currency is over the world, not on a panel.</summary>
    [Fact]
    public void CurrencyPassesThePointerThrough()
    {
        var layout = new HudLayout(new Vector2(HudLayout.ReferenceWidth, HudLayout.ReferenceHeight));

        Assert.False(layout.HitsCluster(layout.Currency.GetCenter()));
    }

    /// <summary>
    /// The scale only ever lands on a whole or a half step.
    /// </summary>
    /// <remarks>
    /// Which is the point of revision two's rendering rule: a pixel face and pixel sprites shimmer
    /// at 1.37 and are exact at 1, 1.5 and 2. The three sizes named here are the three the brief's
    /// acceptance criteria name, with the scales it expects.
    /// </remarks>
    [Theory]
    [InlineData(1920, 1080, 1.0f)]
    [InlineData(2560, 1440, 1.5f)]
    [InlineData(3440, 1440, 1.5f)]
    [InlineData(3840, 2160, 2.0f)]
    public void ScaleSnapsToHalfSteps(int width, int height, float expected)
    {
        Assert.Equal(expected, HudLayout.ScaleFor(new Vector2(width, height)), 3);
    }

    [Fact]
    public void SpaceIsTheReferenceResolutionAtTheReferenceResolution()
    {
        Assert.Equal(new Vector2(1920, 1080), HudLayout.SpaceFor(new Vector2(1920, 1080)));
    }

    /// <summary>
    /// A window too small for the layout drops off the half steps rather than overlapping.
    /// </summary>
    /// <remarks>
    /// The one place the snapping rule gives way, and a deliberate trade. Revision two asks for a
    /// minimum scale of one, which at 1280 by 720 leaves the layout solved in 1280 by 720 -- and
    /// the chat panel, which ends at 550, then runs straight through the vitals, which are centred
    /// on the viewport. A slightly resampled glyph is worse than nothing; two panels on top of each
    /// other are worse than that. So under <see cref="HudLayout.MinimumSpace"/> the scale becomes
    /// whatever fits.
    /// </remarks>
    [Fact]
    public void DeliberatelyFractionalBelowTheMinimum()
    {
        float scale = HudLayout.ScaleFor(new Vector2(1280, 720));
        var space = HudLayout.SpaceFor(new Vector2(1280, 720));

        Assert.True(scale < HudLayout.MinScale, "a window this small cannot hold the layout at 1x");
        Assert.True(space.X >= HudLayout.MinimumSpace.X, $"solved in {space.X}, needs {HudLayout.MinimumSpace.X}");
    }

    /// <summary>An ultrawide keeps its extra width as extra layout, not as a stretched middle.</summary>
    [Fact]
    public void UltrawideKeepsItsExtraWidth()
    {
        var wide = HudLayout.SpaceFor(new Vector2(3440, 1440));
        var square = HudLayout.SpaceFor(new Vector2(2560, 1440));

        Assert.Equal(square.Y, wide.Y, 1);
        Assert.True(wide.X > square.X + 500f);
    }

    /// <summary>
    /// The panel covers no part of the permanent interface, at any resolution.
    /// </summary>
    /// <remarks>
    /// It opens out of the card's icon row, down the left, because it is a thing you flick open to
    /// read one number. A sheet that covered the middle of the screen would have to be closed
    /// before you could play again, which is the opposite of what it is for. The layout is solved
    /// </remarks>
    [Theory]
    [MemberData(nameof(Resolutions))]
    public void ThePanelCoversNothing(int width, int height)
    {
        var space = HudLayout.SpaceFor(new Vector2(width, height));
        var layout = new HudLayout(space);
        var panel = layout.Modal;

        // The column is the one thing it must never cover: half of what the panel says is only
        // meaningful read against the bars beside it.
        Assert.False(panel.Intersects(layout.Column), $"the panel covers the column at {width}x{height}");

        Assert.True(panel.End.X <= space.X && panel.End.Y <= space.Y,
            $"the panel runs off the screen at {width}x{height}");

        // It is docked against the column, which is what makes it read as a second column that
        // slid out from under the first rather than as a dialog that appeared somewhere.
        Assert.Equal(layout.ColumnLeft - HudLayout.ModalGutter, panel.End.X, 1);
    }

    /// <summary>
    /// The minimum space is big enough to leave real gaps, not just to avoid a collision.
    /// </summary>
    /// <remarks>
    /// A layout whose panels miss each other by three pixels is a layout that overlaps as soon as
    /// anything gains a character, and the dark strip between two clusters is where the world is
    /// clicked.
    /// </remarks>
    [Fact]
    public void TheMinimumSpaceLeavesRoomBetweenClusters()
    {
        var layout = new HudLayout(HudLayout.MinimumSpace);

        Assert.True(layout.ColumnLeft - layout.Chat.End.X >= 30f,
            "the chat and the column are too close at the minimum space");

        Assert.True(layout.ColumnLeft - layout.Quest.End.X >= 30f,
            "the quest tracker and the column are too close at the minimum space");

        Assert.True(layout.PartyRowsThatFit >= 1,
            "the minimum space leaves no room for the player list");
    }

    /// <summary>
    /// A player with no guild gets the same card, not one with a blank row in it.
    /// </summary>
    /// <remarks>
    /// The guild sits in the column beside the portrait now that the experience bar has moved to
    /// the vitals, so there is no row beneath it for hiding it to leave a hole in -- the card is as
    /// tall as its portrait either way.
    /// </remarks>
    [Fact]
    public void TheCardIsTheSameHeightWithoutAGuild()
    {
        var space = new Vector2(HudLayout.ReferenceWidth, HudLayout.ReferenceHeight);

        Assert.Equal(HudLayout.CardHeight, new HudLayout(space).PlayerCard.Size.Y, 3);
        Assert.Equal(HudLayout.CardHeight, new HudLayout(space, guild: false).PlayerCard.Size.Y, 3);
    }

    /// <summary>
    /// The reference's own measurements, at the resolution they were taken.
    /// </summary>
    /// <remarks>
    /// A regression test on the design rather than on the code: these are the numbers eyedropped
    /// from the screenshot, and a change to the layout that moves any of them is a change that has
    /// stopped matching it.
    /// </remarks>
    [Fact]
    public void MatchesTheReferenceMeasurements()
    {
        var layout = new HudLayout(new Vector2(HudLayout.ReferenceWidth, HudLayout.ReferenceHeight));

        Assert.Equal(new Rect2(0f, 0f, 420f, 80f), layout.PlayerCard);
        Assert.Equal(new Rect2(1560f, 0f, 360f, 367f), layout.Minimap);
        Assert.Equal(new Rect2(1560f, 367f, 360f, 48f), layout.IconRow);
        Assert.Equal(new Rect2(1569f, 415f, 342f, 34f), layout.FameBar);
        Assert.Equal(new Rect2(1569f, 456f, 342f, 34f), layout.HealthBar);
        Assert.Equal(new Rect2(1569f, 498f, 342f, 34f), layout.ManaBar);
        Assert.Equal(new Rect2(1571f, 540f, 338f, 90f), layout.EquipmentRow);
        Assert.Equal(new Rect2(1567f, 639f, 346f, 45f), layout.HotbarTabs);
        Assert.Equal(new Rect2(1576f, 684f, 328f, 162f), layout.Hotbar);
        Assert.Equal(new Rect2(1576f, 850f, 328f, 41f), layout.PotionRow);

        // The column runs floor to ceiling and is flush to the right edge, which is most of what
        // makes the interface read as this game rather than as a HUD with a map in the corner.
        Assert.Equal(HudLayout.ReferenceWidth, layout.Column.End.X, 1);
        Assert.Equal(HudLayout.ReferenceHeight, layout.Column.End.Y, 1);
    }
}
