using System.Collections.Generic;
using Godot;

namespace Hendra.UI;

/// <summary>
/// Where every cluster of the interface sits, measured at the reference resolution.
/// </summary>
/// <remarks>
/// <para>
/// The whole layout is one struct of arithmetic with no engine in it, which is deliberate: the
/// thing that goes wrong with a corner-anchored HUD is two clusters growing into each other at
/// some window shape nobody opened during development, and that is a property of the numbers
/// rather than of the drawing. Kept here, it can be checked at every resolution the brief names
/// without booting Godot -- see <c>HudLayoutTests</c>.
/// </para>
/// <para>
/// Every measurement below is in reference pixels: the units the design was drawn in, 1920 by 1080.
/// <see cref="ScaleFor"/> turns a real window into the factor between the two, and
/// <see cref="SpaceFor"/> into the rectangle the layout is solved in -- which is not always 1920 by
/// 1080, because the scale is clamped and an ultrawide window keeps its extra width as extra
/// reference pixels rather than as a stretched middle.
/// </para>
/// </remarks>
public readonly struct HudLayout
{
    public const float ReferenceWidth = 1920f;
    public const float ReferenceHeight = 1080f;

    /// <summary>Every cluster's margin from the edge of the viewport.</summary>
    public const float Margin = 20f;

    /// <summary>
    /// The smallest scale the interface is drawn at.
    /// </summary>
    /// <remarks>
    /// One, because everything in the interface is now authored as pixels -- a bitmap face, sprites
    /// on a pixel grid, one-pixel bevels and outlines -- and anything under a whole step resamples
    /// all of it. Above this the scale climbs in half steps; see <see cref="ScaleFor"/>.
    /// </remarks>
    public const float MinScale = 1f;

    /// <summary>
    /// The smallest rectangle the layout fits in without two clusters touching.
    /// </summary>
    /// <remarks>
    /// The width is set by the widest row: the chat panel ends at 550 and the vitals are centred on
    /// the viewport, so the screen has to be wide enough for half the vitals to clear the chat with
    /// a gap left over that is worth calling a gap. The height is set by the party list clearing the
    /// inventory tabs. Both are consequences of the measurements in revision one rather than
    /// choices, which is why the tests assert the clearances rather than these two numbers.
    /// </remarks>
    public static readonly Vector2 MinimumSpace = new(1660f, 680f);

    // --- Player card -------------------------------------------------------------------------
    public const float CardWidth = 300f;

    /// <summary>
    /// The card, which is now as tall as its portrait and its icon row and nothing else.
    /// </summary>
    /// <remarks>
    /// The experience bar left it for the fame row in the vitals: a character cannot earn fame
    /// before level twenty, so one bar can be the level below that and fame above it, and the card
    /// gets its height back. The name and the guild sit in the column beside the portrait, so the
    /// card no longer changes height when a player has no guild -- there is no row under them for a
    /// gap to open in.
    /// </remarks>
    public const float CardHeight = 88f;

    public const float AvatarSize = 48f;

    // --- Minimap -----------------------------------------------------------------------------
    public const float MinimapWidth = 305f;
    public const float MinimapHeight = 300f;

    // --- Party -------------------------------------------------------------------------------
    public const int PartyColumns = 2;
    public const int PartyRows = 3;
    public const float PartyColumnWidth = 135f;
    public const float PartyRowHeight = 28f;

    /// <summary>The gap between the bottom of the minimap and the first party row.</summary>
    public const float PartyTopGap = 22f;

    // --- Chat --------------------------------------------------------------------------------
    public const float ChatWidth = 530f;
    public const float ChatHeight = 195f;
    public const float ChatBottomMargin = 22f;

    // --- Vitals ------------------------------------------------------------------------------
    /// <summary>
    /// How wide a vitals bar is.
    /// </summary>
    /// <remarks>
    /// Thirty-four wider than it was, which is exactly the heart and the gap after it. The icons
    /// beside these three bars are gone -- each said the same thing as the word already written on
    /// the bar, in less space and with less certainty -- and the bars took the room back rather
    /// than the cluster shrinking, so everything measured against <see cref="VitalsWidth"/> is
    /// where it was.
    /// </remarks>
    public const float VitalBarWidth = 354f;
    public const float VitalBarHeight = 27f;

    /// <summary>The gap between the health row and the magic row.</summary>
    public const float VitalRowGap = 4f;

    public const float PotionBoxWidth = 56f;
    public const float AbilityWidth = 70f;
    public const float AbilityHeight = 67f;
    public const float VitalsBottomMargin = 26f;

    /// <summary>
    /// Fame, health and magic, in that order down the stack.
    /// </summary>
    /// <remarks>
    /// Three rows rather than revision one's two. Fame moved down here from nowhere -- the card
    /// keeps the experience bar, because levelling and fame are separate things in this game and
    /// the brief's fallback of deleting one only applies where they are not.
    /// </remarks>
    public const int VitalRows = 3;

    public const float VitalsHeight = VitalRows * VitalBarHeight + (VitalRows - 1) * VitalRowGap;

    /// <summary>Bar, potion counter and the ability button, with the gaps between them.</summary>
    public const float VitalsWidth =
        VitalBarWidth + 6f + PotionBoxWidth + 8f + AbilityWidth;

    // --- Hotbar and equipment ----------------------------------------------------------------
    public const float HotbarSlotWidth = 55f;
    public const float HotbarSlotHeight = 48f;
    public const int HotbarColumns = 4;
    public const int HotbarRows = 2;
    public const float SlotGap = 4f;

    public const float EquipmentSlotWidth = 85f;
    public const float EquipmentSlotHeight = 78f;
    public const int EquipmentSlots = 4;

    /// <summary>The gap between the hotbar and the equipment row under it.</summary>
    public const float HotbarGap = 16f;

    public const float SwapWidth = 30f;
    public const float SwapHeight = 28f;

    public const float HotbarWidth = HotbarColumns * HotbarSlotWidth + (HotbarColumns - 1) * SlotGap;
    public const float HotbarHeight = HotbarRows * HotbarSlotHeight + (HotbarRows - 1) * SlotGap;

    public const float EquipmentWidth =
        EquipmentSlots * EquipmentSlotWidth + (EquipmentSlots - 1) * SlotGap;

    private readonly Vector2 _size;
    private readonly bool _guild;

    /// <param name="size">The space to solve in, in reference pixels. See <see cref="SpaceFor"/>.</param>
    /// <param name="guild">Whether the player is in a guild, which is a line on the card.</param>
    public HudLayout(Vector2 size, bool guild = true)
    {
        _size = size;
        _guild = guild;
    }

    public Vector2 Size => _size;

    /// <summary>
    /// The factor between a real window and the reference resolution, snapped to a half step.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The smaller of the two ratios, so the interface keeps its proportions on any shape of screen
    /// -- scaling by width alone would make an ultrawide's chrome enormous -- and then rounded to
    /// the nearest half. A pixel face and pixel sprites shimmer at 1.37; at 1, 1.5 and 2 every
    /// glyph and every sprite edge lands on a whole device pixel.
    /// </para>
    /// <para>
    /// Rounded rather than floored. The brief's formula floors, but its own acceptance criteria ask
    /// for 1.5 at 2560 by 1440, and flooring gives 1.0 there -- rounding is what produces all three
    /// of the numbers it lists.
    /// </para>
    /// <para>
    /// The last step is the one place this does not snap. A window too small to hold the layout at
    /// a whole step would have its clusters overlapping, which is worse than a resampled glyph, so
    /// under <see cref="MinimumSpace"/> the scale drops to whatever does fit. That only happens
    /// below about 1600 by 660; see <c>DeliberatelyFractionalBelowTheMinimum</c> in the tests.
    /// </para>
    /// </remarks>
    public static float ScaleFor(Vector2 window)
    {
        if (window.X <= 0f || window.Y <= 0f)
            return MinScale;

        float fit = Mathf.Min(window.X / ReferenceWidth, window.Y / ReferenceHeight);
        float snapped = Mathf.Max(MinScale, Mathf.Round(fit * 2f) / 2f);

        if (window.X / snapped >= MinimumSpace.X && window.Y / snapped >= MinimumSpace.Y)
            return snapped;

        return Mathf.Min(window.X / MinimumSpace.X, window.Y / MinimumSpace.Y);
    }

    /// <summary>
    /// The largest scale a window can hold the layout at without two clusters touching.
    /// </summary>
    /// <remarks>
    /// The ceiling on a hand-picked interface scale. Past it the chat panel and the vitals grow
    /// into each other, which is a worse answer to "the text is small" than a smaller number.
    /// Deliberately not floored at <see cref="MinScale"/>: a window smaller than
    /// <see cref="MinimumSpace"/> can only hold the layout at a fractional scale, and flooring here
    /// would hand back a number that overlaps -- which is the one thing this function exists to
    /// rule out. <see cref="ScaleFor"/> makes the same trade for the same reason.
    /// </remarks>
    public static float LargestFor(Vector2 window) =>
        window.X <= 0f || window.Y <= 0f ? MinScale
            : Mathf.Min(window.X / MinimumSpace.X, window.Y / MinimumSpace.Y);

    /// <summary>The rectangle the layout is solved in, for a window of the given size.</summary>
    public static Vector2 SpaceFor(Vector2 window) =>
        window.X <= 0f || window.Y <= 0f ? new Vector2(ReferenceWidth, ReferenceHeight)
            : window / ScaleFor(window);

    /// <summary>Top left: who you are.</summary>
    public Rect2 PlayerCard => new(Margin, Margin, CardWidth, CardHeight);

    /// <summary>Top right, flush to both edges: the map.</summary>
    public Rect2 Minimap => new(_size.X - MinimapWidth, 0f, MinimapWidth, MinimapHeight);

    /// <summary>
    /// Left of the minimap: gems and coins.
    /// </summary>
    /// <remarks>
    /// A box the numbers are right-aligned inside rather than a box they fill, so a number that
    /// grows by a digit grows away from the screen edge instead of into the map.
    /// </remarks>
    public Rect2 Currency => new(_size.X - MinimapWidth - 14f - 300f, 22f, 300f, 30f);

    /// <summary>Under the minimap: which world this is, and how many are in it.</summary>
    public Rect2 PartyHeader => new(
        _size.X - MinimapWidth, MinimapHeight + 2f, PartyColumns * PartyColumnWidth, 18f);

    /// <summary>Under that: everyone else nearby.</summary>
    public Rect2 Party => new(
        _size.X - MinimapWidth,
        MinimapHeight + PartyTopGap,
        PartyColumns * PartyColumnWidth,
        PartyRows * PartyRowHeight);

    /// <summary>
    /// Under the party list: what the realm wants killed next.
    /// </summary>
    /// <remarks>
    /// In the right-hand column with the map and the party, because it is the same kind of thing --
    /// something to glance at between fights rather than during one.
    /// </remarks>
    public Rect2 Quest => new(
        _size.X - MinimapWidth, Party.End.Y + 14f, PartyColumns * PartyColumnWidth, 44f);

    /// <summary>Bottom left: the log.</summary>
    public Rect2 Chat => new(
        Margin, _size.Y - ChatBottomMargin - ChatHeight, ChatWidth, ChatHeight);

    /// <summary>Bottom centre, and the only cluster measured from the middle of the screen.</summary>
    public Rect2 Vitals => new(
        (_size.X - VitalsWidth) / 2f,
        _size.Y - VitalsBottomMargin - VitalsHeight,
        VitalsWidth,
        VitalsHeight);

    /// <summary>
    /// The strip over the hotbar that chooses which eight slots it is showing.
    /// </summary>
    /// <remarks>
    /// Added in revision two, and the one thing here that grows the bottom-right cluster upward.
    /// It sits over the grid rather than pushing it, so every measurement in revision one is where
    /// it was.
    /// </remarks>
    public Rect2 HotbarTabs
    {
        get
        {
            var grid = Hotbar;
            return new Rect2(grid.Position.X, grid.Position.Y - 2f - TabHeight, grid.Size.X, TabHeight);
        }
    }

    public const float TabHeight = 22f;

    /// <summary>Bottom right: the eight carried slots.</summary>
    public Rect2 Hotbar => new(
        _size.X - Margin - HotbarWidth,
        EquipmentRow.Position.Y - HotbarGap - HotbarHeight,
        HotbarWidth,
        HotbarHeight);

    /// <summary>Under the hotbar: what is worn.</summary>
    public Rect2 EquipmentRow => new(
        _size.X - Margin - EquipmentWidth,
        _size.Y - Margin - EquipmentSlotHeight,
        EquipmentWidth,
        EquipmentSlotHeight);

    /// <summary>Left of the equipment row: the loadout cycle.</summary>
    public Rect2 Swap
    {
        get
        {
            var row = EquipmentRow;
            return new Rect2(
                row.Position.X - 12f - SwapWidth,
                row.Position.Y + (row.Size.Y - SwapHeight) / 2f,
                SwapWidth,
                SwapHeight);
        }
    }

    // --- Secondary interface -----------------------------------------------------------------

    /// <summary>The character panel and every panel that follows it.</summary>
    public const float ModalWidth = 480f;

    /// <summary>The gap between the panel and the clusters above and below it.</summary>
    public const float ModalGutter = 12f;

    /// <summary>
    /// The shortest the panel is allowed to be. Under this its list scrolls rather than shrinking
    /// further, because the attribute grid above the list is the part it exists to show.
    /// </summary>
    public const float ModalMinHeight = 400f;

    /// <summary>
    /// Where a panel that opens over the world sits.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Directly under the player card, which is where the button that opens it is: the sheet comes
    /// out of the icon row you pressed rather than appearing somewhere else on the screen. The
    /// button stack that normally occupies this space stands aside while it is open.
    /// </para>
    /// <para>
    /// Down the left, and never in the middle. It is a panel you flick open to read one number and
    /// close again, so it covers world margin and nothing else -- the centre of the screen, where
    /// the fighting is, and the whole right-hand column stay clear.
    /// </para>
    /// </remarks>
    public Rect2 Modal
    {
        get
        {
            float top = PlayerCard.End.Y + ModalGutter;
            float height = Mathf.Max(ModalMinHeight, Chat.Position.Y - ModalGutter - top);

            return new Rect2(Margin, top, ModalWidth, height);
        }
    }

    /// <summary>
    /// Every cluster, named, and whether it takes the pointer.
    /// </summary>
    /// <remarks>
    /// The list the overlap check walks. The currency is in it because it must not collide with the
    /// minimap, but it is marked as passing the pointer through: it is two numbers drawn over the
    /// world, and clicking one should walk the character rather than do nothing. The world-space
    /// overlays are not in the list at all -- they are drawn under the interface and never take a
    /// click.
    /// </remarks>
    public IEnumerable<(string Name, Rect2 Rect, bool Interactive)> Clusters()
    {
        yield return ("player-card", PlayerCard, true);
        yield return ("currency", Currency, false);
        yield return ("minimap", Minimap, true);
        yield return ("party-header", PartyHeader, false);
        yield return ("party", Party, true);
        yield return ("quest", Quest, false);
        yield return ("chat", Chat, true);
        yield return ("vitals", Vitals, true);
        yield return ("hotbar-tabs", HotbarTabs, true);
        yield return ("hotbar", Hotbar, true);
        yield return ("equipment", EquipmentRow, true);
        yield return ("swap", Swap, true);
    }

    /// <summary>
    /// Whether a point in reference pixels is swallowed by the interface.
    /// </summary>
    /// <remarks>
    /// The question the passthrough rule turns on: false means the click belongs to the world, and
    /// clicking the dark space between two panels has to answer false or the character stops
    /// responding wherever the eye says there is nothing.
    /// </remarks>
    public bool HitsCluster(Vector2 point)
    {
        foreach (var (_, rect, interactive) in Clusters())
        {
            if (interactive && rect.HasPoint(point))
                return true;
        }

        return false;
    }
}
