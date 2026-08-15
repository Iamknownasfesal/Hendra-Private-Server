using System;
using Godot;

namespace Hendra.UI;

/// <summary>
/// The shell every panel that opens over the world is built in.
/// </summary>
/// <remarks>
/// <para>
/// Four layers, from the outside in: a light grey outline, a mid-grey shell, a tall header band,
/// and the near-black body the occupant draws in. The outline steps inwards at each corner, so the
/// silhouette is a rectangle with a square notch bitten out of each of its four corners rather than
/// a plain box.
/// </para>
/// <para>
/// It is a shell rather than a panel: it owns the outline, the header, the title, the corner marks
/// and the two header buttons, and hands its occupant the rectangle left inside.
/// </para>
/// <para>
/// It does not dim or block the world. There is no scrim and nothing behind it captures the
/// pointer -- the player can still fight in the part of the screen the panel does not cover.
/// </para>
/// </remarks>
public partial class ModalPanel : Control
{
    /// <summary>The light outline every panel carries, on all four sides.</summary>
    public const float FrameWidth = 4f;

    /// <summary>How far the outline is held off each corner, leaving a square notch.</summary>
    private const float CornerNotch = 9f;

    /// <summary>The step that carries the outline across a notch.</summary>
    private const float CornerStep = 3f;

    public const float HeaderHeight = 76f;

    /// <summary>How much shell shows around the body, on the sides and underneath.</summary>
    public const float ShellInset = 11f;

    /// <summary>How much shell shows between the header and the top of the body.</summary>
    public const float HeaderGap = 9f;

    /// <summary>How far in from the panel's edge the body starts.</summary>
    public const float BodyInset = FrameWidth + ShellInset;

    /// <summary>The rounding on a filled body plate.</summary>
    protected const float BodyRadius = 5f;

    /// <summary>The corner marks, which are a shade of the header rather than of the outline.</summary>
    private static readonly Color OrnamentMark = new("454546");

    /// <summary>
    /// How big a panel's own name is set.
    /// </summary>
    /// <remarks>
    /// Measured off the reference rather than taken from the type scale: the title there stands
    /// twenty-two pixels from baseline to cap, and the interface's face puts a cap at half its
    /// nominal size. The scale in <c>Style</c> is a good deal smaller than the reference is at every
    /// step, which is a change to make in one place rather than eight; until it is made, the panels
    /// that have been measured say so here.
    /// </remarks>
    public const int TitleSize = 44;

    /// <summary>
    /// The corner motif, as rectangles in the top-left corner's own space.
    /// </summary>
    /// <remarks>
    /// Transcribed from the reference pixel for pixel and mirrored into the other corner, rather
    /// than drawn as two lines meeting at a right angle. It is two nested brackets and three loose
    /// squares, and what makes it read as part of the game instead of as a border is exactly that
    /// it does not close: the pieces are broken apart on the same grid the rest of the interface
    /// sits on.
    /// </remarks>
    private static readonly Rect2[] Ornament =
    {
        new(22f, 8f, 19f, 4f),
        new(37f, 8f, 4f, 14f),
        new(22f, 12f, 4f, 5f),
        new(47f, 8f, 5f, 4f),
        new(8f, 22f, 4f, 18f),
        new(13f, 22f, 4f, 4f),
        new(8f, 36f, 14f, 4f),
        new(22f, 22f, 5f, 5f),
        new(8f, 46f, 4f, 5f),
    };

    private readonly string _title;

    private Label _heading;
    private HudIconButton _close;
    private HudIconButton _info;

    public ModalPanel(string title)
    {
        _title = title;

        // The panel's own column takes the pointer; everything outside it belongs to the world.
        MouseFilter = MouseFilterEnum.Stop;
        Visible = false;
    }

    /// <summary>Raised when the panel is dismissed, by the close button or by Escape.</summary>
    public event Action Closed;

    /// <summary>What the occupant may draw in: inside the shell and under the header.</summary>
    public Control Body { get; private set; }

    /// <summary>
    /// How tall this panel's header band is. Overridden by panels whose title needs more room.
    /// </summary>
    protected virtual float Header => HeaderHeight;

    /// <summary>
    /// Whether the header carries a close cross.
    /// </summary>
    /// <remarks>
    /// The vault does without one: it is dismissed by Escape and by walking away from the thing
    /// that opened it, and a cross in the corner of a panel you cannot keep open anyway is a
    /// control that only ever repeats what the player already has.
    /// </remarks>
    protected virtual bool ShowClose => true;

    /// <summary>Whether the header carries the boxed <c>i</c> that explains the panel.</summary>
    protected virtual bool ShowInfo => false;

    /// <summary>
    /// Whether that <c>i</c> follows the title instead of sitting in the corner.
    /// </summary>
    /// <remarks>
    /// Two placements because the reference has two: a wide panel puts it against the right edge,
    /// where there is room for it to be its own thing, and a narrow one hangs it off the end of the
    /// title so the pair still reads as centred.
    /// </remarks>
    protected virtual bool InfoBesideTitle => false;

    /// <summary>
    /// Whether the corners carry the bracket motif.
    /// </summary>
    /// <remarks>
    /// Decoration, and off by default so that existing panels look as they did.
    /// </remarks>
    protected virtual bool ShowOrnaments => false;

    /// <summary>
    /// Whether the shell fills the body's rectangle with the near-black plate.
    /// </summary>
    /// <remarks>
    /// True for a panel whose contents are one block. The vault turns it off because its body is
    /// two plates with the shell showing between them -- the rail down the left is its own
    /// rectangle and ends where the rail ends, rather than running the height of the grid.
    /// </remarks>
    protected virtual bool FillBody => true;

    public override void _Ready()
    {
        _heading = new Label
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
            Text = _title,
        }.Typeset(TitleSize, Style.Text, bold: true);
        AddChild(_heading);

        if (ShowInfo)
        {
            _info = new HudIconButton(InfoMark, "What this is", inset: 0f) { Tint = Style.Text };
            AddChild(_info);
        }

        if (ShowClose)
        {
            _close = new HudIconButton(Cross, "Close [Esc]", inset: 6f);
            _close.Tint = Style.ModalFrame;
            _close.Pressed += Close;
            AddChild(_close);
        }

        Body = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(Body);

        Resized += Fit;
        Fit();
    }

    private static void Cross(CanvasItem into, Rect2 box, Color colour)
    {
        into.DrawLine(box.Position, box.End, colour, 2f);
        into.DrawLine(new Vector2(box.End.X, box.Position.Y), new Vector2(box.Position.X, box.End.Y), colour, 2f);
    }

    /// <summary>The boxed lower-case <c>i</c>: a hollow square with a dot and a stem in it.</summary>
    private static void InfoMark(CanvasItem into, Rect2 box, Color colour)
    {
        float side = Mathf.Floor(Mathf.Min(box.Size.X, box.Size.Y) / 9f) * 9f;
        if (side < 9f)
            side = 9f;

        float unit = side / 9f;
        var origin = box.Position + (box.Size - new Vector2(side, side)) / 2f;

        Rect2 Cell(float x, float y, float w, float h) =>
            new(origin + new Vector2(x, y) * unit, new Vector2(w, h) * unit);

        into.DrawRect(Cell(0f, 0f, 9f, 1f), colour);
        into.DrawRect(Cell(0f, 8f, 9f, 1f), colour);
        into.DrawRect(Cell(0f, 1f, 1f, 7f), colour);
        into.DrawRect(Cell(8f, 1f, 1f, 7f), colour);

        into.DrawRect(Cell(4f, 2f, 1f, 1f), colour);
        into.DrawRect(Cell(4f, 4f, 1f, 3f), colour);
    }

    /// <summary>Places the header and hands the rest to the body.</summary>
    private void Fit()
    {
        if (Body == null)
            return;

        _heading.Position = new Vector2(FrameWidth, FrameWidth);
        _heading.Size = new Vector2(Size.X - FrameWidth * 2f, Header);

        if (_info != null)
        {
            float side = Mathf.Round(Header * 0.47f);
            float top = Mathf.Round(FrameWidth + (Header - side) / 2f);

            // Beside the title, the pair has to be centred together: the label is already centred
            // across the whole header, so the mark takes its place from the measured text.
            float left = InfoBesideTitle
                ? Mathf.Round((Size.X + Style.Measure(_title, TitleSize, bold: true)) / 2f) + 10f
                : Size.X - BodyInset - side - 39f;

            _info.Position = new Vector2(left, top);
            _info.Size = new Vector2(side, side);
        }

        if (_close != null)
        {
            _close.Position = new Vector2(Size.X - FrameWidth - 12f - 20f, FrameWidth + 12f);
            _close.Size = new Vector2(20f, 20f);
        }

        Body.Position = new Vector2(BodyInset, FrameWidth + Header + HeaderGap);
        Body.Size = new Vector2(
            Size.X - BodyInset * 2f,
            Mathf.Max(0f, Size.Y - Body.Position.Y - BodyInset));

        QueueRedraw();
    }

    public void Open()
    {
        Visible = true;
        QueueRedraw();
    }

    public void Close()
    {
        if (!Visible)
            return;

        Visible = false;
        Closed?.Invoke();
    }

    public void Toggle()
    {
        if (Visible)
            Close();
        else
            Open();
    }

    /// <summary>
    /// Escape closes it, and nothing else here touches the keyboard.
    /// </summary>
    /// <remarks>
    /// Handled as unhandled input rather than as a shortcut, so gameplay keys stay live while the
    /// panel is open -- the brief is explicit that the player can keep fighting beside it.
    /// </remarks>
    public override void _UnhandledKeyInput(InputEvent @event)
    {
        if (Visible && @event is InputEventKey { Pressed: true, Keycode: Key.Escape })
        {
            Close();
            GetViewport().SetInputAsHandled();
        }
    }

    public override void _Draw()
    {
        float w = Size.X;
        float h = Size.Y;

        // The silhouette: two overlapping rectangles, which leaves a square notch in each corner.
        DrawRect(new Rect2(0f, CornerNotch, w, h - CornerNotch * 2f), Style.ModalBand);
        DrawRect(new Rect2(CornerNotch, 0f, w - CornerNotch * 2f, h), Style.ModalBand);

        DrawOutline(w, h);

        // The header follows the notch too: it is held off the corners for as long as the outline is.
        DrawRect(new Rect2(CornerNotch + CornerStep, FrameWidth,
            w - (CornerNotch + CornerStep) * 2f, CornerNotch - FrameWidth), Style.ModalHeader);
        DrawRect(new Rect2(FrameWidth, CornerNotch, w - FrameWidth * 2f,
            Header + FrameWidth - CornerNotch), Style.ModalHeader);

        if (ShowOrnaments)
            DrawOrnaments(w);

        if (FillBody && Body != null)
            DrawBodyPlate(new Rect2(Body.Position, Body.Size));
    }

    /// <summary>A near-black plate with soft corners, which is what the body of a panel is.</summary>
    protected void DrawBodyPlate(in Rect2 box)
    {
        var plate = new StyleBoxFlat { BgColor = Style.ModalBody };
        plate.SetCornerRadiusAll((int)BodyRadius);
        DrawStyleBox(plate, box);
    }

    /// <summary>The light edge, following the notched silhouette all the way round.</summary>
    private void DrawOutline(float w, float h)
    {
        var colour = Style.ModalFrame;
        float span = w - CornerNotch * 2f;
        float rise = h - CornerNotch * 2f;

        DrawRect(new Rect2(CornerNotch, 0f, span, FrameWidth), colour);
        DrawRect(new Rect2(CornerNotch, h - FrameWidth, span, FrameWidth), colour);
        DrawRect(new Rect2(0f, CornerNotch, FrameWidth, rise), colour);
        DrawRect(new Rect2(w - FrameWidth, CornerNotch, FrameWidth, rise), colour);

        // Each corner is bridged by a two-piece step, so the edge turns without a gap in it.
        for (int corner = 0; corner < 4; corner++)
        {
            bool right = corner is 1 or 2;
            bool bottom = corner is 2 or 3;

            float upright = right ? w - CornerNotch - CornerStep : CornerNotch;
            float across = bottom ? h - CornerNotch - CornerStep : CornerNotch;

            DrawRect(new Rect2(upright, bottom ? h - CornerNotch : FrameWidth,
                CornerStep, CornerNotch - FrameWidth), colour);
            DrawRect(new Rect2(right ? w - CornerNotch : FrameWidth, across,
                CornerNotch - FrameWidth, CornerStep), colour);
        }
    }

    /// <summary>The bracket motif, in the two top corners, mirrored across the panel's middle.</summary>
    private void DrawOrnaments(float w)
    {
        foreach (var mark in Ornament)
        {
            DrawRect(mark, OrnamentMark);
            DrawRect(new Rect2(w - mark.Position.X - mark.Size.X, mark.Position.Y, mark.Size.X, mark.Size.Y),
                OrnamentMark);
        }
    }
}
