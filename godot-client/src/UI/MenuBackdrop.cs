using Godot;

namespace Hendra.UI;

/// <summary>
/// The page the full-screen menus are printed on: a near-black wash, a scatter of large diamonds
/// and small squares drifting across it, and the bracket ornament that frames the whole screen.
/// </summary>
/// <remarks>
/// <para>
/// Measured off the options and character pages rather than invented. The wash runs from
/// <c>#111111</c> at the top of the screen to <c>#2b2b2b</c> at the bottom, both measured; the shapes sit twelve
/// and twenty-six points above whatever the wash is under them, which is why they are drawn as an
/// offset from the wash instead of as fixed colours — a fixed colour disappears at one end of the
/// page and shouts at the other.
/// </para>
/// <para>
/// The frame is a five-pixel rule down each side, inset fifteen from the edge and stopping seventy
/// short of top and bottom, with short rules at the bottom corners and a scatter of small blocks
/// stepping away from each corner. It is the same ornament on every page in the references, and it
/// is most of what makes a screen there read as a printed page rather than as a window.
/// </para>
/// <para>
/// The scatter drifts. The references are stills and cannot show it, but the screen behind them is
/// a live game and the first thing a player sees should not be a photograph — a pixel every two
/// seconds is enough to say so and slow enough that nobody watches it.
/// </para>
/// </remarks>
public partial class MenuBackdrop : Control
{
    /// <summary>The wash at the top of the screen.</summary>
    private static readonly Color WashTop = new("111111");

    /// <summary>The wash at the bottom of it.</summary>
    private static readonly Color WashBottom = new("2b2b2b");

    /// <summary>How far above the wash the large diamonds sit.</summary>
    private const float DiamondLift = 12f / 255f;

    /// <summary>How far above it the small squares sit. They are the brighter of the two.</summary>
    private const float SquareLift = 26f / 255f;

    private static readonly Color Ornament = new("464646");

    private const int Diamonds = 7;
    private const int Squares = 7;

    /// <summary>Seeded rather than random, so the same page is the same page every time.</summary>
    private const ulong Seed = 0x48_45_4E_44;

    /// <summary>How far the scatter travels in a second, at the reference resolution.</summary>
    private const float Drift = 4.5f;

    /// <summary>The resolution every measurement in here was taken at.</summary>
    private const float ReferenceHeight = 1080f;

    private struct Shape
    {
        public Vector2 At;
        public float Size;
        public bool IsSquare;
        public float Speed;
    }

    private readonly Shape[] _shapes = new Shape[Diamonds + Squares];
    private double _elapsed;

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;
        Scatter();
    }

    public override void _Notification(int what)
    {
        if (what == NotificationResized)
            Scatter();
    }

    public override void _Process(double delta)
    {
        _elapsed += delta;
        QueueRedraw();
    }

    /// <summary>
    /// Lays the shapes out across the page, in fractions of it so a resize keeps the layout.
    /// </summary>
    /// <remarks>
    /// Each kind gets a cell of its own on a coarse grid and is jittered inside it, rather than
    /// being dropped anywhere. Seven independent throws at a screen this size reliably puts four of
    /// them in one corner and leaves the other half empty, which reads as a mistake; the reference
    /// pages have theirs evenly spread, and a jittered grid is what that looks like.
    /// </remarks>
    private void Scatter()
    {
        var random = new RandomNumberGenerator { Seed = Seed };

        for (int i = 0; i < _shapes.Length; i++)
        {
            bool square = i >= Diamonds;
            int index = square ? i - Diamonds : i;
            int count = square ? Squares : Diamonds;

            // Cells run down a diagonal so the two kinds never share one, which would put a square
            // in the middle of a diamond on every page.
            float cell = (index + 0.5f) / count;

            _shapes[i] = new Shape
            {
                At = new Vector2(
                    Mathf.PosMod(cell * 2.6f + (square ? 0.31f : 0f) + random.Randf() * 0.16f, 1f),
                    Mathf.PosMod(cell + random.Randf() * 0.10f, 1f)),
                // The diamond figure is a half-diagonal, so a shape of 0.065 is a diamond a
                // hundred and forty pixels across at 1080 — which is what the references measure.
                Size = square ? 0.024f + random.Randf() * 0.008f : 0.062f + random.Randf() * 0.045f,
                IsSquare = square,

                // A spread of speeds rather than one, so the field never resolves into a single
                // sheet sliding past.
                Speed = 0.6f + random.Randf() * 0.8f,
            };
        }
    }

    public override void _Draw()
    {
        var size = Size;
        if (size.X <= 0f || size.Y <= 0f)
            return;

        DrawWash(size);
        DrawScatter(size);
        DrawFrame(size);
    }

    /// <summary>The vertical wash, as one quad with a colour at each corner.</summary>
    private void DrawWash(Vector2 size)
    {
        DrawPolygon(
            new[] { Vector2.Zero, new Vector2(size.X, 0f), size, new Vector2(0f, size.Y) },
            new[] { WashTop, WashTop, WashBottom, WashBottom });
    }

    private void DrawScatter(Vector2 size)
    {
        float scale = size.Y / ReferenceHeight;
        float travel = (float)_elapsed * Drift * scale;

        foreach (var shape in _shapes)
        {
            float extent = shape.Size * size.Y;

            // Wrapped a shape's width past each edge, so one never appears or vanishes on screen.
            float x = Mathf.PosMod(shape.At.X * (size.X + extent * 2f) - extent, size.X + extent * 2f) - extent;
            float y = Mathf.PosMod(
                shape.At.Y * (size.Y + extent * 2f) - travel * shape.Speed, size.Y + extent * 2f) - extent;

            var centre = new Vector2(x, y);
            var colour = WashAt(y / size.Y);
            float lift = shape.IsSquare ? SquareLift : DiamondLift;
            colour = new Color(colour.R + lift, colour.G + lift, colour.B + lift);

            if (shape.IsSquare)
                DrawRect(new Rect2(centre - Vector2.One * extent * 0.5f, Vector2.One * extent), colour);
            else
                DrawColoredPolygon(
                    new[]
                    {
                        centre + new Vector2(0f, -extent), centre + new Vector2(extent, 0f),
                        centre + new Vector2(0f, extent), centre + new Vector2(-extent, 0f),
                    }, colour);
        }
    }

    private static Color WashAt(float t) => WashTop.Lerp(WashBottom, Mathf.Clamp(t, 0f, 1f));

    /// <summary>
    /// The side rules, the short rules at the bottom corners, and the blocks that step away from
    /// each corner.
    /// </summary>
    private void DrawFrame(Vector2 size)
    {
        float s = size.Y / ReferenceHeight;

        DrawRect(new Rect2(14f * s, 70f * s, 5f * s, size.Y - 140f * s), Ornament);
        DrawRect(new Rect2(size.X - 19f * s, 70f * s, 5f * s, size.Y - 140f * s), Ornament);

        DrawRect(new Rect2(19f * s, size.Y - 22f * s, 142f * s, 7f * s), Ornament);
        DrawRect(new Rect2(size.X - 153f * s, size.Y - 22f * s, 133f * s, 7f * s), Ornament);

        foreach (var block in CornerBlocks)
        {
            // The one ornament, mirrored into all four corners. Every page in the references
            // carries it on all four, and reading it off one is all the measuring it needs.
            DrawRect(new Rect2(block.Position * s, block.Size * s), Ornament);
            DrawRect(new Rect2(
                size.X - (block.Position.X + block.Size.X) * s, block.Position.Y * s,
                block.Size.X * s, block.Size.Y * s), Ornament);
            DrawRect(new Rect2(
                block.Position.X * s, size.Y - (block.Position.Y + block.Size.Y) * s,
                block.Size.X * s, block.Size.Y * s), Ornament);
            DrawRect(new Rect2(
                size.X - (block.Position.X + block.Size.X) * s,
                size.Y - (block.Position.Y + block.Size.Y) * s,
                block.Size.X * s, block.Size.Y * s), Ornament);
        }
    }

    /// <summary>The corner ornament, transcribed pixel for pixel from the options page.</summary>
    private static readonly Rect2[] CornerBlocks =
    {
        new(31f, 18f, 5f, 17f),
        new(42f, 18f, 5f, 14f),
        new(56f, 18f, 5f, 14f),
        new(66f, 18f, 6f, 5f),
        new(23f, 25f, 5f, 5f),
        new(42f, 28f, 19f, 4f),
        new(14f, 35f, 22f, 5f),
        new(14f, 46f, 15f, 4f),
        new(24f, 50f, 5f, 10f),
        new(14f, 60f, 15f, 5f),
    };
}
