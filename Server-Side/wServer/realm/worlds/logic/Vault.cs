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
                //
                // The artwork is the game's own event chest -- thrown open, gold spilling out of it
                // -- drawn at twice size. It is the one thing in the room now and it should read as
                // the reason the room exists, which the little closed chest never did.
                ExtraXML = ExtraXML.Concat(new[]
                {
                    @"	<Objects>
		                    <Object type=""0x0504"" id=""Vault"">
			                    <Class>VaultAccess</Class>
			                    <ShowName/>
			                    <Size>200</Size>
			                    <Texture><File>lofiObj3</File><Index>0x466</Index></Texture>
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
                    }
                }

            vaultChestPosition.Sort((x, y) => Comparer<int>.Default.Compare(
                (x.X - spawn.X) * (x.X - spawn.X) + (x.Y - spawn.Y) * (x.Y - spawn.Y),
                (y.X - spawn.X) * (y.X - spawn.X) + (y.Y - spawn.Y) * (y.Y - spawn.Y)));

            // One access object, on the one tile the map still marks. There used to be eighty of
            // them ringing the garden and the map's own floor was the capacity limit; the chests
            // are not in the world any more -- see VaultState -- and capacity is an integer on the
            // account, so the garden is garden and the middle of it is the chest.
            if (vaultChestPosition.Count > 0)
            {
                var access = new StaticObject(_client.Manager, VaultAccessType, null, true, false, false);
                access.Move(vaultChestPosition[0].X + 0.5f, vaultChestPosition[0].Y + 0.5f);
                EnterWorld(access);

                Crackle(access);
            }

            // Gifts used to be chests of their own along the far wall. They arrive in the vault
            // panel now, sent with everything else -- see VaultUpdate -- so there is nothing to
            // place here and the map no longer marks anywhere to place it.

            // devon roach
            if (_client.Account.Name.Equals("Devon"))
            {
                var e = new Enemy(Manager, 0x12C);
                e.Move(38, 68);
                EnterWorld(e);
            }
        }

        /// <summary>How often the chest throws a spark, in milliseconds.</summary>
        private const int CrackleEveryMs = 2600;

        /// <summary>How far above the chest the arcs come down from, in tiles.</summary>
        private const float CrackleHeight = 3.5f;

        /// <summary>
        /// Keeps the chest arcing.
        /// </summary>
        /// <remarks>
        /// <para>
        /// The lightning is the server's, not the sprite's. Both sides already know how to draw an
        /// arc between two points -- it is what every storm behaviour in the game uses -- so the
        /// chest borrows it rather than the client growing a second animation system for one
        /// object. The sprite is a still, and everything that moves around it is particles.
        /// </para>
        /// <para>
        /// The timer re-arms itself, which is how a world timer repeats here, and it costs one
        /// packet every couple of seconds to whoever is in the room. Nobody else is: the vault is
        /// one account's own world.
        /// </para>
        /// </remarks>
        private void Crackle(Entity chest)
        {
            AddTimer(new WorldTimer(CrackleEveryMs, (world, t) =>
            {
                if (chest.Owner == null)
                    return;

                // Down onto the chest from a point overhead, offset a little each time so the
                // strike does not land in exactly the same place twice.
                var drift = (float)(_crackle.NextDouble() * 2.0 - 1.0) * 1.6f;

                chest.Owner.BroadcastPacket(new ShowEffect
                {
                    EffectType = EffectType.Lightning,
                    TargetObjectId = chest.Id,
                    Pos1 = new Position { X = chest.X + drift, Y = chest.Y - CrackleHeight },
                    Pos2 = new Position { X = 5 },
                    Color = new ARGB(0xffffe9a0)
                }, null);

                Crackle(chest);
            }));
        }

        private readonly Random _crackle = new Random();

        /// <summary>Accounts already handed their vault, so it goes out once and not every tick.</summary>
        private readonly HashSet<int> _sent = new HashSet<int>();

        /// <summary>
        /// Hands a player arriving in the vault the whole of it.
        /// </summary>
        /// <remarks>
        /// <para>
        /// The panel has nothing to draw until this lands: the contents no longer arrive as the
        /// equipment of eight objects the player can see, because there are no longer eight objects.
        /// </para>
        /// <para>
        /// On the tick, and not in EnterWorld, which is where this was and where it did not work.
        /// EnterWorld runs inside the load handler one line above <c>client.State =
        /// ProtocolState.Ready</c>, and before the CreateSuccess that tells the client which object
        /// it is -- so the snapshot went out to a connection that was not yet listening and the
        /// vault drew itself empty every time. Waiting for Ready costs one tick and cannot be got
        /// wrong again by something else reordering the handshake.
        /// </para>
        /// </remarks>
        public override void Tick(RealmTime time)
        {
            base.Tick(time);

            foreach (var player in Players.Values)
            {
                var client = player.Client;
                if (client == null || client.State != ProtocolState.Ready || client.Account == null)
                    continue;

                if (!_sent.Add(client.Account.AccountId))
                    continue;

                client.SendPacket(VaultState.Of(Manager, client.Account).Snapshot());
            }
        }

    }
}
