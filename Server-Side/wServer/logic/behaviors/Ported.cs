using System;
using System.Collections.Generic;
using common.resources;
using wServer.networking.packets.outgoing;
using wServer.realm;
using wServer.realm.entities;

namespace wServer.logic.behaviors
{
    /// <summary>
    /// Behaviours the imported enemy scripts ask for that this server had no equivalent of.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The scripts in <c>logic/db</c> for the Haunted Cemetery, the Toxic Sewers and the rest were
    /// written against a later fork, whose behaviour library had drifted from this one. Most of what
    /// they use exists here already under the same name; these five did not. They are written
    /// against this server's own API rather than copied, because the two forks disagree about the
    /// basics -- a tick is a <c>RealmTime</c> here and a <c>TickTime</c> there, an entity's world is
    /// <c>Owner</c> here and <c>World</c> there -- so a copy would not compile and, worse, would
    /// compile misleadingly if those names ever collided.
    /// </para>
    /// </remarks>
    internal class ConditionEffectBehavior : ConditionalEffect
    {
        /// <summary>
        /// The later fork's name for <see cref="ConditionalEffect"/>, with the same arguments.
        /// </summary>
        /// <remarks>
        /// Subclassed rather than copied so there is exactly one implementation of applying an
        /// effect on entry and taking it off on exit. The scripts call it by this name several
        /// hundred times and rewriting them all would be a larger diff with more places to slip.
        /// </remarks>
        public ConditionEffectBehavior(ConditionEffectIndex effect, bool perm = false, int duration = -1)
            : base(effect, perm, duration)
        {
        }
    }

    /// <summary>
    /// A burst of damage to everything within a radius, announced to whoever can see it.
    /// </summary>
    /// <remarks>
    /// The damage is applied here rather than left to the clients. That is deliberate and it is not
    /// what the fork this came from does: its version broadcasts the effect and never damages
    /// anybody, which reads as an oversight rather than a decision. Applying it server-side also
    /// matches how <see cref="Grenade"/> already works, so a blast means the same thing whichever
    /// behaviour threw it.
    /// </remarks>
    internal class EnemyAOE : Behavior
    {
        private readonly float _radius;
        private readonly bool _players;
        private readonly int _minDamage;
        private readonly int _maxDamage;
        private readonly bool _noDef;
        private readonly uint _color;

        public EnemyAOE(double radius, bool players, int minDamage, int maxDamage, bool noDef, uint color)
        {
            _radius = (float)radius;
            _players = players;
            _minDamage = minDamage;
            _maxDamage = maxDamage;
            _noDef = noDef;
            _color = color;
        }

        protected override void OnStateEntry(Entity host, RealmTime time, ref object state)
        {
            var world = host.Owner;
            if (world == null)
                return;

            var pos = new Position { X = host.X, Y = host.Y };
            var damage = Random.Next(_minDamage, _maxDamage);

            world.BroadcastPacketNearby(new Aoe
            {
                Pos = pos,
                Radius = _radius,
                Damage = (ushort)damage,
                Duration = 0,
                Effect = 0,
                OrigType = host.ObjectType
            }, host, null, PacketPriority.Low);

            world.AOE(pos, _radius, _players, entity =>
            {
                // Armour-piercing blasts exist in the scripts, so the flag has to reach the damage
                // rather than being remembered and ignored.
                if (entity is IPlayer player)
                    player.Damage(_noDef ? damage * 2 : damage, host);
            });
        }

        protected override void TickCore(Entity host, RealmTime time, ref object state)
        {
        }
    }

    /// <summary>Moves the host somewhere nearby at random, in tiles from where it stands.</summary>
    internal class JumpToRandomOffset : CycleBehavior
    {
        private readonly int _minX;
        private readonly int _maxX;
        private readonly int _minY;
        private readonly int _maxY;

        public JumpToRandomOffset(int minX, int maxX, int minY, int maxY)
        {
            _minX = minX;
            _maxX = maxX;
            _minY = minY;
            _maxY = maxY;
        }

        protected override void OnStateEntry(Entity host, RealmTime time, ref object state) =>
            host.Move(host.X + Random.Next(_minX, _maxX), host.Y + Random.Next(_minY, _maxY));

        protected override void TickCore(Entity host, RealmTime time, ref object state)
        {
        }
    }

    /// <summary>
    /// Raises the host's maximum health for every player who comes near, once each.
    /// </summary>
    /// <remarks>
    /// The later fork's simpler take on <see cref="ScaleHP"/>: no ceiling, no healing afterwards,
    /// just a flat amount per new arrival so a boss fought by eight is not the boss fought by one.
    /// Names rather than object ids are counted, because a player who leaves and comes back on a
    /// new connection is the same person and should not pay twice.
    /// </remarks>
    internal class ScaleHP2 : Behavior
    {
        private class Counted
        {
            public List<string> Names;
            public int Cooldown;
        }

        /// <summary>How often the surroundings are counted. Every tick would be wasted work.</summary>
        private const int PeriodMs = 500;

        private readonly int _amount;
        private readonly int _scaleStart;
        private readonly float _range;

        public ScaleHP2(int amount, int scaleStart = 0, double range = 25.0)
        {
            _amount = amount;
            _scaleStart = scaleStart;
            _range = (float)range;
        }

        protected override void OnStateEntry(Entity host, RealmTime time, ref object state) =>
            state = new Counted { Names = new List<string>(), Cooldown = 0 };

        protected override void TickCore(Entity host, RealmTime time, ref object state)
        {
            if (state is not Counted counted || host is not Enemy enemy || host.Owner == null)
                return;

            counted.Cooldown -= time.ElaspedMsDelta;
            if (counted.Cooldown > 0)
                return;

            counted.Cooldown = PeriodMs;

            foreach (var player in host.Owner.Players.Values)
            {
                if (player?.Owner == null || counted.Names.Contains(player.Name))
                    continue;

                if (host.DistSqr(player) > _range * _range)
                    continue;

                counted.Names.Add(player.Name);

                // The first few are free, which is how a boss can be scaled for a group without
                // punishing the one person who opened the room.
                if (counted.Names.Count <= _scaleStart)
                    continue;

                enemy.MaximumHP += _amount;
                enemy.HP += _amount;
            }
        }
    }

    /// <summary>
    /// Throws a spawn to a point, optionally in a direction picked at random each time.
    /// </summary>
    /// <remarks>
    /// This server's <see cref="TossObject"/> already does everything here except the random
    /// direction, which is the whole reason the scripts reach for a second one.
    /// </remarks>
    internal class TossObject2 : Behavior
    {
        private const int LandsAfterMs = 1500;

        private readonly ushort _child;
        private readonly double _range;
        private readonly double? _angle;
        private readonly bool _randomToss;
        private readonly int _coolDownOffset;
        private Cooldown _coolDown;

        public TossObject2(string child, double range = 5, double? angle = null,
            Cooldown coolDown = new Cooldown(), int coolDownOffset = 0, bool randomToss = false)
        {
            _child = BehaviorDb.InitGameData.IdToObjectType[child];
            _range = range;
            _angle = angle * Math.PI / 180;
            _coolDown = coolDown.Normalize();
            _coolDownOffset = coolDownOffset;
            _randomToss = randomToss;
        }

        protected override void OnStateEntry(Entity host, RealmTime time, ref object state) =>
            state = _coolDownOffset;

        protected override void TickCore(Entity host, RealmTime time, ref object state)
        {
            var cool = (int?)state ?? 0;

            if (cool > 0)
            {
                state = cool - time.ElaspedMsDelta;
                return;
            }

            if (host.HasConditionEffect(ConditionEffects.Stunned))
                return;

            var toss = _randomToss ? Random.Next(0, 360) * Math.PI / 180 : _angle;

            Position target;
            if (toss == null)
            {
                // No direction given: thrown at whoever is nearest, and not thrown at all if the
                // room is empty.
                var nearest = host.GetNearestEntity(_range, null);
                if (nearest == null)
                    return;

                target = new Position { X = nearest.X, Y = nearest.Y };
            }
            else
            {
                target = new Position
                {
                    X = host.X + (float)(_range * Math.Cos(toss.Value)),
                    Y = host.Y + (float)(_range * Math.Sin(toss.Value))
                };
            }

            host.Owner.BroadcastPacketNearby(new ShowEffect
            {
                EffectType = EffectType.Throw,
                Color = new ARGB(0xffffbf00),
                TargetObjectId = host.Id,
                Pos1 = target
            }, host, null, PacketPriority.Low);

            var world = host.Owner;
            var terrain = (host as Enemy)?.Terrain;

            world.AddTimer(new WorldTimer(LandsAfterMs, (w, t) =>
            {
                var entity = Entity.Resolve(w.Manager, _child);
                entity.Move(target.X, target.Y);

                // The spawn inherits the thrower's terrain, or it will be culled by the world's own
                // housekeeping the moment it lands.
                if (entity is Enemy spawned && terrain != null)
                    spawned.Terrain = terrain.Value;

                w.EnterWorld(entity);
            }));

            state = _coolDown.Next(Random);
        }
    }

    /// <summary>Whether the world is currently in some state a behaviour cares about.</summary>
    /// <remarks>
    /// The imported scripts gate a few attacks on their surroundings -- a boss that only shoots
    /// while its eggs are still standing. Neither fork this came from ships the conditions those
    /// scripts call for, so they are written here to the shape the call sites imply.
    /// </remarks>
    internal interface ICondition
    {
        bool Holds(Entity host);
    }

    /// <summary>True while more than <c>count</c> of a named thing stand within range.</summary>
    internal class EntityCountGreaterThan : ICondition
    {
        private readonly ushort _target;
        private readonly double _dist;
        private readonly int _count;

        public EntityCountGreaterThan(string target, double dist, int count)
        {
            _target = BehaviorDb.InitGameData.IdToObjectType[target];
            _dist = dist;
            _count = count;
        }

        public bool Holds(Entity host)
        {
            var world = host.Owner;
            if (world == null)
                return false;

            var seen = 0;
            foreach (var entity in world.Enemies.Values)
            {
                if (entity.ObjectType != _target || entity.Owner == null)
                    continue;

                if (host.DistSqr(entity) > _dist * _dist)
                    continue;

                // Counting stops as soon as the answer cannot change, which matters because the
                // scripts run this against ranges wide enough to mean "anywhere in the room".
                if (++seen > _count)
                    return true;
            }

            return false;
        }
    }

    /// <summary>Runs its behaviours only while a condition holds.</summary>
    internal class If : Behavior
    {
        private readonly ICondition _condition;
        private readonly Behavior[] _behaviors;

        public If(ICondition condition, params Behavior[] behaviors)
        {
            _condition = condition;
            _behaviors = behaviors;
        }

        protected override void OnStateEntry(Entity host, RealmTime time, ref object state)
        {
            // Each child keeps its own state against the host, so they are entered and ticked
            // through the public pair rather than having state threaded through this one.
            foreach (var behavior in _behaviors)
                behavior.OnStateEntry(host, time);
        }

        protected override void TickCore(Entity host, RealmTime time, ref object state)
        {
            if (!_condition.Holds(host))
                return;

            foreach (var behavior in _behaviors)
                behavior.Tick(host, time);
        }

        protected override void OnStateExit(Entity host, RealmTime time, ref object state)
        {
            foreach (var behavior in _behaviors)
                behavior.OnStateExit(host, time);
        }
    }

    /// <summary>
    /// Clears a named object off the tiles around the host, which is how a gate opens.
    /// </summary>
    /// <remarks>
    /// The gate is terrain rather than an entity, so opening it means blanking the object on each
    /// tile it occupies and bumping that tile's update count so the change is sent out.
    /// </remarks>
    internal class OpenGate : Behavior
    {
        private readonly ushort _target;
        private readonly int _area;

        public OpenGate(string target, int area = 10)
        {
            _target = BehaviorDb.InitGameData.DisplayIdToObjectType.TryGetValue(target, out var byDisplay)
                ? byDisplay
                : BehaviorDb.InitGameData.IdToObjectType[target];

            _area = area;
        }

        protected override void OnStateEntry(Entity host, RealmTime time, ref object state)
        {
            var map = host.Owner?.Map;
            if (map == null)
                return;

            for (var x = (int)host.X - _area; x <= (int)host.X + _area; x++)
            for (var y = (int)host.Y - _area; y <= (int)host.Y + _area; y++)
            {
                if (!map.Contains(x, y))
                    continue;

                var tile = map[x, y];
                if (tile.ObjType != _target)
                    continue;

                tile.ObjType = 0;
                tile.UpdateCount++;
            }
        }

        protected override void TickCore(Entity host, RealmTime time, ref object state)
        {
        }
    }
}
