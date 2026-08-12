using Godot;

namespace Hendra.UI;

/// <summary>
/// The interface's scrollbar: a track with an arrow at each end and a draggable thumb.
/// </summary>
/// <remarks>
/// <para>
/// Geometry and drawing only -- it holds no scroll position of its own. The two things that use one
/// keep their offset for their own reasons (the chat sticks to the bottom, the character panel
/// holds its place across a tab switch), and a scrollbar that owned the number would have to be
/// told about both.
/// </para>
/// <para>
/// It sits along the right edge of whatever rectangle it is handed, which is the only arrangement
/// either caller wants.
/// </para>
/// </remarks>
public sealed class HudScrollbar
{
    public const float Width = 14f;

    /// <summary>The arrow buttons at the ends of the track.</summary>
    public const float ArrowHeight = 12f;

    /// <summary>The shortest the thumb is allowed to get, however long the content is.</summary>
    private const float MinimumThumb = 20f;

    /// <summary>What part of the bar a point is over.</summary>
    public enum Part
    {
        None,
        Up,
        Down,
        Thumb,
        TrackAbove,
        TrackBelow,
    }

    /// <summary>How far the content can be scrolled, given how much of it there is.</summary>
    public static float MaxOffset(Vector2 size, float content) => Mathf.Max(0f, content - size.Y);

    public static Rect2 Track(Vector2 size) => new(
        size.X - Width, ArrowHeight, Width, Mathf.Max(0f, size.Y - ArrowHeight * 2f));

    public static Rect2 Thumb(Vector2 size, float offset, float content)
    {
        var track = Track(size);

        float shown = Mathf.Max(content, size.Y);
        float height = Mathf.Max(MinimumThumb, track.Size.Y * size.Y / shown);
        float travel = track.Size.Y - height;

        float max = MaxOffset(size, content);
        float fraction = max <= 0f ? 0f : Mathf.Clamp(offset / max, 0f, 1f);

        return new Rect2(
            track.Position.X + 2f, Mathf.Round(track.Position.Y + travel * fraction),
            Width - 4f, Mathf.Round(height));
    }

    /// <summary>What a point in the owner's coordinates is over.</summary>
    public static Part Test(Vector2 size, float offset, float content, Vector2 at)
    {
        if (at.X < size.X - Width)
            return Part.None;

        if (at.Y <= ArrowHeight)
            return Part.Up;

        if (at.Y >= size.Y - ArrowHeight)
            return Part.Down;

        var thumb = Thumb(size, offset, content);
        if (thumb.HasPoint(at))
            return Part.Thumb;

        return at.Y < thumb.Position.Y ? Part.TrackAbove : Part.TrackBelow;
    }

    /// <summary>Where a thumb dragged to this position puts the content.</summary>
    public static float OffsetForThumbTop(Vector2 size, float content, float thumbTop)
    {
        var track = Track(size);
        float travel = track.Size.Y - Thumb(size, 0f, content).Size.Y;

        if (travel <= 0f)
            return 0f;

        return Mathf.Clamp((thumbTop - track.Position.Y) / travel, 0f, 1f) * MaxOffset(size, content);
    }

    public static void Draw(CanvasItem into, Vector2 size, float offset, float content, bool dragging)
    {
        into.DrawRect(new Rect2(size.X - Width, 0f, Width, size.Y), Style.PanelInset);

        HudIcons.Chevron(into,
            new Rect2(size.X - Width + 3f, 2f, Width - 6f, ArrowHeight - 4f), Style.TextDim, up: true);

        HudIcons.Chevron(into,
            new Rect2(size.X - Width + 3f, size.Y - ArrowHeight + 2f, Width - 6f, ArrowHeight - 4f),
            Style.TextDim, up: false);

        if (Track(size).Size.Y <= 0f)
            return;

        var thumb = Thumb(size, offset, content);
        into.DrawRect(thumb, dragging ? Style.SlotBorderHi : Style.ButtonFace);
        into.DrawRect(thumb, Style.PanelEdge, filled: false, width: 1f);
    }
}
