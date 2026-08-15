using System;
using System.Globalization;
using Godot;

namespace Hendra.UI;

/// <summary>
/// The colours the top-left cluster is drawn in that the shared palette does not name.
/// </summary>
/// <remarks>
/// Measured off <c>references/Fullscreen/Interact fs.png</c>, which puts the same cluster over a
/// flat brick floor and so lets a translucent plate be solved for rather than guessed at.
/// </remarks>
internal static class CardInk
{
    /// <summary>
    /// The plate behind the class portrait.
    /// </summary>
    /// <remarks>
    /// Solved from two floor tiles of known colour under it: a grey at seventy-one percent, not a
    /// black at forty. The difference matters -- black darkens whatever is behind it and this
    /// lifts it, which is why the portrait reads the same over water and over brick.
    /// </remarks>
    public static readonly Color Plate = new(86f / 255f, 86f / 255f, 86f / 255f, 0.71f);

    /// <summary>The experience track, which is the same grey as the rule under the map.</summary>
    public static readonly Color XpTrack = new("7b7c7d");

    /// <summary>How long is left of something, in the one orange the interface uses for a clock.</summary>
    public static readonly Color Countdown = new("ffaa00");

    /// <summary>The quest panel's frame, which runs warm from its top edge to its bottom.</summary>
    public static readonly Color FrameTop = new("fed854");

    public static readonly Color FrameBottom = new("ffe693");

    /// <summary>The panel's own body, over whatever the player is standing on.</summary>
    public static readonly Color Body = new(0f, 0f, 0f, 0.72f);

    /// <summary>A quest bar's fill, which is nearly white and lightens along its length.</summary>
    public static readonly Color BarLow = new("f4ecd8");

    public static readonly Color BarHigh = new("ffffff");

    /// <summary>The plate a small count sits on: the objective's tally and the level badge.</summary>
    public static readonly Color Badge = new("545454");

    /// <summary>A guild's name, and the shield beside it.</summary>
    public static readonly Color Guild = new("62d030");

    /// <summary>The star beside the rating.</summary>
    public static readonly Color RatingStar = new("2d54c8");
}

/// <summary>
/// The class portrait, on the one plate the top-left cluster carries.
/// </summary>
/// <remarks>
/// Everything else up here is outlined text drawn straight onto the world -- there is no card
/// behind it in the reference, and putting one there is the single fastest way to look like a
/// different game. The portrait is the exception because a sprite has no outline of its own to
/// hold it off the floor.
/// </remarks>
public sealed partial class CardPortrait : Control
{
    private Assets.Sprite _sprite;

    public CardPortrait()
    {
        MouseFilter = MouseFilterEnum.Ignore;
    }

    public void Set(Assets.Sprite sprite)
    {
        if (_sprite.Sheet == sprite.Sheet && _sprite.Region == sprite.Region)
            return;

        _sprite = sprite;
        QueueRedraw();
    }

    public override void _Draw()
    {
        DrawRect(new Rect2(Vector2.Zero, Size), CardInk.Plate);

        if (!_sprite.IsValid)
            return;

        // A whole multiple of the source, so an eight-pixel sprite lands on whole pixels at any
        // interface scale. See SlotView.Artwork for the same rule and the reason behind it.
        int source = Mathf.Min(_sprite.Region.Size.X, _sprite.Region.Size.Y);
        float side = source > 0
            ? Mathf.Max(source, Mathf.Floor(Size.Y * 0.92f / source) * source)
            : Size.Y;

        this.DrawSprite(_sprite,
            new Rect2(Mathf.Round((Size.X - side) / 2f), Mathf.Round((Size.Y - side) / 2f), side, side));
    }
}

/// <summary>
/// The line under the player's name: the clock, and how far through the level they are.
/// </summary>
/// <remarks>
/// The time is in UTC because that is the clock every timed thing in this game is quoted against.
/// The badge beside it carries the character's level, which is the one number the reference keeps
/// off this line and which nothing else in the permanent interface says.
/// </remarks>
public sealed partial class ClockLine : Control
{
    private const int TimeSize = 30;
    private const int BadgeSize = 24;

    /// <summary>The badge's plate, which is as wide as two digits and their padding.</summary>
    private static readonly Vector2 BadgeBox = new(42f, 33f);

    private string _time = string.Empty;
    private string _level = string.Empty;

    public ClockLine()
    {
        MouseFilter = MouseFilterEnum.Ignore;
    }

    /// <summary>
    /// Says whether there is a character to caption. The clock reads itself.
    /// </summary>
    /// <remarks>
    /// The badge reads <c>XP</c>, not the level. It captions the rule under it -- that rule is the
    /// experience track -- and the reference draws exactly those two letters there whatever level
    /// the character is. The level itself is on the attributes sheet and on the character rows.
    /// </remarks>
    public void Set(int level)
    {
        string text = level > 0 ? "XP" : string.Empty;
        if (_level == text)
            return;

        _level = text;
        QueueRedraw();
    }

    public override void _Process(double delta)
    {
        string now = DateTime.UtcNow.ToString("HH:mm:ss", CultureInfo.InvariantCulture) + " UTC";
        if (_time == now)
            return;

        _time = now;
        QueueRedraw();
    }

    public override void _Draw()
    {
        float middle = Size.Y / 2f;

        // The clock face: a ring with two hands, drawn rather than written, because a glyph is
        // what says "this is a time" before the digits beside it have been read.
        var centre = new Vector2(14f, middle);
        DrawArc(centre, 12f, 0f, Mathf.Tau, 24, Style.Text, 3f);
        DrawLine(centre, centre + new Vector2(0f, -7f), Style.Text, 3f);
        DrawLine(centre, centre + new Vector2(5f, 0f), Style.Text, 3f);

        this.DrawOverWorld(
            new Vector2(32f, Style.BaselineIn(Size.Y, TimeSize)), _time, TimeSize, Style.Text);

        if (_level.Length == 0)
            return;

        var plate = new Rect2(
            Mathf.Round(32f + Style.Measure(_time, TimeSize) + 12f),
            Mathf.Round(middle - BadgeBox.Y / 2f),
            BadgeBox.X,
            BadgeBox.Y);

        DrawRect(plate, CardInk.Badge);

        this.DrawText(
            new Vector2(
                plate.Position.X + Mathf.Round((plate.Size.X - Style.Measure(_level, BadgeSize)) / 2f),
                plate.Position.Y + Style.BaselineIn(plate.Size.Y, BadgeSize)),
            _level, BadgeSize, Style.StatNumber);
    }
}

/// <summary>
/// The experience bar: a four-pixel rule across the top-left corner with a marker on it.
/// </summary>
/// <remarks>
/// Four pixels and no frame, which is the whole of it in the reference. It is a thing you notice
/// filling out of the corner of your eye rather than a gauge you read, and giving it a border and
/// a number would make it compete with the three bars in the column that do have to be read.
/// </remarks>
public sealed partial class XpBar : Control
{
    private float _fraction;

    public XpBar()
    {
        MouseFilter = MouseFilterEnum.Ignore;
    }

    public void Set(float fraction)
    {
        fraction = Mathf.Clamp(fraction, 0f, 1f);
        if (Mathf.IsEqualApprox(_fraction, fraction))
            return;

        _fraction = fraction;
        QueueRedraw();
    }

    public override void _Draw()
    {
        DrawRect(new Rect2(Vector2.Zero, Size), CardInk.XpTrack);

        float width = Mathf.Round(Size.X * _fraction);
        if (width >= 1f)
            DrawRect(new Rect2(0f, 0f, width, Size.Y), Style.XpFillHigh);

        // The marker rides the left end of the track, pointing at it: a small gold chevron with a
        // shoulder, which is what the reference puts there.
        var tip = new Vector2(-16f, Size.Y);
        DrawColoredPolygon(
            new[]
            {
                tip + new Vector2(-12f, -12f),
                tip + new Vector2(12f, -12f),
                tip + new Vector2(12f, -6f),
                tip + new Vector2(0f, 2f),
                tip + new Vector2(-12f, -6f),
            },
            Style.FameFillHigh);
    }
}

/// <summary>
/// The objective tracker under the card: a heading, and a framed panel with the objective in it.
/// </summary>
/// <remarks>
/// Two tiers, as the reference has them. The heading is the longer-running thing and carries its
/// own countdown; the panel is the objective you are working on now, with a bar and a tally. Both
/// are drawn rather than built out of labels, because the panel is one shape -- a gradient frame
/// around a translucent body -- and a StyleBox cannot grade a border.
/// </remarks>
public sealed partial class QuestTracker : Control
{
    private const int HeadingSize = 30;
    private const int TitleSize = 30;
    private const int BarTextSize = 28;
    private const int TallySize = 28;

    /// <summary>The frame's thickness, and the gap between the heading and the panel under it.</summary>
    private const float Frame = 4f;

    private const float HeadingHeight = 23f;

    private string _heading = string.Empty;
    private string _headingNote = string.Empty;
    private string _title = string.Empty;
    private string _note = string.Empty;
    private string _progress = string.Empty;
    private string _tally = string.Empty;

    private float _fraction;

    public QuestTracker()
    {
        MouseFilter = MouseFilterEnum.Ignore;
        Visible = false;
    }

    /// <summary>
    /// Sets both tiers at once. An empty title hides the whole tracker.
    /// </summary>
    /// <param name="heading">The longer-running thing, over the panel.</param>
    /// <param name="headingNote">What is left of it, in the countdown orange.</param>
    /// <param name="title">The objective itself.</param>
    /// <param name="note">What is left of the objective.</param>
    /// <param name="progress">The figure written on the bar.</param>
    /// <param name="fraction">How far along the bar is.</param>
    /// <param name="tally">The count in the box at the bar's end.</param>
    public void Set(
        string heading, string headingNote, string title, string note,
        string progress, float fraction, string tally)
    {
        _fraction = Mathf.Clamp(fraction, 0f, 1f);

        Visible = !string.IsNullOrEmpty(title);

        if (_heading == heading && _headingNote == headingNote && _title == title
            && _note == note && _progress == progress && _tally == tally)
        {
            return;
        }

        _heading = heading ?? string.Empty;
        _headingNote = headingNote ?? string.Empty;
        _title = title ?? string.Empty;
        _note = note ?? string.Empty;
        _progress = progress ?? string.Empty;
        _tally = tally ?? string.Empty;
        QueueRedraw();
    }

    public override void _Draw()
    {
        // The heading, over the world with nothing behind it.
        if (_heading.Length > 0)
        {
            this.DrawOverWorld(new Vector2(15f, HeadingHeight), _heading, HeadingSize, Style.Text);

            this.DrawOverWorld(
                new Vector2(Size.X - 17f - Style.Measure(_headingNote, HeadingSize), HeadingHeight),
                _headingNote, HeadingSize, CardInk.Countdown);
        }

        var panel = new Rect2(0f, HeadingHeight + 5f, Size.X, Size.Y - HeadingHeight - 5f);

        DrawRect(panel, CardInk.Body);
        DrawFrame(panel);

        float titleBaseline = panel.Position.Y + 30f;
        this.DrawOverWorld(new Vector2(panel.Position.X + 15f, titleBaseline), _title, TitleSize, Style.Text);

        this.DrawOverWorld(
            new Vector2(panel.End.X - 17f - Style.Measure(_note, TitleSize), titleBaseline),
            _note, TitleSize, CardInk.Countdown);

        DrawObjective(panel);
    }

    /// <summary>The frame, graded from its top edge to its bottom in one-pixel steps.</summary>
    private void DrawFrame(in Rect2 panel)
    {
        for (int i = 0; i < Frame; i++)
        {
            var ring = panel.Grow(-i);
            float t = i / Frame;

            DrawRect(ring, CardInk.FrameTop.Lerp(CardInk.FrameBottom, t), filled: false, width: 1f);
        }
    }

    /// <summary>The bar and the tally box at the end of it.</summary>
    private void DrawObjective(in Rect2 panel)
    {
        const float Tally = 43f;

        var row = new Rect2(
            panel.Position.X + 7f,
            panel.Position.Y + 39f,
            panel.Size.X - 14f,
            panel.Size.Y - 47f);

        var bar = new Rect2(row.Position, new Vector2(row.Size.X - Tally - 4f, row.Size.Y));

        // The empty part is the panel's own body showing through, so an objective barely started
        // reads as a bar with a little in it rather than as a second plate.
        DrawRect(bar, Style.PanelInset);

        float width = Mathf.Round(bar.Size.X * _fraction);
        if (width >= 1f)
        {
            // Lightening along its length, which is what stops a near-white bar reading as flat.
            for (int x = 0; x < width; x++)
            {
                DrawRect(new Rect2(bar.Position.X + x, bar.Position.Y, 1f, bar.Size.Y),
                    CardInk.BarLow.Lerp(CardInk.BarHigh, x / Mathf.Max(1f, width - 1f)));
            }
        }

        // Dark while the fill is under it, light once the bar has emptied out from behind it. The
        // reference only ever shows the first case because its objectives start part-done, and a
        // near-black figure on a near-black trough is invisible exactly when the bar says least.
        var at = new Vector2(
            bar.Position.X + 8f, bar.Position.Y + Style.BaselineIn(bar.Size.Y, BarTextSize));

        if (width >= 8f + Style.Measure(_progress, BarTextSize))
            this.DrawText(at, _progress, BarTextSize, Style.PanelEdge);
        else
            this.DrawOverWorld(at, _progress, BarTextSize, Style.TextDim);

        var box = new Rect2(bar.End.X + 4f, row.Position.Y, Tally, row.Size.Y);
        DrawRect(box, CardInk.Badge);

        this.DrawText(
            new Vector2(
                box.Position.X + Mathf.Round((box.Size.X - Style.Measure(_tally, TallySize)) / 2f),
                box.Position.Y + Style.BaselineIn(box.Size.Y, TallySize)),
            _tally, TallySize, Style.Text);
    }
}
