using Godot;

namespace Hendra.UI;

/// <summary>
/// A large clickable plate that holds its own contents.
/// </summary>
/// <remarks>
/// <para>
/// What a character box and a class tile are: a picture and some words you click, rather than a
/// word in a box. <see cref="GameButton"/> draws its own label because it has only a label; this
/// draws the plate and lets whatever is parented to it sit on top, because children are drawn after
/// their parent and the built-in stylebox is cleared out of the way.
/// </para>
/// <para>
/// It is a slot, deliberately. The character list is a grid of things you pick between, which is
/// what the inventory is, so it borrows the inventory's plate and two-pixel border wholesale — see
/// <see cref="SlotView"/>. The accent survives as a stripe down the left edge, which is the one
/// thing a slot has no equivalent for and the character boxes need: a mark that says which row you
/// are on without ringing the whole square.
/// </para>
/// </remarks>
public partial class CardButton : Button
{
    /// <summary>Matches a slot's border, for the same reason: one pixel disappears.</summary>
    private const float Border = 2f;

    private readonly Color _accent;
    private float _glow;

    public CardButton(Color accent)
    {
        _accent = accent;

        Text = string.Empty;
        FocusMode = FocusModeEnum.None;

        foreach (string state in new[] { "normal", "hover", "pressed", "disabled", "focus" })
            AddThemeStyleboxOverride(state, new StyleBoxEmpty());
    }

    public override void _Process(double delta)
    {
        float target = IsHovered() && !Disabled ? 1f : 0f;
        float eased = Mathf.MoveToward(_glow, target, (float)delta * 8f);

        if (Mathf.IsEqualApprox(eased, _glow))
            return;

        _glow = eased;
        QueueRedraw();
    }

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);

        // The occupied slot's plate, lifted a little under the pointer -- the same twelve per cent
        // a hovered inventory square takes.
        DrawRect(full, Style.Slot.Lightened(_glow * 0.12f));

        // Drawn inside the bounds rather than centred on them, or half of every border would fall
        // into the gutter and the column would sit a pixel off.
        DrawRect(full.Grow(-Border / 2f), _glow > 0.5f ? Style.SlotBorderHi : Style.SlotBorder,
            filled: false, width: Border);

        // The accent stripe, inset so it reads as a marker on the row rather than as a second
        // border fighting the first.
        DrawRect(new Rect2(Border, Border + 2f, 3f, Size.Y - (Border + 2f) * 2f),
            _accent with { A = 0.45f + _glow * 0.55f });
    }
}
