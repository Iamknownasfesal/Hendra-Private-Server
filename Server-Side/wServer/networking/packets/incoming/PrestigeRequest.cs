using common;

namespace wServer.networking.packets.incoming
{
    public class PrestigeRequest : IncomingMessage
    {

        public override PacketId ID => PacketId.PRESTIGEREQUEST;
        public override Packet CreateInstance() { return new PrestigeRequest(); }

        protected override void Read(NReader rdr)
        {

        }

        protected override void Write(NWriter wtr)
        {

        }
    }
}