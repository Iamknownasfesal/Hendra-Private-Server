using Godot;

namespace Hendra.UI;

/// <summary>
/// Every glyph the interface needs, drawn rather than loaded.
/// </summary>
/// <remarks>
/// <para>
/// The brief asks for a sprite sheet and a JSON atlas. There is no artwork to put in one: the
/// original's interface icons are compiled graphics inside a SWF and cannot be extracted, and the
/// game's own sheets are world sprites at eight pixels square, which is a quarter the size these
/// are drawn at and would look like a different game blown up next to anti-aliased text. So the
/// glyphs are geometry, in the same spirit as the star the fame display already drew by hand.
/// </para>
/// <para>
/// Every one is written in a unit square and mapped onto whatever rectangle it is given, so the
/// same call draws a 14-pixel chat rank and a 28-pixel heart without a second set of numbers. That
/// also means they scale with <c>--hud-scale</c> exactly, with no resampling and nothing to
/// pixel-snap.
/// </para>
/// </remarks>
public static class HudIcons
{
    /// <summary>Maps a point in the unit square onto the rectangle an icon was asked for.</summary>
    private static Vector2 At(in Rect2 box, float x, float y) =>
        box.Position + new Vector2(box.Size.X * x, box.Size.Y * y);

    private static float Span(in Rect2 box) => Mathf.Min(box.Size.X, box.Size.Y);

    private static Vector2[] Map(in Rect2 box, params float[] unit)
    {
        var points = new Vector2[unit.Length / 2];
        for (int i = 0; i < points.Length; i++)
            points[i] = At(box, unit[i * 2], unit[i * 2 + 1]);

        return points;
    }

    /// <summary>
    /// The health icon.
    /// </summary>
    /// <remarks>
    /// The classic parametric heart rather than two circles over a triangle, which reads as a
    /// clover at this size -- the lobes have to meet in a cusp for the shape to be recognised.
    /// </remarks>
    public static void Heart(CanvasItem into, Rect2 box, Color colour)
    {
        const int Steps = 28;
        var points = new Vector2[Steps];

        for (int i = 0; i < Steps; i++)
        {
            float t = i / (float)Steps * Mathf.Tau;
            float x = 16f * Mathf.Pow(Mathf.Sin(t), 3f);
            float y = 13f * Mathf.Cos(t) - 5f * Mathf.Cos(2f * t)
                      - 2f * Mathf.Cos(3f * t) - Mathf.Cos(4f * t);

            // The curve runs -17..17 across and -17..13 down, with y up; the map puts it in the
            // unit square with y down.
            points[i] = At(box, 0.5f + x / 36f, 0.52f - y / 34f);
        }

        into.DrawColoredPolygon(points, colour);
    }

    /// <summary>The magic icon: a conical flask.</summary>
    public static void Flask(CanvasItem into, Rect2 box, Color colour)
    {
        into.DrawColoredPolygon(
            Map(box, 0.36f, 0.06f, 0.64f, 0.06f, 0.60f, 0.38f, 0.90f, 0.94f, 0.10f, 0.94f, 0.40f, 0.38f),
            colour);

        // The lip, so the neck reads as glass rather than as a spike.
        into.DrawRect(new Rect2(At(box, 0.30f, 0.03f), new Vector2(box.Size.X * 0.40f, box.Size.Y * 0.08f)), colour);
    }

    /// <summary>
    /// A stacked potion, as the counters beside the bars show them.
    /// </summary>
    /// <remarks>
    /// A round-bottomed bottle with a cork: the shape has to survive at fourteen pixels, and the
    /// cork is what tells it apart from the coin at that size.
    /// </remarks>
    public static void Potion(CanvasItem into, Rect2 box, Color colour)
    {
        float span = Span(box);

        into.DrawCircle(At(box, 0.5f, 0.66f), span * 0.31f, colour);
        into.DrawRect(new Rect2(At(box, 0.38f, 0.28f), new Vector2(box.Size.X * 0.24f, box.Size.Y * 0.28f)), colour);
        into.DrawRect(new Rect2(At(box, 0.33f, 0.10f), new Vector2(box.Size.X * 0.34f, box.Size.Y * 0.16f)),
            colour.Darkened(0.35f));

        // A highlight down the left of the glass.
        into.DrawRect(new Rect2(At(box, 0.30f, 0.58f), new Vector2(box.Size.X * 0.08f, box.Size.Y * 0.16f)),
            Colors.White with { A = 0.45f });
    }

    /// <summary>The gem currency: a cut diamond, lit along the top facet.</summary>
    public static void Gem(CanvasItem into, Rect2 box, Color colour)
    {
        into.DrawColoredPolygon(Map(box, 0.5f, 0.06f, 0.94f, 0.5f, 0.5f, 0.94f, 0.06f, 0.5f), colour);
        into.DrawColoredPolygon(Map(box, 0.5f, 0.06f, 0.94f, 0.5f, 0.5f, 0.5f, 0.06f, 0.5f),
            colour.Lightened(0.25f));
    }

    /// <summary>
    /// The fame currency.
    /// </summary>
    /// <remarks>
    /// A four-pointed spark rather than a five-pointed star, so it is not mistaken for the rank
    /// star beside a name or the one on a chat line -- three stars in one interface all meaning
    /// different things is two too many.
    /// </remarks>
    public static void Fame(CanvasItem into, Rect2 box, Color colour)
    {
        var centre = At(box, 0.5f, 0.5f);
        float outer = Span(box) / 2f;

        var points = new Vector2[8];
        for (int i = 0; i < points.Length; i++)
        {
            float radius = i % 2 == 0 ? outer : outer * 0.34f;
            float angle = -Mathf.Pi / 2f + i * Mathf.Pi / 4f;
            points[i] = centre + new Vector2(Mathf.Cos(angle), Mathf.Sin(angle)) * radius;
        }

        into.DrawColoredPolygon(points, colour);
        into.DrawCircle(centre, outer * 0.22f, colour.Lightened(0.5f));
    }

    /// <summary>The main carried page, as the grid of slots it is.</summary>
    public static void Grid(CanvasItem into, Rect2 box, Color colour)
    {
        float cell = Mathf.Max(2f, Mathf.Round(box.Size.X * 0.38f));
        float gap = Mathf.Max(1f, Mathf.Round(box.Size.X * 0.12f));

        for (int y = 0; y < 2; y++)
        {
            for (int x = 0; x < 2; x++)
            {
                into.DrawRect(new Rect2(
                    box.Position.X + x * (cell + gap), box.Position.Y + y * (cell + gap), cell, cell), colour);
            }
        }
    }

    /// <summary>The second carried page: the bag the extra eight slots come in.</summary>
    public static void Backpack(CanvasItem into, Rect2 box, Color colour)
    {
        // A body under a wider flap, with two straps standing over it. An arching handle instead
        // of the straps reads as a padlock, which is the one thing this must not look like next to
        // a tab that opens something.
        into.DrawRect(new Rect2(At(box, 0.30f, 0.06f),
            new Vector2(box.Size.X * 0.12f, box.Size.Y * 0.28f)), colour);

        into.DrawRect(new Rect2(At(box, 0.58f, 0.06f),
            new Vector2(box.Size.X * 0.12f, box.Size.Y * 0.28f)), colour);

        into.DrawRect(new Rect2(At(box, 0.10f, 0.28f),
            new Vector2(box.Size.X * 0.80f, box.Size.Y * 0.22f)), colour);

        into.DrawRect(new Rect2(At(box, 0.18f, 0.50f),
            new Vector2(box.Size.X * 0.64f, box.Size.Y * 0.44f)), colour);
    }

    /// <summary>
    /// The shop: an awning over a doorway.
    /// </summary>
    /// <remarks>
    /// A storefront rather than a bag, because the bag shape is already the backpack tab over the
    /// hotbar and two bags in one interface meaning different things is one too many.
    /// </remarks>
    public static void Shop(CanvasItem into, Rect2 box, Color colour)
    {
        // The awning, wider than the shop under it.
        into.DrawColoredPolygon(Map(box, 0.04f, 0.40f, 0.18f, 0.10f, 0.82f, 0.10f, 0.96f, 0.40f), colour);

        into.DrawRect(new Rect2(At(box, 0.14f, 0.44f), new Vector2(box.Size.X * 0.72f, box.Size.Y * 0.50f)), colour);

        // The doorway, punched out in the darker tone the panels use.
        into.DrawRect(new Rect2(At(box, 0.40f, 0.58f), new Vector2(box.Size.X * 0.22f, box.Size.Y * 0.36f)),
            colour.Lerp(Style.PanelEdge, 0.8f));
    }

    /// <summary>
    /// News: a folded sheet with a headline and column rules on it.
    /// </summary>
    /// <remarks>
    /// Legible at twenty-eight pixels because it is three shapes, not a drawing of a newspaper --
    /// the fold down the left, a solid block for the headline, and two rules under it.
    /// </remarks>
    public static void News(CanvasItem into, Rect2 box, Color colour)
    {
        into.DrawRect(new Rect2(At(box, 0.06f, 0.14f), new Vector2(box.Size.X * 0.88f, box.Size.Y * 0.72f)), colour);

        var paper = colour.Lerp(Style.PanelEdge, 0.8f);
        into.DrawRect(new Rect2(At(box, 0.18f, 0.26f), new Vector2(box.Size.X * 0.64f, box.Size.Y * 0.16f)), paper);
        into.DrawRect(new Rect2(At(box, 0.18f, 0.50f), new Vector2(box.Size.X * 0.64f, box.Size.Y * 0.08f)), paper);
        into.DrawRect(new Rect2(At(box, 0.18f, 0.64f), new Vector2(box.Size.X * 0.44f, box.Size.Y * 0.08f)), paper);
    }

    /// <summary>The minimap's zoom steps.</summary>
    public static void Plus(CanvasItem into, Rect2 box, Color colour)
    {
        float thickness = Mathf.Max(2f, Mathf.Round(Span(box) * 0.2f));

        into.DrawRect(new Rect2(box.Position.X, At(box, 0f, 0.5f).Y - thickness / 2f, box.Size.X, thickness), colour);
        into.DrawRect(new Rect2(At(box, 0.5f, 0f).X - thickness / 2f, box.Position.Y, thickness, box.Size.Y), colour);
    }

    public static void Minus(CanvasItem into, Rect2 box, Color colour)
    {
        float thickness = Mathf.Max(2f, Mathf.Round(Span(box) * 0.2f));

        into.DrawRect(new Rect2(box.Position.X, At(box, 0f, 0.5f).Y - thickness / 2f, box.Size.X, thickness), colour);
    }

    /// <summary>The coin currency.</summary>
    public static void Coin(CanvasItem into, Rect2 box, Color colour)
    {
        var centre = At(box, 0.5f, 0.5f);
        float radius = Span(box) / 2f;

        into.DrawCircle(centre, radius, colour.Darkened(0.25f));
        into.DrawCircle(centre, radius * 0.82f, colour);
        into.DrawCircle(centre - new Vector2(0f, radius * 0.3f), radius * 0.34f,
            Colors.White with { A = 0.35f });
    }

    /// <summary>A five-pointed star: the chat rank, and the rating beside the player's name.</summary>
    public static void Star(CanvasItem into, Rect2 box, Color colour)
    {
        var centre = At(box, 0.5f, 0.5f);
        float outer = Span(box) / 2f;

        var points = new Vector2[10];
        for (int i = 0; i < points.Length; i++)
        {
            float radius = i % 2 == 0 ? outer : outer * 0.44f;
            float angle = -Mathf.Pi / 2f + i * Mathf.Pi / 5f;
            points[i] = centre + new Vector2(Mathf.Cos(angle), Mathf.Sin(angle)) * radius;
        }

        into.DrawColoredPolygon(points, colour);
    }

    /// <summary>The character sheet: a head and shoulders.</summary>
    public static void Bust(CanvasItem into, Rect2 box, Color colour)
    {
        into.DrawCircle(At(box, 0.5f, 0.32f), Span(box) * 0.20f, colour);

        // The shoulders, as the top half of a disc clipped by the bottom of the icon.
        const int Steps = 16;
        var shoulders = new Vector2[Steps + 2];
        for (int i = 0; i <= Steps; i++)
        {
            float angle = Mathf.Pi + i / (float)Steps * Mathf.Pi;
            shoulders[i] = At(box, 0.5f + Mathf.Cos(angle) * 0.38f, 0.98f + Mathf.Sin(angle) * 0.42f);
        }

        shoulders[^1] = At(box, 0.5f, 0.98f);
        into.DrawColoredPolygon(shoulders, colour);
    }

    /// <summary>The stats panel: a bar chart.</summary>
    public static void BarChart(CanvasItem into, Rect2 box, Color colour)
    {
        float width = box.Size.X * 0.22f;

        into.DrawRect(new Rect2(At(box, 0.10f, 0.52f), new Vector2(width, box.Size.Y * 0.38f)), colour);
        into.DrawRect(new Rect2(At(box, 0.39f, 0.28f), new Vector2(width, box.Size.Y * 0.62f)), colour);
        into.DrawRect(new Rect2(At(box, 0.68f, 0.10f), new Vector2(width, box.Size.Y * 0.80f)), colour);
    }

    /// <summary>The pets panel: a paw.</summary>
    public static void Paw(CanvasItem into, Rect2 box, Color colour)
    {
        float span = Span(box);

        // Four toes clear of the pad rather than touching it: at sixteen pixels a paw whose toes
        // meet its pad is a cloud.
        into.DrawCircle(At(box, 0.16f, 0.30f), span * 0.11f, colour);
        into.DrawCircle(At(box, 0.39f, 0.15f), span * 0.11f, colour);
        into.DrawCircle(At(box, 0.62f, 0.15f), span * 0.11f, colour);
        into.DrawCircle(At(box, 0.85f, 0.30f), span * 0.11f, colour);
        into.DrawCircle(At(box, 0.50f, 0.74f), span * 0.23f, colour);
    }

    /// <summary>Settings: a cogwheel.</summary>
    public static void Gear(CanvasItem into, Rect2 box, Color colour)
    {
        var centre = At(box, 0.5f, 0.5f);
        float span = Span(box);

        const int Teeth = 8;
        for (int i = 0; i < Teeth; i++)
        {
            float angle = i / (float)Teeth * Mathf.Tau;
            var direction = new Vector2(Mathf.Cos(angle), Mathf.Sin(angle));
            var side = new Vector2(-direction.Y, direction.X) * span * 0.10f;

            into.DrawColoredPolygon(
                new[]
                {
                    centre + direction * span * 0.22f + side,
                    centre + direction * span * 0.48f + side * 0.7f,
                    centre + direction * span * 0.48f - side * 0.7f,
                    centre + direction * span * 0.22f - side,
                },
                colour);
        }

        // A ring rather than a disc with a hole punched in it: there is no background colour to
        // punch with, since these sit over the world as often as over a panel.
        into.DrawArc(centre, span * 0.26f, 0f, Mathf.Tau, 28, colour, span * 0.16f);
    }

    /// <summary>The chat button: a speech bubble.</summary>
    public static void SpeechBubble(CanvasItem into, Rect2 box, Color colour)
    {
        var body = new Rect2(At(box, 0.04f, 0.12f), new Vector2(box.Size.X * 0.92f, box.Size.Y * 0.56f));
        float radius = body.Size.Y / 2f;

        into.DrawRect(new Rect2(body.Position + new Vector2(radius * 0.6f, 0f),
            new Vector2(body.Size.X - radius * 1.2f, body.Size.Y)), colour);
        into.DrawCircle(new Vector2(body.Position.X + radius * 0.6f, body.Position.Y + radius), radius, colour);
        into.DrawCircle(new Vector2(body.End.X - radius * 0.6f, body.Position.Y + radius), radius, colour);

        into.DrawColoredPolygon(Map(box, 0.22f, 0.60f, 0.46f, 0.60f, 0.24f, 0.96f), colour);
    }

    /// <summary>
    /// The mouse button a slot answers to, in its corner.
    /// </summary>
    /// <param name="right">Whether the right button is the one highlighted.</param>
    public static void MouseButton(CanvasItem into, Rect2 box, Color colour, bool right)
    {
        var outline = new Rect2(At(box, 0.18f, 0.04f), new Vector2(box.Size.X * 0.64f, box.Size.Y * 0.92f));
        float radius = outline.Size.X / 2f;

        // The body: a capsule, drawn as a rectangle between two discs.
        into.DrawCircle(outline.Position + new Vector2(radius, radius), radius, colour);
        into.DrawCircle(new Vector2(outline.Position.X + radius, outline.End.Y - radius), radius, colour);
        into.DrawRect(new Rect2(outline.Position.X, outline.Position.Y + radius,
            outline.Size.X, outline.Size.Y - radius * 2f), colour);

        // The button that is not bound is hollowed out, so which half is filled is the whole
        // message the glyph carries.
        var hollow = new Rect2(
            right ? outline.Position.X + 1f : outline.Position.X + outline.Size.X / 2f,
            outline.Position.Y + 1f,
            outline.Size.X / 2f - 1f,
            outline.Size.Y * 0.42f);

        into.DrawRect(hollow, Style.PanelSolid with { A = 0.75f });
    }

    /// <summary>The loadout cycle: two arrows chasing each other.</summary>
    public static void SwapArrows(CanvasItem into, Rect2 box, Color colour)
    {
        Arrow(into, box, colour, 0.30f, pointsRight: true);
        Arrow(into, box, colour, 0.70f, pointsRight: false);
    }

    private static void Arrow(CanvasItem into, in Rect2 box, Color colour, float y, bool pointsRight)
    {
        float shaftLeft = pointsRight ? 0.10f : 0.34f;
        float shaftRight = pointsRight ? 0.66f : 0.90f;

        into.DrawRect(new Rect2(At(box, shaftLeft, y - 0.09f),
            new Vector2(box.Size.X * (shaftRight - shaftLeft), box.Size.Y * 0.18f)), colour);

        into.DrawColoredPolygon(pointsRight
            ? Map(box, 0.62f, y - 0.26f, 0.94f, y, 0.62f, y + 0.26f)
            : Map(box, 0.38f, y - 0.26f, 0.06f, y, 0.38f, y + 0.26f), colour);
    }

    /// <summary>The way home: the Nexus, as the temple the original draws it.</summary>
    public static void Temple(CanvasItem into, Rect2 box, Color colour)
    {
        into.DrawColoredPolygon(Map(box, 0.5f, 0.06f, 0.96f, 0.34f, 0.04f, 0.34f), colour);
        into.DrawRect(new Rect2(At(box, 0.06f, 0.36f), new Vector2(box.Size.X * 0.88f, box.Size.Y * 0.08f)), colour);

        for (int i = 0; i < 4; i++)
        {
            into.DrawRect(new Rect2(At(box, 0.14f + i * 0.21f, 0.46f),
                new Vector2(box.Size.X * 0.11f, box.Size.Y * 0.36f)), colour);
        }

        into.DrawRect(new Rect2(At(box, 0.04f, 0.84f), new Vector2(box.Size.X * 0.92f, box.Size.Y * 0.10f)), colour);
    }

    /// <summary>A chevron, for the minimap's zoom and the chat's scroll arrows.</summary>
    public static void Chevron(CanvasItem into, Rect2 box, Color colour, bool up)
    {
        into.DrawColoredPolygon(up
            ? Map(box, 0.5f, 0.18f, 0.94f, 0.78f, 0.06f, 0.78f)
            : Map(box, 0.5f, 0.82f, 0.94f, 0.22f, 0.06f, 0.22f), colour);
    }

    /// <summary>A round dot with a dark rim, as the guild and party markers use.</summary>
    public static void Dot(CanvasItem into, Rect2 box, Color colour)
    {
        var centre = At(box, 0.5f, 0.5f);
        float radius = Span(box) / 2f;

        into.DrawCircle(centre, radius, new Color(0f, 0f, 0f, 0.55f));
        into.DrawCircle(centre, radius - Mathf.Max(1f, radius * 0.16f), colour);
    }
}
