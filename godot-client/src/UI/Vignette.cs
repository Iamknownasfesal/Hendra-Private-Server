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
    /// <summary>
    /// How dark the middle of each edge goes.
    /// </summary>
    /// <remarks>
    /// The edges, not the corners: the four bands overlap where they meet, so a corner ends up
    /// close to twice this. That is the shape a vignette should have anyway.
    /// </remarks>
    private readonly float _strength;

    public Vignette(float strength = 0.20f) => _strength = strength;

    /// <remarks>Offsets as well as anchors. See the note in <see cref="Starfield"/>.</remarks>
    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;
        SetAnchorsAndOffsetsPreset(LayoutPreset.FullRect);
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

        // Four gradient bands rather than a stack of flat rings.
        //
        // Rings were the first attempt and they cannot be made to work here: each one is a single
        // flat alpha, and at the strengths a vignette wants the difference between neighbours
        // rounds to one or two colour levels. Over artwork that is itself flat -- which the title
        // graphic is, a single grey -- that lands as a set of visible contour lines, and slicing
        // more finely only moves them closer together.
        //
        // Interpolating the colour between a polygon's vertices hands the blend to the GPU, which
        // dithers it. Four quads, drawn once per resize.
        float reach = Mathf.Min(size.X, size.Y) * 0.42f;

        var dark = new Color(0f, 0f, 0f, _strength);
        var clear = new Color(0f, 0f, 0f, 0f);

        Band(new Vector2(0f, 0f), new Vector2(size.X, 0f),
            new Vector2(size.X, reach), new Vector2(0f, reach), dark, clear);

        Band(new Vector2(0f, size.Y), new Vector2(size.X, size.Y),
            new Vector2(size.X, size.Y - reach), new Vector2(0f, size.Y - reach), dark, clear);

        Band(new Vector2(0f, 0f), new Vector2(0f, size.Y),
            new Vector2(reach, size.Y), new Vector2(reach, 0f), dark, clear);

        Band(new Vector2(size.X, 0f), new Vector2(size.X, size.Y),
            new Vector2(size.X - reach, size.Y), new Vector2(size.X - reach, 0f), dark, clear);
    }

    /// <summary>A quad carrying <paramref name="outer"/> on its first edge and fading to nothing.</summary>
    private void Band(Vector2 a, Vector2 b, Vector2 c, Vector2 d, Color outer, Color inner) =>
        DrawPolygon(new[] { a, b, c, d }, new[] { outer, outer, inner, inner });
}
