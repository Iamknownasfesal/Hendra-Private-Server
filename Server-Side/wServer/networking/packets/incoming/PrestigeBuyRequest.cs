using common;

namespace wServer.networking.packets.incoming
{
    public class PrestigeBuyRequest : IncomingMessage
    {
        public int BuyId { get; set; }

        public override PacketId ID => PacketId.PRESTIGEBUYREQUEST;
        public override Packet CreateInstance() { return new PrestigeBuyRequest(); }

        protected override void Read(NReader rdr)
        {
            BuyId = rdr.ReadInt32();
        }

        protected override void Write(NWriter wtr)
        {
            wtr.Write(BuyId);
        }
    }
}