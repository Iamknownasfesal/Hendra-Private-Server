using wServer.networking.packets;
using wServer.networking.packets.incoming;
using System;
using System.Linq;
using wServer.realm.entities;

namespace wServer.networking.handlers
{
    internal class PrestigeHandler : PacketHandlerBase<PrestigeRequest>
    {
        public override PacketId ID => PacketId.PRESTIGEREQUEST;

        protected override void HandlePacket(Client client, PrestigeRequest packet)
        {
            Handle(client, packet);
        }

        private void Handle(Client client, PrestigeRequest packet)
        {

            var pd = client.Player.Manager.Resources.GameData.Classes[client.Player.ObjectType];
            int currentPrestige = client.Account.Prestige;

            if (client.Character.Fame < 1499)
            {
                client.Player.SendError("You need to be 1500 fame or more!");
                return;
            }

            if (client.Character.Fame >= 1500)
            {
                do
                {
                    client.Character.Fame = client.Character.Fame - 1500;
                    client.Player.Prestige = client.Account.Prestige = currentPrestige + 1;
                    currentPrestige++;
                }
                while (client.Character.Fame >= 1500);

                // When doWhile finishs

                client.Player.Experience = client.Character.Experience = 0;
                client.Player.Level = client.Character.Level = 1;
                client.Character.Fame = 0;
                client.Player.FameGoal = 0;
                for (int i = 0; i < client.Character.FameStats.Length; i++)
                {
                    client.Character.FameStats[i] = 0;
                }

                // Save to Char and Calculate
                client.Player.CalculateFame();
                client.Player.SaveToCharacter();
                client.Player.CalculateFame();

                // Stats
                client.Player.Stats.Base[0] = pd.Stats[0].StartingValue;
                client.Player.Stats.Base[1] = pd.Stats[1].StartingValue;
                client.Player.Stats.Base[2] = pd.Stats[2].StartingValue;
                client.Player.Stats.Base[3] = pd.Stats[3].StartingValue;
                client.Player.Stats.Base[4] = pd.Stats[4].StartingValue;
                client.Player.Stats.Base[5] = pd.Stats[5].StartingValue;
                client.Player.Stats.Base[6] = pd.Stats[6].StartingValue;
                client.Player.Stats.Base[7] = pd.Stats[7].StartingValue;
                client.Player.SaveToCharacter();

                // Force Updates
                client.Player.ForceUpdate(client.Player.Prestige);
                client.Player.ForceUpdate(client.Player.CurrentFame);
                client.Player.ForceUpdate(client.Character.Fame);
                client.Player.CalculateFame();

                // Disconnect and Force Update
                client.Disconnect();
                client.Player.ForceUpdate(client.Player.Prestige);
                client.Player.ForceUpdate(client.Player.CurrentFame);
                client.Player.ForceUpdate(client.Character.Fame);
                client.Player.UpdateCount++;
            }

        }
    }
}