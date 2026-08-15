using Godot;

namespace Hendra.UI;

/// <summary>
/// A button on one of the screens either side of the game: the title, the login page, the death
/// summary.
/// </summary>
/// <remarks>
/// <para>
/// The same plate the interface uses everywhere else — a flat face capped by a lighter band along
/// the top and a darker one along the bottom, inverted while held, with the label shifting a pixel
/// down and right so the plate visibly goes in.
/// </para>
/// <para>
/// It used to be a cut-cornered, gradient-filled, drop-shadowed plate that rose under the pointer.
/// That was the older of the two design languages this client carried, and the menus were the last
/// place it survived: the same press in the same session produced a soft rounded button on the
/// login page and a hard square one in the HUD ten seconds later. This is not a downgrade of the
/// menus so much as the end of the split — and a Button subclass rather than a Control, so every
/// call site keeps its <c>Pressed</c> and its <c>Disabled</c> and nothing else had to change.
/// </para>
/// <para>
/// The plate is the label. One button per screen may be <paramref name="primary"/> and takes the
/// olive commit plate — the action the screen exists for; the rest take steel, and the one that
/// quits or closes is given the red plate explicitly.
/// </para>
/// </remarks>
public partial class GameButton : Button
{
    private readonly string _label;
    private readonly bool _primary;
    private readonly bool _compact;
    private readonly Style.ButtonPlate _plate;

    /// <summary>Eased towards one while hovered, so the lift is a movement rather than a jump.</summary>
    private float _glow;

    /// <param name="primary">
    /// Whether this is the action the screen exists for. There should be one per screen, and it is
    /// the one that carries the accent.
    /// </param>
    /// <param name="compact">
    /// For a button inside a panel rather than on a menu: no width of its own, and short enough to
    /// sit in a row of them.
    /// </param>
    /// <param name="plate">
    /// Which of the interface's plates to wear. Left unset, a primary button takes the commit
    /// plate and everything else the steel one, which is what the reference does; pass
    /// <see cref="Style.PlateDanger"/> for the button that quits or closes.
    /// </param>
    public GameButton(string text, bool primary = false, bool compact = false,
        Style.ButtonPlate? plate = null)
    {
        _label = text;
        _primary = primary;
        _compact = compact;
        _plate = plate ?? (primary ? Style.PlateCommit : Style.PlateSteel);

        // The plate and the text are drawn here, so the built-in ones are cleared rather than
        // fought with.
        Text = string.Empty;
        FocusMode = FocusModeEnum.None;
        CustomMinimumSize = compact
            ? new Vector2(0, 30)
            : new Vector2(primary ? 190 : 150, primary ? 44 : 38);

        foreach (string state in new[] { "normal", "hover", "pressed", "disabled", "focus" })
            AddThemeStyleboxOverride(state, new StyleBoxEmpty());
    }

    /// <summary>The label's size, from the shared type scale rather than a per-button number.</summary>
    private int FontSize => _compact ? Style.FontSmall : _primary ? Style.FontName : Style.FontBody;

    /// <summary>
    /// Re-measures once the theme is available.
    /// </summary>
    /// <remarks>
    /// The first minimum-size query happens before the node is in the tree, when there is no font
    /// to measure with, and Godot caches what it is told. Without this every button keeps the
    /// no-font answer and a row of them collapses onto one spot.
    /// </remarks>
    public override void _Ready() => UpdateMinimumSize();

    public override Vector2 _GetMinimumSize()
    {
        float padding = _compact ? 20f : 36f;

        return new Vector2(
            Mathf.Max(Style.Measure(_label, FontSize) + padding, CustomMinimumSize.X),
            Mathf.Max(_compact ? 30f : _primary ? 44f : 38f, CustomMinimumSize.Y));
    }

    public override void _Process(double delta)
    {
        float target = IsHovered() && !Disabled ? 1f : 0f;
        float eased = Mathf.MoveToward(_glow, target, (float)delta * 8f);

        if (Mathf.IsEqualApprox(eased, _glow))
            return;

        _glow = eased;
        QueueRedraw();
    }

    public override void _Draw()
    {
        bool held = ButtonPressed || IsPressed();
        var full = new Rect2(Vector2.Zero, Size);

        var face = _plate.Face;

        if (Disabled)
            face = face.Darkened(0.45f);
        else if (held)
            face = face.Darkened(0.25f);
        else if (_glow > 0f)
            face = face.Lerp(face.Lightened(0.18f), _glow);

        DrawRect(full, face);
        DrawBevel(full, inverted: held);

        // The label moves with the plate, so a held button reads as pressed rather than repainted.
        var shift = held ? Vector2.One : Vector2.Zero;

        // White on every plate. The reference sets Play and Continue in white on the olive just as
        // it sets Options in white on the steel; only a disabled plate drops to the muted grey.
        var colour = Disabled ? Style.TextDim : Style.Text;

        var at = new Vector2(
            Mathf.Round((Size.X - Style.Measure(_label, FontSize)) / 2f),
            Style.BaselineIn(Size.Y, FontSize)) + shift;

        this.DrawText(at, _label, FontSize, colour);
    }

    /// <summary>
    /// The two bands that give the plate its thickness.
    /// </summary>
    /// <remarks>
    /// Not an edge around all four sides. In the reference the light band sits along the top and
    /// the dark one along the bottom, both four pixels tall and inset five pixels from each end,
    /// leaving the corners as bare face; the sides carry nothing at all. Holding the button
    /// swaps the two, so the plate reads as pushed in rather than repainted.
    /// </remarks>
    private void DrawBevel(in Rect2 full, bool inverted)
    {
        var high = inverted ? _plate.Low : _plate.High;
        var low = inverted ? _plate.High : _plate.Low;

        float inset = Style.ButtonCapInset;
        float cap = Style.ButtonCapHeight;
        float width = full.Size.X - inset * 2f;

        if (width <= 0f)
            return;

        DrawRect(new Rect2(full.Position.X + inset, full.Position.Y, width, cap), high);
        DrawRect(new Rect2(full.Position.X + inset, full.End.Y - cap, width, cap), low);
    }
}
