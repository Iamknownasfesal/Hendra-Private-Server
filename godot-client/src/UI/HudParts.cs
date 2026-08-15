using System;
using Godot;

namespace Hendra.UI;

/// <summary>
/// A panel behind a cluster: opaque grey plate with a one-pixel dark edge, square cornered.
/// </summary>
/// <remarks>
/// Stops the pointer, which is the whole reason it is a node rather than a rectangle drawn by
/// whoever owns it. The overlay root passes clicks through to the world; a panel is the exception,
/// and the exception has to be a control the engine can hit-test.
/// </remarks>
public partial class HudPanel : Control
{
    private Color _background;

    public HudPanel(Color background)
    {
        _background = background;
        MouseFilter = MouseFilterEnum.Stop;
    }

    /// <summary>Repaints the panel in another colour. Used by the minimap, which is drawn on.</summary>
    public Color Background
    {
        get => _background;
        set { _background = value; QueueRedraw(); }
    }

    /// <summary>Whether to draw the one-pixel outer border. Off for a panel inside another.</summary>
    public bool Edged { get; set; } = true;

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);

        DrawRect(full, _background);

        if (Edged)
            DrawRect(full, Style.PanelEdge, filled: false, width: 1f);
    }
}

/// <summary>
/// A labelled bar. Fame, health and magic are all this control.
/// </summary>
/// <remarks>
/// <para>
/// Three bands rather than a fill and an edge: a lighter strip along the top five pixels, the flat
/// fill under it, and a hard dark line along the bottom three. That is what gives the bar its
/// thickness in the reference, and it is why there is no outer border -- the light band and the
/// dark line are the border, and they belong to the <i>fill</i>, so an emptying bar loses them
/// with the colour rather than keeping a frame around a hole.
/// </para>
/// <para>
/// The stat's name sits inset on the left and its value is centred on the whole bar, not on the
/// filled part. Centring is the thing that reads as this game rather than as a progress bar: the
/// number stays where the eye already is however much health is left.
/// </para>
/// <para>
/// The fill eases towards its value over about 150 milliseconds rather than snapping, and a drop
/// leaves a lighter chip behind that catches up over 400. A bar that jumps tells you the number
/// changed; a bar that slides tells you which way and by how much, which is the thing you actually
/// need while something is hitting you.
/// </para>
/// </remarks>
public partial class HudBar : Control
{
    /// <summary>How far in from the left edge the stat's name sits.</summary>
    private const float Inset = 9f;

    /// <summary>How fast the fill converges. Higher is quicker; twenty settles in about 150ms.</summary>
    private const float FillRate = 20f;

    /// <summary>The chip's rate, slow enough that a hit leaves a mark you can see after it lands.</summary>
    private const float ChipRate = 6f;

    /// <summary>How much of a fill's own colour survives in the line under it.</summary>
    private const float BottomLineDarkening = 0.855f;

    private readonly int _fontSize;

    private string _name = string.Empty;
    private string _value = string.Empty;

    private float _target;
    private float _shown;
    private float _chip;

    /// <summary>
    /// What a bar's label and value are set at.
    /// </summary>
    /// <remarks>
    /// Thirty, which is the size at which this face's capitals stand sixteen pixels tall -- the
    /// height the reference's own do. It is the largest type in the interface and deliberately so:
    /// these three strings are the ones that have to be readable without looking away from what is
    /// hitting you.
    /// </remarks>
    public const int BarFontSize = 30;

    public HudBar(Color fill, Color high, int fontSize = BarFontSize)
    {
        _fill = fill;
        _high = high;
        _fontSize = fontSize;
        MouseFilter = MouseFilterEnum.Ignore;
    }

    private Color _fill;
    private Color _high;

    /// <summary>
    /// The bar's colour. Repaints on assignment, since a full bar has nothing else to redraw for.
    /// </summary>
    public Color Fill
    {
        get => _fill;
        set
        {
            if (_fill == value)
                return;

            _fill = value;
            QueueRedraw();
        }
    }

    /// <summary>The lighter band along the top of the fill.</summary>
    public Color High
    {
        get => _high;
        set
        {
            if (_high == value)
                return;

            _high = value;
            QueueRedraw();
        }
    }

    /// <summary>
    /// What the centred value is written in.
    /// </summary>
    /// <remarks>
    /// Green on health and magic, white on fame, which is the reference's own arrangement: the two
    /// numbers that change while you are being hit share a colour so a glance finds both at once.
    /// </remarks>
    public Color ValueColour { get; set; } = Style.Text;

    /// <summary>Sets the fill, the name on the left and the value centred on the bar.</summary>
    public void Set(int current, int maximum, string name, string value)
    {
        _target = maximum > 0 ? Mathf.Clamp(current / (float)maximum, 0f, 1f) : 0f;

        if (_name == name && _value == value)
            return;

        _name = name;
        _value = value;
        QueueRedraw();
    }

    public override void _Process(double delta)
    {
        float before = _shown;

        _shown = Mathf.Lerp(_shown, _target, 1f - Mathf.Exp(-FillRate * (float)delta));
        if (Mathf.Abs(_target - _shown) < 0.0005f)
            _shown = _target;

        _chip = _chip < _shown
            ? _shown
            : Mathf.Lerp(_chip, _shown, 1f - Mathf.Exp(-ChipRate * (float)delta));

        if (!Mathf.IsEqualApprox(before, _shown) || _chip > _shown + 0.0005f)
            QueueRedraw();
    }

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);

        DrawRect(full, Style.BarTrack);

        if (_chip > _shown)
            DrawRect(new Rect2(0f, 0f, Size.X * _chip, Size.Y), Fill.Lerp(Style.Text, 0.45f));

        float width = Mathf.Round(Size.X * _shown);
        if (width >= 1f)
        {
            float top = HudLayout.BarTopBand;
            float bottom = HudLayout.BarBottomLine;

            DrawRect(new Rect2(0f, 0f, width, top), High);
            DrawRect(new Rect2(0f, top, width, Size.Y - top - bottom), Fill);
            DrawRect(new Rect2(0f, Size.Y - bottom, width, bottom), Fill.Darkened(BottomLineDarkening));
        }

        // Both strings outlined, because the bar under them is a saturated colour and the value is
        // written in another one -- green on green is exactly the case an outline exists for.
        float baseline = Style.BaselineIn(Size.Y, _fontSize);

        if (_name.Length > 0)
            this.DrawOverWorld(new Vector2(Inset, baseline), _name, _fontSize, Style.Text);

        if (_value.Length == 0)
            return;

        this.DrawOverWorld(
            new Vector2(Mathf.Round((Size.X - Style.Measure(_value, _fontSize)) / 2f), baseline),
            _value, _fontSize, ValueColour);
    }
}

/// <summary>
/// One of the small square buttons on the player card.
/// </summary>
/// <remarks>
/// A Control rather than a Button. The engine's Button would want to draw its own plate under the
/// glyph, and -- more to the point -- a focused Button eats Enter, which in this client opens the
/// chat box. See the note on focus in <see cref="HudView"/>.
/// </remarks>
public partial class HudIconButton : Control
{
    private readonly Action<CanvasItem, Rect2, Color> _icon;
    private readonly float _inset;

    private bool _hovered;

    public HudIconButton(Action<CanvasItem, Rect2, Color> icon, string tooltip, float inset = 3f)
    {
        _icon = icon;
        _inset = inset;
        TooltipText = tooltip;
        MouseFilter = MouseFilterEnum.Stop;
        FocusMode = FocusModeEnum.None;
    }

    public event Action Pressed;

    public Color Tint { get; set; } = Style.Text;

    /// <summary>The plate that appears under the glyph while the pointer is on it.</summary>
    /// <remarks>
    /// Settable because the button appears on two different greys: on the column it has to be a
    /// step off <see cref="Style.Panel"/>, and the steel of an ordinary button there reads as a
    /// coloured square somebody left behind.
    /// </remarks>
    public Color Hover { get; set; } = Style.ButtonHover;

    /// <summary>
    /// A button for something this build has no panel for.
    /// </summary>
    /// <remarks>
    /// Greyed and inert rather than absent. The reference's own icon row has one of these in it --
    /// the party sword, with no party to open -- and a row that closes up around a missing icon
    /// moves every icon after it.
    /// </remarks>
    public bool Disabled { get; set; }

    private int _badge;

    /// <summary>
    /// A count in the corner, or zero for none.
    /// </summary>
    /// <remarks>
    /// Drawn as a bare mark when the number is one, because "1" beside an icon reads as a label
    /// rather than as a count and the only thing that matters is that there is something new.
    /// </remarks>
    public int Badge
    {
        get => _badge;
        set
        {
            if (_badge == value)
                return;

            _badge = value;
            QueueRedraw();
        }
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

        if (!Disabled)
            Pressed?.Invoke();
    }

    public override void _Draw()
    {
        // A lit plate under the glyph rather than a white wash over it: the chrome is opaque grey
        // now and a translucent overlay on it just looks like a smudge.
        if (_hovered && !Disabled)
            DrawRect(new Rect2(Vector2.Zero, Size), Hover);

        _icon?.Invoke(this, new Rect2(Vector2.Zero, Size).Grow(-_inset),
            Disabled ? Tint.Darkened(0.5f) : Tint);

        if (_badge <= 0)
            return;

        if (_badge == 1)
        {
            DrawRect(new Rect2(Size.X - 8f, 1f, 6f, 6f), Style.PanelEdge);
            DrawRect(new Rect2(Size.X - 7f, 2f, 4f, 4f), Style.HpFill);
            return;
        }

        string count = _badge.ToString(System.Globalization.CultureInfo.InvariantCulture);
        float width = Style.Measure(count, Style.FontTag) + 4f;
        var plate = new Rect2(Size.X - width, 0f, width, 11f);

        DrawRect(plate, Style.HpFill);
        this.DrawToken(new Vector2(plate.Position.X + 2f, plate.End.Y - 3f), count, Style.FontTag, Style.Text);
    }
}

/// <summary>
/// One of the buttons under the player card.
/// </summary>
/// <remarks>
/// A flat face with a one-pixel two-tone bevel -- light along the top and left, dark along the
/// bottom and right -- which inverts while the button is held, and the label shifts a pixel down
/// and right with it. That is the whole of the press feedback, and at this scale it is enough:
/// the plate visibly goes in.
/// </remarks>
public partial class HudMenuButton : Control
{
    private string _label;

    private bool _hovered;
    private bool _held;
    private bool _badged;
    private bool _disabled;

    public HudMenuButton(string label)
    {
        _label = label;
        MouseFilter = MouseFilterEnum.Stop;
        FocusMode = FocusModeEnum.None;
    }

    public event Action Pressed;

    /// <summary>What the plate says. Settable, because the interact plate changes verb.</summary>
    public string Label
    {
        get => _label;
        set
        {
            if (_label == value)
                return;

            _label = value ?? string.Empty;
            QueueRedraw();
        }
    }

    /// <summary>
    /// The face colour, for the one button that carries the accent.
    /// </summary>
    /// <remarks>
    /// Special Offer, and nothing else. One saturated thing in the corner is a thing being pointed
    /// at; three are a decorated corner.
    /// </remarks>
    public Color Face
    {
        get => Plate.Face;
        set => Plate = Plate with { Face = value };
    }

    /// <summary>The whole plate -- face, light frame and dark foot -- picked as one thing.</summary>
    public Style.ButtonPlate Plate { get; set; } = Style.PlateSteel;

    /// <summary>
    /// Whether the button is refusing presses.
    /// </summary>
    /// <remarks>
    /// Dimmed rather than hidden. The vendor's Buy button is the one that uses this, and a button
    /// that disappears when you cannot afford something reads as a broken panel rather than as a
    /// price you have not met.
    /// </remarks>
    public bool Disabled
    {
        get => _disabled;
        set
        {
            if (_disabled == value)
                return;

            _disabled = value;
            QueueRedraw();
        }
    }

    /// <summary>Whether to show the unread badge in the corner.</summary>
    public bool Badged
    {
        get => _badged;
        set
        {
            if (_badged == value)
                return;

            _badged = value;
            QueueRedraw();
        }
    }

    public override void _Ready()
    {
        MouseEntered += () => { _hovered = true; QueueRedraw(); };
        MouseExited += () => { _hovered = false; _held = false; QueueRedraw(); };
    }

    public override void _GuiInput(InputEvent @event)
    {
        if (@event is not InputEventMouseButton { ButtonIndex: MouseButton.Left } button)
            return;

        // A disabled button still swallows the click. Letting it through would walk the character
        // to wherever the button is, which is worse than nothing happening.
        AcceptEvent();
        if (_disabled)
            return;

        _held = button.Pressed;
        QueueRedraw();

        if (!button.Pressed)
            Pressed?.Invoke();
    }

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);

        var face = _disabled ? Face.Darkened(0.45f)
            : _held ? Face.Darkened(0.25f)
            : _hovered ? Face.Lightened(0.18f)
            : Face;

        // The plate sits on whatever is behind it rather than floating over it, so a hard shadow
        // goes down first, inset to the same width the frame's bands run at.
        DrawRect(
            new Rect2(full.Position.X + Style.ButtonFrameSide, full.End.Y,
                full.Size.X - Style.ButtonFrameSide * 2f, Style.ButtonShadowHeight),
            Style.ButtonShadow);

        DrawRect(
            new Rect2(full.Position.X + Style.ButtonFrameSide, full.Position.Y + Style.ButtonFrameTop,
                full.Size.X - Style.ButtonFrameSide * 2f, full.Size.Y - Style.ButtonFrameTop * 2f),
            face);

        DrawBevel(full, inverted: _held);

        // The label shifts with the plate, so a held button reads as pressed rather than as
        // repainted.
        var shift = _held ? Vector2.One : Vector2.Zero;
        float baseline = Style.BaselineIn(Size.Y, Style.FontBody);

        var at = new Vector2(Mathf.Round((Size.X - Style.Measure(_label, Style.FontBody)) / 2f), baseline) + shift;
        this.DrawText(at, _label, Style.FontBody, _disabled ? Style.TextDim : Style.Text);

        if (!_badged)
            return;

        // The unread mark, at the top right where a notification is looked for. Square, like
        // everything else here.
        DrawRect(new Rect2(Size.X - 9f, 3f, 6f, 6f), Style.PanelEdge);
        DrawRect(new Rect2(Size.X - 8f, 4f, 4f, 4f), Style.HpFill);
    }

    /// <summary>
    /// The frame that gives the plate its thickness: light along the top and both sides, dark
    /// along the bottom, with all four corners notched out.
    /// </summary>
    /// <remarks>
    /// The same shape <see cref="GameButton"/> draws, off the same measured figures, so the escape
    /// menu's plates and the ones over the world are one button rather than two that resemble each
    /// other. Holding swaps light for dark, which is what makes the plate read as pushed in.
    /// </remarks>
    private void DrawBevel(in Rect2 full, bool inverted)
    {
        var high = inverted ? Plate.Low : Plate.High;
        var low = inverted ? Plate.High : Plate.Low;

        float side = Style.ButtonFrameSide;
        float cap = Style.ButtonFrameTop;
        float inner = full.Size.X - side * 2f;
        float tall = full.Size.Y - cap * 2f;

        if (inner <= 0f || tall <= 0f)
            return;

        DrawRect(new Rect2(full.Position.X + side, full.Position.Y, inner, cap), high);
        DrawRect(new Rect2(full.Position.X, full.Position.Y + cap, side, tall), high);
        DrawRect(new Rect2(full.End.X - side, full.Position.Y + cap, side, tall), high);
        DrawRect(new Rect2(full.Position.X + side, full.End.Y - cap, inner, cap), low);
    }
}

/// <summary>
/// A glyph with nothing behind it: the heart and the flask beside the vital bars.
/// </summary>
/// <remarks>
/// Ignores the pointer. These sit over the world at the bottom of the screen and there is nothing
/// to click on them, so swallowing a click there would stop the character moving for no reason.
/// </remarks>
public partial class HudGlyph : Control
{
    private Action<CanvasItem, Rect2, Color> _icon;

    public HudGlyph(Action<CanvasItem, Rect2, Color> icon, Color colour)
    {
        _icon = icon;
        _tint = colour;
        MouseFilter = MouseFilterEnum.Ignore;
    }

    /// <summary>The glyph itself, which the merchant panel swaps between gold and fame.</summary>
    public Action<CanvasItem, Rect2, Color> Icon
    {
        get => _icon;
        set
        {
            if (_icon == value)
                return;

            _icon = value;
            QueueRedraw();
        }
    }

    private Color _tint;

    /// <summary>The glyph's colour. Repaints on assignment, since nothing else here would.</summary>
    public Color Tint
    {
        get => _tint;
        set
        {
            if (_tint == value)
                return;

            _tint = value;
            QueueRedraw();
        }
    }

    private float _pulse = 1f;

    /// <summary>Beats between one and a little over one. Used to pulse the heart at low health.</summary>
    public float Pulse
    {
        get => _pulse;
        set
        {
            if (Mathf.IsEqualApprox(_pulse, value))
                return;

            _pulse = value;
            QueueRedraw();
        }
    }

    public override void _Draw()
    {
        var box = new Rect2(Vector2.Zero, Size);
        if (!Mathf.IsEqualApprox(Pulse, 1f))
            box = box.GrowIndividual(
                Size.X * (Pulse - 1f) / 2f, Size.Y * (Pulse - 1f) / 2f,
                Size.X * (Pulse - 1f) / 2f, Size.Y * (Pulse - 1f) / 2f);

        _icon?.Invoke(this, box, _tint);
    }
}
