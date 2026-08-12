using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using common.resources;
using log4net;

namespace wServer.realm.entities
{
    public interface IProjectileOwner
    {
        Projectile[] Projectiles { get; }
        Entity Self { get; }
    }

    public class Projectile : Entity
    {
        /// <summary>
        /// Half the width of the square a bullet is caught by, in tiles.
        /// </summary>
        /// <remarks>
        /// The client's <c>GameObject.radius_</c>, which in this fork is half a tile for everything
        /// and is never read from the object's own data. It is a square, not a circle: the client
        /// tests each axis on its own, and so does everything here that has to agree with it.
        /// </remarks>
        public const float HitBox = 0.5f;

        /// <summary>The box the server claims a hit inside, when deciding one for itself.</summary>
        /// <remarks>
        /// <para>
        /// Deliberately far smaller than the client's. The server is second-guessing a machine that
        /// saw the same bullet from a slightly different vantage, and it should only ever insist on
        /// hits that machine could not reasonably have called a miss. A graze the two disagree
        /// about is given to the player.
        /// </para>
        /// <para>
        /// Set from the disagreements rather than from taste. An evening of an honest client playing
        /// normally produced a hundred and seventeen hits the server applied and the client had not
        /// reported, and every single one of them passed the player between a fifth and two fifths
        /// of a tile off centre -- edge cases, all of them. Not one was a bullet through the middle.
        /// At two tenths, a hundred and thirteen of those hundred and seventeen go away.
        /// </para>
        /// <para>
        /// What this gives up is real and worth saying: a client that lies about being hit now gets
        /// away with everything that passes it more than a fifth of a tile off centre. That is the
        /// trade. Godmode has to dodge every bullet, and the ones through the middle are the ones a
        /// player standing in a boss room cannot avoid, so the server only has to be sure about
        /// those to make lying about them useless.
        /// </para>
        /// </remarks>
        private const float SweepBox = 0.2f;

        /// <summary>How much of a bullet's flight the server will second-guess the client about.</summary>
        /// <remarks>
        /// The two sides agree about where a bullet is at the instant it is fired and drift apart
        /// from there: a clock offset estimated over a noisy link, a wavy shot's phase, and a trail
        /// sampled ten times a second all accumulate. Past a second of flight the server's idea of
        /// where the bullet was is worth less than the client's, and the disagreements bear that out
        /// -- two thirds of the false positives were on bullets older than this.
        /// </remarks>
        private const int MostSweepMs = 1000;

        /// <summary>Finest a swept path is walked, in tiles. Comfortably inside the hitbox.</summary>
        private const float StepTiles = 0.25f;

        /// <summary>Steps one sweep will walk before it gives up and retires the rest unchecked.</summary>
        private const int MostSteps = 64;

        /// <summary>
        /// How long a spent bullet is kept so the last of its flight can still be accounted for.
        /// </summary>
        /// <remarks>
        /// Long enough for a Move packet covering the end of the flight to arrive: a tick of the
        /// client's own reporting and a round trip. Without it, every bullet's last leg would go
        /// unchecked, which is exactly the leg a long shot lands on.
        /// </remarks>
        private const int SweepTailMs = 800;

        public IProjectileOwner ProjectileOwner { get; set; }
        public ushort Container { get; set; }
        public ProjectileDesc ProjDesc { get; }
        public long CreationTime { get; set; }
        private bool _used { get; set; }

        public byte ProjectileId { get; set; }
        public Position StartPos { get; set; }
        public float Angle { get; set; }
        public int Damage { get; set; }

        /// <summary>
        /// What this bullet has hit, and what it has been reported as passing through.
        /// </summary>
        /// <remarks>
        /// Both built on first use rather than in the constructor. Bullets are the most numerous
        /// thing the server makes -- a room of bosses fires thousands a second -- and the great
        /// majority of them hit nothing at all, so a set each was two allocations apiece for
        /// answers that stayed empty. The lock guards creation as well as contents, since the sweep
        /// runs on the world thread and the hit reports arrive on the logic thread.
        /// </remarks>
        private HashSet<Entity> _hit;

        private HashSet<Entity> _noted;

        private readonly object _hitLock = new object();

        /// <summary>
        /// How much of this bullet's flight has been checked against the players it passed.
        /// </summary>
        /// <remarks>
        /// It advances only as far as the players' own position trails can answer for, so a leg
        /// flown before the covering Move packet arrived is retried on the next tick rather than
        /// skipped. See <see cref="SweepPlayers"/>.
        /// </remarks>
        private long _swept;

        /// <summary>Whether the flight ran into something solid and stopped being worth walking.</summary>
        private bool _stopped;

        public Projectile(RealmManager manager, ProjectileDesc desc)
            : base(manager, manager.Resources.GameData.IdToObjectType[desc.ObjectId])
        {
            ProjDesc = desc;
        }

        public void Destroy()
        {
            Owner?.LeaveWorld(this);
        }

        public override void Dispose()
        {
            base.Dispose();
            ProjectileOwner.Projectiles[ProjectileId] = null;
            //ProjectileOwner = null;
        }

        public override void Tick(RealmTime time)
        {
            var elapsed = time.TotalElapsedMs - CreationTime;
            if (elapsed > ProjDesc.LifetimeMS)
            {
                if (!NeedsSweep)
                {
                    Destroy();
                    return;
                }

                // Held a moment past the end of its flight, and only for as long as the accounting
                // takes. The trail that says where people were during its last leg is still in the
                // post; the bullet cannot travel any further, only be answered for.
                SweepPlayers(time, SweepLimit);

                if (_swept >= SweepLimit || elapsed > ProjDesc.LifetimeMS + SweepTailMs)
                    Destroy();

                return;
            }

            SweepPlayers(time, elapsed);

            base.Tick(time);
        }

        /// <summary>Whether this bullet is one that can hit players, and so is worth walking.</summary>
        private bool NeedsSweep
        {
            get { return ProjectileOwner != null && !(ProjectileOwner.Self is Player); }
        }

        /// <summary>How far into the flight the sweep will go: the shorter of the two limits.</summary>
        private long SweepLimit
        {
            get { return Math.Min(ProjDesc.LifetimeMS, MostSweepMs); }
        }

        /// <summary>
        /// Walks the flight just made and notes any player it passed through.
        /// </summary>
        /// <remarks>
        /// <para>
        /// The reason this exists is that nothing else on the server ever decides that a player was
        /// hit. Damage from a bullet arrives only when the client volunteers a PlayerHit, so a
        /// client that simply never sends one takes no damage from anything that shoots -- which is
        /// what godmode is, and it needs no cleverness at all.
        /// </para>
        /// <para>
        /// What is compared is not where the player is now but where the player itself said it was
        /// at the moment the bullet was there, read out of the trail its own Move packets carry. The
        /// client starts its copy of this bullet when the EnemyShoot packet lands rather than when
        /// the server made it, so the reading is taken a one-way trip later, which is where the
        /// client's bullet is that far into its flight. Both sides then agree about the same instant
        /// and the same half-tile square, and a player who genuinely dodged is genuinely missed.
        /// </para>
        /// <para>
        /// Nothing is applied here. A pass-through is only reported to the player, who gives its
        /// client a grace period to own up to it first -- see <c>Player.NoteUnacknowledgedHit</c>.
        /// </para>
        /// </remarks>
        private void SweepPlayers(RealmTime time, long upTo)
        {
            var world = Owner;

            // Players' own bullets never hurt players, so there is nothing here to check for them.
            if (world == null || ProjectileOwner == null || ProjectileOwner.Self is Player)
                return;

            // Nothing past the first second of flight is judged at all: see MostSweepMs.
            if (upTo > SweepLimit)
                upTo = SweepLimit;

            if (_stopped || upTo <= _swept || world.Players.Count == 0)
                return;

            var from = _swept;

            // A leg too long to walk finely is walked from as far back as the budget reaches; the
            // rest is retired unchecked rather than sampled so coarsely that it steps over people.
            var span = (upTo - from) * ProjDesc.Speed / 10000.0;
            if (span > MostSteps * StepTiles)
            {
                from = upTo - (long)(MostSteps * StepTiles * 10000.0 / Math.Max(ProjDesc.Speed, 0.0001f));
                span = MostSteps * StepTiles;
            }

            var steps = (int)Math.Ceiling(span / StepTiles);
            if (steps < 1)
                steps = 1;

            // Walked once for the terrain before anyone is judged by it. The server's projectiles
            // pass through walls -- nothing here has ever collided them -- and the client's do not,
            // so a bullet the player watched break against a wall would otherwise carry on and be
            // swept through whoever was sheltering behind it.
            for (var i = 1; i <= steps; i++)
            {
                var e = from + (upTo - from) * i / steps;
                if (!Blocked(world, GetPosition(e)))
                    continue;

                _stopped = true;
                upTo = e;
                steps = i;
                break;
            }

            var start = GetPosition(from);
            var end = GetPosition(upTo);

            // Enough room for the leg itself, the excursion a wavy or parametric bullet makes off
            // its own line, and the gap between where a player is now and where the trail will say
            // it was. Erring wide only costs a distance check; erring narrow silently misses people.
            var reach = Dist(start, end) +
                        Math.Max(ProjDesc.Amplitude, ProjDesc.Magnitude) + 4f;

            // The sweep retires only as far as every player it looked at could be answered for.
            var retire = upTo;

            foreach (var player in world.Players.Values)
            {
                if (player?.Owner == null || player.IsInvulnerable() ||
                    HasHit(player) || _noted != null && _noted.Contains(player))
                    continue;

                var dx = player.X - start.X;
                var dy = player.Y - start.Y;
                if (dx * dx + dy * dy > reach * reach)
                    continue;

                var answered = from;

                // The closest the bullet came over the whole leg, rather than the first point at
                // which it was near enough. With a box this tight the difference matters: the walk
                // is a quarter of a tile at a time, and stopping at the first sample under the
                // threshold judges the bullet by a point that can be a good deal worse than the one
                // it actually passed through.
                var closest = float.MaxValue;
                var closestAt = from;

                for (var i = 1; i <= steps; i++)
                {
                    var e = from + (upTo - from) * i / steps;
                    var said = player.History.At(
                        (int)player.S2CTime((int)(CreationTime + e)) + player.Latency);

                    // The trail does not reach this far yet. Leave the rest of the leg for a later
                    // tick rather than treating silence as a miss.
                    if (said == null)
                        break;

                    answered = e;

                    var at = GetPosition(e);
                    var off = Math.Max(Math.Abs(said.Value.X - at.X), Math.Abs(said.Value.Y - at.Y));
                    if (off >= closest)
                        continue;

                    closest = off;
                    closestAt = e;
                }

                if (closest <= SweepBox)
                {
                    (_noted ??= new HashSet<Entity>()).Add(player);
                    player.NoteUnacknowledgedHit(this, time, SweepBox - closest, closestAt);
                    answered = upTo;
                }

                if (answered < retire)
                    retire = answered;
            }

            _swept = retire;
        }

        /// <summary>
        /// Whether a bullet reaching this point would have broken against something.
        /// </summary>
        /// <remarks>
        /// Mirrors the client's own test, which is the one the player watched happen: off the edge
        /// of the map, an unwalkable tile, or an object occupying the square that the shot does not
        /// pass. An object that is itself a target does not stop a shot aimed at it.
        /// </remarks>
        private bool Blocked(worlds.World world, Position at)
        {
            var x = (int)at.X;
            var y = (int)at.Y;

            if (!world.Map.Contains(x, y))
                return true;

            var tile = world.Map[x, y];

            var tileDesc = Manager.Resources.GameData.Tiles[tile.TileId];
            if (tileDesc.NoWalk)
                return true;

            if (tile.ObjType == 0)
                return false;

            ObjectDesc objDesc;
            if (!Manager.Resources.GameData.ObjectDescs.TryGetValue(tile.ObjType, out objDesc) ||
                objDesc == null || objDesc.Enemy)
                return false;

            return objDesc.EnemyOccupySquare ||
                   !ProjDesc.PassesCover && objDesc.OccupySquare;
        }

        private static float Dist(Position a, Position b)
        {
            var dx = a.X - b.X;
            var dy = a.Y - b.Y;
            return (float)Math.Sqrt(dx * dx + dy * dy);
        }

        /// <summary>How close this bullet's whole flight passes to a point, in tiles.</summary>
        /// <remarks>
        /// <para>
        /// Used to judge a client's claim that the bullet hit something. The whole path is measured
        /// rather than the position at one instant, because the instant is the client's word too and
        /// there is no sense weighing one claim against another. The path is not its word: it
        /// follows from the origin and the angle the shot was fired at, both of which are checked
        /// when the shot arrives and neither of which can be revised afterwards.
        /// </para>
        /// <para>
        /// Nearly every bullet in the game flies in a straight line, and for those this is a
        /// point-to-segment distance rather than a walk -- worth the special case, because this
        /// runs once per bullet fired per monster claimed.
        /// </para>
        /// </remarks>
        public float DistanceToPath(float x, float y)
        {
            var reach = ProjDesc.LifetimeMS * ProjDesc.Speed / 10000.0;

            if (!ProjDesc.Wavy && !ProjDesc.Parametric && ProjDesc.Amplitude == 0)
            {
                // A boomerang turns round halfway and comes back along the same line it went out on.
                if (ProjDesc.Boomerang)
                    reach /= 2;

                return DistanceToSegment(
                    x, y,
                    StartPos.X, StartPos.Y,
                    (float)(StartPos.X + reach * Math.Cos(Angle)),
                    (float)(StartPos.Y + reach * Math.Sin(Angle)));
            }

            var steps = (int)Math.Ceiling(reach / StepTiles);
            if (steps < 1)
                steps = 1;
            if (steps > MostSteps)
                steps = MostSteps;

            var closest = float.MaxValue;
            for (var i = 0; i <= steps; i++)
            {
                var at = GetPosition((long)ProjDesc.LifetimeMS * i / steps);
                var dx = at.X - x;
                var dy = at.Y - y;
                var d = dx * dx + dy * dy;
                if (d < closest)
                    closest = d;
            }

            return (float)Math.Sqrt(closest);
        }

        private static float DistanceToSegment(float x, float y, float ax, float ay, float bx, float by)
        {
            var vx = bx - ax;
            var vy = by - ay;
            var lengthSqr = vx * vx + vy * vy;

            var t = lengthSqr <= 0f ? 0f : ((x - ax) * vx + (y - ay) * vy) / lengthSqr;
            if (t < 0f) t = 0f;
            if (t > 1f) t = 1f;

            var dx = x - (ax + vx * t);
            var dy = y - (ay + vy * t);
            return (float)Math.Sqrt(dx * dx + dy * dy);
        }

        public Position GetPosition(long elapsedTicks)
        {
            var x = (double)StartPos.X;
            var y = (double)StartPos.Y;

            var dist = elapsedTicks * ProjDesc.Speed / 10000.0;
            var period = ProjectileId % 2 == 0 ? 0 : Math.PI;

            if (ProjDesc.Wavy)
            {
                // Divided, not multiplied: a wobble of about three degrees, which is what the client
                // draws and therefore what the player dodges. Written the other way up here it swept
                // some thirty full turns, which no longer merely looked wrong now that this path is
                // walked to decide whether a bullet went through somebody.
                var theta = Angle + (Math.PI / 64) * Math.Sin(period + 6 * Math.PI * (elapsedTicks / 1000.0));
                x += dist * Math.Cos(theta);
                y += dist * Math.Sin(theta);
            }
            else if (ProjDesc.Parametric)
            {
                var theta = (double)elapsedTicks / ProjDesc.LifetimeMS * 2 * Math.PI;
                var a = Math.Sin(theta) * (ProjectileId % 2 != 0 ? 1 : -1);
                var b = Math.Sin(theta * 2) * (ProjectileId % 4 < 2 ? 1 : -1);
                var c = Math.Sin(Angle);
                var d = Math.Cos(Angle);
                x += (a * d - b * c) * ProjDesc.Magnitude;
                y += (a * c + b * d) * ProjDesc.Magnitude;
            }
            else
            {
                if (ProjDesc.Boomerang)
                {
                    var d = (ProjDesc.LifetimeMS * ProjDesc.Speed / 10000.0) / 2;
                    if (dist > d)
                        dist = d - (dist - d);
                }
                x += dist * Math.Cos(Angle);
                y += dist * Math.Sin(Angle);
                if (ProjDesc.Amplitude != 0)
                {
                    var d = ProjDesc.Amplitude * Math.Sin(period + (double)elapsedTicks / ProjDesc.LifetimeMS * ProjDesc.Frequency * 2 * Math.PI);
                    x += d * Math.Cos(Angle + Math.PI / 2);
                    y += d * Math.Sin(Angle + Math.PI / 2);
                }
            }

            return new Position() { X = (float)x, Y = (float)y };
        }

        public void ForceHit(Entity entity, RealmTime time)
        {
            if (!ProjDesc.MultiHit && _used && !(entity is Player))
                return;

            bool first;
            lock (_hitLock)
                first = (_hit ??= new HashSet<Entity>()).Add(entity);

            if (first)
                entity.HitByProjectile(this, time);

            _used = true;
        }

        /// <summary>Whether this bullet has already been spent on that entity.</summary>
        /// <remarks>
        /// Read from the world tick and written from the packet handlers, hence the lock: it is
        /// what tells a deferred server-side hit that the client owned up to it in the meantime.
        /// </remarks>
        public bool HasHit(Entity entity)
        {
            lock (_hitLock)
                return _hit != null && _hit.Contains(entity);
        }
    }
}
