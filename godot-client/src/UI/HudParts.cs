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
/// A labelled bar. Fame, experience, health and magic are all this control.
/// </summary>
/// <remarks>
/// <para>
/// Every bar in the interface reads the same way, which is the point: a one-pixel dark edge, a
/// track, a flat fill with a one-pixel highlight along the top of the <i>fill</i> -- it moves with
/// the fill rather than spanning the track -- the stat's name inset on the left and its value inset
/// on the right. Revision one centred a single label; that made four bars that each had to be read
/// before it could be told apart.
/// </para>
/// <para>
/// The fill eases towards its value over about 150 milliseconds rather than snapping, and a drop
/// leaves a lighter chip behind that catches up over 400. A bar that jumps tells you the number
/// changed; a bar that slides tells you which way and by how much, which is the thing you actually
/// need while something is hitting you.
/// </para>
/// <para>
/// Both texts are drawn rather than made into labels, because they have to sit over the fill and
/// keep their outline at every percentage -- and because two labels per bar across four bars is
/// eight nodes to keep in step for text that never moves.
/// </para>
/// </remarks>
public partial class HudBar : Control
{
    /// <summary>How far in from each end the two texts sit.</summary>
    private const float Inset = 6f;

    /// <summary>How fast the fill converges. Higher is quicker; twenty settles in about 150ms.</summary>
    private const float FillRate = 20f;

    /// <summary>The chip's rate, slow enough that a hit leaves a mark you can see after it lands.</summary>
    private const float ChipRate = 6f;

    private readonly int _fontSize;

    private string _name = string.Empty;
    private string _value = string.Empty;
    private string _bonus = string.Empty;

    private float _target;
    private float _shown;
    private float _chip;

    public HudBar(Color fill, int fontSize = Style.FontBody)
    {
        _fill = fill;
        _fontSize = fontSize;
        MouseFilter = MouseFilterEnum.Ignore;
    }

    private Color _fill;

    /// <summary>
    /// The bar's colour, which the top row changes when it stops being the level bar.
    /// </summary>
    /// <remarks>
    /// Repaints on assignment. It used to be a plain property, and a bar that was full when its
    /// colour changed kept the old one until something else happened to it.
    /// </remarks>
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

    /// <summary>Sets the fill, the name on the left and the value on the right.</summary>
    public void Set(int current, int maximum, string name, string value, string bonus = "")
    {
        _target = maximum > 0 ? Mathf.Clamp(current / (float)maximum, 0f, 1f) : 0f;

        if (_name == name && _value == value && _bonus == bonus)
            return;

        _name = name;
        _value = value;
        _bonus = bonus;
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
            DrawRect(new Rect2(1f, 1f, (Size.X - 2f) * _chip, Size.Y - 2f),
                Fill.Lightened(0.4f) with { A = 0.55f });

        float width = (Size.X - 2f) * _shown;
        if (width >= 1f)
        {
            DrawRect(new Rect2(1f, 1f, width, Size.Y - 2f), Fill);

            // The fill's own highlight, one pixel along its top edge only.
            DrawRect(new Rect2(1f, 1f, width, 1f), Style.BarHighlight);
        }

        DrawRect(full, Style.BarEdge, filled: false, width: 1f);

        // Both texts on the same baseline, which is the vertical middle of the bar. Kept whole
        // white at every fill: a value that dims as the bar empties is unreadable exactly when it
        // matters.
        float baseline = Style.BaselineIn(Size.Y, _fontSize);

        if (_name.Length > 0)
            this.DrawOverWorld(new Vector2(Inset, baseline), _name, _fontSize, Style.Text);

        if (_value.Length == 0)
            return;

        // What equipment is adding, in green immediately after the value. Omitted at zero: six
        // bars all saying "(+0)" is noise, and the point of the suffix is that it stands out.
        float bonusWidth = _bonus.Length == 0 ? 0f : Style.Measure(_bonus, _fontSize) + 4f;
        float right = Size.X - Inset - bonusWidth;

        this.DrawOverWorld(
            new Vector2(right - Style.Measure(_value, _fontSize), baseline), _value, _fontSize, Style.Text);

        if (_bonus.Length > 0)
            this.DrawOverWorld(new Vector2(right + 4f, baseline), _bonus, _fontSize, Style.StatBonus);
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
        if (@event is InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
        {
            Pressed?.Invoke();
            AcceptEvent();
        }
    }

    public override void _Draw()
    {
        // A lit plate under the glyph rather than a white wash over it: the chrome is opaque grey
        // now and a translucent overlay on it just looks like a smudge.
        if (_hovered)
            DrawRect(new Rect2(Vector2.Zero, Size), Style.ButtonHover);

        _icon?.Invoke(this, new Rect2(Vector2.Zero, Size).Grow(-_inset), Tint);

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
    private readonly string _label;

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

    /// <summary>
    /// The face colour, for the one button that carries the accent.
    /// </summary>
    /// <remarks>
    /// Special Offer, and nothing else. One saturated thing in the corner is a thing being pointed
    /// at; three are a decorated corner.
    /// </remarks>
    public Color Face { get; set; } = Style.ButtonFace;

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
            : _hovered && !_held ? Face.Lerp(Style.ButtonHover, 0.6f)
            : Face;

        DrawRect(full, face);
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

    /// <summary>The one-pixel two-tone edge that gives the plate its thickness.</summary>
    private void DrawBevel(in Rect2 full, bool inverted)
    {
        var high = inverted ? Style.ButtonBevelLow : Style.ButtonBevelHigh;
        var low = inverted ? Style.ButtonBevelHigh : Style.ButtonBevelLow;

        DrawRect(new Rect2(full.Position, new Vector2(full.Size.X, 1f)), high);
        DrawRect(new Rect2(full.Position, new Vector2(1f, full.Size.Y)), high);
        DrawRect(new Rect2(full.Position.X, full.End.Y - 1f, full.Size.X, 1f), low);
        DrawRect(new Rect2(full.End.X - 1f, full.Position.Y, 1f, full.Size.Y), low);
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
