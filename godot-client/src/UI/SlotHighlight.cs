using Godot;

namespace Hendra.UI;

/// <summary>
/// What a slot's plate is saying about the item on it, beyond what the artwork says.
/// </summary>
/// <remarks>
/// <para>
/// A channel rather than a flag: a slot asks for a highlight by name and gets a fill and a border
/// back. Adding a second meaning later is a member here and a case in <see cref="Pair"/>, and
/// nothing that draws a slot has to learn about it.
/// </para>
/// <para>
/// One meaning is wired at present -- red for an item this character cannot equip -- and it is the
/// same red the hotbar has always used for the same thing, resolved from here rather than from a
/// second constant that merely matched.
/// </para>
/// </remarks>
public enum SlotHighlight
{
    /// <summary>The ordinary plate. Almost every slot, almost always.</summary>
    None,

    /// <summary>This character cannot equip what is on the slot.</summary>
    Red,
}

public static class SlotHighlights
{
    /// <summary>
    /// The plate under an item this character cannot use.
    /// </summary>
    /// <remarks>
    /// The tile is the signal, not a ring around it. The first pass put a near-black red under a
    /// bright red border, so what you saw was the border -- one loud outline per unusable item,
    /// and a bag with twenty of them in it was a screen of red rings. Lifting the fill until it is
    /// plainly red and calming the border down puts the meaning on the whole square, which is
    /// where it belongs: the item is the thing that is wrong, not its edge.
    ///
    /// Still twenty-one points of luminance clear of the board it sits on, which is the ladder's
    /// rule and the reason this is not darker still.
    /// </remarks>
    public static readonly Color RedFill = new("5f1e1e");

    public static readonly Color RedEdge = new("8f2222");

    /// <summary>
    /// The fill and border a highlight asks for, or the neutral pair for none.
    /// </summary>
    /// <remarks>
    /// The neutral pair is two plates, not one. An empty slot is the lighter of them: there is
    /// nothing on it, so the plate itself is what you are looking at and it should sit clearly off
    /// the board. A slot with an item goes a step darker, because now the artwork is the bright
    /// thing and the plate is behind it. Both are far enough from the near-black board to survive
    /// being desaturated, which is the test the value ladder exists to pass.
    /// </remarks>
    public static (Color Fill, Color Edge) Pair(SlotHighlight highlight, bool occupied) =>
        highlight switch
        {
            SlotHighlight.Red => (RedFill, RedEdge),
            _ => occupied ? (Style.Slot, Style.SlotBorder) : (Style.SlotEmpty, Style.SlotEmptyEdge),
        };
}
