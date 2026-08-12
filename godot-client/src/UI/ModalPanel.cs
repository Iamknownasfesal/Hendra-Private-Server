using System;
using Godot;

namespace Hendra.UI;

/// <summary>
/// The shell every panel that opens over the world is built in.
/// </summary>
/// <remarks>
/// <para>
/// A gold frame around a near-black body, which is what marks secondary interface apart from the
/// flat grey chrome that lives on the screen permanently. It is a shell rather than a panel: it
/// owns the frame, the header, the title and the close affordance, and hands its occupant the
/// rectangle left inside.
/// </para>
/// <para>
/// It does not dim or block the world. There is no scrim and nothing behind it captures the
/// pointer -- the player can still fight in the two thirds of the screen the panel does not cover,
/// which is the whole reason it is docked to a column instead of centred.
/// </para>
/// </remarks>
public partial class ModalPanel : Control
{
    /// <summary>The frame: three pixels of gold with a dark line either side of it.</summary>
    public const float FrameWidth = 3f;

    public const float HeaderHeight = 44f;

    private readonly string _title;

    private Label _heading;
    private HudIconButton _close;

    public ModalPanel(string title)
    {
        _title = title;

        // The panel's own column takes the pointer; everything outside it belongs to the world.
        MouseFilter = MouseFilterEnum.Stop;
        Visible = false;
    }

    /// <summary>Raised when the panel is dismissed, by the close button or by Escape.</summary>
    public event Action Closed;

    /// <summary>What the occupant may draw in: inside the frame and under the header.</summary>
    public Control Body { get; private set; }

    public override void _Ready()
    {
        _heading = new Label
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
            Text = _title,
        }.Typeset(20, Style.Text);
        AddChild(_heading);

        _close = new HudIconButton(Cross, "Close [Esc]", inset: 6f);
        _close.Tint = Style.ModalFrame;
        _close.Pressed += Close;
        AddChild(_close);

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

    /// <summary>Places the header and hands the rest to the body.</summary>
    private void Fit()
    {
        if (Body == null)
            return;

        float inset = FrameWidth + 1f;

        _heading.Position = new Vector2(inset, inset);
        _heading.Size = new Vector2(Size.X - inset * 2f, HeaderHeight);

        _close.Position = new Vector2(Size.X - inset - 12f - 20f, inset + 12f);
        _close.Size = new Vector2(20f, 20f);

        Body.Position = new Vector2(inset, inset + HeaderHeight);
        Body.Size = new Vector2(Size.X - inset * 2f, Mathf.Max(0f, Size.Y - inset * 2f - HeaderHeight));

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
        var full = new Rect2(Vector2.Zero, Size);

        // Outer dark line, gold frame, inner dark line, then the body.
        DrawRect(full, Style.ModalFrameDark);
        DrawRect(full.Grow(-1f), Style.ModalFrame);
        DrawRect(full.Grow(-1f - FrameWidth), Style.ModalFrameDark);

        var inside = full.Grow(-(FrameWidth + 1f));
        DrawRect(inside, Style.ModalBody);

        // The header sits on its own darker band with a rule under it.
        DrawRect(new Rect2(inside.Position, new Vector2(inside.Size.X, HeaderHeight)), Style.ModalHeader);
        DrawRect(new Rect2(inside.Position.X, inside.Position.Y + HeaderHeight, inside.Size.X, 1f),
            Style.ModalFrameDark);
    }
}
