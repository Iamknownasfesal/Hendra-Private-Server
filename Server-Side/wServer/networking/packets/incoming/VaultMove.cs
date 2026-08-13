using common;

namespace wServer.networking.packets.incoming
{
    /// <summary>
    /// Move an item within the vault, or between the vault and the player's own inventory.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Both ends are named the same way -- a place and an index -- because the swap is symmetric and
    /// the server should not need two packets to say which direction it went in. A chest index of
    /// <see cref="Player"/> means the index is a slot in the player's own twenty-four; anything else
    /// is a chest the account owns, and the index is one of its eight.
    /// </para>
    /// <para>
    /// The version is the vault as the client last saw it. Two clients on one account is an ordinary
    /// state, not an edge case, and a move computed against a vault that has since changed underneath
    /// is refused rather than applied to whatever happens to be in the slot now -- which is how the
    /// same stack gets moved twice and becomes two stacks.
    /// </para>
    /// </remarks>
    public class VaultMove : IncomingMessage
    {
        /// <summary>Chest index meaning "not a chest -- the player's own inventory".</summary>
        public const short Player = -1;

        /// <summary>
        /// Chest index meaning the gifts waiting to be claimed.
        /// </summary>
        /// <remarks>
        /// A source and never a destination. A gift is claimed by moving it out, which is the only
        /// thing that has ever been possible with one, and claiming is what removes it.
        /// </remarks>
        public const short Gifts = -2;

        public int Version { get; set; }

        /// <summary>
        /// A destination chest of this means one of the character's potion stacks, and
        /// <see cref="ToSlot"/> chooses which.
        /// </summary>
        /// <remarks>
        /// A stack is not a container and cannot be named by a chest and a slot, but it is a place
        /// an item can go, and the alternative for the player is dragging a potion out to the bag
        /// and then onto the counter. It is a destination only: nothing comes back out of a stack
        /// except by drinking it.
        /// </remarks>
        public const short Stacks = -3;

        public short FromChest { get; set; }
        public short FromSlot { get; set; }
        public short ToChest { get; set; }
        public short ToSlot { get; set; }

        public override PacketId ID => PacketId.VAULTMOVE;
        public override Packet CreateInstance() { return new VaultMove(); }

        protected override void Read(NReader rdr)
        {
            Version = rdr.ReadInt32();
            FromChest = rdr.ReadInt16();
            FromSlot = rdr.ReadInt16();
            ToChest = rdr.ReadInt16();
            ToSlot = rdr.ReadInt16();
        }

        protected override void Write(NWriter wtr)
        {
            wtr.Write(Version);
            wtr.Write(FromChest);
            wtr.Write(FromSlot);
            wtr.Write(ToChest);
            wtr.Write(ToSlot);
        }
    }
}
