using System.Collections.Generic;
using Godot;

namespace Hendra.UI;

/// <summary>
/// Where every cluster of the interface sits, measured at the reference resolution.
/// </summary>
/// <remarks>
/// <para>
/// The whole layout is one struct of arithmetic with no engine in it, which is deliberate: the
/// thing that goes wrong with a HUD is two clusters growing into each other at some window shape
/// nobody opened during development, and that is a property of the numbers rather than of the
/// drawing. Kept here, it can be checked at every resolution the brief names without booting Godot
/// -- see <c>HudLayoutTests</c>.
/// </para>
/// <para>
/// The shape is one contiguous column down the right edge, 360 reference pixels wide, holding the
/// map, the icon row, the three bars, the worn equipment, the inventory, the potions, the world's
/// name and the players in it -- in that order, each band butted against the one above it. Every
/// y below is measured off <c>references/Menu/Player UI.png</c>, which is that column at 1:1, and
/// every x is measured in the same image and offset by the column's left edge.
/// </para>
/// <para>
/// The column is pinned to the top and grows downward, so a shorter screen loses rows off the
/// bottom of the player list rather than compressing the bands above it -- which is what the
/// original does and the only arrangement in which a band's height is a constant.
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
    /// The width is the column plus the chat panel plus a gap worth calling a gap. The height is
    /// everything in the column down to the potion row with one row of the player list under it;
    /// below that the list simply runs out of rows, which is a state the column is built for.
    /// </remarks>
    public static readonly Vector2 MinimumSpace = new(1560f, 960f);

    // --- The right-hand column ---------------------------------------------------------------

    /// <summary>The column's width. Measured in the reference; the whole band structure assumes it.</summary>
    public const float ColumnWidth = 360f;

    /// <summary>The band the map fills, frame included.</summary>
    public const float MinimapHeight = 367f;

    /// <summary>The light frame around the map, which doubles as the rule under it.</summary>
    public const float MinimapFrame = 4f;

    /// <summary>The zoom buttons stacked against the map's right edge.</summary>
    public const float ZoomButtonWidth = 36f;

    public const float ZoomButtonHeight = 35f;

    /// <summary>The row of small buttons between the map and the bars.</summary>
    public const float IconRowTop = MinimapHeight;

    public const float IconRowHeight = 48f;

    /// <summary>A button in that row is square and sits on the row's own centre line.</summary>
    public const float IconSize = 33f;

    /// <summary>Where each button in the icon row starts, from the column's left edge.</summary>
    /// <remarks>
    /// Transcribed rather than derived. The first four are evenly spaced, the fifth stands off on
    /// its own, and the last is pushed against the right edge -- there is no pitch that produces
    /// all three, and inventing one moves five icons to fix an arithmetic itch.
    /// </remarks>
    public static readonly float[] IconStops = { 10f, 51f, 93f, 134f, 201f, 317f };

    /// <summary>How far in from the column's edges a bar sits.</summary>
    public const float BarInset = 9f;

    public const float BarWidth = ColumnWidth - 2f * BarInset;
    public const float BarHeight = 34f;

    /// <summary>The lighter band along a bar's top edge, and the dark line along its bottom.</summary>
    public const float BarTopBand = 5f;

    public const float BarBottomLine = 3f;

    public const float FameBarTop = 415f;
    public const float HealthBarTop = 456f;
    public const float ManaBarTop = 498f;

    // --- Worn equipment ------------------------------------------------------------------------

    /// <summary>The strip the four worn slots sit on, which carries its own light frame.</summary>
    public const float EquipmentTop = 540f;

    public const float EquipmentHeight = 90f;
    public const float EquipmentLeft = 11f;
    public const float EquipmentStripWidth = 338f;

    /// <summary>A worn slot, counted to the outside of its dark bevel.</summary>
    public const float EquipmentSlotWidth = 78f;

    public const float EquipmentSlotHeight = 83f;

    public const int EquipmentSlots = 4;

    /// <summary>The bevel between a worn slot's plate and the light frame around it.</summary>
    public const float EquipmentBevel = 4f;

    // --- Inventory -----------------------------------------------------------------------------

    /// <summary>The dark page the tabs, the slots and the potions all sit on.</summary>
    public const float InventoryLeft = 7f;

    public const float InventoryWidth = 346f;
    public const float InventoryTop = 639f;
    public const float InventoryBottom = 900f;

    public const float TabHeight = 45f;
    public const float TabGap = 4f;

    public const float HotbarLeft = 16f;
    public const float HotbarTop = 684f;

    /// <summary>A carried slot, counted to the outside of its four-pixel border.</summary>
    public const float HotbarSlotWidth = 79f;

    public const float HotbarSlotHeight = 79f;

    /// <summary>The gutter between two slots, which is the page showing through.</summary>
    public const float SlotGap = 4f;

    public const int HotbarColumns = 4;
    public const int HotbarRows = 2;

    public const float HotbarWidth =
        HotbarColumns * HotbarSlotWidth + (HotbarColumns - 1) * SlotGap;

    public const float HotbarHeight = HotbarRows * HotbarSlotHeight + (HotbarRows - 1) * SlotGap;

    /// <summary>The stacked potions, along the bottom of the page.</summary>
    public const float PotionTop = 850f;

    public const float PotionHeight = 41f;

    /// <summary>
    /// How many cells the rack is divided into, which is not how many a character has.
    /// </summary>
    /// <remarks>
    /// The rack spans the four carried slots and is cut into three, so a cell is 107 wide with the
    /// usual four-pixel gutter -- measured off the reference, whose player had unlocked the third.
    /// A fresh character has two, and they sit in the first two of these three positions rather
    /// than sharing the width out between them, so unlocking one moves nothing.
    /// </remarks>
    public const int PotionRackColumns = 3;

    // --- The world and the players in it -------------------------------------------------------

    public const float WorldNameTop = 900f;

    public const float WorldNameHeight = 40f;

    public const float PartyTop = 940f;
    public const int PartyColumns = 2;
    public const float PartyColumnWidth = 177f;
    public const float PartyRowHeight = 46.5f;
    public const float PartyLeft = 12f;

    /// <summary>The class portrait beside a name in the list.</summary>
    public const float PartyPortrait = 32f;

    /// <summary>The most rows the list is ever built with. Any that do not fit are hidden.</summary>
    public const int PartyRows = 4;

    // --- Player card ---------------------------------------------------------------------------

    /// <summary>
    /// The identity block in the top-left corner: portrait, name, guild and rating.
    /// </summary>
    /// <remarks>
    /// Flush to both edges rather than inset by a margin. There is no plate under it in the
    /// reference -- only the portrait carries one -- so a margin would be a gap around nothing.
    /// </remarks>
    public const float CardWidth = 420f;

    public const float CardHeight = 80f;

    /// <summary>The portrait's plate, which is the only opaque thing in the corner.</summary>
    public const float AvatarSize = 72f;

    public const float AvatarLeft = 7f;
    public const float AvatarTop = 7f;

    /// <summary>Where the name and the guild start, clear of the portrait.</summary>
    public const float CardTextLeft = 87f;

    // --- Chat ----------------------------------------------------------------------------------
    public const float ChatWidth = 640f;

    public const float ChatHeight = 240f;

    /// <summary>The folded row sits on the screen's bottom edge, not on a margin off it.</summary>
    public const float ChatBottomMargin = 6f;

    // --- Kept for the panels that measure against the old bottom-centre cluster ----------------

    /// <summary>How much room the bottom edge of the screen still owes a panel opening over it.</summary>
    public const float VitalsBottomMargin = 26f;

    public const float VitalsHeight = 3f * BarHeight;

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
    /// The last step is the one place this does not snap. A window too small to hold the layout at
    /// a whole step would have its clusters overlapping, which is worse than a resampled glyph, so
    /// under <see cref="MinimumSpace"/> the scale drops to whatever does fit.
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
    public static float LargestFor(Vector2 window) =>
        window.X <= 0f || window.Y <= 0f ? MinScale
            : Mathf.Max(MinScale, Mathf.Min(window.X / MinimumSpace.X, window.Y / MinimumSpace.Y));

    /// <summary>The rectangle the layout is solved in, for a window of the given size.</summary>
    public static Vector2 SpaceFor(Vector2 window) =>
        window.X <= 0f || window.Y <= 0f ? new Vector2(ReferenceWidth, ReferenceHeight)
            : window / ScaleFor(window);

    /// <summary>The column's left edge, which almost everything on the right is measured from.</summary>
    public float ColumnLeft => _size.X - ColumnWidth;

    /// <summary>The whole right-hand column, floor to ceiling.</summary>
    public Rect2 Column => new(ColumnLeft, 0f, ColumnWidth, _size.Y);

    /// <summary>A rectangle given in the reference crop's own coordinates.</summary>
    private Rect2 InColumn(float x, float y, float width, float height) =>
        new(ColumnLeft + x, y, width, height);

    /// <summary>Top right, flush to both edges: the map and its frame.</summary>
    public Rect2 Minimap => InColumn(0f, 0f, ColumnWidth, MinimapHeight);

    /// <summary>The map's painted area, inside the frame.</summary>
    public Rect2 MinimapFace => Minimap.Grow(-MinimapFrame);

    /// <summary>Under the map: stats, pet, alignment, quests, party and settings.</summary>
    public Rect2 IconRow => InColumn(0f, IconRowTop, ColumnWidth, IconRowHeight);

    /// <summary>Where one icon in that row sits, by its index in <see cref="IconStops"/>.</summary>
    public Rect2 IconAt(int index) => InColumn(
        IconStops[Mathf.Clamp(index, 0, IconStops.Length - 1)],
        IconRowTop + Mathf.Round((IconRowHeight - IconSize) / 2f),
        IconSize,
        IconSize);

    public Rect2 FameBar => InColumn(BarInset, FameBarTop, BarWidth, BarHeight);

    public Rect2 HealthBar => InColumn(BarInset, HealthBarTop, BarWidth, BarHeight);

    public Rect2 ManaBar => InColumn(BarInset, ManaBarTop, BarWidth, BarHeight);

    /// <summary>The lighter strip the four worn slots are set into.</summary>
    public Rect2 EquipmentRow =>
        InColumn(EquipmentLeft, EquipmentTop, EquipmentStripWidth, EquipmentHeight);

    /// <summary>
    /// One worn slot, counted to the outside of its bevel.
    /// </summary>
    /// <remarks>
    /// The pitch is fractional in the reference -- the four cells are 70, 71, 71 and 70 wide -- so
    /// the position is rounded per slot rather than the width being fudged to make it whole.
    /// </remarks>
    public Rect2 EquipmentSlot(int index) => InColumn(
        Mathf.Round(17f + index * 82.5f),
        EquipmentTop + 6f,
        EquipmentSlotWidth,
        EquipmentSlotHeight);

    /// <summary>The dark page: tabs at the top, then the carried slots, then the potions.</summary>
    public Rect2 InventoryPanel => InColumn(
        InventoryLeft, InventoryTop, InventoryWidth, InventoryBottom - InventoryTop);

    /// <summary>The two tabs over the page, which choose which eight slots it is showing.</summary>
    public Rect2 HotbarTabs => InColumn(InventoryLeft, InventoryTop, InventoryWidth, TabHeight);

    public Rect2 Hotbar => InColumn(HotbarLeft, HotbarTop, HotbarWidth, HotbarHeight);

    public Rect2 HotbarSlot(int index) => InColumn(
        HotbarLeft + index % HotbarColumns * (HotbarSlotWidth + SlotGap),
        HotbarTop + index / HotbarColumns * (HotbarSlotHeight + SlotGap),
        HotbarSlotWidth,
        HotbarSlotHeight);

    /// <summary>The stacked potions, which are addressed by slot id rather than by index.</summary>
    public Rect2 PotionRow => InColumn(HotbarLeft, PotionTop, HotbarWidth, PotionHeight);

    /// <summary>
    /// One potion cell. The rack is cut into three across the span the four carried slots use.
    /// </summary>
    public Rect2 PotionSlot(int index)
    {
        float pitch = (HotbarWidth + SlotGap) / PotionRackColumns;
        float left = Mathf.Round(HotbarLeft + index * pitch);
        float right = Mathf.Round(HotbarLeft + (index + 1) * pitch) - SlotGap;

        return InColumn(left, PotionTop, right - left, PotionHeight);
    }

    /// <summary>Under the page: which world this is, and how many are in it.</summary>
    public Rect2 PartyHeader => InColumn(0f, WorldNameTop, ColumnWidth, WorldNameHeight);

    /// <summary>Under that: everyone else nearby, two to a row.</summary>
    public Rect2 Party => InColumn(
        PartyLeft,
        PartyTop,
        PartyColumns * PartyColumnWidth,
        Mathf.Max(0f, _size.Y - PartyTop));

    /// <summary>Where one row of the list sits, relative to <see cref="Party"/>.</summary>
    public Rect2 PartyEntry(int index) => new(
        index % PartyColumns * PartyColumnWidth,
        Mathf.Round(index / PartyColumns * PartyRowHeight),
        PartyColumnWidth,
        PartyPortrait);

    /// <summary>How many rows of the list the screen actually has room for.</summary>
    public int PartyRowsThatFit =>
        Mathf.Clamp(Mathf.FloorToInt((_size.Y - PartyTop) / PartyRowHeight), 0, PartyRows);

    /// <summary>Top left: who you are.</summary>
    public Rect2 PlayerCard => new(0f, 0f, CardWidth, CardHeight);

    /// <summary>Under it: the time, and how far through the level the character is.</summary>
    public Rect2 Clock => new(13f, 84f, 300f, 36f);

    /// <summary>A rule across the corner, under the clock.</summary>
    public Rect2 XpBar => new(60f, 140f, 353f, 4f);

    /// <summary>
    /// Left of the column, along the top: gems and coins.
    /// </summary>
    /// <remarks>
    /// A box the numbers are right-aligned inside rather than a box they fill, so a number that
    /// grows by a digit grows away from the screen edge instead of into the map.
    /// </remarks>
    public Rect2 Currency => new(ColumnLeft - 14f - 300f, 22f, 300f, 30f);

    /// <summary>Under the experience bar: the heading and the objective panel under it.</summary>
    public Rect2 Quest => new(16f, 164f, 396f, 103f);

    /// <summary>Bottom left: the log.</summary>
    public Rect2 Chat => new(
        0f, _size.Y - ChatBottomMargin - ChatHeight, ChatWidth, ChatHeight);

    // --- Secondary interface -----------------------------------------------------------------

    /// <summary>The character panel and every panel that follows it.</summary>
    public const float ModalWidth = 432f;

    /// <summary>The gap between the panel and the column beside it.</summary>
    public const float ModalGutter = 10f;

    /// <summary>
    /// The shortest the panel is allowed to be. Under this its list scrolls rather than shrinking
    /// further, because the attribute grid above the list is the part it exists to show.
    /// </summary>
    public const float ModalMinHeight = 400f;

    /// <summary>
    /// Where a panel that opens over the world sits.
    /// </summary>
    /// <remarks>
    /// Butted against the left edge of the column and running the full height of the screen, which
    /// is where the reference puts it: the panel reads as a second column that slid out from under
    /// the first rather than as a dialog that appeared somewhere. It covers world and nothing else
    /// in the permanent interface -- the column stays fully visible beside it, which is the point,
    /// since half of what the panel says is only meaningful next to the bars.
    /// </remarks>
    public Rect2 Modal => new(
        ColumnLeft - ModalGutter - ModalWidth,
        4f,
        ModalWidth,
        Mathf.Max(ModalMinHeight, _size.Y - 8f));

    /// <summary>
    /// The bottom of the column, when the player is standing on something they can enter.
    /// </summary>
    /// <remarks>
    /// It takes the space the world's name and the player list occupy, rather than opening
    /// somewhere else: what is under your feet is more urgent than who else is in the room, and
    /// the two never need reading at the same moment.
    /// </remarks>
    public Rect2 Interact => InColumn(0f, WorldNameTop, ColumnWidth, Mathf.Max(0f, _size.Y - WorldNameTop));

    /// <summary>The steel plate in that block, which is where the reference puts it exactly.</summary>
    public Rect2 InteractButton => InColumn(27f, 996f, 306f, 53f);

    /// <summary>
    /// Every cluster, named, and whether it takes the pointer.
    /// </summary>
    /// <remarks>
    /// The list the overlap check walks. The currency is in it because it must not collide with the
    /// column, but it is marked as passing the pointer through: it is two numbers drawn over the
    /// world, and clicking one should walk the character rather than do nothing.
    /// </remarks>
    public IEnumerable<(string Name, Rect2 Rect, bool Interactive)> Clusters()
    {
        // The corner is outlined text over the world with one plate under the portrait, and none
        // of it is a button any more -- the panels it used to open are on the column's icon row.
        yield return ("player-card", PlayerCard, false);
        yield return ("quest", Quest, false);
        yield return ("currency", Currency, false);
        yield return ("column", Column, true);
        yield return ("chat", Chat, true);
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
