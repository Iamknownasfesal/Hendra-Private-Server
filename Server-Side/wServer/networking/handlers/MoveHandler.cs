using System;
using wServer.realm.entities;
using wServer.networking.packets;
using wServer.networking.packets.incoming;
using wServer.realm;
using common.resources;

namespace wServer.networking.handlers
{
    internal class MoveHandler : PacketHandlerBase<Move>
    {
        public override PacketId ID => PacketId.MOVE;

        protected override void HandlePacket(Client client, Move packet)
        {
            client.Manager.Logic.AddPendingAction(t => Handle(client.Player, t, packet));
        }

        private static void Handle(Player player, RealmTime time, Move packet) {
            if (player?.Owner == null)
                return;

            var newX = packet.NewPosition.X;
            var newY = packet.NewPosition.Y;

            if (newX != -1 && newX != player.X ||
                  newY != -1 && newY != player.Y) {
                player.Move(newX, newY);
            }

            // Keeps the trail of timestamped positions this packet carries, and holds the move to a
            // speed the player's stats can reach. Both matter to more than movement: the server
            // works out for itself whether a bullet passed through someone, and it can only do that
            // against an account of where they were that they cannot simply write for themselves.
            player.RecordMove(time, packet);

            CheckLabConditions(player, packet);
            player.MoveReceived(time, packet);
        }

        private static void CheckLabConditions(Entity player, Move packet)
        {
            var x = (int)packet.NewPosition.X;
            var y = (int)packet.NewPosition.Y;

            // Minus one is the protocol's way of saying the player did not move, not a position,
            // and indexing the map with it threw -- which skipped MoveReceived, and with it the
            // tick accounting the connection depends on, so the player never finished arriving.
            if (!player.Owner.Map.Contains(x, y))
                return;

            var tile = player.Owner.Map[x, y];
            switch (tile.TileId)
            {
                //Green water
                case 0xa9:
                case 0x82:
                    if (tile.ObjId != 0)
                        return;
                    if (!player.HasConditionEffect(ConditionEffects.Hexed) ||
                        !player.HasConditionEffect(ConditionEffects.Stunned) ||
                        !player.HasConditionEffect(ConditionEffects.Speedy))
                    {
                        player.ApplyConditionEffect(ConditionEffectIndex.Hexed);
                        player.ApplyConditionEffect(ConditionEffectIndex.Stunned);
                        player.ApplyConditionEffect(ConditionEffectIndex.Speedy);
                    }
                    break;
                //Blue water
                case 0xa7:
                case 0x83:
                    if (tile.ObjId != 0)
                        return;
                    if (player.HasConditionEffect(ConditionEffects.Hexed) ||
                        player.HasConditionEffect(ConditionEffects.Stunned) ||
                        player.HasConditionEffect(ConditionEffects.Speedy))
                    {
                        player.ApplyConditionEffect(ConditionEffectIndex.Hexed, 0);
                        player.ApplyConditionEffect(ConditionEffectIndex.Stunned, 0);
                        player.ApplyConditionEffect(ConditionEffectIndex.Speedy, 0);
                    }
                    break;
            }
        }
    }
}
