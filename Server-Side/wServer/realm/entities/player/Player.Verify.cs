using System;
using System.Collections.Concurrent;
using common.resources;
using log4net;
using wServer.networking.packets.incoming;

namespace wServer.realm.entities
{
    /// <summary>
    /// What the server checks for itself rather than taking the client's word for.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The protocol this fork speaks was written on the assumption that the client is honest. It
    /// reports the damage it took, the damage it dealt, and where it is, and the server wrote all
    /// three down. That makes two cheats free to anyone who can edit a client: never report being
    /// hit, and never be hit; and claim a hit on anything at all, which with a multi-shot weapon is
    /// a screen-clearing area attack that costs a single volley.
    /// </para>
    /// <para>
    /// The answer here is not to distrust the client's reports -- they are still the fast path, and
    /// still what the player feels -- but to be able to check them. Two things make that possible
    /// and both were already on the wire: the trail of timestamped positions in every Move packet,
    /// which says where the player claims to have been, and <c>Projectile.GetPosition</c>, which
    /// models a bullet's whole flight. Between them the server can ask whether a bullet and a body
    /// were ever in the same place, and answer without asking the client anything.
    /// </para>
    /// </remarks>
    partial class Player
    {
        private static readonly ILog VerifyLog = LogManager.GetLogger("CheatLog");

        /// <summary>Strikes tolerated inside one window before the connection is cut.</summary>
        /// <remarks>
        /// High enough that a bad minute of packet loss cannot reach it, low enough that a client
        /// actually doing any of this reaches it in seconds.
        /// </remarks>
        private const int StrikeLimit = 12;

        private const int StrikeWindowMs = 10000;

        /// <summary>
        /// How long a client is given to admit to a hit the server saw before the server applies it.
        /// </summary>
        private const int AcknowledgeGraceMs = 600;

        private const int MostGraceMs = 2000;

        /// <summary>Pass-throughs the client failed to report before one of them counts as a strike.</summary>
        private const int SilentHitsPerStrike = 5;

        /// <summary>How far a shot may start from where the shooter was, in tiles.</summary>
        /// <remarks>
        /// On top of whatever the player could have walked since it last said where it was. The
        /// flat part covers the muzzle the client fires from, a third of a tile ahead of itself,
        /// and the rounding either side of it.
        /// </remarks>
        private const float ShootOriginSlack = 1.5f;

        /// <summary>How stale the shooter's last word about itself may be, in milliseconds.</summary>
        /// <remarks>
        /// Past this the allowance would grow without limit and stop meaning anything, so it stops
        /// growing instead. A client whose trail is a second behind has worse problems than aim.
        /// </remarks>
        private const int MostShootDriftMs = 1000;

        /// <summary>How far off its arc a shot in a volley may be, in radians.</summary>
        private const float ArcSlack = 0.08f;

        /// <summary>
        /// How far from a bullet's flight the thing it claims to have hit may be, in tiles.
        /// </summary>
        /// <remarks>
        /// On top of the half-tile hitbox. The slack is for the target, not the bullet: the server
        /// and the client agree about where a bullet goes to the last decimal, having both been
        /// given the same origin and angle, but they disagree about where a moving monster is by
        /// however far it travelled during one tick and one trip down the wire.
        /// </remarks>
        private const float HitClaimSlack = 1.5f;

        /// <summary>How much of an enemy's recent movement a hit claim is allowed to be judged by.</summary>
        private const int HitClaimHistoryTicks = 8;

        /// <summary>Movement allowed over the stat, as a multiplier. Terrain only ever slows.</summary>
        private const float MoveSlack = 1.5f;

        /// <summary>Movement allowed on top of that, in tiles, per packet.</summary>
        private const float MoveGraceTiles = 1f;

        /// <summary>How long after a teleport movement goes unjudged.</summary>
        /// <remarks>
        /// A Goto does not move the player on the server -- it asks the client to move and waits to
        /// be told it did -- so the jump arrives as an ordinary Move that no speed on earth explains.
        /// </remarks>
        private const int MoveGraceMs = 3000;

        private struct DeferredHit
        {
            public Projectile Projectile;
            public long Due;

            /// <summary>How far inside the box the bullet was, and how far into its flight.</summary>
            /// <remarks>
            /// Carried only so an applied hit can say how sure of itself it was. A server that
            /// disagrees with an honest client about grazes reads very differently in the log from
            /// one that disagrees about direct hits, and only the second is a bug worth chasing.
            /// </remarks>
            public float Miss;

            public long Elapsed;
        }

        private readonly ConcurrentQueue<DeferredHit> _deferredHits = new ConcurrentQueue<DeferredHit>();

        private int _strikes;
        private long _strikeWindowEnd;
        private int _silentHits;

        private long _moveGraceUntil;
        private int _lastMoveClientTime = -1;
        private float _lastMoveX;
        private float _lastMoveY;

        private int _volleyTime = -1;
        private int _volleyShots;
        private float _volleyLastAngle;

        private long _groundDamageDue;

        /// <summary>Where this player has said it was, in its own clock.</summary>
        public PositionTimeline History { get; } = new PositionTimeline();

        /// <summary>Stops movement being judged for a while, after something legitimately moved it.</summary>
        /// <remarks>
        /// The window opens on the next Move rather than now, because the callers -- entering a
        /// world, being sent somewhere -- do not all have the clock the window is measured against.
        /// </remarks>
        public void GrantMoveGrace()
        {
            _moveGraceUntil = -1;
            _lastMoveClientTime = -1;
            History.Clear();
        }

        /// <summary>
        /// Records a violation, and cuts the connection once they stop looking like accidents.
        /// </summary>
        /// <remarks>
        /// The account is left alone deliberately. Everything in here is a judgement made from
        /// timings and distances over a network, and the log is meant to be read by someone before
        /// anything permanent happens to a player.
        /// </remarks>
        public void Strike(string what, string detail)
        {
            // Subtraction rather than comparison: TickCount wraps, and a strike either side of the
            // wrap should not read as a window that has been open for seven weeks.
            var now = Environment.TickCount;

            if (now - (int)_strikeWindowEnd > 0)
                _strikes = 0;

            _strikeWindowEnd = now + StrikeWindowMs;
            _strikes++;

            VerifyLog.Info($"{Name} ({AccountId}) {what}: {detail} [{_strikes}/{StrikeLimit}]");

            if (_strikes >= StrikeLimit)
            {
                _strikes = 0;
                _client?.Disconnect($"Kicked for {what}.");
            }
        }

        // ─── being hit ────────────────────────────────────────────────────────────────────────

        /// <summary>
        /// Takes note that a bullet passed through this player, to be applied if the client does not.
        /// </summary>
        /// <remarks>
        /// Deferred rather than applied, because the client's own PlayerHit is still the fast path
        /// and is the one that arrives first for anybody playing normally. Waiting a grace period
        /// means an honest client is never touched by this and damage can never land twice: by the
        /// time the deferral comes due, an admitted hit is already recorded on the bullet.
        /// </remarks>
        public void NoteUnacknowledgedHit(Projectile projectile, RealmTime time, float miss, long elapsed)
        {
            var grace = Math.Min(AcknowledgeGraceMs + Latency, MostGraceMs);
            _deferredHits.Enqueue(new DeferredHit
            {
                Projectile = projectile,
                Due = time.TotalElapsedMs + grace,
                Miss = miss,
                Elapsed = elapsed
            });
        }

        /// <summary>Applies the pass-throughs the client never owned up to.</summary>
        private void ApplyDeferredHits(RealmTime time)
        {
            DeferredHit deferred;
            while (_deferredHits.TryPeek(out deferred) && deferred.Due <= time.TotalElapsedMs)
            {
                _deferredHits.TryDequeue(out deferred);

                var projectile = deferred.Projectile;
                if (projectile == null || projectile.HasHit(this))
                    continue;

                projectile.ForceHit(this, time);

                _silentHits++;
                VerifyLog.Debug(
                    $"{Name} ({AccountId}) unreported hit {_silentHits}: " +
                    $"{projectile.ProjDesc.ObjectId} bullet {projectile.ProjectileId}, " +
                    $"{deferred.Miss:0.00} inside the box at {deferred.Elapsed}ms of flight, " +
                    $"latency {Latency}ms");

                if (_silentHits % SilentHitsPerStrike == 0)
                    Strike("not reporting hits taken", $"{_silentHits} bullets applied by the server");
            }
        }

        // ─── standing in fire ─────────────────────────────────────────────────────────────────

        /// <summary>
        /// Applies ground damage the client should have reported and did not.
        /// </summary>
        /// <remarks>
        /// <para>
        /// Damaging ground is client-reported for the same reason bullets are, and is suppressed the
        /// same way. It cannot simply be moved to the server, because the roll comes off the shared
        /// random stream that both sides step in lockstep -- rolling it in both places would put
        /// every later shot's damage prediction out. So the server waits well past the client's own
        /// half-second cadence and only then rolls it itself, which draws exactly once either way.
        /// </para>
        /// <para>
        /// A client that skipped the roll locally has desynchronised its own stream by doing so.
        /// That is its problem, and a fairly loud one.
        /// </para>
        /// </remarks>
        private void CheckGroundDamage(RealmTime time)
        {
            if (Owner == null || IsInvulnerable())
            {
                _groundDamageDue = 0;
                return;
            }

            var tile = Owner.Map[(int)X, (int)Y];
            var tileDesc = Manager.Resources.GameData.Tiles[tile.TileId];
            var objDesc = tile.ObjType == 0 ? null : Manager.Resources.GameData.ObjectDescs[tile.ObjType];

            // MaxDamage rather than Damaging: the flag is set by either bound being present, and a
            // tile that can only ever roll zero is one the client rightly says nothing about.
            if (!tileDesc.Damaging || tileDesc.MaxDamage <= 0 ||
                objDesc != null && objDesc.ProtectFromGroundDamage)
            {
                _groundDamageDue = 0;
                return;
            }

            if (_groundDamageDue == 0 || time.TotalElapsedMs < _groundDamageDue)
            {
                if (_groundDamageDue == 0)
                    GroundDamageTaken(time);
                return;
            }

            Strike("not reporting ground damage", $"standing on {tileDesc.ObjectId}");
            GroundDamageTaken(time);
            ApplyGroundDamage(tileDesc, tile);
        }

        /// <summary>Called whenever ground damage lands, however it was decided.</summary>
        private void GroundDamageTaken(RealmTime time)
        {
            _groundDamageDue = time.TotalElapsedMs + GroundDamagePeriodMs + GroundDamageGraceMs;
        }

        // ─── shooting ─────────────────────────────────────────────────────────────────────────

        /// <summary>
        /// Checks that a shot came out of the player and went where the weapon can send it.
        /// </summary>
        /// <remarks>
        /// <para>
        /// Both numbers in a PlayerShoot were taken on trust. The starting position was, which lets
        /// a bullet be born anywhere on the map -- on top of whatever it means to kill. And the
        /// angle was, which for a multi-shot weapon is the more useful of the two: the shots of one
        /// volley are meant to leave in a fixed fan, and aiming each of them separately turns a
        /// three-shot bow into three independent guns fired at three different monsters.
        /// </para>
        /// <para>
        /// Rate of fire and the number of shots per volley were already checked, in
        /// <see cref="ValidatePlayerShoot"/>. This is the geometry the same volley implies.
        /// </para>
        /// </remarks>
        public bool ValidateShotGeometry(Item item, PlayerShoot packet)
        {
            // A shot is sent between two Move packets, so the trail rarely covers the moment it was
            // fired. Rather than widen the slack to cover the staleness, the shooter is anchored to
            // the last thing it did say and allowed whatever travelling its own stats could have
            // done since -- which is tight when the trail is fresh and forgiving when it is not.
            var anchor = new Position { X = X, Y = Y };
            var since = 0;

            var said = History.At(packet.Time);
            if (said != null)
            {
                anchor = said.Value;
            }
            else
            {
                int at;
                Position newest;
                if (History.TryNewest(out at, out newest))
                {
                    anchor = newest;
                    since = Math.Min(Math.Max(packet.Time - at, 0), MostShootDriftMs);
                }
            }

            var away = Away(packet.StartingPos.X, packet.StartingPos.Y, anchor.X, anchor.Y);
            var allowed = Stats.GetSpeed() * MoveSlack * since / 1000f + ShootOriginSlack;

            if (away > allowed)
            {
                Strike("shooting from somewhere else",
                    $"bullet at {packet.StartingPos.X:0.0},{packet.StartingPos.Y:0.0}, " +
                    $"player {away:0.0} tiles away, {allowed:0.0} allowed");
                return false;
            }

            if (item.NumProjectiles <= 1)
                return true;

            if (packet.Time != _volleyTime)
            {
                _volleyTime = packet.Time;
                _volleyShots = 1;
                _volleyLastAngle = packet.Angle;
                return true;
            }

            _volleyShots++;
            var arc = (float)(item.ArcGap * Math.PI / 180);
            var off = Math.Abs(Normalize(packet.Angle - _volleyLastAngle) - arc);
            _volleyLastAngle = packet.Angle;

            if (off > ArcSlack)
            {
                Strike("aiming a volley shot by shot",
                    $"{item.ObjectId} shot {_volleyShots} is {off:0.000} rad off its arc");
                return false;
            }

            return true;
        }

        /// <summary>
        /// Checks a claim that one of this player's bullets hit something.
        /// </summary>
        /// <remarks>
        /// <para>
        /// This is the one that matters. The client decides what its bullets hit and the server
        /// wrote it down without looking, so a client that widens its own hit test -- one line --
        /// claims every monster on the screen for every bullet it fires. With a multi-shot weapon
        /// that is an area attack, and it is the reason this whole file exists.
        /// </para>
        /// <para>
        /// What is checked is the flight, not the instant. The moment of the hit is the client's
        /// word too and there is no point weighing one lie against another, but the flight is not:
        /// it follows from the origin and angle the shot was fired with, both of which are now
        /// checked and neither of which can be revised afterwards. A bullet that never went near
        /// the monster never hit it, whatever time is written on the claim.
        /// </para>
        /// <para>
        /// The monster is judged by where it has recently been as well as where it is, because the
        /// client is watching it from a tick and a wire away and is entitled to be behind.
        /// </para>
        /// </remarks>
        public bool ValidateEnemyHit(Projectile projectile, Entity target)
        {
            var slack = Projectile.HitBox + HitClaimSlack;

            if (projectile.DistanceToPath(target.X, target.Y) <= slack)
                return true;

            for (var back = 1; back <= HitClaimHistoryTicks; back++)
            {
                var was = target.TryGetHistory(back);
                if (was == null)
                    break;

                if (projectile.DistanceToPath(was.Value.X, was.Value.Y) <= slack)
                    return true;
            }

            Strike("claiming hits it could not make",
                $"bullet {projectile.ProjectileId} of {projectile.ProjDesc.ObjectId} " +
                $"never came within {slack:0.0} of {target.Name ?? target.ObjectType.ToString()} " +
                $"at {target.X:0.0},{target.Y:0.0}");
            return false;
        }

        // ─── moving ───────────────────────────────────────────────────────────────────────────

        /// <summary>
        /// Keeps the position trail, and holds the player to a speed its stats can reach.
        /// </summary>
        /// <remarks>
        /// <para>
        /// The speed check the original left commented out is the thing that makes everything else
        /// in this file worth doing. Server-side hit detection asks where the player was; if the
        /// player can write any answer it likes, it can simply never be where a bullet is, and
        /// godmode comes back wearing a different hat.
        /// </para>
        /// <para>
        /// A move beyond what the stats allow is clamped rather than refused. Refusing it and
        /// sending the player back would rubber-band anybody with a bad connection, and the honest
        /// reading of a long jump -- packets held up and released together -- is already accounted
        /// for by measuring the gap in the client's own clock, which does not stop during a lag
        /// spike. Clamping keeps the server's idea of the player inside what the stats permit,
        /// which is all the rest of this needs.
        /// </para>
        /// </remarks>
        public void RecordMove(RealmTime time, Move packet)
        {
            // Minus one is the protocol's way of saying the player did not move, not a position.
            if (packet.NewPosition.X == -1 || packet.NewPosition.Y == -1)
                return;

            if (_moveGraceUntil < 0)
                _moveGraceUntil = time.TotalElapsedMs + MoveGraceMs;

            // Every sample in this packet is measured from where the last packet left the player,
            // rather than from the sample before it. Chaining them would hand out the slack once per
            // sample, and a packet carries eleven.
            var judge = _lastMoveClientTime >= 0 && time.TotalElapsedMs >= _moveGraceUntil;
            var fromTime = _lastMoveClientTime;
            var fromX = _lastMoveX;
            var fromY = _lastMoveY;

            // The trail between packets is the player's word too, and is what the hit detection
            // will actually be read out of, so it is held to the same reach as the endpoint.
            if (packet.Records != null)
                foreach (var record in packet.Records)
                    Keep(fromTime, fromX, fromY, record.Time,
                        record.Position.X, record.Position.Y, judge, false);

            var over = Keep(fromTime, fromX, fromY, packet.Time,
                packet.NewPosition.X, packet.NewPosition.Y, judge, true);

            // Clamping rather than refusing: sending the player back would rubber-band anyone with a
            // bad connection, and the honest reading of a long jump -- packets held up and released
            // together -- is already covered by measuring the gap in the client's own clock, which
            // does not stop during a lag spike. What matters is that the server's idea of where the
            // player is stays inside what the stats allow, because that is what bullets meet.
            if (over)
                Move(_lastMoveX, _lastMoveY);
        }

        /// <summary>
        /// Adds one position to the trail, pulled back to within reach of where the player was.
        /// </summary>
        /// <returns>Whether it had to be pulled back.</returns>
        private bool Keep(
            int fromTime, float fromX, float fromY,
            int clientTime, float x, float y, bool judge, bool endpoint)
        {
            var dt = clientTime - fromTime;
            var accept = !judge || fromTime < 0 || dt <= 0 || dt > 5000;

            if (!accept)
            {
                var dx = x - fromX;
                var dy = y - fromY;
                var moved = (float)Math.Sqrt(dx * dx + dy * dy);
                var allowed = Stats.GetSpeed() * MoveSlack * dt / 1000f + MoveGraceTiles;

                if (moved > allowed)
                {
                    // Only the endpoint is worth a strike. One impossible move arrives as a packet
                    // full of impossible samples and should read as one.
                    if (endpoint)
                        Strike("moving faster than it can",
                            $"{moved:0.0} tiles in {dt}ms, {allowed:0.0} allowed");

                    var scale = allowed / moved;
                    x = fromX + dx * scale;
                    y = fromY + dy * scale;
                }
                else
                {
                    accept = true;
                }
            }

            History.Add(clientTime, x, y);

            if (endpoint)
            {
                _lastMoveClientTime = clientTime;
                _lastMoveX = x;
                _lastMoveY = y;
            }

            return endpoint && !accept;
        }

        private static float Away(float x, float y, float toX, float toY)
        {
            var dx = x - toX;
            var dy = y - toY;
            return (float)Math.Sqrt(dx * dx + dy * dy);
        }

        /// <summary>The signed difference between two angles, in radians, nearest to zero.</summary>
        private static float Normalize(float radians)
        {
            const float twoPi = (float)(Math.PI * 2);
            radians %= twoPi;
            if (radians > Math.PI)
                radians -= twoPi;
            if (radians < -Math.PI)
                radians += twoPi;
            return radians;
        }
    }
}
