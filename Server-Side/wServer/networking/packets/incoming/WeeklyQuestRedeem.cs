using common;

namespace wServer.networking.packets.incoming
{
    public class WeeklyQuestRedeem : IncomingMessage
    {
        public ObjectSlot Object { get; set; }

        public override PacketId ID => PacketId.WEEKLYQUESTREDEEM;
        public override Packet CreateInstance() { return new WeeklyQuestRedeem(); }

        protected override void Read(NReader rdr)
        {
            Object = ObjectSlot.Read(rdr);
        }

        protected override void Write(NWriter wtr)
        {
            Object.Write(wtr);
        }
    }
}
