using System.Collections.Generic;
using Godot;

namespace Hendra.UI;

/// <summary>
/// The game's panel shape: a rectangle with its corners cut off at forty-five degrees.
/// </summary>
/// <remarks>
/// <para>
/// Every panel in the original is drawn by one routine, <c>GraphicsUtil.drawCutEdgeRect</c>, which
/// takes a corner size and four flags saying which corners to cut. It is a small thing that carries
/// most of the interface's character — square corners read as a different game entirely.
/// </para>
/// <para>
/// A container rather than a style box, because Godot's style boxes cannot describe an arbitrary
/// polygon. A Control paints itself before its children, so drawing here puts the shape behind
/// whatever the panel holds without a separate background node.
/// </para>
/// </remarks>
public partial class CutEdgePanel : MarginContainer
{
    /// <summary>How far each corner is cut, in pixels. The original's figure for its panels.</summary>
    public const int CutSize = 6;

    /// <summary>The original's TabConstants.BACKGROUND_COLOR.</summary>
    public static readonly Color PanelBackground = new("242222");

    /// <summary>The original's TabConstants.TAB_COLOR.</summary>
    public static readonly Color TabColour = new("6b6a6a");

    private Color _background = PanelBackground;
    private Color _border = new(0f, 0f, 0f, 0f);

    /// <summary>Which corners are cut, clockwise from the top left.</summary>
    private bool[] _cuts = { true, true, true, true };

    public Color Background
    {
        get => _background;
        set { _background = value; QueueRedraw(); }
    }

    /// <summary>Outline colour. Fully transparent by default, as most of the original's panels are.</summary>
    public Color Border
    {
        get => _border;
        set { _border = value; QueueRedraw(); }
    }

    public float Opacity
    {
        get => _background.A;
        set { _background.A = value; QueueRedraw(); }
    }

    /// <summary>Sets which corners are cut, clockwise from the top left.</summary>
    public CutEdgePanel Cuts(bool topLeft, bool topRight, bool bottomRight, bool bottomLeft)
    {
        _cuts = new[] { topLeft, topRight, bottomRight, bottomLeft };
        QueueRedraw();
        return this;
    }

    /// <summary>Sets the same padding on all four sides.</summary>
    public CutEdgePanel Padded(int pixels)
    {
        foreach (string side in new[] { "margin_left", "margin_top", "margin_right", "margin_bottom" })
            AddThemeConstantOverride(side, pixels);
        return this;
    }

    public override void _Notification(int what)
    {
        // The shape is built from the current size, so it has to be rebuilt when that changes.
        if (what == NotificationResized)
            QueueRedraw();
    }

    public override void _Draw()
    {
        var points = Outline(Size, CutSize, _cuts);

        if (_background.A > 0f)
            DrawColoredPolygon(points, _background);

        if (_border.A > 0f)
        {
            // Closed by repeating the first point: DrawPolyline draws an open path.
            var closed = new Vector2[points.Length + 1];
            points.CopyTo(closed, 0);
            closed[^1] = points[0];
            DrawPolyline(closed, _border, 1f, antialiased: false);
        }
    }

    /// <summary>
    /// The outline of a cut-corner rectangle, clockwise from the top-left corner.
    /// </summary>
    /// <param name="cuts">Which corners are cut, clockwise from the top left.</param>
    public static Vector2[] Outline(Vector2 size, float cut, IReadOnlyList<bool> cuts)
    {
        // A cut larger than half the box would fold the shape inside out.
        cut = Mathf.Min(cut, Mathf.Min(size.X, size.Y) / 2f);

        var points = new List<Vector2>(8);

        void Corner(bool isCut, Vector2 corner, Vector2 before, Vector2 after)
        {
            if (isCut)
            {
                points.Add(before);
                points.Add(after);
            }
            else
            {
                points.Add(corner);
            }
        }

        Corner(cuts[0], Vector2.Zero, new Vector2(0f, cut), new Vector2(cut, 0f));
        Corner(cuts[1], new Vector2(size.X, 0f), new Vector2(size.X - cut, 0f), new Vector2(size.X, cut));
        Corner(cuts[2], size, new Vector2(size.X, size.Y - cut), new Vector2(size.X - cut, size.Y));
        Corner(cuts[3], new Vector2(0f, size.Y), new Vector2(cut, size.Y), new Vector2(0f, size.Y - cut));

        return points.ToArray();
    }
}
