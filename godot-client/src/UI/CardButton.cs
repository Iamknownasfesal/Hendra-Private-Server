using Godot;

namespace Hendra.UI;

/// <summary>
/// A large clickable plate that holds its own contents.
/// </summary>
/// <remarks>
/// <para>
/// What a character box and a class tile are: a picture and some words you click, rather than a
/// word in a box. <see cref="GameButton"/> draws its own label because it has only a label; this
/// draws the plate and lets whatever is parented to it sit on top, because children are drawn after
/// their parent and the built-in stylebox is cleared out of the way.
/// </para>
/// <para>
/// It lifts, brightens and takes an edge in its accent under the pointer. On a screen whose whole
/// purpose is choosing between several of these, the one you are pointing at has to be obvious
/// without being read.
/// </para>
/// </remarks>
public partial class CardButton : Button
{
    private const int Cut = 8;

    private static readonly bool[] AllCorners = { true, true, true, true };

    private readonly Color _accent;
    private float _glow;

    public CardButton(Color accent)
    {
        _accent = accent;

        Text = string.Empty;
        FocusMode = FocusModeEnum.None;

        foreach (string state in new[] { "normal", "hover", "pressed", "disabled", "focus" })
            AddThemeStyleboxOverride(state, new StyleBoxEmpty());
    }

    public override void _Process(double delta)
    {
        float target = IsHovered() && !Disabled ? 1f : 0f;
        float eased = Mathf.MoveToward(_glow, target, (float)delta * 7f);

        if (Mathf.IsEqualApprox(eased, _glow))
            return;

        _glow = eased;
        QueueRedraw();
    }

    public override void _Draw()
    {
        var outline = CutEdgePanel.Outline(Size, Cut, AllCorners);

        // A shadow that grows with the lift, which is what sells the plate coming off the page.
        var shadow = new Vector2[outline.Length];
        for (int i = 0; i < outline.Length; i++)
            shadow[i] = outline[i] + new Vector2(0f, 3f + _glow * 3f);

        DrawColoredPolygon(shadow, new Color(0f, 0f, 0f, 0.4f));

        var top = Style.PanelTop.Lerp(Style.ControlHoverTop, _glow * 0.8f);
        var bottom = Style.PanelBottom.Lerp(Style.ControlBottom, _glow * 0.8f);

        var shades = new Color[outline.Length];
        float height = Mathf.Max(Size.Y, 1f);
        for (int i = 0; i < outline.Length; i++)
            shades[i] = top.Lerp(bottom, Mathf.Clamp(outline[i].Y / height, 0f, 1f));

        DrawPolygon(outline, shades);

        // A stripe of the accent down the left edge, at full strength once pointed at. It reads as
        // a marker on a row rather than as another border around a box.
        DrawRect(new Rect2(0f, Cut, 3f, Size.Y - Cut * 2f), _accent with { A = 0.35f + _glow * 0.65f });

        var closed = new Vector2[outline.Length + 1];
        outline.CopyTo(closed, 0);
        closed[^1] = outline[0];

        DrawPolyline(closed, Style.Edge.Lerp(_accent, _glow) with { A = 0.5f + _glow * 0.5f },
            1.5f, antialiased: true);
    }
}
