using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using common;
using common.resources;
using wServer.networking;
using wServer.networking.packets.outgoing;
using wServer.realm.entities;
using wServer.realm.entities.vendors;
using wServer.realm.terrain;

namespace wServer.realm.worlds.logic
{
    public class Vault : World
    {
        /// <summary>The object the vault panel opens from. See the class remarks.</summary>
        public const ushort VaultAccessType = 0x0504;

        public int AccountId { get; private set; }

        private readonly Client _client;

        public Vault(ProtoWorld proto, Client client = null) : base(proto)
        {
            if (client != null)
            {
                _client = client;
                AccountId = _client.Account.AccountId;

                // One object, where there used to be one per eight slots. It keeps the type number
                // the chests had so that nothing else in the data files has to be renumbered, but it
                // is not a container: it holds nothing, and its class is what tells the client to
                // open the vault panel rather than an eight-slot grid.
                ExtraXML = ExtraXML.Concat(new[]
                {
                    @"	<Objects>
		                    <Object type=""0x0504"" id=""Vault"">
			                    <Class>VaultAccess</Class>
			                    <ShowName/>
			                    <Texture><File>lofiObj2</File><Index>0x0e</Index></Texture>
		                    </Object>
	                    </Objects>"
                }).ToArray();
            }
        }

        public override bool AllowedAccess(Client client)
        {
            return base.AllowedAccess(client) && AccountId == client.Account.AccountId || AccountId == 1 || AccountId == 17;
        }

        protected override void Init()
        {
            if (IsLimbo)
                return;

            FromWorldMap(new MemoryStream(Manager.Resources.Worlds[Name].wmap[0]));
            InitVault();
        }

        void InitVault()
        {
            var vaultChestPosition = new List<IntPoint>();
            var giftChestPosition = new List<IntPoint>();
            var spawn = new IntPoint(0, 0);

            var w = Map.Width;
            var h = Map.Height;

            for (var y = 0; y < h; y++)
                for (var x = 0; x < w; x++)
                {
                    var tile = Map[x, y];
                    switch (tile.Region)
                    {
                        case TileRegion.Spawn:
                            spawn = new IntPoint(x, y);
                            break;
                        case TileRegion.Vault:
                            vaultChestPosition.Add(new IntPoint(x, y));
                            break;
                        case TileRegion.Gifting_Chest:
                            giftChestPosition.Add(new IntPoint(x, y));
                            break;
                    }
                }

            vaultChestPosition.Sort((x, y) => Comparer<int>.Default.Compare(
                (x.X - spawn.X) * (x.X - spawn.X) + (x.Y - spawn.Y) * (x.Y - spawn.Y),
                (y.X - spawn.X) * (y.X - spawn.X) + (y.Y - spawn.Y) * (y.Y - spawn.Y)));

            giftChestPosition.Sort((x, y) => Comparer<int>.Default.Compare(
                (x.X - spawn.X) * (x.X - spawn.X) + (x.Y - spawn.Y) * (x.Y - spawn.Y),
                (y.X - spawn.X) * (y.X - spawn.X) + (y.Y - spawn.Y) * (y.Y - spawn.Y)));

            // One access object, on the vault tile nearest the spawn, and nothing on the rest of
            // them. The chests are not in the world any more -- see VaultState -- so the remaining
            // floor is floor. Capacity is an integer on the account and no longer a count of how
            // much room the map happens to have.
            if (vaultChestPosition.Count > 0)
            {
                var access = new StaticObject(_client.Manager, VaultAccessType, null, true, false, false);
                access.Move(vaultChestPosition[0].X + 0.5f, vaultChestPosition[0].Y + 0.5f);
                EnterWorld(access);
            }

            var gifts = _client.Account.Gifts.ToList();
            while (gifts.Count > 0 && giftChestPosition.Count > 0)
            {
                var c = Math.Min(8, gifts.Count);
                var items = gifts.GetRange(0, c);
                gifts.RemoveRange(0, c);
                if (c < 8)
                    items.AddRange(Enumerable.Repeat(ushort.MaxValue, 8 - c));

                var con = new GiftChest(_client.Manager, 0x0744, null, false);
                con.BagOwners = new int[] { _client.Account.AccountId };
                con.Inventory.SetItems(items);
                con.Move(giftChestPosition[0].X + 0.5f, giftChestPosition[0].Y + 0.5f);
                EnterWorld(con);
                giftChestPosition.RemoveAt(0);
            }
            foreach (var i in giftChestPosition)
            {
                var x = new StaticObject(_client.Manager, 0x0743, null, true, false, false);
                x.Move(i.X + 0.5f, i.Y + 0.5f);
                EnterWorld(x);
            }

            // devon roach
            if (_client.Account.Name.Equals("Devon"))
            {
                var e = new Enemy(Manager, 0x12C);
                e.Move(38, 68);
                EnterWorld(e);
            }
        }

        /// <summary>
        /// Hands a player arriving in the vault the whole of it.
        /// </summary>
        /// <remarks>
        /// The panel has nothing to draw until this lands: the contents no longer arrive as the
        /// equipment of eight objects the player can see, because there are no longer eight objects.
        /// </remarks>
        public override int EnterWorld(Entity entity)
        {
            var id = base.EnterWorld(entity);

            var player = entity as Player;
            if (player?.Client?.Account != null)
                player.Client.SendPacket(VaultState.Of(Manager, player.Client.Account).Snapshot());

            return id;
        }

        public override void LeaveWorld(Entity entity)
        {
            base.LeaveWorld(entity);

            if (entity.ObjectType != 0x0744)
                return;

            var x = new StaticObject(_client.Manager, 0x0743, null, true, false, false);
            x.Move(entity.X, entity.Y);
            EnterWorld(x);

            if (_client.Account.Gifts.Length <= 0)
                _client.SendPacket(new GlobalNotification
                {
                    Text = "giftChestEmpty"
                });
        }
    }
}
