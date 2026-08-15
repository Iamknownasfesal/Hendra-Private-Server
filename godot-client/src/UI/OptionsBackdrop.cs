using Godot;

namespace Hendra.UI;

/// <summary>
/// The field a full-screen page is laid on: a vertical gradient with a slow drift of large
/// diamonds and small squares across it, and a thin rule down each side.
/// </summary>
/// <remarks>
/// <para>
/// The original's menu screens are opaque -- the world behind them is covered rather than dimmed --
/// and what fills the space is this: near-black at the top, a shade under charcoal at the bottom,
/// and a scatter of outlined shapes moving slowly up and to the left. It is dark enough that the
/// page's own plates read as the only structure on screen, and moving enough that the screen is
/// not a flat rectangle.
/// </para>
/// <para>
/// The scatter is generated rather than stored: a lattice of cells, each one hashed to a shape, a
/// size and an offset within its cell. That makes it endless in every direction for the cost of the
/// cells actually on screen, and identical from one run to the next, which matters because a
/// screenshot has to be comparable with the last one.
/// </para>
/// </remarks>
public sealed partial class OptionsBackdrop : Control
{
    /// <summary>The gradient's ends, sampled from the reference at the top and bottom of frame.</summary>
    private static readonly Color Top = new("0f0f0f");

    private static readonly Color Bottom = new("2d2d2d");

    /// <summary>The shapes are white at a few per cent, so they ride the gradient rather than sit on it.</summary>
    private static readonly Color Ink = new(1f, 1f, 1f, 0.055f);

    private static readonly Color InkBright = new(1f, 1f, 1f, 0.105f);
    private static readonly Color Rule = new(1f, 1f, 1f, 0.19f);

    /// <summary>How far apart the lattice's cells are, and so how sparse the scatter is.</summary>
    private const float Cell = 380f;

    /// <summary>Pixels a second, up and to the left.</summary>
    private static readonly Vector2 Drift = new(-7f, -11f);

    /// <summary>How many bands the gradient is drawn in. Enough that no step is visible.</summary>
    private const int Bands = 48;

    private float _time;
    private float _drawnAt = float.NegativeInfinity;

    public override void _Ready() => MouseFilter = MouseFilterEnum.Ignore;

    public override void _Process(double delta)
    {
        if (!IsVisibleInTree())
            return;

        _time += (float)delta;

        // Redrawn on whole pixels of travel rather than every frame: the drift is slow enough that
        // the difference is invisible and the whole screen is repainted each time.
        if (Mathf.Abs(_time - _drawnAt) * Drift.Length() >= 1f)
            QueueRedraw();
    }

    public override void _Draw()
    {
        _drawnAt = _time;

        float height = Mathf.Max(Size.Y, 1f);
        float step = height / Bands;

        for (int i = 0; i < Bands; i++)
            DrawRect(
                new Rect2(0f, Mathf.Floor(i * step), Size.X, Mathf.Ceil(step) + 1f),
                Top.Lerp(Bottom, (i + 0.5f) / Bands));

        DrawScatter();
        DrawRules();
    }

    /// <summary>The drifting shapes, one per lattice cell that touches the screen.</summary>
    private void DrawScatter()
    {
        var shift = Drift * _time;

        int firstX = Mathf.FloorToInt((-shift.X - Cell) / Cell);
        int firstY = Mathf.FloorToInt((-shift.Y - Cell) / Cell);
        int lastX = Mathf.CeilToInt((Size.X - shift.X + Cell) / Cell);
        int lastY = Mathf.CeilToInt((Size.Y - shift.Y + Cell) / Cell);

        for (int cy = firstY; cy <= lastY; cy++)
        {
            for (int cx = firstX; cx <= lastX; cx++)
            {
                uint hash = Hash(cx, cy);

                var at = new Vector2(cx * Cell, cy * Cell) + shift
                    + new Vector2(Fraction(hash, 0) * Cell, Fraction(hash, 8) * Cell);

                at = at.Round();

                switch (hash % 6)
                {
                    case 0:
                        Diamond(at, 60f + Fraction(hash, 16) * 30f, filled: true);
                        break;
                    case 1:
                        Diamond(at, 55f + Fraction(hash, 16) * 35f, filled: false);
                        break;
                    case 2:
                    case 3:
                    case 4:
                        Square(at, Mathf.Round(14f + Fraction(hash, 16) * 16f), filled: true);
                        break;
                    default:
                        Square(at, Mathf.Round(24f + Fraction(hash, 16) * 26f), filled: false);
                        break;
                }
            }
        }
    }

    private void Diamond(Vector2 centre, float radius, bool filled)
    {
        var points = new[]
        {
            centre + new Vector2(0f, -radius),
            centre + new Vector2(radius, 0f),
            centre + new Vector2(0f, radius),
            centre + new Vector2(-radius, 0f),
        };

        if (filled)
        {
            DrawColoredPolygon(points, Ink);
            return;
        }

        DrawPolyline(new[] { points[0], points[1], points[2], points[3], points[0] }, InkBright, 5f);
    }

    private void Square(Vector2 centre, float side, bool filled)
    {
        var box = new Rect2(centre - new Vector2(side, side) / 2f, new Vector2(side, side));

        if (filled)
            DrawRect(box, InkBright);
        else
            DrawRect(box, Ink, filled: false, width: 5f);
    }

    /// <summary>The rule down each edge of the screen, with a stepped bracket at each end of it.</summary>
    private void DrawRules()
    {
        const float inset = 14f;
        const float thick = 5f;

        DrawRect(new Rect2(inset, 0f, thick, Size.Y), Rule);
        DrawRect(new Rect2(Size.X - inset - thick, 0f, thick, Size.Y), Rule);

        foreach (bool right in new[] { false, true })
        {
            foreach (bool bottom in new[] { false, true })
                Bracket(right, bottom);
        }
    }

    /// <summary>
    /// The corner motif: three squared-off hooks stepping in from the corner, each smaller than
    /// the last, in the same ink as the rules.
    /// </summary>
    private void Bracket(bool right, bool bottom)
    {
        const float thick = 5f;

        float x = right ? Size.X - 14f - thick : 14f;
        float y = bottom ? Size.Y : 0f;
        float toward = right ? -1f : 1f;
        float down = bottom ? -1f : 1f;

        for (int step = 0; step < 3; step++)
        {
            float arm = 24f - step * 7f;
            float outX = x + toward * (14f + step * 18f);
            float outY = y + down * (26f + step * 22f);

            DrawRect(Normalised(new Rect2(outX, outY, toward * arm, thick)), Rule);
            DrawRect(Normalised(new Rect2(outX, outY, thick, down * arm)), Rule);
        }
    }

    /// <summary>A rectangle given by two corners, whichever way round they were handed over.</summary>
    private static Rect2 Normalised(Rect2 box) => box.Abs();

    /// <summary>A stable value for a lattice cell, so the scatter is the same every run.</summary>
    private static uint Hash(int x, int y)
    {
        uint h = (uint)(x * 374761393) + (uint)(y * 668265263);
        h = (h ^ (h >> 13)) * 1274126177;
        return h ^ (h >> 16);
    }

    /// <summary>Eight bits of the hash as nought to one.</summary>
    private static float Fraction(uint hash, int shift) => ((hash >> shift) & 0xFF) / 255f;
}
