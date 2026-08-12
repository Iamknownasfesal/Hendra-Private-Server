using Godot;

namespace Hendra.UI;

/// <summary>
/// A darkening towards the edges of the screen.
/// </summary>
/// <remarks>
/// Menus sit in the middle and the artwork behind them runs to the corners, so the corners are
/// exactly where the eye should not be. A vignette is the cheapest way to say so — and it lets the
/// backdrop be brighter than it otherwise could be without the buttons losing their footing.
/// </remarks>
public partial class Vignette : Control
{
    /// <summary>How dark the corners go.</summary>
    private readonly float _strength;

    public Vignette(float strength = 0.55f) => _strength = strength;

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;
        SetAnchorsPreset(LayoutPreset.FullRect);
    }

    public override void _Notification(int what)
    {
        if (what == NotificationResized)
            QueueRedraw();
    }

    public override void _Draw()
    {
        var size = Size;
        if (size.X <= 0f || size.Y <= 0f)
            return;

        // Drawn as a stack of rings rather than with a shader: it is a menu backdrop, it is drawn
        // once per resize, and a shader for it would be a file and a material to keep in step.
        const int Rings = 26;
        float step = Mathf.Max(size.X, size.Y) / (Rings * 2f);

        for (int i = 0; i < Rings; i++)
        {
            float inset = i * step;
            float strength = _strength * Mathf.Pow(i / (float)Rings, 2.5f) / Rings * 6f;

            DrawRect(new Rect2(inset, inset, size.X - inset * 2f, size.Y - inset * 2f),
                new Color(0f, 0f, 0f, strength), filled: false, width: step + 1f);
        }
    }
}
