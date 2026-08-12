using Godot;

namespace Hendra.UI;

/// <summary>
/// Stars: the rating an account carries in front of its name.
/// </summary>
/// <remarks>
/// <para>
/// The original's <c>FameUtil</c>. A character earns a star for each fame threshold it passes, up
/// to five, and an account's rating is the sum of those across every class — so it measures how
/// widely you have played rather than how far you have taken one character. The thresholds and the
/// colours here are its numbers exactly.
/// </para>
/// <para>
/// The colour is the point: it says at a glance roughly how far along someone is. It steps every
/// time the total passes another multiple of the number of classes, which is what makes the top
/// colour mean "has done this with everything" rather than an arbitrary score.
/// </para>
/// </remarks>
public static class Fame
{
    /// <summary>The fame a character needs for each of its five stars.</summary>
    public static readonly int[] Thresholds = { 20, 150, 400, 800, 2000 };

    /// <summary>The original's star colours, in the order it steps through them.</summary>
    private static readonly Color[] Colours =
    {
        new(138 / 255f, 152 / 255f, 222 / 255f),
        new(49 / 255f, 77 / 255f, 219 / 255f),
        new(193 / 255f, 39 / 255f, 45 / 255f),
        new(247 / 255f, 147 / 255f, 30 / 255f),
        new(1f, 1f, 0f),
    };

    /// <summary>Administrators get their own colour, ahead of every rating.</summary>
    private static readonly Color Admin = new(0f, 1f, 0f);

    /// <summary>How many stars a single character's fame is worth, from zero to five.</summary>
    public static int Stars(int fame)
    {
        int stars = 0;
        while (stars < Thresholds.Length && fame >= Thresholds[stars])
            stars++;

        return stars;
    }

    /// <summary>
    /// The colour for a star rating.
    /// </summary>
    /// <param name="stars">The account's total across all its classes.</param>
    /// <param name="classes">
    /// How many classes the game has, which is the width of each colour band. Fourteen here.
    /// </param>
    public static Color Colour(int stars, int classes, bool isAdmin = false)
    {
        if (isAdmin)
            return Admin;

        if (classes <= 0)
            return Colours[0];

        int band = Mathf.Clamp(stars / classes, 0, Colours.Length - 1);
        return Colours[band];
    }
}

/// <summary>
/// A star, drawn at whatever size it is given.
/// </summary>
/// <remarks>
/// The original composites a star graphic out of its assets onto a translucent disc. The graphic is
/// compiled artwork inside a SWF and cannot be pulled out, so the star is drawn here — five points,
/// the same disc behind it.
/// </remarks>
public partial class StarIcon : Control
{
    private readonly Color _colour;

    public StarIcon(Color colour, int size = 16)
    {
        _colour = colour;
        CustomMinimumSize = new Vector2(size, size);
        MouseFilter = MouseFilterEnum.Ignore;
    }

    public override void _Draw()
    {
        var centre = Size / 2f;
        float outer = Mathf.Min(Size.X, Size.Y) / 2f;

        DrawCircle(centre, outer, new Color(0f, 0f, 0f, 0.4f));

        // A five-pointed star is ten vertices, alternating between the outer and inner radius, the
        // first pointing straight up.
        var points = new Vector2[10];
        for (int i = 0; i < points.Length; i++)
        {
            float radius = (i % 2 == 0 ? outer * 0.86f : outer * 0.38f);
            float angle = -Mathf.Pi / 2f + i * Mathf.Pi / 5f;
            points[i] = centre + new Vector2(Mathf.Cos(angle), Mathf.Sin(angle)) * radius;
        }

        DrawColoredPolygon(points, _colour);
    }
}
