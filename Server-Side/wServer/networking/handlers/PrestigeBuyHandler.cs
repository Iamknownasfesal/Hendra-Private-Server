using wServer.networking.packets;
using wServer.networking.packets.incoming;
using System;
using System.Linq;
using wServer.realm.entities;

namespace wServer.networking.handlers
{
    internal class PrestigeBuyHandler : PacketHandlerBase<PrestigeBuyRequest>
    {
        public override PacketId ID => PacketId.PRESTIGEBUYREQUEST;

        protected override void HandlePacket(Client client, PrestigeBuyRequest packet)
        {
            Handle(client, packet);
        }

        private void Handle(Client client, PrestigeBuyRequest packet)
        {
            int Item1Money = 50;
            int Item2Money = 150;

            ushort[] ItemList = { 0xffe, 0x40a9, 0x40a8, 0x40a7 };

            if(packet.BuyId < 0 || packet.BuyId > 8)
            {
                client.Player.SendError("ERROR");
            }
            else
            {
                if(packet.BuyId == 1)
                {
                    var id = client.Manager.Database.ResolveId(client.Player.Name);
                    var acc = client.Manager.Database.GetAccount(id);
                    if (client.Account.Prestige >= Item1Money)
                    {
                        client.Player.Prestige = client.Player.Prestige - Item1Money;
                        client.Account.Prestige = client.Player.Prestige;
                        client.Player.ForceUpdate(client.Player.Prestige);
                        client.Player.ForceUpdate(client.Account.Prestige);

                        var result = client.Player.Manager.Database.AddGift(acc, ItemList[0]);
                        if (!result)
                        {
                            client.Player.SendError("Error! Talk with Fesal!");
                            return;
                        }
                        client.Player.SendInfo("The Item, added to your gift chest!");
                        return;
                    }
                    else
                    {
                        client.Player.SendError("You cant buy this item.");
                        return;
                    }
                }
                if (packet.BuyId == 2)
                {
                    var id = client.Manager.Database.ResolveId(client.Player.Name);
                    var acc = client.Manager.Database.GetAccount(id);
                    if (client.Account.Prestige >= Item2Money)
                    {
                        client.Player.Prestige = client.Player.Prestige - Item2Money;
                        client.Account.Prestige = client.Player.Prestige;
                        client.Player.ForceUpdate(client.Player.Prestige);
                        client.Player.ForceUpdate(client.Account.Prestige);

                        var result = client.Player.Manager.Database.AddGift(acc, ItemList[1]);
                        if (!result)
                        {
                            client.Player.SendError("Error! Talk with Fesal!");
                            return;
                        }
                        client.Player.SendInfo("The Item, added to your gift chest!");
                        return;
                    }
                    else
                    {
                        client.Player.SendError("You cant buy this item.");
                        return;
                    }
                }
                if (packet.BuyId == 3)
                {
                    var id = client.Manager.Database.ResolveId(client.Player.Name);
                    var acc = client.Manager.Database.GetAccount(id);
                    if (client.Account.Prestige >= Item2Money)
                    {
                        client.Player.Prestige = client.Player.Prestige - Item2Money;
                        client.Account.Prestige = client.Player.Prestige;
                        client.Player.ForceUpdate(client.Player.Prestige);
                        client.Player.ForceUpdate(client.Account.Prestige);

                        var result = client.Player.Manager.Database.AddGift(acc, ItemList[2]);
                        if (!result)
                        {
                            client.Player.SendError("Error! Talk with Fesal!");
                            return;
                        }
                        client.Player.SendInfo("The Item, added to your gift chest!");
                        return;
                    }
                    else
                    {
                        client.Player.SendError("You cant buy this item.");
                        return;
                    }
                }
                if (packet.BuyId == 4)
                {
                    var id = client.Manager.Database.ResolveId(client.Player.Name);
                    var acc = client.Manager.Database.GetAccount(id);
                    if (client.Account.Prestige >= Item2Money)
                    {
                        client.Player.Prestige = client.Player.Prestige - Item2Money;
                        client.Account.Prestige = client.Player.Prestige;
                        client.Player.ForceUpdate(client.Player.Prestige);
                        client.Player.ForceUpdate(client.Account.Prestige);

                        var result = client.Player.Manager.Database.AddGift(acc, ItemList[3]);
                        if (!result)
                        {
                            client.Player.SendError("Error! Talk with Fesal!");
                            return;
                        }
                        client.Player.SendInfo("The Item, added to your gift chest!");
                        return;
                    }
                    else
                    {
                        client.Player.SendError("You cant buy this item.");
                        return;
                    }
                }
                else
                {
                    client.Player.SendError("ERROR!");
                    return;
                }
            }
        }
    }
}