namespace Hendra.World;

/// <summary>Which set of slots an address refers to.</summary>
public enum SlotOwner
{
    /// <summary>The player's own: equipment, inventory and backpack, in one numbering.</summary>
    Player,

    /// <summary>Whatever container is open at the player's feet.</summary>
    Container,

    /// <summary>
    /// The vault, by flat index across every chest the account owns.
    /// </summary>
    /// <remarks>
    /// The index is a display convenience and never leaves the client in this form: chest and slot
    /// are worked out from it on the way to the wire, because what is stored is still one array of
    /// eight per chest.
    /// </remarks>
    Vault,

    /// <summary>
    /// A gift waiting to be claimed, indexed in its own dense list.
    /// </summary>
    /// <remarks>
    /// Separate from <see cref="Vault"/> because it is not storage: it accepts nothing, holds no
    /// empty slots, and claiming one removes it from the account rather than moving it.
    /// </remarks>
    VaultGift,
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
