using System;
using System.Collections.Generic;
using Hendra.Resources;

namespace Hendra.World;

/// <summary>
/// A projectile in flight.
/// </summary>
/// <remarks>
/// Position is recomputed from the spawn point and elapsed time on every frame rather than
/// integrated step by step. That makes flight paths frame-rate independent and, more importantly,
/// identical to the server's — which computes the same closed form in
/// <c>Projectile.GetPosition</c> — so both sides agree on where a bullet is without exchanging
/// anything about it after the shot.
/// </remarks>
public sealed class Projectile : Entity
{
    /// <summary>
    /// Speed is expressed in tiles per ten seconds, so this converts it to tiles per millisecond.
    /// </summary>
    private const float SpeedScale = 1f / 10000f;

    public ProjectileDesc ProjectileDesc;

    /// <summary>When this shot may next leave a spark behind it. See WorldController.LeaveTrail.</summary>
    public int NextTrailMs;

    /// <summary>The type of the object that fired, which is what its artwork comes from.</summary>
    public ushort ContainerType;

    public int OwnerId;

    /// <summary>The id the shooter assigned. Also seeds the alternating phase of wavy paths.</summary>
    public byte BulletId;

    public int StartTimeMs;
    public float StartX;
    public float StartY;
    public float Angle;
    public int Damage;

    public bool DamagesPlayers;
    public bool DamagesEnemies;

    /// <summary>Targets already hit, for projectiles that pass through more than one.</summary>
    private HashSet<int> _alreadyHit;

    public override float HitRadius => 0f;

    /// <summary>
    /// Where the projectile is after <paramref name="elapsedMs"/> of flight.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The alternating phase deserves a note: consecutive bullet ids get opposite phases, so the
    /// two halves of a wavy volley weave in opposite directions instead of moving as one. The same
    /// trick gives parametric projectiles their four-way variation, keyed off the id modulo two and
    /// four.
    /// </para>
    /// <para>
    /// A boomerang reverses at half its lifetime's reach by reflecting the distance travelled,
    /// which is why it retraces its exact outbound path rather than following a curve.
    /// </para>
    /// </remarks>
    public static void PositionAt(
        ProjectileDesc desc,
        float startX,
        float startY,
        float angle,
        byte bulletId,
        int elapsedMs,
        out float x,
        out float y)
    {
        float distance = elapsedMs * (desc.Speed * SpeedScale);
        float phase = (bulletId % 2 == 0) ? 0f : MathF.PI;

        if (desc.Wavy)
        {
            // The client and the server genuinely disagree here. The AS3 client uses PI / 64, a
            // wobble of about three degrees; wServer's Projectile.GetPosition uses PI * 64, which
            // sweeps some thirty full turns and is plainly a mistyped divide. The client's value is
            // the one that produces the intended visual, and it is also the one that matters:
            // projectile hits are client-authoritative, so the server's copy of the path is never
            // consulted for anything a player can observe. Kept as the client had it.
            const float WaveAmplitude = MathF.PI / 64f;
            const float WaveFrequency = 6f * MathF.PI;

            float wavyAngle = angle + WaveAmplitude * MathF.Sin(phase + WaveFrequency * (elapsedMs / 1000f));
            x = startX + distance * MathF.Cos(wavyAngle);
            y = startY + distance * MathF.Sin(wavyAngle);
            return;
        }

        if (desc.Parametric)
        {
            float t = elapsedMs / (float)Math.Max(desc.LifetimeMs, 1) * MathF.Tau;
            float px = MathF.Sin(t) * ((bulletId % 2) != 0 ? 1f : -1f);
            float py = MathF.Sin(2f * t) * ((bulletId % 4) < 2 ? 1f : -1f);

            float sin = MathF.Sin(angle);
            float cos = MathF.Cos(angle);
            x = startX + (px * cos - py * sin) * desc.Magnitude;
            y = startY + (px * sin + py * cos) * desc.Magnitude;
            return;
        }

        if (desc.Boomerang)
        {
            float halfway = desc.LifetimeMs * (desc.Speed * SpeedScale) / 2f;
            if (distance > halfway)
                distance = halfway - (distance - halfway);
        }

        x = startX + distance * MathF.Cos(angle);
        y = startY + distance * MathF.Sin(angle);

        if (desc.Amplitude != 0f)
        {
            float offset = desc.Amplitude *
                MathF.Sin(phase + elapsedMs / (float)Math.Max(desc.LifetimeMs, 1) * desc.Frequency * MathF.Tau);

            // Displaced perpendicular to the direction of travel.
            x += offset * MathF.Cos(angle + MathF.PI / 2f);
            y += offset * MathF.Sin(angle + MathF.PI / 2f);
        }
    }

    /// <summary>Has this projectile already hit the given target?</summary>
    public bool HasHit(int objectId) => _alreadyHit != null && _alreadyHit.Contains(objectId);

    public void RecordHit(int objectId)
    {
        _alreadyHit ??= new HashSet<int>();
        _alreadyHit.Add(objectId);
    }

    /// <summary>
    /// Whether this projectile is allowed to hit <paramref name="target"/>.
    /// </summary>
    /// <remarks>
    /// The side test is the important half: a projectile damages either players or enemies, never
    /// both, decided by which side fired it.
    /// </remarks>
    public bool CanHit(Entity target)
    {
        if (target == null || !target.CanBeHit || ReferenceEquals(target, this))
            return false;

        if (target.ObjectId == OwnerId)
            return false;

        if (ProjectileDesc is { MultiHit: true } && HasHit(target.ObjectId))
            return false;

        bool targetIsEnemy = target.Desc is { IsEnemy: true };
        bool targetIsPlayer = target.Desc is { IsPlayer: true };

        return (DamagesEnemies && targetIsEnemy) || (DamagesPlayers && targetIsPlayer);
    }

    /// <summary>
    /// Advances the projectile, returning false once it should be removed.
    /// </summary>
    /// <param name="nowMs">Current client clock.</param>
    /// <param name="map">Used for terrain and cover collision.</param>
    /// <param name="outcome">What ended the flight, for the caller to report to the server.</param>
    public bool Advance(int nowMs, GameMap map, out ProjectileOutcome outcome)
    {
        outcome = default;

        int elapsed = nowMs - StartTimeMs;
        if (ProjectileDesc == null || elapsed > ProjectileDesc.LifetimeMs)
        {
            // Expiring naturally is not reported; the server times it out on its own schedule.
            outcome = new ProjectileOutcome(ProjectileEnding.Expired, null);
            return false;
        }

        PositionAt(ProjectileDesc, StartX, StartY, Angle, BulletId, elapsed, out float x, out float y);

        var square = map.GetSquare(x, y);
        if (square == null || square.StopsProjectiles)
        {
            outcome = new ProjectileOutcome(ProjectileEnding.HitTerrain, null);
            return false;
        }

        map.MoveEntity(this, x, y);

        // Cover: a static object stops the shot unless the projectile explicitly passes cover, or
        // the object is itself a valid target.
        var blocker = square.StaticObject;
        if (blocker?.Desc != null && !(blocker.Desc.IsEnemy && DamagesEnemies))
        {
            bool blocks = blocker.Desc.EnemyOccupySquare ||
                          (!ProjectileDesc.PassesCover && blocker.Desc.OccupySquare);

            if (blocks)
            {
                outcome = new ProjectileOutcome(ProjectileEnding.HitCover, blocker);
                return false;
            }
        }

        var target = FindTarget(map, x, y);
        if (target != null)
        {
            outcome = new ProjectileOutcome(ProjectileEnding.HitEntity, target);

            if (ProjectileDesc.MultiHit)
            {
                RecordHit(target.ObjectId);
                return true;
            }

            return false;
        }

        return true;
    }

    /// <summary>
    /// The nearest valid target overlapping this point.
    /// </summary>
    /// <remarks>
    /// Hit tests are axis-aligned against each entity's half-tile box rather than a circle, which
    /// is what the original did and what the generous feel of the game depends on.
    /// </remarks>
    private Entity FindTarget(GameMap map, float x, float y)
    {
        Entity best = null;
        float bestDistance = float.MaxValue;

        // Only the tile under this point and the eight around it. Nothing is hit from further than
        // half a tile away, so anything that could overlap is standing in one of those nine, and
        // asking the whole world instead was costing the entire frame.
        int tileX = (int)x;
        int tileY = (int)y;

        for (int dx = -1; dx <= 1; dx++)
        for (int dy = -1; dy <= 1; dy++)
        {
            var bucket = map.HitBucket(tileX + dx, tileY + dy);
            if (bucket == null)
                continue;

            foreach (var entity in bucket)
                Consider(entity, x, y, ref best, ref bestDistance);
        }

        return best;
    }

    private void Consider(Entity entity, float x, float y, ref Entity best, ref float bestDistance)
    {
        {
            if (!CanHit(entity))
                return;

            float radius = entity.HitRadius;
            float dx = entity.X - x;
            float dy = entity.Y - y;

            if (dx > radius || dx < -radius || dy > radius || dy < -radius)
                return;

            float distance = dx * dx + dy * dy;
            if (distance < bestDistance)
            {
                bestDistance = distance;
                best = entity;
            }
        }
    }
}

/// <summary>How a projectile's flight ended.</summary>
public enum ProjectileEnding
{
    /// <summary>Ran out of lifetime. Not reported to the server.</summary>
    Expired,

    /// <summary>Hit a wall or the edge of the known map. Reported as SquareHit.</summary>
    HitTerrain,

    /// <summary>Stopped by a static object. Reported as OtherHit.</summary>
    HitCover,

    /// <summary>Hit an entity. Reported as PlayerHit, EnemyHit or OtherHit depending on who.</summary>
    HitEntity,
}

/// <summary>The result of advancing a projectile one frame.</summary>
public readonly struct ProjectileOutcome
{
    public readonly ProjectileEnding Ending;
    public readonly Entity Target;

    public ProjectileOutcome(ProjectileEnding ending, Entity target)
    {
        Ending = ending;
        Target = target;
    }
}
