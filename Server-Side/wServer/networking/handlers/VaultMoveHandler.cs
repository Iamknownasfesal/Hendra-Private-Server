using wServer.networking.packets;
using wServer.networking.packets.incoming;
using wServer.realm;
using wServer.realm.worlds.logic;

namespace wServer.networking.handlers
{
    /// <summary>
    /// Moves an item in the vault.
    /// </summary>
    /// <remarks>
    /// Run on the logic thread rather than the socket's, like every other handler that touches a
    /// player: the inventory it swaps against is the same one the tick is reading.
    /// </remarks>
    class VaultMoveHandler : PacketHandlerBase<VaultMove>
    {
        public override PacketId ID => PacketId.VAULTMOVE;

        protected override void HandlePacket(Client client, VaultMove packet)
        {
            client.Manager.Logic.AddPendingAction(t => Handle(client, packet));
        }

        private void Handle(Client client, VaultMove packet)
        {
            var player = client.Player;
            if (player?.Owner == null || client.Account == null)
                return;

            // Only from inside the vault. The panel closes when the player walks away from the
            // access object, and a move arriving from a world with no vault in it is either a stale
            // packet or someone reaching into their storage from a dungeon.
            if (!(player.Owner is Vault))
                return;

            var vault = VaultState.Of(client.Manager, client.Account);

            string refusal;
            if (vault.TryMove(player, packet, out refusal))
            {
                vault.Broadcast();
                return;
            }

            // Refused. The client is told what the vault actually is, which both corrects the
            // optimistic move it already drew and gives it the version to try again with.
            client.SendPacket(vault.Snapshot());
        }
    }
}
