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
    public static readonly Color RedFill = new("4a1414");
    public static readonly Color RedEdge = new("c02020");

    /// <summary>The fill and border a highlight asks for, or the neutral pair for none.</summary>
    public static (Color Fill, Color Edge) Pair(SlotHighlight highlight) => highlight switch
    {
        SlotHighlight.Red => (RedFill, RedEdge),
        _ => (Style.Slot, Style.SlotBorder),
    };
}
