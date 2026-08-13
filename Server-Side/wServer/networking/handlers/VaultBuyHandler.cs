using wServer.networking.packets;
using wServer.networking.packets.incoming;
using wServer.networking.packets.outgoing;
using wServer.realm;
using wServer.realm.worlds.logic;

namespace wServer.networking.handlers
{
    /// <summary>Buys another vault chest, which used to be a purchase from an object in the room.</summary>
    class VaultBuyHandler : PacketHandlerBase<VaultBuy>
    {
        public override PacketId ID => PacketId.VAULTBUY;

        protected override void HandlePacket(Client client, VaultBuy packet)
        {
            client.Manager.Logic.AddPendingAction(t => Handle(client, packet));
        }

        private void Handle(Client client, VaultBuy packet)
        {
            var player = client.Player;
            if (player?.Owner == null || client.Account == null)
                return;

            if (!(player.Owner is Vault))
                return;

            var vault = VaultState.Of(client.Manager, client.Account);

            string refusal;
            if (!vault.TryBuy(client, packet.ChestCount, out refusal))
            {
                client.SendPacket(new BuyResult { Result = 1, ResultString = refusal });
                client.SendPacket(vault.Snapshot());
                return;
            }

            client.SendPacket(new BuyResult { Result = 0, ResultString = "Vault chest purchased!" });
            vault.Broadcast();
        }
    }
}
