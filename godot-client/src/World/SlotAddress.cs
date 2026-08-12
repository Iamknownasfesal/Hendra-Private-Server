namespace Hendra.World;

/// <summary>Which set of slots an address refers to.</summary>
public enum SlotOwner
{
    /// <summary>The player's own: equipment, inventory and backpack, in one numbering.</summary>
    Player,

    /// <summary>Whatever container is open at the player's feet.</summary>
    Container,
}

/// <summary>
/// One slot, anywhere the player can move an item to or from.
/// </summary>
/// <remarks>
/// Dragging needs to name a source and a destination that may belong to different things, and the
/// wire already works this way: an InvSwap carries two slots, each tagged with the object that owns
/// it. This is that pair of fields with a name.
/// </remarks>
public readonly struct SlotAddress
{
    public readonly SlotOwner Owner;
    public readonly int Index;

    public SlotAddress(SlotOwner owner, int index)
    {
        Owner = owner;
        Index = index;
    }

    public bool Equals(SlotAddress other) => Owner == other.Owner && Index == other.Index;
}
