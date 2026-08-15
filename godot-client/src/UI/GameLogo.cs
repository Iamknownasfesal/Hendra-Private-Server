using Godot;

namespace Hendra.UI;

/// <summary>
/// The game's mark, drawn rather than loaded: the word on its banner, with the edition line under
/// it and the short wordmark below that.
/// </summary>
/// <remarks>
/// <para>
/// It is drawn from polygons and a generated glow instead of shipped as a bitmap so that one node
/// serves every size it is asked for — the title screen wants it eight hundred pixels across, the
/// Escape menu wants the same mark at three hundred in the top of its column, and a bitmap authored
/// for either is soft at the other.
/// </para>
/// <para>
/// Everything inside is expressed as a fraction of <see cref="LogoWidth"/>, so the whole
/// composition scales as one thing and the only number a caller ever supplies is a width.
/// </para>
/// <para>
/// The ink is the mark's own and deliberately not the interface palette: the interface is grey
/// plate and white type, and a logo mixed from those tokens would disappear into the page it sits
/// on. These four colours — the two ends of the red gradient, the dark edge and the amber keyline —
/// exist only here.
/// </para>
/// </remarks>
/// <example>
/// Mounting it in a panel column, which is what the Escape menu needs:
/// <code>
/// var logo = new GameLogo(PanelWidth - Padding * 2f);
/// logo.Position = new Vector2(Padding, Padding);
/// panel.Body.AddChild(logo);
/// </code>
/// The node sizes itself; <see cref="SizeFor"/> answers how much room to leave for it without
/// having to build one first.
/// </example>
public partial class GameLogo : Control
{
    /// <summary>The word the mark is of.</summary>
    private const string Word = "HENDRA";

    /// <summary>The edition line under the rule, in the interface's own face.</summary>
    private const string Edition = "REBORN";

    /// <summary>The short wordmark below the banner, the way a boxed game carries its ticker.</summary>
    private const string Mark = "HNDR©";

    /// <summary>How tall the mark is for a given width. Fixed, so the composition never distorts.</summary>
    public const float Aspect = 0.61f;

    // The composition, in fractions of the mark's width. Read down the mark: banner, word, rule,
    // edition, then the wordmark hanging below the banner.
    //
    // The word is wider than the banner on purpose. An emblem the type sits neatly inside reads as
    // a frame around a label; one the type overhangs reads as a shape behind a word, which is what
    // the reference's own mark does with the letters that break past its arms.
    private const float BannerInset = 0.130f;
    private const float BannerTop = 0.055f;
    private const float BannerBottom = 0.500f;
    private const float BannerCut = 0.075f;
    private const float WordWidth = 0.900f;
    private const float WordTop = 0.105f;
    private const float RuleY = 0.300f;
    private const float RuleHalfWidth = 0.200f;
    private const float EditionBaseline = 0.400f;
    private const float MarkBaseline = 0.570f;
    private const float TrademarkBaseline = 0.075f;

    // Type sizes, also as fractions of the width.
    private const float EditionSize = 0.066f;
    private const float MarkSize = 0.064f;
    private const float TrademarkSize = 0.038f;

    /// <summary>How far apart the edition's letters are set, as a fraction of its size.</summary>
    private const float EditionTracking = 0.34f;

    // The mark's ink.
    private static readonly Color InkTop = new("ff8c4a");
    private static readonly Color InkUpper = new("ff4622");
    private static readonly Color InkLower = new("c9140c");
    private static readonly Color InkBottom = new("5b0902");

    /// <summary>The dark edge around every letter, and the colour its counters are punched in.</summary>
    private static readonly Color InkEdge = new("2a0503");

    /// <summary>The thin warm keyline that follows each letter, and the banner's edge.</summary>
    private static readonly Color Keyline = new("e6a23c");

    private static readonly Color BannerFill = new("18100e");
    private static readonly Color Glow = new("c0240e");

    private float _width;

    /// <param name="width">How wide the mark should be. Its height follows from <see cref="Aspect"/>.</param>
    public GameLogo(float width = 800f)
    {
        MouseFilter = MouseFilterEnum.Ignore;
        LogoWidth = width;
    }

    /// <summary>How wide the mark is drawn. Setting it resizes the node to match.</summary>
    public float LogoWidth
    {
        get => _width;
        set
        {
            if (Mathf.IsEqualApprox(_width, value))
                return;

            _width = Mathf.Max(value, 1f);
            CustomMinimumSize = SizeFor(_width);
            Size = CustomMinimumSize;
            QueueRedraw();
        }
    }

    /// <summary>The room a mark of this width needs, for laying out around one that does not exist yet.</summary>
    public static Vector2 SizeFor(float width) => new(width, Mathf.Round(width * Aspect));

    public override void _Draw()
    {
        float w = _width;
        if (w <= 1f)
            return;

        DrawGlow(w);
        DrawBanner(w);
        DrawWord(w);
        DrawRule(w);
        DrawEdition(w);
        DrawTrademark(w);
        DrawWordmark(w);
    }

    /// <summary>
    /// The warm bloom behind the banner.
    /// </summary>
    /// <remarks>
    /// A generated texture rather than a ring of polygons: the falloff has to be smooth across
    /// several hundred pixels, and stacking translucent shapes to fake that leaves visible bands
    /// exactly where the mark is brightest.
    /// </remarks>
    private void DrawGlow(float w)
    {
        float radius = w * 0.58f;
        var centre = new Vector2(w * 0.5f, w * 0.240f);

        DrawTextureRect(
            Bloom, new Rect2(centre - Vector2.One * radius, Vector2.One * radius * 2f), false,
            Glow with { A = 0.8f });
    }

    /// <summary>
    /// The shield behind the word.
    /// </summary>
    /// <remarks>
    /// Cut corners along the top, which is the interface's own shape, and diagonals converging on a
    /// shallow point at the bottom, which is not — a shape that is a rectangle on all four sides
    /// reads as a panel that happens to have a logo in it however dark it is filled.
    /// </remarks>
    private void DrawBanner(float w)
    {
        float left = w * BannerInset;
        float right = w * (1f - BannerInset);
        float top = w * BannerTop;
        float cut = w * BannerCut;
        float shoulder = w * (BannerTop + (BannerBottom - BannerTop) * 0.55f);
        float point = w * BannerBottom;

        var outline = new[]
        {
            new Vector2(left, top + cut), new Vector2(left + cut, top),
            new Vector2(right - cut, top), new Vector2(right, top + cut),
            new Vector2(right, shoulder), new Vector2(w * 0.5f, point),
            new Vector2(left, shoulder),
        };

        DrawColoredPolygon(outline, BannerFill);
        DrawClosed(outline, Keyline with { A = 0.55f }, Mathf.Max(1f, w * 0.0035f));
    }

    /// <summary>The word itself: dark edge, graded fill, punched counters, keyline.</summary>
    private void DrawWord(float w)
    {
        float cap = w * WordWidth / LogoFont.WidthOf(Word);
        float edge = Mathf.Max(1.5f, cap * 0.042f);
        var origin = new Vector2(Mathf.Round((w - w * WordWidth) / 2f), w * WordTop);

        var letters = LogoFont.Lay(Word, cap, origin);

        // Eight offset copies under the letters, the same way the interface edges a sprite. A
        // stroke along the path would not do: it would run through the seams where two letters
        // nearly touch and read as a chain rather than as six separate shapes.
        foreach (var offset in Around)
            foreach (var letter in letters)
                DrawColoredPolygon(Shift(letter.Outline, offset * edge), InkEdge);

        foreach (var letter in letters)
            DrawPolygon(letter.Outline, Shades(letter.Outline, origin.Y, cap * LogoFont.Depth));

        // The counters are punched in the edge colour rather than left as holes: the bloom behind
        // would otherwise shine through the middle of every bowl.
        foreach (var letter in letters)
            foreach (var counter in letter.Counters)
                DrawColoredPolygon(counter, InkEdge);

        float keyline = Mathf.Max(1f, cap * 0.018f);
        foreach (var letter in letters)
        {
            DrawClosed(letter.Outline, Keyline, keyline);
            foreach (var counter in letter.Counters)
                DrawClosed(counter, Keyline, keyline);
        }
    }

    /// <summary>The hairline between the word and the edition, notched at the middle.</summary>
    private void DrawRule(float w)
    {
        float y = Mathf.Round(w * RuleY);
        float thickness = Mathf.Max(1f, Mathf.Round(w * 0.0035f));
        float half = w * RuleHalfWidth;
        float notch = w * 0.018f;

        var colour = Keyline with { A = 0.7f };
        DrawRect(new Rect2(w * 0.5f - half, y, half - notch * 1.6f, thickness), colour);
        DrawRect(new Rect2(w * 0.5f + notch * 1.6f, y, half - notch * 1.6f, thickness), colour);

        // A small diamond in the gap, which is the same break the panels put in their dividers.
        var centre = new Vector2(w * 0.5f, y + thickness * 0.5f);
        DrawColoredPolygon(
            new[]
            {
                centre + new Vector2(0f, -notch), centre + new Vector2(notch, 0f),
                centre + new Vector2(0f, notch), centre + new Vector2(-notch, 0f),
            }, colour);
    }

    private void DrawEdition(float w)
    {
        int size = Mathf.Max(Style.SmallestReadable, Mathf.RoundToInt(w * EditionSize));
        DrawTracked(Edition, size, size * EditionTracking, w * 0.5f, w * EditionBaseline, Style.Text);
    }

    private void DrawWordmark(float w)
    {
        int size = Mathf.Max(Style.SmallestReadable, Mathf.RoundToInt(w * MarkSize));
        DrawTracked(Mark, size, size * 0.06f, w * 0.5f, w * MarkBaseline, Style.Text);
    }

    /// <summary>The trademark tick, above the banner's top right corner.</summary>
    private void DrawTrademark(float w)
    {
        int size = Mathf.Max(12, Mathf.RoundToInt(w * TrademarkSize));
        float width = Style.Measure("TM", size);
        this.DrawText(new Vector2(w * 0.925f - width, w * TrademarkBaseline), "TM", size, Style.Text);
    }

    /// <summary>Draws a string letter by letter so the tracking can be opened up.</summary>
    private void DrawTracked(string text, int size, float tracking, float centreX, float baseline, Color colour)
    {
        float total = -tracking;
        foreach (char letter in text)
            total += Style.Measure(letter.ToString(), size) + tracking;

        float x = Mathf.Round(centreX - total / 2f);
        foreach (char letter in text)
        {
            string glyph = letter.ToString();
            this.DrawText(new Vector2(x, Mathf.Round(baseline)), glyph, size, colour);
            x += Style.Measure(glyph, size) + tracking;
        }
    }

    /// <summary>One colour per vertex, taken from where that vertex sits down the letter.</summary>
    private static Color[] Shades(Vector2[] points, float top, float height)
    {
        var shades = new Color[points.Length];
        for (int i = 0; i < points.Length; i++)
            shades[i] = Sample(Mathf.Clamp((points[i].Y - top) / height, 0f, 1f));

        return shades;
    }

    /// <summary>The red gradient, bright at the top of a letter and nearly black at its foot.</summary>
    private static Color Sample(float t) => t switch
    {
        < 0.30f => InkTop.Lerp(InkUpper, t / 0.30f),
        < 0.62f => InkUpper.Lerp(InkLower, (t - 0.30f) / 0.32f),
        _ => InkLower.Lerp(InkBottom, (t - 0.62f) / 0.38f),
    };

    private static Vector2[] Shift(Vector2[] points, Vector2 by)
    {
        var moved = new Vector2[points.Length];
        for (int i = 0; i < points.Length; i++)
            moved[i] = points[i] + by;

        return moved;
    }

    /// <summary>Strokes a polygon's own outline, which needs the first point repeated at the end.</summary>
    private void DrawClosed(Vector2[] points, Color colour, float width)
    {
        var closed = new Vector2[points.Length + 1];
        points.CopyTo(closed, 0);
        closed[^1] = points[0];
        DrawPolyline(closed, colour, width);
    }

    private static readonly Vector2[] Around =
    {
        new(-1f, 0f), new(1f, 0f), new(0f, -1f), new(0f, 1f),
        new(-1f, -1f), new(1f, -1f), new(-1f, 1f), new(1f, 1f),
    };

    private static ImageTexture _bloom;

    /// <summary>A white disc that fades to nothing at its rim, tinted at each use.</summary>
    private static ImageTexture Bloom
    {
        get
        {
            if (_bloom != null)
                return _bloom;

            const int Size = 128;
            var image = Image.CreateEmpty(Size, Size, false, Image.Format.Rgba8);

            for (int y = 0; y < Size; y++)
                for (int x = 0; x < Size; x++)
                {
                    float distance = new Vector2(x - Size / 2f + 0.5f, y - Size / 2f + 0.5f).Length()
                                     / (Size / 2f);

                    // Squared falloff, so the bloom is concentrated behind the word rather than a
                    // flat wash across the whole node.
                    float alpha = Mathf.Pow(Mathf.Clamp(1f - distance, 0f, 1f), 2.2f);
                    image.SetPixel(x, y, new Color(1f, 1f, 1f, alpha));
                }

            return _bloom = ImageTexture.CreateFromImage(image);
        }
    }
}
