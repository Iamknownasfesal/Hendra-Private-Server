using common.resources;
using wServer.networking.packets;
using wServer.networking.packets.incoming;
using wServer.networking.packets.outgoing;

namespace wServer.networking.handlers
{
    class WeeklyQuestHandler : PacketHandlerBase<WeeklyQuestRedeem>
    {
        public override PacketId ID => PacketId.WEEKLYQUESTREDEEM;
        protected override void HandlePacket(Client client, WeeklyQuestRedeem packet)
        {
            Handle(client, packet);
        }

        private void Handle(Client client, Packet name)
        {

        }
    }
}
