namespace Hendra.Net.Packets;

/// <summary>
/// Moves an item within the vault, or between the vault and the player's own inventory.
/// </summary>
/// <remarks>
/// <para>
/// Both ends are named the same way -- a chest and a slot -- because the operation is a swap and is
/// symmetric. A chest of <see cref="PlayerChest"/> means the slot is one of the player's own
/// twenty-four rather than a chest's eight.
/// </para>
/// <para>
/// The version is the vault as this client last saw it. The server refuses a move quoting a version
/// it has moved past and answers with the truth, which is how two clients on one account are kept
/// from both moving the same stack.
/// </para>
/// </remarks>
public sealed class VaultMovePacket : ClientPacket
{
    /// <summary>The chest number meaning "the player's own inventory".</summary>
    public const short PlayerChest = -1;

    /// <summary>
    /// The chest number meaning the gifts waiting to be claimed.
    /// </summary>
    /// <remarks>
    /// A source and never a destination: a gift is claimed by moving it out, and moving it out is
    /// what removes it from the account.
    /// </remarks>
    public const short GiftChest = -2;

    public override PacketId Id => PacketId.VaultMove;

    public int Version;
    public short FromChest;
    public short FromSlot;
    public short ToChest;
    public short ToSlot;

    public override void Write(NetWriter w)
    {
        w.Write(Version);
        w.Write(FromChest);
        w.Write(FromSlot);
        w.Write(ToChest);
        w.Write(ToSlot);
    }
}

/// <summary>Buys one more chest. Carries the count we believe we own, so a double click buys one.</summary>
public sealed class VaultBuyPacket : ClientPacket
{
    public override PacketId Id => PacketId.VaultBuy;

    public int ChestCount;

    public override void Write(NetWriter w) => w.Write(ChestCount);
}

/// <summary>
/// The whole vault, as the server has it.
/// </summary>
/// <remarks>
/// Sent whole rather than as a delta: on arrival in the vault, after every accepted move, and to
/// the loser of a race. A client told everything cannot drift out of step with the server, and
/// drifting out of step is what duplicates items.
/// </remarks>
public sealed class VaultUpdatePacket : ServerPacket
{
    public override PacketId Id => PacketId.VaultUpdate;

    public int Version;
    public int ChestCount;
    public int MaxChests;
    public int NextChestPrice;

    /// <summary>Item types, eight per chest, flat. 0xffff is an empty slot.</summary>
    public ushort[] Slots = System.Array.Empty<ushort>();

    /// <summary>Gifts waiting to be claimed. Dense, and one way out of the panel.</summary>
    public ushort[] Gifts = System.Array.Empty<ushort>();

    public override void Read(ref NetReader r)
    {
        Version = r.ReadInt32();
        ChestCount = r.ReadInt32();
        MaxChests = r.ReadInt32();
        NextChestPrice = r.ReadInt32();

        Slots = new ushort[r.ReadUInt16()];
        for (int i = 0; i < Slots.Length; i++)
            Slots[i] = r.ReadUInt16();

        Gifts = new ushort[r.ReadUInt16()];
        for (int i = 0; i < Gifts.Length; i++)
            Gifts[i] = r.ReadUInt16();
    }
}
