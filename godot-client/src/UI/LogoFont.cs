using System.Collections.Generic;
using Godot;

namespace Hendra.UI;

/// <summary>
/// The angular display face the game's mark is set in.
/// </summary>
/// <remarks>
/// <para>
/// A handful of letters described as outlines rather than a font file. A display face is not a
/// reading face: it is used at one size, in one word, and every corner in it is deliberate — which
/// makes six polygons cheaper to carry, and far easier to keep on a whole-pixel grid, than a
/// typeface installed for one string.
/// </para>
/// <para>
/// Each letter is one closed contour plus any counters it needs, laid out on a grid where the cap
/// line is <c>0</c>, the baseline is <c>1</c>, and the flared feet run down to
/// <see cref="Depth"/>. One contour rather than a pile of strokes is what lets the mark carry a
/// keyline: a letter built from overlapping bars has seams inside it, and a stroke along its edge
/// draws every one of them.
/// </para>
/// <para>
/// Only the letters the mark uses are described. Anything else falls back to nothing rather than to
/// a wrong shape, so a changed word fails visibly instead of quietly dropping a letter's character.
/// </para>
/// </remarks>
public static class LogoFont
{
    /// <summary>How far below the baseline the flared feet reach, in cap heights.</summary>
    public const float Depth = 1.10f;

    /// <summary>The gap between letters, in cap heights.</summary>
    public const float Tracking = 0.06f;

    /// <summary>A letter: how much room it takes, its contour, and the holes punched in it.</summary>
    private readonly struct Glyph
    {
        public readonly float Advance;
        public readonly Vector2[] Outline;
        public readonly Vector2[][] Counters;

        public Glyph(float advance, Vector2[] outline, Vector2[][] counters = null)
        {
            Advance = advance;
            Outline = outline;
            Counters = counters ?? System.Array.Empty<Vector2[]>();
        }
    }

    /// <summary>A letter placed at a size and a position, ready to draw.</summary>
    public readonly struct Placed
    {
        public readonly Vector2[] Outline;
        public readonly Vector2[][] Counters;

        public Placed(Vector2[] outline, Vector2[][] counters)
        {
            Outline = outline;
            Counters = counters;
        }
    }

    private static Vector2[] Path(params float[] pairs)
    {
        var points = new Vector2[pairs.Length / 2];
        for (int i = 0; i < points.Length; i++)
            points[i] = new Vector2(pairs[i * 2], pairs[i * 2 + 1]);

        return points;
    }

    private static readonly Dictionary<char, Glyph> Letters = new()
    {
        ['H'] = new Glyph(0.98f, Path(
            0.00f, 0.16f, 0.16f, 0.00f, 0.36f, 0.00f, 0.36f, 0.42f, 0.62f, 0.42f, 0.62f, 0.00f,
            0.82f, 0.00f, 0.98f, 0.16f, 0.98f, 0.88f, 1.12f, 1.10f, 0.76f, 1.10f, 0.62f, 0.90f,
            0.62f, 0.62f, 0.36f, 0.62f, 0.36f, 0.90f, 0.50f, 1.10f, 0.14f, 1.10f, 0.00f, 0.88f)),

        ['E'] = new Glyph(0.86f, Path(
            0.00f, 0.16f, 0.16f, 0.00f, 0.86f, 0.00f, 0.86f, 0.19f, 0.36f, 0.19f, 0.36f, 0.42f,
            0.70f, 0.42f, 0.70f, 0.60f, 0.36f, 0.60f, 0.36f, 0.88f, 0.92f, 0.88f, 0.92f, 1.10f,
            0.14f, 1.10f, 0.00f, 0.88f)),

        ['N'] = new Glyph(1.08f, Path(
            0.00f, 0.16f, 0.16f, 0.00f, 0.38f, 0.00f, 0.70f, 0.58f, 0.70f, 0.00f, 0.90f, 0.00f,
            1.06f, 0.16f, 1.06f, 0.88f, 1.20f, 1.10f, 0.84f, 1.10f, 0.70f, 0.90f, 0.36f, 0.32f,
            0.36f, 0.90f, 0.50f, 1.10f, 0.14f, 1.10f, 0.00f, 0.88f)),

        ['D'] = new Glyph(0.94f, Path(
            0.00f, 0.16f, 0.16f, 0.00f, 0.62f, 0.00f, 0.94f, 0.26f, 0.94f, 0.80f, 0.62f, 1.10f,
            0.14f, 1.10f, 0.00f, 0.88f),
            new[]
            {
                Path(0.36f, 0.22f, 0.58f, 0.22f, 0.72f, 0.36f, 0.72f, 0.72f, 0.58f, 0.86f, 0.36f, 0.86f),
            }),

        ['R'] = new Glyph(0.96f, Path(
            0.00f, 0.16f, 0.16f, 0.00f, 0.60f, 0.00f, 0.88f, 0.20f, 0.88f, 0.42f, 0.66f, 0.58f,
            1.02f, 1.10f, 0.68f, 1.10f, 0.48f, 0.64f, 0.36f, 0.64f, 0.36f, 0.90f, 0.50f, 1.10f,
            0.14f, 1.10f, 0.00f, 0.88f),
            new[]
            {
                Path(0.36f, 0.20f, 0.58f, 0.20f, 0.70f, 0.30f, 0.70f, 0.40f, 0.58f, 0.50f, 0.36f, 0.50f),
            }),

        ['A'] = new Glyph(1.04f, Path(
            0.44f, 0.00f, 0.64f, 0.00f, 1.04f, 1.10f, 0.72f, 1.10f, 0.64f, 0.84f, 0.40f, 0.84f,
            0.32f, 1.10f, 0.00f, 1.10f),
            new[]
            {
                Path(0.54f, 0.22f, 0.67f, 0.76f, 0.39f, 0.76f),
            }),
    };

    /// <summary>How wide a word is, in cap heights, tracking included.</summary>
    public static float WidthOf(string word)
    {
        float width = 0f;
        foreach (char letter in word)
            if (Letters.TryGetValue(letter, out var glyph))
                width += glyph.Advance + Tracking;

        return Mathf.Max(width - Tracking, 0.001f);
    }

    /// <summary>Places a word at a cap height, with the cap line of the first letter at <paramref name="at"/>.</summary>
    public static Placed[] Lay(string word, float cap, Vector2 at)
    {
        var placed = new List<Placed>(word.Length);
        float x = at.X;

        foreach (char letter in word)
        {
            if (!Letters.TryGetValue(letter, out var glyph))
                continue;

            var origin = new Vector2(x, at.Y);
            var counters = new Vector2[glyph.Counters.Length][];
            for (int i = 0; i < counters.Length; i++)
                counters[i] = Scale(glyph.Counters[i], cap, origin);

            placed.Add(new Placed(Scale(glyph.Outline, cap, origin), counters));
            x += (glyph.Advance + Tracking) * cap;
        }

        return placed.ToArray();
    }

    private static Vector2[] Scale(Vector2[] points, float cap, Vector2 origin)
    {
        var scaled = new Vector2[points.Length];
        for (int i = 0; i < points.Length; i++)
            scaled[i] = origin + points[i] * cap;

        return scaled;
    }
}
