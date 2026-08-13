using common;

namespace wServer.networking.packets.incoming
{
    /// <summary>
    /// Buy one more chest.
    /// </summary>
    /// <remarks>
    /// Carries nothing but the count the client believes it owns. The price and the purse are the
    /// server's business and are never sent by the client; the count is here so that a second click
    /// arriving while the first is still in flight buys one chest rather than two.
    /// </remarks>
    public class VaultBuy : IncomingMessage
    {
        public int ChestCount { get; set; }

        public override PacketId ID => PacketId.VAULTBUY;
        public override Packet CreateInstance() { return new VaultBuy(); }

        protected override void Read(NReader rdr)
        {
            ChestCount = rdr.ReadInt32();
        }

        protected override void Write(NWriter wtr)
        {
            wtr.Write(ChestCount);
        }
    }
}
