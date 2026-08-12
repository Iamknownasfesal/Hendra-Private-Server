using Godot;

namespace Hendra.UI;

/// <summary>
/// A button in the game's own shape.
/// </summary>
/// <remarks>
/// <para>
/// The original's buttons are cut-corner plates drawn by <c>GraphicsUtil.drawCutEdgeRect</c>, the
/// same routine behind every panel in the interface — square corners read as a different game. A
/// Godot StyleBox cannot describe a forty-five degree cut, so the plate is drawn here, and the
/// label with it: the built-in text is drawn before a script's own drawing and would be painted
/// over.
/// </para>
/// <para>
/// Three states, and the difference between them is deliberately larger than the original's. It
/// lifts and takes a gold edge under the pointer, and presses down into the plate when held —
/// motion is what tells you a button is a button, and a menu whose buttons only change tint feels
/// dead.
/// </para>
/// </remarks>
public partial class GameButton : Button
{
    /// <summary>How far the corners are cut, matching the game's panels.</summary>
    private const int Cut = 6;

    private static readonly bool[] AllCorners = { true, true, true, true };

    private readonly string _label;
    private readonly bool _primary;
    private readonly bool _compact;

    /// <summary>Eased towards one while hovered, so the lift is a movement rather than a jump.</summary>
    private float _glow;

    /// <param name="primary">
    /// Whether this is the action the screen exists for. There should be one per screen, and it is
    /// the one that carries the accent when idle rather than only when pointed at.
    /// </param>
    /// <param name="compact">
    /// For a button inside a panel rather than on a menu: no width of its own, and short enough to
    /// sit in a row of them.
    /// </param>
    public GameButton(string text, bool primary = false, bool compact = false)
    {
        _label = text;
        _primary = primary;
        _compact = compact;

        // The plate and the text are drawn here, so the built-in ones are cleared rather than
        // fought with.
        Text = string.Empty;
        FocusMode = FocusModeEnum.None;
        CustomMinimumSize = compact
            ? new Vector2(0, 28)
            : new Vector2(primary ? 190 : 150, primary ? 46 : 40);

        AddThemeStyleboxOverride("normal", new StyleBoxEmpty());
        AddThemeStyleboxOverride("hover", new StyleBoxEmpty());
        AddThemeStyleboxOverride("pressed", new StyleBoxEmpty());
        AddThemeStyleboxOverride("disabled", new StyleBoxEmpty());
        AddThemeStyleboxOverride("focus", new StyleBoxEmpty());
    }

    public override void _Process(double delta)
    {
        float target = IsHovered() && !Disabled ? 1f : 0f;
        float eased = Mathf.MoveToward(_glow, target, (float)delta * 6f);

        if (Mathf.IsEqualApprox(eased, _glow))
            return;

        _glow = eased;
        QueueRedraw();
    }

    public override void _Draw()
    {
        bool held = ButtonPressed || IsPressed();

        // Held buttons sink into the plate; hovered ones rise off it.
        float lift = held ? 1f : -_glow * 2f;
        var size = new Vector2(Size.X, Size.Y);
        var offset = new Vector2(0f, lift);

        var outline = CutEdgePanel.Outline(size, _compact ? 4 : Cut, AllCorners);
        for (int i = 0; i < outline.Length; i++)
            outline[i] += offset;

        if (!held)
            DrawShadow(size, offset);

        DrawPlate(outline, held);
        DrawEdge(outline);
        DrawLabel(offset, held);
    }

    private void DrawShadow(Vector2 size, Vector2 offset)
    {
        var shadow = CutEdgePanel.Outline(size, _compact ? 4 : Cut, AllCorners);
        for (int i = 0; i < shadow.Length; i++)
            shadow[i] += offset + new Vector2(0f, 3f + _glow * 2f);

        DrawColoredPolygon(shadow, new Color(0f, 0f, 0f, 0.45f));
    }

    /// <summary>The face, graded down the plate and warmed towards the accent while hovered.</summary>
    private void DrawPlate(Vector2[] outline, bool held)
    {
        var top = Style.ControlTop.Lerp(Style.ControlHoverTop, _glow);
        var bottom = Style.ControlBottom.Lerp(Style.ControlHoverBottom, _glow);

        if (_primary)
        {
            top = top.Lerp(Style.GoldDim, 0.35f + _glow * 0.2f);
            bottom = bottom.Lerp(Style.GoldDim, 0.15f + _glow * 0.2f);
        }

        if (held)
        {
            top = top.Darkened(0.25f);
            bottom = bottom.Darkened(0.25f);
        }

        if (Disabled)
        {
            top = top.Darkened(0.4f);
            bottom = bottom.Darkened(0.4f);
        }

        var shades = new Color[outline.Length];
        float height = Mathf.Max(Size.Y, 1f);
        for (int i = 0; i < outline.Length; i++)
            shades[i] = top.Lerp(bottom, Mathf.Clamp(outline[i].Y / height, 0f, 1f));

        DrawPolygon(outline, shades);
    }

    private void DrawEdge(Vector2[] outline)
    {
        var closed = new Vector2[outline.Length + 1];
        outline.CopyTo(closed, 0);
        closed[^1] = outline[0];

        var edge = Style.Edge.Lerp(Style.Gold, _primary ? 0.5f + _glow * 0.5f : _glow);
        DrawPolyline(closed, edge with { A = Disabled ? 0.3f : 0.9f }, 1.5f, antialiased: true);
    }

    private void DrawLabel(Vector2 offset, bool held)
    {
        var font = GetThemeDefaultFont();
        int size = GetThemeDefaultFontSize() + (_compact ? -1 : _primary ? 5 : 2);

        var colour = Disabled
            ? Style.Faint
            : Style.Muted.Lerp(Colors.White, _primary ? 0.7f + _glow * 0.3f : _glow);

        var measured = font.GetStringSize(_label, HorizontalAlignment.Left, -1, size);
        var at = new Vector2(
            (Size.X - measured.X) / 2f,
            (Size.Y + measured.Y) / 2f - font.GetDescent(size)) + offset;

        // A shadow under the text, so it holds up over the brighter primary plate too.
        if (!held)
            DrawString(font, at + new Vector2(0f, 1f), _label, HorizontalAlignment.Left, -1, size,
                new Color(0f, 0f, 0f, 0.55f));

        DrawString(font, at, _label, HorizontalAlignment.Left, -1, size, colour);
    }
}
