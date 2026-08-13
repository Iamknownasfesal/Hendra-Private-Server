using common;

namespace wServer.networking.packets.outgoing
{
    /// <summary>
    /// The whole vault, as the server has it.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Sent whole rather than as a delta. A vault is a few hundred item types at the very most --
    /// two bytes each, so a large one is under a kilobyte -- and a client that is told everything
    /// cannot drift out of step with the server, which is the failure that duplicates items. It is
    /// sent on entering the vault, after every accepted move, and to the loser of a race.
    /// </para>
    /// <para>
    /// The slots are flat and dense: chest <c>i</c> owns indices <c>i * 8</c> to <c>i * 8 + 7</c>.
    /// That is the presentation order the panel draws in, and it is derived here rather than being
    /// stored anywhere -- what is persisted is still one array of eight per purchased chest.
    /// </para>
    /// </remarks>
    public class VaultUpdate : OutgoingMessage
    {
        /// <summary>What the vault was at when this was written. Moves quote it back.</summary>
        public int Version { get; set; }

        public int ChestCount { get; set; }

        /// <summary>The most chests this account may ever own, so the panel can stop offering more.</summary>
        public int MaxChests { get; set; }

        /// <summary>What the next chest costs, in gold.</summary>
        public int NextChestPrice { get; set; }

        /// <summary>Item types, <see cref="ChestCount"/> times eight of them. 0xffff is empty.</summary>
        public ushort[] Slots { get; set; }

        /// <summary>
        /// Gifts waiting to be claimed, which arrive in the same panel and come out of it one way.
        /// </summary>
        /// <remarks>
        /// The gift chests were objects in this room too, and they went the same way as the vault
        /// chests: one place to look rather than several to walk between. They are not storage --
        /// nothing can be put into them -- so they are sent separately rather than as more slots.
        /// </remarks>
        public ushort[] Gifts { get; set; }

        public override PacketId ID => PacketId.VAULTUPDATE;
        public override Packet CreateInstance() { return new VaultUpdate(); }

        protected override void Read(NReader rdr)
        {
            Version = rdr.ReadInt32();
            ChestCount = rdr.ReadInt32();
            MaxChests = rdr.ReadInt32();
            NextChestPrice = rdr.ReadInt32();

            Slots = new ushort[rdr.ReadUInt16()];
            for (var i = 0; i < Slots.Length; i++)
                Slots[i] = rdr.ReadUInt16();

            Gifts = new ushort[rdr.ReadUInt16()];
            for (var i = 0; i < Gifts.Length; i++)
                Gifts[i] = rdr.ReadUInt16();
        }

        protected override void Write(NWriter wtr)
        {
            wtr.Write(Version);
            wtr.Write(ChestCount);
            wtr.Write(MaxChests);
            wtr.Write(NextChestPrice);

            wtr.Write((ushort)Slots.Length);
            foreach (var slot in Slots)
                wtr.Write(slot);

            wtr.Write((ushort)Gifts.Length);
            foreach (var gift in Gifts)
                wtr.Write(gift);
        }
    }
}
