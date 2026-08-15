using System;
using System.Globalization;
using Godot;

namespace Hendra.UI;

/// <summary>
/// The greys the right-hand column is built out of that the shared palette does not name.
/// </summary>
/// <remarks>
/// Every one is eyedropped from <c>references/Menu/Player UI.png</c> and used in exactly one place,
/// which is why they are here rather than in <see cref="Style"/>: a token is for a colour two
/// clusters have to agree on, and none of these is. If a second cluster ever wants one, it should
/// move rather than be copied.
/// </remarks>
internal static class ColumnInk
{
    /// <summary>The frame around the map, whose bottom edge is also the rule under it.</summary>
    public static readonly Color Frame = new("7d7d7d");

    /// <summary>The light frame around the strip of worn equipment.</summary>
    public static readonly Color StripEdge = new("9c9c9c");

    /// <summary>The dark step between that frame and a worn slot's plate.</summary>
    public static readonly Color StripBevel = new("4f4f4f");

    /// <summary>A potion cell's plate, a shade under an inventory slot's.</summary>
    public static readonly Color PotionPlate = new("545454");

    /// <summary>An unselected inventory tab. The selected one is the page colour itself.</summary>
    public static readonly Color TabIdle = new("474747");

    /// <summary>The world's name under the inventory.</summary>
    public static readonly Color WorldName = new("7b7c7d");

    /// <summary>A nearby player's name: the game's gold, held well back so the bars stay loudest.</summary>
    public static readonly Color PlayerName = new("80741b");

    /// <summary>A guildmate's name, in the same register as the gold.</summary>
    public static readonly Color GuildName = new("318769");

    /// <summary>A map-zoom button that can still be pressed, and one that has run out of steps.</summary>
    public static readonly Color ButtonPlate = new("7b7b7b");

    public static readonly Color ButtonPlateSpent = new("303030");

    /// <summary>An icon-row button for a panel this server does not have.</summary>
    public static readonly Color IconDisabled = new("7b7b7b");

    /// <summary>The number an empty carried slot carries, which is barely off its own plate.</summary>
    public static readonly Color EmptySlotNumber = new("464646");

    /// <summary>How far the column's shadow reaches over the world beside it.</summary>
    public const float ShadowWidth = 5f;

    /// <summary>How much of the world that shadow takes out.</summary>
    public static readonly Color Shadow = new(0f, 0f, 0f, 0.55f);
}

/// <summary>
/// The plate the whole right-hand column is drawn on.
/// </summary>
/// <remarks>
/// One rectangle rather than a plate per band. The bands butt against each other with no gaps in
/// the reference, and drawing each of them its own background is how a one-pixel seam appears
/// between two of them at some scale nobody tested.
/// </remarks>
public sealed partial class ColumnPlate : Control
{
    public ColumnPlate()
    {
        // The one part of the interface that is genuinely solid: a click anywhere on the column
        // belongs to the column, including the dark space between two bands.
        MouseFilter = MouseFilterEnum.Stop;
    }

    public override void _Draw()
    {
        // The shadow first, outside the plate, so the column reads as sitting over the world
        // rather than as a hole cut in it.
        DrawRect(new Rect2(-ColumnInk.ShadowWidth, 0f, ColumnInk.ShadowWidth, Size.Y), ColumnInk.Shadow);
        DrawRect(new Rect2(Vector2.Zero, Size), Style.Panel);
    }
}

/// <summary>
/// The strip the four worn slots sit in: a light frame with a dark step inside it.
/// </summary>
/// <remarks>
/// The frame is drawn by the strip and not by the slots, because in the reference the four cells
/// share their dividers -- there is one four-pixel light line between two slots, not two.
/// </remarks>
public sealed partial class EquipmentStrip : Control
{
    /// <summary>Where each slot sits, so the strip knows where to put its dividers.</summary>
    public Rect2[] Cells { get; set; } = Array.Empty<Rect2>();

    public EquipmentStrip()
    {
        MouseFilter = MouseFilterEnum.Ignore;
    }

    public override void _Draw()
    {
        DrawRect(new Rect2(Vector2.Zero, Size), ColumnInk.StripEdge);

        // Each cell's dark step. The slots themselves draw their plates on top of these.
        foreach (var cell in Cells)
            DrawRect(cell, ColumnInk.StripBevel);
    }
}

/// <summary>The near-black page the tabs, the carried slots and the potions are bedded into.</summary>
public sealed partial class InventoryPage : Control
{
    public InventoryPage()
    {
        MouseFilter = MouseFilterEnum.Ignore;
    }

    public override void _Draw() => DrawRect(new Rect2(Vector2.Zero, Size), Style.PanelInset);
}

/// <summary>
/// One of the two tabs over the carried slots.
/// </summary>
/// <remarks>
/// The selected tab is the page's own colour and the unselected one is lighter, which is the
/// reverse of the usual arrangement and is what the reference does: the selected tab is a hole
/// continuous with the page under it, and the other is a lid still closed over its own page.
/// </remarks>
public sealed partial class ColumnTab : Control
{
    private readonly Action<CanvasItem, Rect2, Color> _icon;

    private bool _hovered;
    private bool _active;
    private bool _enabled = true;

    /// <summary>The glyph's side, which the reference draws at nineteen pixels.</summary>
    private const float GlyphSize = 19f;

    /// <summary>How far above the tab's own centre the glyph sits.</summary>
    private const float GlyphRise = 5f;

    public ColumnTab(Action<CanvasItem, Rect2, Color> icon)
    {
        _icon = icon;
        MouseFilter = MouseFilterEnum.Stop;
        FocusMode = FocusModeEnum.None;
    }

    public event Action Pressed;

    public bool Active
    {
        get => _active;
        set { _active = value; QueueRedraw(); }
    }

    /// <summary>A page the character does not have -- a backpack they have not bought.</summary>
    public bool Enabled
    {
        get => _enabled;
        set { _enabled = value; QueueRedraw(); }
    }

    public override void _Ready()
    {
        MouseEntered += () => { _hovered = true; QueueRedraw(); };
        MouseExited += () => { _hovered = false; QueueRedraw(); };
    }

    public override void _GuiInput(InputEvent @event)
    {
        if (@event is not InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
            return;

        AcceptEvent();

        if (_enabled)
            Pressed?.Invoke();
    }

    public override void _Draw()
    {
        var face = _active ? Style.PanelInset
            : _hovered && _enabled ? ColumnInk.TabIdle.Lightened(0.12f)
            : ColumnInk.TabIdle;

        DrawRect(new Rect2(Vector2.Zero, Size), face);

        var box = new Rect2(
            Mathf.Round((Size.X - GlyphSize) / 2f),
            Mathf.Round((Size.Y - GlyphSize) / 2f) - GlyphRise,
            GlyphSize,
            GlyphSize);

        _icon(this, box, _enabled ? Style.Text : Style.TextDim.Darkened(0.4f));
    }
}

/// <summary>
/// One of the three stacked potions along the bottom of the inventory.
/// </summary>
/// <remarks>
/// A slot rather than a chip: the same plate and border as the carried slots, holding the potion,
/// how many are held and how many can be. It is a button as well as a counter -- the potions live
/// outside the inventory array, addressed on the wire by slot id rather than by index, so this is
/// the only place they can be clicked.
/// </remarks>
public sealed partial class PotionCell : Control
{
    /// <summary>The border, which is the same four pixels a carried slot carries.</summary>
    private const float Border = 4f;

    private readonly bool _health;

    private Assets.Sprite _bottle;
    private int _count;
    private int _of;
    private bool _hovered;

    public PotionCell(bool health, int of)
    {
        _health = health;
        _of = of;
        MouseFilter = MouseFilterEnum.Stop;
        FocusMode = FocusModeEnum.None;
        TooltipText = health ? "Drink a health potion" : "Drink a magic potion";
    }

    public event Action Pressed;

    /// <summary>Raised when a potion is dropped onto this cell, with where it came from.</summary>
    public event Action<World.SlotAddress> Filled;

    /// <summary>
    /// Only takes what it is a counter for.
    /// </summary>
    /// <remarks>
    /// Godot asks this while the drag is over the control and refuses the drop itself when it
    /// answers false, so a health potion dragged onto the magic stack never leaves the cursor.
    /// </remarks>
    public Func<World.SlotAddress, bool> Accepts { get; set; }

    public override bool _CanDropData(Vector2 atPosition, Variant data) =>
        SlotView.PayloadAddress(data, out var from) && (Accepts?.Invoke(from) ?? false);

    public override void _DropData(Vector2 atPosition, Variant data)
    {
        if (SlotView.PayloadAddress(data, out var from))
            Filled?.Invoke(from);
    }

    public void UseSprite(Assets.Sprite sprite)
    {
        _bottle = sprite;
        QueueRedraw();
    }

    public void Set(int count, int of)
    {
        if (_count == count && _of == of)
            return;

        _count = count;
        _of = of;
        QueueRedraw();
    }

    public override void _Ready()
    {
        MouseEntered += () => { _hovered = true; QueueRedraw(); };
        MouseExited += () => { _hovered = false; QueueRedraw(); };
    }

    public override void _GuiInput(InputEvent @event)
    {
        if (@event is InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
        {
            Pressed?.Invoke();
            AcceptEvent();
        }
    }

    /// <summary>
    /// What colour a count of something you can run out of is written in.
    /// </summary>
    /// <remarks>
    /// Green at full, amber below it, red at the last one. The reference reads exactly this way --
    /// six of six green, four of six amber, one of four red -- and it is the fact you want off a
    /// glance while something is hitting you.
    /// </remarks>
    private static Color Supply(int held, int of) =>
        held <= 0 ? Style.StatPenalty
        : held >= of ? Style.StatNumber
        : held * 3 <= of ? Style.ButtonDangerHigh
        : Style.TierSpecial;

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);

        DrawRect(full, _hovered ? ColumnInk.PotionPlate.Lightened(0.10f) : ColumnInk.PotionPlate);
        DrawRect(full.Grow(-Border / 2f), _hovered ? Style.SlotBorderHi : Style.SlotBorder,
            filled: false, width: Border);

        // A cell for a stack this server does not carry is a plate and nothing else. A permanent
        // nought out of nought would be a number that is not true of anything.
        if (_of <= 0)
            return;

        // The game's own potion, not a drawing of one.
        if (_bottle.IsValid)
        {
            float side = Mathf.Min(Size.Y - 4f, 34f);
            this.DrawSprite(_bottle, new Rect2(6f, Mathf.Round((Size.Y - side) / 2f), side, side));
        }

        string held = _count.ToString(CultureInfo.InvariantCulture);
        string of = "/" + _of.ToString(CultureInfo.InvariantCulture);

        // The two halves are set differently on purpose: the number you have is the one being read
        // and the ceiling is a caption on it, so it is a size down, grey, and sitting on the floor
        // of the cell rather than on the count's own baseline.
        float left = Mathf.Round(Size.X * 0.48f);

        this.DrawOverWorld(
            new Vector2(left, Style.BaselineIn(Size.Y, CountSize)), held, CountSize, Supply(_count, _of));

        this.DrawOverWorld(
            new Vector2(left + Style.Measure(held, CountSize) + 3f, Size.Y - 7f),
            of, Style.FontSmall, Style.TextDim);
    }

    /// <summary>How many are held, which is the largest number in the column after the bars.</summary>
    private const int CountSize = 28;
}

/// <summary>
/// One nearby player: their class portrait and their name, two to a row.
/// </summary>
/// <remarks>
/// Drawn rather than made into a label and a portrait, because the whole list is redrawn twice a
/// second and six rows of two nodes each is twelve controls to keep in step for text that never
/// moves. The portrait is dimmed to the same degree the name is: this list is the quietest thing
/// in the column and has to stay behind the bars above it.
/// </remarks>
public sealed partial class PartyRow : Control
{
    /// <summary>How much of the sprite's own colour survives. The reference holds it well back.</summary>
    private static readonly Color PortraitDim = new(0.62f, 0.62f, 0.62f, 1f);

    private Assets.Sprite _portrait;
    private string _name = string.Empty;
    private bool _guildmate;
    private bool _hovered;

    public PartyRow()
    {
        MouseFilter = MouseFilterEnum.Stop;
        FocusMode = FocusModeEnum.None;
    }

    public event Action<string> Activated;

    public void Set(string name, Assets.Sprite portrait, bool guildmate)
    {
        if (_name == name && _guildmate == guildmate && _portrait.Region == portrait.Region)
        {
            Visible = true;
            return;
        }

        _name = name ?? string.Empty;
        _portrait = portrait;
        _guildmate = guildmate;
        Visible = true;
        TooltipText = _name;
        QueueRedraw();
    }

    public override void _Ready()
    {
        MouseEntered += () => { _hovered = true; QueueRedraw(); };
        MouseExited += () => { _hovered = false; QueueRedraw(); };
    }

    public override void _GuiInput(InputEvent @event)
    {
        if (@event is InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
        {
            Activated?.Invoke(_name);
            AcceptEvent();
        }
    }

    public override void _Draw()
    {
        float side = HudLayout.PartyPortrait;

        if (_portrait.IsValid)
        {
            DrawTextureRectRegion(
                _portrait.Sheet, new Rect2(0f, 0f, side, side), _portrait.Region, PortraitDim);
        }

        var colour = _guildmate ? ColumnInk.GuildName : ColumnInk.PlayerName;
        if (_hovered)
            colour = colour.Lightened(0.45f);

        this.DrawText(
            new Vector2(side + 8f, Style.BaselineIn(side, NameSize)), _name, NameSize, colour);
    }

    /// <summary>A name in the list, set at the same size as the bars' own labels.</summary>
    private const int NameSize = 24;
}
