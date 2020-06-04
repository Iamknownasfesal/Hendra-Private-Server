using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using common;
using common.resources;
using log4net;
using Newtonsoft.Json;
using wServer.networking;
using wServer.networking.packets.outgoing;
using wServer.realm.setpieces;
using wServer.realm.entities;
using wServer.realm.worlds;
using wServer.realm.worlds.logic;
using System.Collections.Specialized;
using wServer.discord;

namespace wServer.realm.commands
{
    class SpawnCommand : Command
    {
        static readonly ILog log = LogManager.GetLogger(typeof(SpawnCommand));

        private struct JsonSpawn
        {
            public string notif;
            public SpawnProperties[] spawns;
        }

        private struct SpawnProperties
        {
            public string name;
            public int? hp;
            public int? size;
            public int? count;
            public int[] x;
            public int[] y;
            public bool? target;
        }

        private const int Delay = 3; // in seconds

        class SpawnBCommand : Command
        {
            static readonly ILog log = LogManager.GetLogger(typeof(SpawnCommand));

            private struct JsonSpawn
            {
                public string notif;
                public SpawnProperties[] spawns;
            }

            private struct SpawnProperties
            {
                public string name;
                public int? hp;
                public int? size;
                public int? count;
                public int[] x;
                public int[] y;
                public bool? target;
            }

            private const int Delay = 3; // in seconds

            public SpawnBCommand() : base("lootspawn", alias: "ls", permLevel: 90) { }

            protected override bool Process(Player player, RealmTime time, string args)
            {
                args = args.Trim();
                return args.StartsWith("{") ?
                    SpawnJson(player, args) :
                    SpawnBasic(player, args);
            }

            private bool SpawnJson(Player player, string json)
            {
                var gameData = player.Manager.Resources.GameData;

                JsonSpawn props;
                try
                {
                    props = JsonConvert.DeserializeObject<JsonSpawn>(json);
                }
                catch (Exception)
                {
                    player.SendError("JSON not formatted correctly!");
                    return false;
                }

                if (props.spawns != null)
                    foreach (var spawn in props.spawns)
                    {
                        if (spawn.name == null)
                        {
                            player.SendError("No mob specified. Every entry needs a name property.");
                            return false;
                        }

                        var objType = GetSpawnObjectType(gameData, spawn.name);
                        if (objType == null)
                        {
                            player.SendError("Unknown entity!");
                            return false;
                        }

                        var desc = gameData.ObjectDescs[objType.Value];

                        if (player.Client.Account.Rank < 90 &&
                            desc.ObjectId.Contains("Fountain"))
                        {
                            player.SendError("Insufficient rank.");
                            return false;
                        }

                        var hp = desc.MaxHP;
                        if (spawn.hp > hp && spawn.hp < int.MaxValue)
                            hp = spawn.hp.Value;

                        var size = desc.MinSize;
                        if (spawn.size >= 25 && spawn.size <= 500)
                            size = spawn.size.Value;

                        var count = 1;
                        if (spawn.count > count && spawn.count <= 500)
                            count = spawn.count.Value;

                        int[] x = null;
                        int[] y = null;

                        if (spawn.x != null)
                            x = new int[spawn.x.Length];

                        if (spawn.y != null)
                            y = new int[spawn.y.Length];

                        if (x != null)
                        {
                            for (int i = 0; i < x.Length && i < count; i++)
                            {
                                if (spawn.x[i] > 0 && spawn.x[i] <= player.Owner.Map.Width)
                                {
                                    x[i] = spawn.x[i];
                                }

                            }

                        }

                        if (y != null)
                        {
                            for (int i = 0; i < y.Length && i < count; i++)
                            {
                                if (spawn.y[i] > 0 && spawn.y[i] <= player.Owner.Map.Height)
                                {
                                    y[i] = spawn.y[i];
                                }

                            }

                        }

                        bool target = false;
                        if (spawn.target != null)
                            target = spawn.target.Value;

                        QueueSpawnEvent(player, count, objType.Value, hp, size, x, y, target);
                    }

                if (props.notif != null)
                {
                    NotifySpawn(player, props.notif);
                }


                return true;
            }

            private bool SpawnBasic(Player player, string args)
            {
                var gameData = player.Manager.Resources.GameData;

                // split argument
                var index = args.IndexOf(' ');
                int num;
                var name = args;
                if (args.IndexOf(' ') > 0 && int.TryParse(args.Substring(0, args.IndexOf(' ')), out num)) //multi
                    name = args.Substring(index + 1);
                else
                    num = 1;

                var objType = GetSpawnObjectType(gameData, name);
                if (objType == null)
                {
                    player.SendError("Unknown entity!");
                    return false;
                }

                if (num <= 0)
                {
                    player.SendInfo($"Really? {num} {name}? I'll get right on that...");
                    return false;
                }

                var id = player.Manager.Resources.GameData.ObjectTypeToId[objType.Value];
                if (player.Client.Account.Rank < 100 &&
                    id.Contains("Fountain"))
                {
                    player.SendError("Insufficient rank.");
                    return false;
                }

                NotifySpawn(player, id, num);
                QueueSpawnEvent(player, num, objType.Value);
                return true;
            }

            private ushort? GetSpawnObjectType(XmlData gameData, string name)
            {
                ushort objType;
                if (!gameData.IdToObjectType.TryGetValue(name, out objType) ||
                    !gameData.ObjectDescs.ContainsKey(objType))
                {
                    // no match found, try to get partial match
                    var mobs = gameData.IdToObjectType
                        .Where(m => m.Key.ContainsIgnoreCase(name) && gameData.ObjectDescs.ContainsKey(m.Value))
                        .Select(m => gameData.ObjectDescs[m.Value]);

                    if (!mobs.Any())
                        return null;

                    var maxHp = mobs.Max(e => e.MaxHP);
                    objType = mobs.First(e => e.MaxHP == maxHp).ObjectType;
                }

                return objType;
            }

            private void NotifySpawn(Player player, string mob, int? num = null)
            {
                var w = player.Owner;

                var notif = mob;
                if (num != null)
                    notif = "Spawning " + ((num > 1) ? num + " " : "") + mob;

                w.BroadcastPacket(new Notification
                {
                    Color = new ARGB(0xffff0000),
                    ObjectId = (player.IsControlling) ? player.SpectateTarget.Id : player.Id,
                    Message = notif
                }, null, PacketPriority.Low);

                if (player.IsControlling)
                    w.BroadcastPacket(new Text
                    {
                        Name = $"#{player.SpectateTarget.ObjectDesc.DisplayId}",
                        NumStars = -1,
                        BubbleTime = 0,
                        Txt = notif
                    }, null, PacketPriority.Low);
                else
                    w.BroadcastPacket(new Text
                    {
                        Name = $"#{player.Name}",
                        NumStars = player.Stars,
                        Admin = player.Admin,
                        BubbleTime = 0,
                        Txt = notif
                    }, null, PacketPriority.Low);
            }

            private void QueueSpawnEvent(
                Player player,
                int num,
                ushort mobObjectType, int? hp = null, int? size = null,
                int[] x = null, int[] y = null,
                bool? target = false)
            {
                var pX = player.X;
                var pY = player.Y;

                player.Owner.Timers.Add(new WorldTimer(Delay * 1000, (world, t) => // spawn mob in delay seconds
                {
                    for (var i = 0; i < num && i < 500; i++)
                    {
                        Entity entity;
                        try
                        {
                            entity = Entity.Resolve(world.Manager, mobObjectType);
                        }
                        catch (Exception e)
                        {
                            log.ErrorFormat(e.ToString());
                            return;
                        }


                        var enemy = entity as Enemy;
                        if (enemy != null)
                        {
                            if (hp != null)
                            {
                                enemy.HP = hp.Value;
                                enemy.MaximumHP = enemy.HP;
                            }

                            if (size != null)
                                enemy.SetDefaultSize(size.Value);

                            if (target == true)
                                enemy.AttackTarget = player;
                        }

                        var sX = (x != null && i < x.Length) ? x[i] : pX;
                        var sY = (y != null && i < y.Length) ? y[i] : pY;

                        entity.Move(sX, sY);

                        if (!world.Deleted)
                            world.EnterWorld(entity);
                    }
                }));
            }
        }
        public SpawnCommand() : base("spawn", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            args = args.Trim();
            return args.StartsWith("{") ?
                SpawnJson(player, args) :
                SpawnBasic(player, args);
        }

        private bool SpawnJson(Player player, string json)
        {
            var gameData = player.Manager.Resources.GameData;

            JsonSpawn props;
            try
            {
                props = JsonConvert.DeserializeObject<JsonSpawn>(json);
            }
            catch (Exception)
            {
                player.SendError("JSON not formatted correctly!");
                return false;
            }

            if (props.spawns != null)
                foreach (var spawn in props.spawns)
                {
                    if (spawn.name == null)
                    {
                        player.SendError("No mob specified. Every entry needs a name property.");
                        return false;
                    }

                    var objType = GetSpawnObjectType(gameData, spawn.name);
                    if (objType == null)
                    {
                        player.SendError("Unknown entity!");
                        return false;
                    }

                    var desc = gameData.ObjectDescs[objType.Value];

                    if (player.Client.Account.Rank < 100 &&
                        desc.ObjectId.Contains("Fountain"))
                    {
                        player.SendError("Insufficient rank.");
                        return false;
                    }

                    var hp = desc.MaxHP;
                    if (spawn.hp > hp && spawn.hp < int.MaxValue)
                        hp = spawn.hp.Value;

                    var size = desc.MinSize;
                    if (spawn.size >= 25 && spawn.size <= 500)
                        size = spawn.size.Value;

                    var count = 1;
                    if (spawn.count > count && spawn.count <= 500)
                        count = spawn.count.Value;

                    int[] x = null;
                    int[] y = null;

                    if (spawn.x != null)
                        x = new int[spawn.x.Length];

                    if (spawn.y != null)
                        y = new int[spawn.y.Length];

                    if (x != null)
                    {
                        for (int i = 0; i < x.Length && i < count; i++)
                        {
                            if (spawn.x[i] > 0 && spawn.x[i] <= player.Owner.Map.Width)
                            {
                                x[i] = spawn.x[i];
                            }

                        }

                    }

                    if (y != null)
                    {
                        for (int i = 0; i < y.Length && i < count; i++)
                        {
                            if (spawn.y[i] > 0 && spawn.y[i] <= player.Owner.Map.Height)
                            {
                                y[i] = spawn.y[i];
                            }

                        }

                    }

                    bool target = false;
                    if (spawn.target != null)
                        target = spawn.target.Value;

                    QueueSpawnEvent(player, count, objType.Value, hp, size, x, y, target);
                }

            if (props.notif != null)
            {
                NotifySpawn(player, props.notif);
            }


            return true;
        }

        private bool SpawnBasic(Player player, string args)
        {
            var gameData = player.Manager.Resources.GameData;

            // split argument
            var index = args.IndexOf(' ');
            int num;
            var name = args;
            if (args.IndexOf(' ') > 0 && int.TryParse(args.Substring(0, args.IndexOf(' ')), out num)) //multi
                name = args.Substring(index + 1);
            else
                num = 1;

            var objType = GetSpawnObjectType(gameData, name);
            if (objType == null)
            {
                player.SendError("Unknown entity!");
                return false;
            }

            if (num <= 0)
            {
                player.SendInfo($"Really? {num} {name}? I'll get right on that...");
                return false;
            }

            var id = player.Manager.Resources.GameData.ObjectTypeToId[objType.Value];
            if (player.Client.Account.Rank < 100 &&
                id.Contains("Fountain"))
            {
                player.SendError("Insufficient rank.");
                return false;
            }

            NotifySpawn(player, id, num);
            QueueSpawnEvent(player, num, objType.Value);
            return true;
        }

        private ushort? GetSpawnObjectType(XmlData gameData, string name)
        {
            ushort objType;
            if (!gameData.IdToObjectType.TryGetValue(name, out objType) ||
                !gameData.ObjectDescs.ContainsKey(objType))
            {
                // no match found, try to get partial match
                var mobs = gameData.IdToObjectType
                    .Where(m => m.Key.ContainsIgnoreCase(name) && gameData.ObjectDescs.ContainsKey(m.Value))
                    .Select(m => gameData.ObjectDescs[m.Value]);

                if (!mobs.Any())
                    return null;

                var maxHp = mobs.Max(e => e.MaxHP);
                objType = mobs.First(e => e.MaxHP == maxHp).ObjectType;
            }

            return objType;
        }

        private void NotifySpawn(Player player, string mob, int? num = null)
        {
            var w = player.Owner;

            var notif = mob;
            if (num != null)
                notif = "Spawning " + ((num > 1) ? num + " " : "") + mob + "...";

            w.BroadcastPacket(new Notification
            {
                Color = new ARGB(0xffff0000),
                ObjectId = (player.IsControlling) ? player.SpectateTarget.Id : player.Id,
                Message = notif
            }, null, PacketPriority.Low);

            if (player.IsControlling)
                w.BroadcastPacket(new Text
                {
                    Name = $"#{player.SpectateTarget.ObjectDesc.DisplayId}",
                    NumStars = -1,
                    BubbleTime = 0,
                    Txt = notif
                }, null, PacketPriority.Low);
            else
                w.BroadcastPacket(new Text
                {
                    Name = $"#{player.Name}",
                    NumStars = player.Stars,
                    Admin = player.Admin,
                    BubbleTime = 0,
                    Txt = notif
                }, null, PacketPriority.Low);
        }

        private void QueueSpawnEvent(
            Player player,
            int num,
            ushort mobObjectType, int? hp = null, int? size = null,
            int[] x = null, int[] y = null,
            bool? target = false)
        {
            var pX = player.X;
            var pY = player.Y;

            player.Owner.Timers.Add(new WorldTimer(Delay * 1000, (world, t) => // spawn mob in delay seconds
            {
                for (var i = 0; i < num && i < 500; i++)
                {
                    Entity entity;
                    try
                    {
                        entity = Entity.Resolve(world.Manager, mobObjectType);
                    }
                    catch (Exception e)
                    {
                        log.ErrorFormat(e.ToString());
                        return;
                    }

                    entity.Spawned = true;

                    var enemy = entity as Enemy;
                    if (enemy != null)
                    {
                        if (hp != null)
                        {
                            enemy.HP = hp.Value;
                            enemy.MaximumHP = enemy.HP;
                        }

                        if (size != null)
                            enemy.SetDefaultSize(size.Value);

                        if (target == true)
                            enemy.AttackTarget = player;

                        enemy.ApplyConditionEffect(new ConditionEffect()
                        {
                            Effect = ConditionEffectIndex.Invisible,
                            DurationMS = -1
                        });
                    }

                    var sX = (x != null && i < x.Length) ? x[i] : pX;
                    var sY = (y != null && i < y.Length) ? y[i] : pY;

                    entity.Move(sX, sY);

                    if (!world.Deleted)
                        world.EnterWorld(entity);
                }
            }));
        }
    }


    class ClearSpawnsCommand : Command
    {
        public ClearSpawnsCommand() : base("clearspawn", permLevel: 80, alias: "cs") { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var iterations = 0;
            var lastKilled = -1;
            var removed = 0;
            while (removed != lastKilled)
            {
                lastKilled = removed;
                foreach (var entity in player.Owner.Enemies.Values.Where(e => e.Spawned == true))
                {
                    entity.Death(time);
                    removed++;
                }
                foreach (var entity in player.Owner.StaticObjects.Values.Where(e => e.Spawned == true))
                {
                    player.Owner.LeaveWorld(entity);
                    removed++;
                }
                if (++iterations >= 5)
                    break;
            }

            player.SendInfo($"{removed} spawned entities removed!");
            return true;
        }
    }

    class ClearGravesCommand : Command
    {
        public ClearGravesCommand() : base("cleargraves", permLevel: 80, alias: "cgraves") { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var removed = 0;
            foreach (var entity in player.Owner.StaticObjects.Values)
            {
                if (entity is Container || entity.ObjectDesc == null)
                    continue;

                if (entity.ObjectDesc.ObjectId.StartsWith("Gravestone") && entity.Dist(player) < 15)
                {
                    player.Owner.LeaveWorld(entity);
                    removed++;
                }
            }

            player.SendInfo($"{removed} gravestones removed!");
            return true;
        }
    }

    class ToggleEffCommand : Command
    {
        public ToggleEffCommand() : base("eff", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            ConditionEffectIndex effect;
            if (!Enum.TryParse(args, true, out effect))
            {
                player.SendError("Invalid effect!");
                return false;
            }

            var target = player.IsControlling ? player.SpectateTarget : player;
            if ((target.ConditionEffects & (ConditionEffects)((ulong)1 << (int)effect)) != 0)
            {
                //remove
                target.ApplyConditionEffect(new ConditionEffect()
                {
                    Effect = effect,
                    DurationMS = 0
                });
            }
            else
            {
                //add
                target.ApplyConditionEffect(new ConditionEffect()
                {
                    Effect = effect,
                    DurationMS = -1
                });
            }
            return true;
        }
    }

    class GuildRankCommand : Command
    {
        public GuildRankCommand() : base("grank", permLevel: 95) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            if (player == null)
                return false;

            // verify argument
            var index = args.IndexOf(' ');
            if (string.IsNullOrWhiteSpace(args) || index == -1)
            {
                player.SendInfo("Usage: /grank <player name> <guild rank>");
                return false;
            }

            // get command args
            var playerName = args.Substring(0, index);
            var rank = args.Substring(index + 1).IsInt() ? args.Substring(index + 1).ToInt32() : RankNumberFromName(args.Substring(index + 1));
            if (rank == -1)
            {
                player.SendError("Unknown rank!");
                return false;
            }
            else if (rank % 10 != 0)
            {
                player.SendError("Valid ranks are multiples of 10!");
                return false;
            }

            // get player account
            if (Database.GuestNames.Contains(playerName, StringComparer.InvariantCultureIgnoreCase))
            {
                player.SendError("Cannot rank the unnamed...");
                return false;
            }
            var id = player.Manager.Database.ResolveId(playerName);
            var acc = player.Manager.Database.GetAccount(id);
            if (id == 0 || acc == null)
            {
                player.SendError("Account not found!");
                return false;
            }

            // change rank
            acc.GuildRank = rank;
            acc.FlushAsync();

            // send out success notifications
            player.SendInfo($"You changed the guildrank of player {acc.Name} to {rank}.");
            var target = player.Manager.Clients.Keys.SingleOrDefault(p => p.Account.AccountId == acc.AccountId);
            if (target?.Player == null) return true;
            target.Player.GuildRank = rank;
            target.Player.SendInfo("Your guild rank was changed");
            return true;
        }

        private int RankNumberFromName(string val)
        {
            switch (val.ToLower())
            {
                case "initiate":
                    return 0;
                case "member":
                    return 10;
                case "officer":
                    return 20;
                case "leader":
                    return 30;
                case "founder":
                    return 40;
            }
            return -1;
        }
    }

    class GimmeCommand : Command
    {
        public GimmeCommand() : base("gimme", permLevel: 40, alias: "give") { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var gameData = player.Manager.Resources.GameData;

            ushort objType;

            // allow both DisplayId and Id for query
            if (!gameData.DisplayIdToObjectType.TryGetValue(args, out objType))
            {
                if (!gameData.IdToObjectType.TryGetValue(args, out objType))
                {
                    player.SendError("Unknown item type!");
                    return false;
                }
            }

            if (!gameData.Items.ContainsKey(objType))
            {
                player.SendError("Not an item!");
                return false;
            }

            var item = gameData.Items[objType];

            if (player.Client.Account.Rank < 90 &&
                (item.DisplayName.Equals("Boshy Gun") ||
                 item.DisplayName.Equals("Boshy Shotgun") ||
                 item.DisplayName.Equals("Admin Sword") ||
                 item.DisplayName.Equals("Admin Wand") ||
                 item.DisplayName.Equals("Admin Bow") ||
                 item.DisplayName.Equals("Admin Katana") ||
                 item.DisplayName.Equals("Admin Dagger") ||
                 item.DisplayName.Equals("Admin Staff") ||
                 item.DisplayName.Equals("Crown") ||
                 item.DisplayName.Equals("Lost Halls Key") ||
                 item.DisplayName.Equals("Gold Medal") ||
                 item.DisplayName.Equals("Ent World Key") ||
                 item.DisplayName.Equals("Strike Amulet") ||
                 item.DisplayName.Equals("Oryx's Arena Key")))
            {
                player.SendError("Insufficient rank for that item.");
                return false;
            }

            var availableSlot = player.Inventory.GetAvailableInventorySlot(item);
            if (availableSlot != -1)
            {
                player.Inventory[availableSlot] = item;
                return true;
            }

            /*if (player.Client.Account.Rank == 70)


            {
                player.SendError("Insufficient rank for that item.");
                return false;
            } */

            player.SendError("Not enough space in inventory!");
            return false;
        }
    }
    class SetpieceCommand : Command
    {
        public SetpieceCommand() : base("setpiece", permLevel: 100) { }

        protected override bool Process(Player player, RealmTime time, string setPiece)
        {
            if (String.IsNullOrWhiteSpace(setPiece))
            {
                var type = typeof(ISetPiece);
                var types = AppDomain.CurrentDomain.GetAssemblies()
                    .SelectMany(s => s.GetTypes())
                    .Where(p => type.IsAssignableFrom(p) && !p.IsAbstract);
                var msg = types.Aggregate(
                    "Valid SetPieces: ", (c, p) => c + (p.Name) + ", ");
                player.SendInfo(msg.Substring(0, msg.Length - 2) + ".");
                return false;
            }

            if (!player.Owner.Name.Equals("Nexusaaa"))
            {
                try
                {
                    ISetPiece piece = (ISetPiece)Activator.CreateInstance(Type.GetType(
                    "wServer.realm.setpieces." + setPiece, true, true));
                    piece.RenderSetPiece(player.Owner, new IntPoint((int)player.X + 1, (int)player.Y + 1));
                    return true;
                }
                catch (Exception)
                {
                    player.SendError("Invalid SetPiece.");
                    return false;
                }
            }
            else
            {
                player.SendInfo("/setpiece not allowed in Nexus. (Yes, I'm talking to you Q.)");
                return false;
            }
        }
    }

    class DebugCommand : Command
    {
        class Locater : Enemy
        {
            Player player;
            public Locater(Player player)
                : base(player.Manager, 0x0d5d)
            {
                this.player = player;
                Move(player.X, player.Y);
                ApplyConditionEffect(new ConditionEffect()
                {
                    Effect = ConditionEffectIndex.Invincible,
                    DurationMS = -1
                });
            }
            public override void Tick(RealmTime time)
            {
                Move(player.X, player.Y);
                base.Tick(time);
            }
        }

        public DebugCommand() : base("debug", permLevel: 100, listCommand: false) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var debugything = new Locater(player) { Spawned = true };
            player.Owner.EnterWorld(debugything);
            return true;
        }
    }

    class KillAllCommand : Command
    {
        public KillAllCommand() : base("killAll", permLevel: 90, alias: "ka") { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var iterations = 0;
            var lastKilled = -1;
            var killed = 0;
            while (killed != lastKilled)
            {
                lastKilled = killed;
                foreach (var i in player.Owner.Enemies.Values.Where(e =>
                    e.ObjectDesc != null && e.ObjectDesc.ObjectId != null
                    && e.ObjectDesc.Enemy && e.ObjectDesc.ObjectId != "Tradabad Nexus Crier"
                    && e.ObjectDesc.ObjectId.ContainsIgnoreCase(args)))
                {
                    i.Spawned = true;
                    i.Death(time);
                    killed++;
                }
                if (++iterations >= 5)
                    break;
            }

            player.SendInfo($"{killed} enemy killed!");
            return true;
        }
    }

    class KickCommand : Command
    {
        public KickCommand() : base("kick", permLevel: 80) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            foreach (var i in player.Manager.Clients.Keys)
            {
                if (i.Account.Name.EqualsIgnoreCase(args))
                {
                    // probably if someone is hidden doesn't want to be kicked, so we leave it as before
                    if (i.Account.Hidden)
                        break;

                    i.Disconnect();
                    player.SendInfo("Player disconnected!");
                    return true;
                }
            }
            player.SendError($"Player '{args}' could not be found!");
            return false;
        }
    }

    class GetQuestCommand : Command
    {
        public GetQuestCommand() : base("getQuest", permLevel:90) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            if (player.Quest == null)
            {
                player.SendError("Player does not have a quest!");
                return false;
            }
            player.SendInfo("Quest location: (" + player.Quest.X + ", " + player.Quest.Y + ")");
            return true;
        }
    }

    class OryxSayCommand : Command
    {
        public OryxSayCommand() : base("oryxSay", permLevel: 90, alias: "osay") { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            player.Manager.Chat.Oryx(player.Owner, args);
            return true;
        }
    }

    class AnnounceCommand : Command
    {
        public AnnounceCommand() : base("announce", permLevel: 80) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            player.Manager.Chat.Announce(args);
            return true;
        }
    }

    class SummonCommand : Command
    {
        public SummonCommand() : base("summon", permLevel: 80) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {

            foreach (var i in player.Owner.Players)
            {
                if (i.Value.Name.EqualsIgnoreCase(args))
                {
                    // probably someone hidden doesn't want to be summoned, so we leave it as before here
                    if (i.Value.HasConditionEffect(ConditionEffects.Hidden))
                        break;

                    i.Value.Teleport(time, player.Id, true);
                    i.Value.SendInfo($"You've been summoned by {player.Name}.");
                    player.SendInfo("Player summoned!");
                    return true;
                }
            }
            player.SendError($"Player '{args}' could not be found!");
            return false;
        }
    }

    class SummonAllCommand : Command
    {
        public SummonAllCommand() : base("summonall", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            foreach (var i in player.Owner.Players)
            {
                // probably someone hidden doesn't want to be summoned, so we leave it as before here
                if (i.Value.HasConditionEffect(ConditionEffects.Hidden))
                    break;

                i.Value.Teleport(time, player.Id, true);
                i.Value.SendInfo($"You've been summoned by {player.Name}.");
            }

            player.SendInfo("All players summoned!");
            return true;
        }
    }

    class KillPlayerCommand : Command
    {
        public KillPlayerCommand() : base("killPlayer", permLevel: 100) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            foreach (var i in player.Manager.Clients.Keys)
            {
                if (i.Account.Name.EqualsIgnoreCase(args))
                {
                    i.Player.HP = 0;
                    i.Player.Death(player.Name);
                    player.SendInfo("Player killed!");
                    return true;
                }
            }
            player.SendError($"Player '{args}' could not be found!");
            return false;
        }
    }

    class SizeCommand : Command
    {
        public SizeCommand() : base("size", permLevel: 10) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            if (string.IsNullOrEmpty(args))
            {
                player.SendError("Usage: /size <positive integer>. Using 0 will restore the default size for the sprite.");
                return false;
            }

            var size = Utils.FromString(args);
            var min = player.Rank < 100 ? 75 : 0;
            var max = player.Rank < 100 ? 125 : 500;
            if (size < min && size != 0 || size > max)
            {
                player.SendError($"Invalid size. Size needs to be within the range: {min}-{max}. Use 0 to reset size to default.");
                return false;
            }

            var acc = player.Client.Account;
            acc.Size = size;
            acc.FlushAsync();

            var target = player.IsControlling ? player.SpectateTarget : player;
            if (size == 0)
                target.RestoreDefaultSize();
            else
                target.Size = size;

            return true;
        }
    }

    class RebootCommand : Command
    {
        // Command actually closes the program.
        // An external program is used to monitor the world server existance.
        // If !exist it automatically restarts it.

        public RebootCommand() : base("reboot", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string name)
        {
            var manager = player.Manager;
            var servers = manager.InterServer.GetServerList();

            // display help if no argument supplied
            if (string.IsNullOrEmpty(name))
            {
                var sb = new StringBuilder("Current servers available for rebooting:\n");
                for (var i = 0; i < servers.Length; i++)
                {
                    if (i != 0)
                        sb.Append(", ");

                    sb.Append($"{servers[i].name} [{servers[i].type}]");
                }

                player.SendInfo("Usage: /reboot < server name | $all | $wserver | $account >");
                player.SendInfo(sb.ToString());
                return true;
            }

            // attempt to find server match
            foreach (var server in servers)
            {
                if (!server.name.Equals(name, StringComparison.InvariantCultureIgnoreCase))
                    continue;

                RebootServer(player, 0, server.instanceId);
                player.SendInfo("Reboot command sent.");
                return true;
            }

            // no match found, attempt to match special cases
            switch (name.ToLower())
            {
                case "$all":
                    RebootServer(player, 29000, servers
                        .Select(s => s.instanceId)
                        .ToArray());
                    player.SendInfo("Reboot command sent.");
                    return true;
                case "$wserver":
                    RebootServer(player, 0, servers
                        .Where(s => s.type == ServerType.World)
                        .Select(s => s.instanceId)
                        .ToArray());
                    player.SendInfo("Reboot command sent.");
                    return true;
                case "$account":
                    RebootServer(player, 0, servers
                        .Where(s => s.type == ServerType.Account)
                        .Select(s => s.instanceId)
                        .ToArray());
                    player.SendInfo("Reboot command sent.");
                    return true;
            }

            player.SendInfo("Server not found.");
            return false;
        }



        private void RebootServer(Player issuer, int delay, params string[] instanceIds)
        {
            foreach (var instanceId in instanceIds)
            {
                issuer.Manager.InterServer.Publish(Channel.Control, new ControlMsg()
                {
                    Type = ControlType.Reboot,
                    TargetInst = instanceId,
                    Issuer = issuer.Name,
                    Delay = delay
                });
            }
        }
    }




    class ReSkinCommand : Command
    {
        public ReSkinCommand() : base("reskin", permLevel: 10) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var skins = player.Manager.Resources.GameData.Skins
                .Where(d => d.Value.PlayerClassType == player.ObjectType)
                .Select(d => d.Key)
                .ToArray();

            if (String.IsNullOrEmpty(args))
            {
                var choices = skins.ToCommaSepString();
                player.SendError("Usage: /reskin <positive integer>");
                player.SendError("Choices: " + choices);
                return false;
            }

            var skin = (ushort)Utils.FromString(args);

            if (skin != 0 && !skins.Contains(skin))
            {
                player.SendError("Error setting skin. Either the skin type doesn't exist or the skin is for another class.");
                return false;
            }

            var skinDesc = player.Manager.Resources.GameData.Skins[skin];
            var playerExclusive = skinDesc.PlayerExclusive;
            var size = skinDesc.Size;
            if (playerExclusive != null && !player.Name.Equals(playerExclusive))
            {
                skin = 0;
                size = 100;
            }

            player.SetDefaultSkin(skin);
            player.SetDefaultSize(size);
            return true;
        }
    }

    class MaxCommand : Command
    {
        public MaxCommand() : base("max", permLevel: 40) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var pd = player.Manager.Resources.GameData.Classes[player.ObjectType];

            player.Stats.Base[0] = pd.Stats[0].MaxValue;
            player.Stats.Base[1] = pd.Stats[1].MaxValue;
            player.Stats.Base[2] = pd.Stats[2].MaxValue;
            player.Stats.Base[3] = pd.Stats[3].MaxValue;
            player.Stats.Base[4] = pd.Stats[4].MaxValue;
            player.Stats.Base[5] = pd.Stats[5].MaxValue;
            player.Stats.Base[6] = pd.Stats[6].MaxValue;
            player.Stats.Base[7] = pd.Stats[7].MaxValue;

            player.SendInfo("Your character stats have been maxed.");
            return true;
        }
    }

    /*class ResetServerFame : Command
    {
        public ResetServerFame() : base("resetFame", permLevel: 100) { }

        // resets leaderboards, account stars, and account fame to 0
        protected override bool Process(Player player, RealmTime time, string args)
        {
            // needed to make sure characters connected have fame reset properly
            foreach (var client in player.Manager.Clients.Values)
            {
                client.Player.Experience = 0;
                client.Player.Fame = 0;
                client.Character.Experience = 0;
                client.Character.Fame = 0;
            }

            Program.Stop();

            player.Manager.Database.ResetFame();
            return true;
        }
    }*/

   /* class WipeServer : Command
    {
        public WipeServer() : base("wipeServer", permLevel: 100) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            // close all worlds / disconnect all players
            foreach (var w in player.Manager.Worlds.Values)
            {
                w.Closed = true;
                foreach (var p in w.Players.Values)
                    p.Client.Disconnect();
            }

            player.Manager.Database.Wipe(player.Manager.Resources.GameData);

            Program.Stop();
            return true;
        }
    }
    */

    class TpQuestCommand : Command
    {
        public TpQuestCommand() : base("tq", permLevel: 100) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            if (player.Quest == null)
            {
                player.SendError("Player does not have a quest!");
                return false;
            }

            player.SetNewbiePeriod();
            player.TeleportPosition(time, player.Quest.RealX, player.Quest.RealY, true);
            player.SendInfo("Teleported to Quest Location: (" + player.Quest.X + ", " + player.Quest.Y + ")");
            return true;
        }
    }

    class RankCommand : Command
    {
        public RankCommand() : base("rank", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var index = args.IndexOf(' ');
            if (string.IsNullOrEmpty(args) || index == -1)
            {
                player.SendInfo("Usage: /rank <player name> <rank>\\n0: Normal Player, 20: Donor, 70: Former Staff, 80: GM, 90: Dev, 100: Owner");
                return false;
            }

            var name = args.Substring(0, index);
            var rank = int.Parse(args.Substring(index + 1));

            if (Database.GuestNames.Contains(name, StringComparer.InvariantCultureIgnoreCase))
            {
                player.SendError("Cannot rank unnamed accounts.");
                return false;
            }

            var id = player.Manager.Database.ResolveId(name);
            if (id == player.AccountId)
            {
                player.SendError("Cannot rank self.");
                return false;
            }

            var acc = player.Manager.Database.GetAccount(id);
            if (id == 0 || acc == null)
            {
                player.SendError("Account not found!");
                return false;
            }

            // kick player from server to set rank
            foreach (var i in player.Manager.Clients.Keys)
                if (i.Account.Name.EqualsIgnoreCase(name))
                    i.Disconnect();

            if (acc.Admin && rank < 80)
            {
                // reset account
                player.Manager.Database.WipeAccount(
                    acc, player.Manager.Resources.GameData, player.Name);
                acc.Reload();
            }

            acc.Admin = rank >= 80;
            acc.LegacyRank = rank;
            acc.Hidden = false;
            acc.FlushAsync();

            player.SendInfo($"{acc.Name} given legacy rank {acc.LegacyRank}{((acc.Admin) ? " and now has admin status" : "")}.");
            return true;
        }
    }

    class MuteCommand : Command
    {
        private static readonly Regex CmdParams = new Regex(@"^(\w+)( \d+)?$", RegexOptions.IgnoreCase);

        private readonly RealmManager _manager;

        public MuteCommand(RealmManager manager) : base("mute", permLevel: 80)
        {
            _manager = manager;
            _manager.DbEvents.Expired += HandleUnMute;
        }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var match = CmdParams.Match(args);
            if (!match.Success)
            {
                player?.SendError("Usage: /mute <player name> <time out in minutes>\\n" +
                                 "Time parameter is optional. If left out player will be muted until unmuted.");
                return false;
            }

            // gather arguments
            var name = match.Groups[1].Value;
            var id = _manager.Database.ResolveId(name);
            var acc = _manager.Database.GetAccount(id);
            int timeout;
            if (string.IsNullOrEmpty(match.Groups[2].Value))
            {
                timeout = -1;
            }
            else
            {
                int.TryParse(match.Groups[2].Value, out timeout);
            }

            // run through checks
            if (id == 0 || acc == null)
            {
                player?.SendError("Account not found!");
                return false;
            }
            if (acc.IP == null)
            {
                player?.SendError("Account has no associated IP address. Player must login at least once before being muted.");
                return false;
            }
            if (acc.IP.Equals(player?.Client.Account.IP))
            {
                player?.SendError("Mute failed. That action would cause yourself to be muted (IPs are the same).");
                return false;
            }
            if (acc.Admin)
            {
                player?.SendError("Cannot mute other admins.");
                return false;
            }

            // mute player if currently connected
            foreach (var client in _manager.Clients.Keys
                        .Where(c => c.Player != null && c.IP.Equals(acc.IP) && !c.Player.Client.Account.Admin))
            {
                client.Player.Muted = true;
                client.Player.ApplyConditionEffect(ConditionEffectIndex.Muted);
            }

            if (player != null)
            {
                if (timeout > 0)
                    _manager.Chat.SendInfo(id, "You have been muted by " + player.Name + " for " + timeout + " minutes.");
                else
                    _manager.Chat.SendInfo(id, "You have been muted by " + player.Name + ".");
            }

            // mute ip address
            if (timeout < 0)
            {
                _manager.Database.Mute(acc.IP);
                player?.SendInfo(name + " successfully muted indefinitely.");
            }
            else
            {
                _manager.Database.Mute(acc.IP, TimeSpan.FromMinutes(timeout));
                player?.SendInfo(name + " successfully muted for " + timeout + " minutes.");
            }

            return true;
        }

        private void HandleUnMute(object entity, DbEventArgs expired)
        {
            var key = expired.Message;

            if (!key.StartsWith("mutes:"))
                return;

            foreach (var client in _manager.Clients.Keys.Where(c =>
                c.Player != null && c.IP.Equals(key.Substring(6)) && !c.Player.Client.Account.Admin))
            {
                client.Player.Muted = false;
                client.Player.ApplyConditionEffect(ConditionEffectIndex.Muted, 0);
                client.Player.SendInfo("You are no longer muted. Please do not spam. Thank you.");
            }
        }
    }

    class UnMuteCommand : Command
    {
        public UnMuteCommand() : base("unmute", permLevel: 80) { }

        protected override bool Process(Player player, RealmTime time, string name)
        {
            if (string.IsNullOrWhiteSpace(name))
            {
                player.SendError("Usage: /unmute <player name>");
                return false;
            }

            // gather needed info
            var id = player.Manager.Database.ResolveId(name);
            var acc = player.Manager.Database.GetAccount(id);

            // run checks
            if (id == 0 || acc == null)
            {
                player.SendError("Account not found!");
                return false;
            }
            if (acc.IP == null)
            {
                player.SendError("Account has no associated IP address. Player must login at least once before being unmuted.");
                return false;
            }

            // unmute ip address
            player.Manager.Database.IsMuted(acc.IP).ContinueWith(t =>
            {
                if (!t.IsCompleted)
                {
                    player.SendInfo("Db access error while trying to unmute.");
                    return;
                }

                if (t.Result)
                {
                    player.Manager.Database.Mute(acc.IP, TimeSpan.FromSeconds(1));
                    player.SendInfo(name + " successfully unmuted.");
                }
                else
                {
                    player.SendInfo(name + " wasn't muted...");
                }
            });

            // expire event will handle unmuting of connected players
            return true;
        }
    }

    class BanAccountCommand : Command
    {
        public BanAccountCommand() : base("ban", permLevel: 80) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            BanInfo bInfo;
            if (args.StartsWith("{"))
            {
                bInfo = Utils.FromJson<BanInfo>(args);
            }
            else
            {
                bInfo = new BanInfo();

                // validate command
                var rgx = new Regex(@"^(\w+) (.+)$");
                var match = rgx.Match(args);
                if (!match.Success)
                {
                    player.SendError("Usage: /ban <account id or name> <reason>");
                    return false;
                }

                // get info from args
                bInfo.Name = match.Groups[1].Value;
                if (!int.TryParse(bInfo.Name, out bInfo.accountId))
                {
                    bInfo.accountId = player.Manager.Database.ResolveId(bInfo.Name);
                }
                bInfo.banReasons = match.Groups[2].Value;
                bInfo.banLiftTime = -1;
            }

            // run checks
            if (Database.GuestNames.Any(n => n.ToLower().Equals(bInfo.Name?.ToLower())))
            {
                player.SendError("If you specify a player name to ban, the name needs to be unique.");
                return false;
            }
            if (bInfo.accountId == 0)
            {
                player.SendError("Account not found...");
                return false;
            }
            if (string.IsNullOrWhiteSpace(bInfo.banReasons))
            {
                player.SendError("A reason must be provided.");
                return false;
            }
            var acc = player.Manager.Database.GetAccount(bInfo.accountId);
            if (player.AccountId != acc.AccountId && player.Rank <= acc.Rank)
            {
                player.SendError("Cannot ban players of equal or higher rank than yourself.");
                return false;
            }

            // ban player + disconnect if currently connected
            player.Manager.Database.Ban(bInfo.accountId, bInfo.banReasons, bInfo.banLiftTime);
            var target = player.Manager.Clients.Keys
                .SingleOrDefault(c => c.Account != null && c.Account.AccountId == bInfo.accountId);
            target?.Disconnect();

            player.SendInfo(!string.IsNullOrEmpty(bInfo.Name) ?
                $"{bInfo.Name} successfully banned." :
                "Ban successful.");

            SendWebHook.Post(selectedTitle: "Someone banned via BAN Command!", selectedDescription: "Someone banned via BAN Command!", name: bInfo.Name, selectedType: 1, reason: bInfo.banReasons, whoBanned: player.Name);

            return true;
        }

        private class BanInfo
        {
            public int accountId;
            public string Name;
            public string banReasons;
            public int banLiftTime;
        }
    }

    class BanIPCommand : Command
    {
        public BanIPCommand() : base("banip", permLevel: 80, alias: "ipban") { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var manager = player.Manager;
            var db = manager.Database;

            // validate command
            var rgx = new Regex(@"^(\w+) (.+)$");
            var match = rgx.Match(args);
            if (!match.Success)
            {
                player.SendError("Usage: /banip <account id or name> <reason>");
                return false;
            }

            // get info from args
            int id;
            var idstr = match.Groups[1].Value;
            if (!int.TryParse(idstr, out id))
            {
                id = db.ResolveId(idstr);
            }
            var reason = match.Groups[2].Value;

            // run checks
            if (Database.GuestNames.Any(n => n.ToLower().Equals(idstr.ToLower())))
            {
                player.SendError("If you specify a player name to ban, the name needs to be unique.");
                return false;
            }
            if (id == 0)
            {
                player.SendError("Account not found...");
                return false;
            }
            if (string.IsNullOrWhiteSpace(reason))
            {
                player.SendError("A reason must be provided.");
                return false;
            }
            var acc = db.GetAccount(id);
            if (string.IsNullOrEmpty(acc.IP))
            {
                player.SendError("Failed to ip ban player. IP not logged...");
                return false;
            }
            if (player.AccountId != acc.AccountId && acc.IP.Equals(player.Client.Account.IP))
            {
                player.SendError("IP ban failed. That action would cause yourself to be banned (IPs are the same).");
                return false;
            }
            if (player.AccountId != acc.AccountId && player.Rank <= acc.Rank)
            {
                player.SendError("Cannot ban players of equal or higher rank than yourself.");
                return false;
            }

            // ban
            db.Ban(acc.AccountId, reason);
            db.BanIp(acc.IP, reason);

            // disconnect currently connected
            var targets = manager.Clients.Keys.Where(c => c.IP.Equals(acc.IP));
            foreach (var t in targets)
                t.Disconnect();

            // send notification
            player.SendInfo($"Banned {acc.Name} (both account and ip).");

            SendWebHook.Post(selectedTitle: "Someone banned via BANIP Command!", selectedDescription: "Someone banned via BANIP Command!", name: acc.Name, selectedType: 2, ipAdress: acc.IP, reason: reason, whoBanned: player.Name);

            return true;
        }
    }

    class UnBanAccountCommand : Command
    {
        public UnBanAccountCommand() : base("unban", permLevel: 80) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var db = player.Manager.Database;

            // validate command
            var rgx = new Regex(@"^(\w+)$");
            if (!rgx.IsMatch(args))
            {
                player.SendError("Usage: /unban <account id or name>");
                return false;
            }

            // get info from args
            int id;
            if (!int.TryParse(args, out id))
                id = db.ResolveId(args);

            // run checks
            if (id == 0)
            {
                player.SendError("Account doesn't exist...");
                return false;
            }

            var acc = db.GetAccount(id);

            // unban
            var banned = db.UnBan(id);
            var ipBanned = acc.IP != null && db.UnBanIp(acc.IP);

            // send notification
            if (!banned && !ipBanned)
            {
                player.SendInfo($"{acc.Name} wasn't banned...");
                return true;
            }
            if (banned && ipBanned)
            {
                SendWebHook.Post(selectedTitle: "Someone unbanned via UNBAN Command!", selectedDescription: "Someone unbanned via UNBAN Command!", name: acc.Name, selectedType: 3, ipAdress: acc.IP, whoBanned: player.Name);
                player.SendInfo($"Success! {acc.Name}'s account and IP no longer banned.");
                return true;
            }
            if (banned)
            {
                SendWebHook.Post(selectedTitle: "Someone unbanned via UNBAN Command!", selectedDescription: "Someone unbanned via UNBAN Command!", name: acc.Name, selectedType: 3, ipAdress: acc.IP, whoBanned: player.Name);
                player.SendInfo($"Success! {acc.Name}'s account no longer banned.");
                return true;
            }

            player.SendInfo($"Success! {acc.Name}'s IP no longer banned.");

            SendWebHook.Post(selectedTitle: "Someone unbanned via UNBAN Command!", selectedDescription: "Someone unbanned via UNBAN Command!", name: acc.Name, selectedType: 3, ipAdress: acc.IP, whoBanned: player.Name);

            return true;
        }
    }

    class ClearInvCommand : Command
    {
        public ClearInvCommand() : base("clearinv", permLevel: 80) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            for (int i = 4; i < 12; i++)
                player.Inventory[i] = null;
            player.SendInfo("Inventory Cleared.");
            return true;
        }
    }

    class MusicCommand : Command
    {
        public MusicCommand() : base("music", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string music)
        {
            var resources = player.Manager.Resources;

            if (string.IsNullOrWhiteSpace(music))
            {
                var msg = resources.MusicNames.Aggregate(
                    "Music Choices: ", (c, p) => c + (p + ", "));
                player.SendInfo(msg.Substring(0, msg.Length - 2) + ".");
                return false;
            }

            var properName = resources.MusicNames
                .FirstOrDefault(s => s.Equals(music, StringComparison.InvariantCultureIgnoreCase));
            if (properName == null)
            {
                player.SendError($"Music \"{music}\" not found!");
                return false;
            }

            var owner = player.Owner;
            owner.Music = properName;

            foreach (var plr in owner.Players.Values)
                plr.SendInfo($"World music changed to {properName}.");

            var i = 0;
            foreach (var plr in owner.Players.Values)
            {
                owner.Timers.Add(new WorldTimer(100 * i, (w, t) =>
                {
                    if (plr == null)
                        return;

                    plr.Client.SendPacket(new SwitchMusic()
                    {
                        Music = properName
                    });
                }));
                i++;
            }
            return true;
        }
    }

    class CloseRealmCommand : Command
    {
        public CloseRealmCommand() : base("closerealm", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var gw = player.Manager.Worlds[World.Realm] as Realm;

            if (gw == null)
            {
                player.SendError("An undefined error occurred.");
                return false;
            }

            if (gw.IsClosing())
            {
                player.SendError("Realm already closing.");
                return false;
            }

            gw.CloseRealm();
            return true;
        }
    }

    class QuakeCommand : Command
    {
        public QuakeCommand() : base("quake", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string worldName)
        {
            var worldProtoData = player.Manager.Resources.Worlds.Data;

            if (String.IsNullOrWhiteSpace(worldName))
            {
                var msg = worldProtoData.Aggregate(
                    "Valid World Names: ", (c, p) => c + ((!p.Value.setpiece) ? (p.Key + ", ") : ""));
                player.SendInfo(msg.Substring(0, msg.Length - 2) + ".");
                return false;
            }

            if (player.Owner is Nexus)
            {
                player.SendError("Cannot use /quake in Nexus.");
                return false;
            }

            var worldNameProper =
                player.Manager.Resources.Worlds.Data.FirstOrDefault(
                    p => p.Key.Equals(worldName, StringComparison.InvariantCultureIgnoreCase)).Key;

            ProtoWorld proto;
            if (worldNameProper == null || (proto = worldProtoData[worldNameProper]).setpiece)
            {
                player.SendError("Invalid world.");
                return false;
            }

            World world;
            if (proto.persist)
                world = player.Manager.Worlds[proto.id];
            else
            {
                DynamicWorld.TryGetWorld(proto, player.Client, out world);
                world = player.Manager.AddWorld(world ?? new World(proto));
            }

            player.Owner.QuakeToWorld(world);
            return true;
        }
    }

    class VisitCommand : Command
    {
        public VisitCommand() : base("visit", permLevel: 80) { }

        protected override bool Process(Player player, RealmTime time, string name)
        {
            if (string.IsNullOrWhiteSpace(name))
            {
                player.SendInfo("Usage: /visit <player name>");
                return true;
            }

            var target = player.Manager.Clients.Keys
                .SingleOrDefault(c => c.Account != null &&
                                      c.Account.Name.Equals(name, StringComparison.InvariantCultureIgnoreCase));

            if (target?.Player?.Owner == null ||
                !target.Player.CanBeSeenBy(player))
            {
                player.SendError("Player not found!");
                return false;
            }

            var owner = target.Player.Owner;
            player.Client.Reconnect(new Reconnect()
            {
                Host = "",
                GameId = owner.Id,
                Name = owner.SBName
            });
            return true;
        }
    }

    class HideCommand : Command
    {
        public HideCommand() : base("hide", permLevel: 80, alias: "h") { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var acc = player.Client.Account;

            acc.Hidden = !acc.Hidden;
            acc.FlushAsync();

            if (acc.Hidden)
            {
                player.ApplyConditionEffect(ConditionEffectIndex.Hidden);
                player.ApplyConditionEffect(ConditionEffectIndex.Invincible);
                player.Manager.Clients[player.Client].Hidden = true;
            }
            else
            {
                player.ApplyConditionEffect(ConditionEffectIndex.Hidden, 0);
                player.ApplyConditionEffect(ConditionEffectIndex.Invincible, 0);
                player.Manager.Clients[player.Client].Hidden = false;
            }

            return true;
        }
    }

    class GlowCommand : Command
    {
        public GlowCommand() : base("glow", permLevel: 10) { }

        protected override bool Process(Player player, RealmTime time, string color)
        {
            if (String.IsNullOrWhiteSpace(color))
            {
                player.SendInfo("Usage: /glow <color>");
                return true;
            }

            player.Glow = Utils.FromString(color);

            var acc = player.Client.Account;
            acc.GlowColor = player.Glow;
            acc.FlushAsync();

            return true;
        }
    }

    class LinkCommand : Command
    {
        public LinkCommand() : base("link", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            if (player?.Owner == null)
                return false;

            var world = player.Owner;
            if (world.Id < 0 || (player.Rank < 80 && !(world is Test)))
            {
                player.SendError("Forbidden.");
                return false;
            }

            if (!player.Manager.Monitor.AddPortal(world.Id))
            {
                player.SendError("Link already exists.");
                return false;
            }

            return true;
        }
    }

    class UnLinkCommand : Command
    {
        public UnLinkCommand() : base("unlink", permLevel: 8) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            if (player?.Owner == null)
                return false;

            var world = player.Owner;
            if (world.Id < 0 || (player.Rank < 80 && !(world is Test)))
            {
                player.SendError("Forbidden.");
                return false;
            }


            if (!player.Manager.Monitor.RemovePortal(player.Owner.Id))
                player.SendError("Link not found.");
            else
                player.SendInfo("Link removed.");

            return true;
        }
    }

    class GiftCommand : Command
    {
        public GiftCommand() : base("gift", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            if (player == null)
                return false;

            var manager = player.Manager;

            // verify argument
            var index = args.IndexOf(' ');
            if (string.IsNullOrWhiteSpace(args) || index == -1)
            {
                player.SendInfo("Usage: /gift <player name> <item name>");
                return false;
            }

            // get command args
            var playerName = args.Substring(0, index);
            var item = GetItem(player, args.Substring(index + 1));
            if (item == null)
            {
                return false;
            }

            // get player account
            if (Database.GuestNames.Contains(playerName, StringComparer.InvariantCultureIgnoreCase))
            {
                player.SendError("Cannot gift the unnamed...");
                return false;
            }
            var id = manager.Database.ResolveId(playerName);
            var acc = manager.Database.GetAccount(id);
            if (id == 0 || acc == null)
            {
                player.SendError("Account not found!");
                return false;
            }

            // add gift
            var result = player.Manager.Database.AddGift(acc, item.ObjectType);
            if (!result)
            {
                player.SendError("Gift not added. Something happened with the adding process.");
                return false;
            }

            // send out success notifications
            player.SendInfoFormat("You gifted {0} one {1}.", acc.Name, item.DisplayName);
            var gifted = player.Manager.Clients.Keys
                .SingleOrDefault(p => p.Account.AccountId == acc.AccountId);
            gifted?.Player?.SendInfoFormat(
                "You received a gift from {0}. Enjoy your {1}.",
                player.Name,
                item.DisplayName);
            return true;
        }

        private Item GetItem(Player player, string itemName)
        {
            var gameData = player.Manager.Resources.GameData;

            ushort objType;

            // allow both DisplayId and Id for query
            if (!gameData.DisplayIdToObjectType.TryGetValue(itemName, out objType))
            {
                if (!gameData.IdToObjectType.TryGetValue(itemName, out objType))
                    player.SendError("Unknown item type!");
                return null;
            }

            if (!gameData.Items.ContainsKey(objType))
            {
                player.SendError("Not an item!");
                return null;
            }


            return gameData.Items[objType];
        }
    }

  /*  class RemoveServerGold : Command
    {
        public RemoveServerGold() : base("removeAllGold", permLevel: 100) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            // close all worlds / disconnect all players
            foreach (var w in player.Manager.Worlds.Values)
            {
                w.Closed = true;
                foreach (var p in w.Players.Values)
                    p.Client.Disconnect();
            }

            player.Manager.Database.RemoveAllGold();

            Program.Stop();
            return true;
        }
    }

    /*class WellcomePrivateMessage : Command
    {
        public WellcomePrivateMessage() : base("addWelcomeMessage", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            if (player.Name != "ossimc82" && player.Name != "TheRegal")
                return false;

            var numAccounts = int.Parse(player.Manager.Database.Conn.StringGet("nextAccId"));
            for (var i = 1; i <= numAccounts; i++)
            {
                try
                {
                    var acc = new DbAccount(player.Manager.Database.Conn, i);
                    acc.AddPrivateMessage(player.AccountId, "Welcome to the new Message System",
                        "Hello guys, this is the new offline message system, this message was auto-generated by the server to show you how it works. Its very simple, just click on the \"SEND NEW MESSAGE\" button to send a new message. Please report any bugs on the forums or send me (ossimc82) a pm\n\n- ossimc82\n- Nillys Realm Team");
                    Client c;
                    if ((c = player.Manager.Clients.Keys.FirstOrDefault(_ => _?.Account?.AccountId == acc.AccountId)) !=
                        null)
                    {
                        c.SendPacket(new GlobalNotification
                        {
                            Text = "forcePrivateMessageRefresh"
                        });
                    }
                }
                catch (Exception)
                {
                }
            }
            return true;
        }
    }*/

    class OverrideAccountCommand : Command
    {
        public OverrideAccountCommand() : base("override", permLevel: 100) { }

        protected override bool Process(Player player, RealmTime time, string name)
        {
            var acc = player.Client.Account;

            if (string.IsNullOrWhiteSpace(name))
            {
                player.SendError("Usage: /override <player name>");
                return false;
            }

            var id = player.Manager.Database.ResolveId(name);
            if (id == 0)
            {
                player.SendError("Account not found!");
                return false;
            }

            acc.AccountIdOverride = id;
            acc.FlushAsync();
            player.SendInfo("Account override set.");
            return true;
        }
    }

    internal class Level20Command : Command
    {
        public Level20Command(RealmManager manager) : base("level20", permLevel: 0, alias: "l20")
        {
            _manager = manager;
        }

        private readonly RealmManager _manager;

        protected override bool Process(Player player, RealmTime time, string args)
        {
            if (player.Level < 20)
            {
                var statInfo = _manager.Resources.GameData.Classes[player.ObjectType].Stats;
                for (var v = 0; v < statInfo.Length; v++)
                {
                    player.Stats.Base[v] +=
                    (statInfo[v].MaxIncrease + statInfo[v].MinIncrease) * (21 - player.Level) / 2;
                    if (player.Stats.Base[v] > statInfo[v].MaxValue)
                        player.Stats.Base[v] = statInfo[v].MaxValue;
                }
                player.Level = 20;
                return true;
            }
            return false;
        }
    }


    class RenameCommand : Command
    {
        public RenameCommand() : base("rename", permLevel: 80) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var index = args.IndexOf(' ');
            if (string.IsNullOrWhiteSpace(args) || index == -1)
            {
                player.SendInfo("Usage: /rename <player name> <new player name>");
                return false;
            }

            var playerName = args.Substring(0, index);
            var newPlayerName = args.Substring(index + 1);

            var id = player.Manager.Database.ResolveId(playerName);
            if (id == 0)
            {
                player.SendError("Player account not found!");
                return false;
            }

            if (newPlayerName.Length < 3 || newPlayerName.Length > 15 || !newPlayerName.All(char.IsLetter) ||
                Database.GuestNames.Contains(newPlayerName, StringComparer.InvariantCultureIgnoreCase))
            {
                player.SendError("New name is invalid. Must be between 3-15 char long and contain only letters.");
                return false;
            }

            string lockToken = null;
            var key = Database.NAME_LOCK;
            var db = player.Manager.Database;

            try
            {
                while ((lockToken = db.AcquireLock(key)) == null) ;

                if (db.Conn.HashExists("names", newPlayerName.ToUpperInvariant()))
                {
                    player.SendError("Name already taken");
                    return false;
                }

                var acc = db.GetAccount(id);
                if (acc == null)
                {
                    player.SendError("Account doesn't exist.");
                    return false;
                }

                using (var l = db.Lock(acc))
                    if (db.LockOk(l))
                    {
                        while (!db.RenameIGN(acc, newPlayerName, lockToken)) ;
                        player.SendInfo("Rename successful.");
                    }
                    else
                        player.SendError("Account in use.");
            }
            finally
            {
                if (lockToken != null)
                    db.ReleaseLock(key, lockToken);
            }

            return true;
        }
    }

    class UnnameCommand : Command
    {
        public UnnameCommand() : base("unname", permLevel: 80) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            if (string.IsNullOrWhiteSpace(args))
            {
                player.SendInfo("Usage: /unname <player name>");
                return false;
            }

            var playerName = args;

            var id = player.Manager.Database.ResolveId(playerName);
            if (id == 0)
            {
                player.SendError("Player account not found!");
                return false;
            }

            string lockToken = null;
            var key = Database.NAME_LOCK;
            var db = player.Manager.Database;

            try
            {
                while ((lockToken = db.AcquireLock(key)) == null) ;

                var acc = db.GetAccount(id);
                if (acc == null)
                {
                    player.SendError("Account doesn't exist.");
                    return false;
                }

                using (var l = db.Lock(acc))
                    if (db.LockOk(l))
                    {
                        while (!db.UnnameIGN(acc, lockToken)) ;
                        player.SendInfo("Account succesfully unnamed.");
                    }
                    else
                        player.SendError("Account in use.");
            }
            finally
            {
                if (lockToken != null)
                    db.ReleaseLock(key, lockToken);
            }

            return true;
        }
    }

    class WargCommand : Command
    {
        public WargCommand() : base("warg", permLevel: 100) { }

        protected override bool Process(Player player, RealmTime time, string name)
        {
            if (string.IsNullOrWhiteSpace(name))
            {
                player.SendError("Usage: /warg <mob name>");
                return false;
            }

            var target = player.GetNearestEntityByName(2900, name);
            if (target == null)
            {
                player.SendError("Mob not found.");
                return false;
            }

            if (target.Controller != null)
            {
                player.SendError("Only one person can control a mob at a time.");
                return false;
            }

            if (player.SpectateTarget != null)
            {
                player.SpectateTarget.FocusLost -= player.ResetFocus;
                player.SpectateTarget.Controller = null;
            }

            player.ApplyConditionEffect(ConditionEffectIndex.Paused);
            target.FocusLost += player.ResetFocus;
            target.Controller = player;
            player.SpectateTarget = target;
            player.Sight.UpdateCount++;

            player.Owner.Timers.Add(new WorldTimer(500, (w, t) =>
            {
                player.Client.SendPacket(new SetFocus()
                {
                    ObjectId = target.Id
                });
            }));
            return true;
        }
    }

    class CompactLOHCommand : Command
    {
        public CompactLOHCommand() : base("compactLOH", permLevel: 100, listCommand: false) { }

        protected override bool Process(Player player, RealmTime time, string name)
        {
            GCSettings.LargeObjectHeapCompactionMode = GCLargeObjectHeapCompactionMode.CompactOnce;
            GC.Collect();
            return true;
        }
    }

    class SetFameCommand : Command
    {
        public SetFameCommand() : base("setfame", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var index = args.IndexOf(' ');
            var playerName = args.Substring(0, index);
            var fame = Int32.Parse(args.Substring(index + 1));

            if (Database.GuestNames.Contains(playerName, StringComparer.InvariantCultureIgnoreCase))
            {
                player.SendError("Cannot set fame of the unnamed...");
                return false;
            }
            var id = player.Manager.Database.ResolveId(playerName);
            var acc = player.Manager.Database.GetAccount(id);
            if (id == 0 || acc == null)
            {
                player.SendError("Account not found!");
                return false;
            }


            acc.FlushAsync();
            var target = player.Manager.Clients.Keys.SingleOrDefault(p => p.Account.AccountId == acc.AccountId);
            target.Account.TotalFame = fame;
            target.Account.Fame = fame;
            player.SendInfo("Success!");
            target.Player.SendInfo("Your fame set in to this : " + fame);
            player.ForceUpdate(player.Fame);
            player.ForceUpdate(player.CurrentFame);
            player.ForceUpdate(player.FameGoal);
            return true;
        }
    }

    class SetGoldCommand : Command
    {
        public SetGoldCommand() : base("setgold", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var index = args.IndexOf(' ');
            var playerName = args.Substring(0, index);
            var fame = Int32.Parse(args.Substring(index + 1));

            var id = player.Manager.Database.ResolveId(playerName);
            var acc = player.Manager.Database.GetAccount(id);
            if (id == 0 || acc == null)
            {
                player.SendError("Account not found!");
                return false;
            }
            acc.FlushAsync();
            player.SendInfo("Success!");
            var target = player.Manager.Clients.Keys.SingleOrDefault(p => p.Account.AccountId == acc.AccountId);
            target.Account.Credits = fame;
            target.Player.Credits = fame;
            target.Player.SendInfo("Your gold set in to this : " + fame);
            player.ForceUpdate(player.Credits);
            return true;
        }
    }

    class SetPrestigeCommand : Command
    {
        public SetPrestigeCommand() : base("setprestige", permLevel: 90) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var index = args.IndexOf(' ');
            var playerName = args.Substring(0, index);
            var fame = Int32.Parse(args.Substring(index + 1));

            var id = player.Manager.Database.ResolveId(playerName);
            var acc = player.Manager.Database.GetAccount(id);
            if (id == 0 || acc == null)
            {
                player.SendError("Account not found!");
                return false;
            }
            acc.FlushAsync();
            player.SendInfo("Success!");
            var target = player.Manager.Clients.Keys.SingleOrDefault(p => p.Account.AccountId == acc.AccountId);
            target.Account.Prestige = fame;
            target.Player.Prestige = fame;
            target.Player.SendInfo("Your prestige set in to this : " + fame);
            player.ForceUpdate(player.Prestige);
            player.ForceUpdate(target.Account.Prestige);
            return true;
        }
    }

    internal class DonorShopCommand : Command
    {
        public DonorShopCommand() : base("donorshop", permLevel: 10) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            player.Client.Reconnect(new Reconnect()
            {
                Host = "",
                Port = 2050,
                GameId = -16,
                Name = "Donor Shop"
            });
            return true;
        }
    }

    internal class SetStarCommand : Command
    {
        public SetStarCommand() : base("setstar", permLevel: 100) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            var index = args.IndexOf(' ');
            var playerName = args.Substring(0, index);
            var star = Int32.Parse(args.Substring(index + 1));
            if (star >= 71)
            {
                return false;
            }
            if (star <= -1)
            {
                return false;
            }

            var id = player.Manager.Database.ResolveId(playerName);
            var acc = player.Manager.Database.GetAccount(id);
            if (id == 0 || acc == null)
            {
                player.SendError("Account not found!");
                return false;
            }
            acc.FlushAsync();
            player.SendInfo("Success!");
            var target = player.Manager.Clients.Keys.SingleOrDefault(p => p.Account.AccountId == acc.AccountId);
            target.Player.Stars = star;
            target.Player.SendInfo("Your star set in to this : " + star);
            player.ForceUpdate(player.Stars);
            return true;
        }
    }

    class GearCommand : Command
    {
        public GearCommand() : base("Set", permLevel: 0) { }

        protected override bool Process(Player player, RealmTime time, string args)
        {
            string item = args;

            if (player.Inventory[4] != null || player.Inventory[5] != null || player.Inventory[6] != null || player.Inventory[7] != null)
            {
                player.SendHelp("Not enough space");
                return false;
            }
            if (player.ObjectType == 2)
            {

            }
            switch (item)
            {
                case "rogue":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa19];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa59];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xad3];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];

                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Rogue class' gear");
                    break;
                case "Rogue":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa19];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa59];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xad3];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Rogue class' gear");
                    break;
                case "archer":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa1e];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa65];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xad3];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Archer class' gear");
                    break;
                case "Archer":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa1e];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa65];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xad3];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Archer class' gear");
                    break;
                case "wizard":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa9f];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xad6];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa60];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Wizard class' gear");
                    break;
                case "Wizard":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa9f];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xad6];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa60];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Wizard class' gear");
                    break;
                case "priest":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa07];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa33];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa60];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Priest class' gear");
                    break;
                case "Priest":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa07];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa33];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa60];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Priest class' gear");
                    break;
                case "warrior":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa82];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa6a];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa13];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Warrior class' gear");
                    break;
                case "Warrior":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa82];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa6a];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa13];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Warrior class' gear");
                    break;
                case "knight":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa82];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa0b];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa13];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Knight class' gear");
                    break;
                case "Knight":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa82];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa0b];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa13];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Knight class' gear");
                    break;
                case "paladin":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa82];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa54];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa13];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Paladin class' gear");
                    break;
                case "Paladin":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa82];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa54];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa13];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Paladin class' gear");
                    break;
                case "assassin":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa19];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xaa7];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xad3];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Assassin class' gear");
                    break;
                case "Assassin":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa19];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xaa7];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xad3];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Assassin class' gear");
                    break;
                case "necromancer":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa9f];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xaae];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa60];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Necromancer class' gear");
                    break;
                case "Necromancer":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa9f];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xaae];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa60];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Necromancer class' gear");
                    break;
                case "huntress":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa1e];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xab5];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xad3];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Huntress class' gear");
                    break;
                case "Huntress":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa1e];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xab5];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xad3];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Huntress class' gear");
                    break;
                case "mystic":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa9f];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa45];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa60];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained mystic gear");
                    break;
                case "Mystic":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa9f];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xa45];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa60];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained mystic gear");
                    break;
                case "trickster":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa19];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xb1f];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xad3];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Trickster class' gear");
                    break;
                case "Trickster":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa19];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xb1f];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xad3];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Trickster class' gear");
                    break;
                case "sorcerer":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa07];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xb31];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa60];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Sorcerer class' gear");
                    break;
                case "Sorcerer":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xa07];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xb31];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xa60];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Sorcerer class' gear");
                    break;
                case "ninja":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xc4];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xc57];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xad3];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Ninja class' gear");
                    break;
                case "Ninja":
                    player.Inventory[4] = player.Manager.Resources.GameData.Items[0xc4];
                    player.Inventory[5] = player.Manager.Resources.GameData.Items[0xc57];
                    player.Inventory[6] = player.Manager.Resources.GameData.Items[0xad3];
                    player.Inventory[7] = player.Manager.Resources.GameData.Items[0xabd];
                    player.UpdateCount++;
                    player.SendInfo("Successfully obtained Ninja class' gear");
                    break;
                default:
                    player.SendError("Usage: /set <ClassName>");
                    player.SendInfo("Classes: Rogue, Archer, Wizard, Priest, Warrior, Knight, Paladin, Assassin, Necromancer, Huntress, Mystic, Trickster, Sorcerer, Ninja");
                    break;
            }
            return true;
        }
    }
}


