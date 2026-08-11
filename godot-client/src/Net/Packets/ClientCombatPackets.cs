using Hendra.Data;

namespace Hendra.Net.Packets;

/// <summary>
/// Reports a shot we fired. One packet per projectile.
/// </summary>
/// <remarks>
/// The client owns the bullet id space for weapon fire (a byte cycling 0..127); abilities are
/// allocated server-side instead. Every projectile of a multishot volley must carry the *same*
/// <see cref="Time"/> — the server groups a volley by that value and rejects the batch if the count
/// disagrees with the weapon's NumProjectiles. It also tracks the ratio of our reported clock
/// deltas to real elapsed time over a 20-sample window and silently voids shots once that ratio
/// leaves 0.92..1.08.
///
/// A rejected shot makes the server burn one draw of the shared RNG to stay in step with us, so a
/// shot we fire and it refuses does not desynchronise damage prediction.
/// </remarks>
public sealed class PlayerShootPacket : ClientPacket
{
    public override PacketId Id => PacketId.PlayerShoot;

    public int Time;
    public byte BulletId;
    public ushort ContainerType;
    public WorldPos StartingPos;
    public float Angle;

    public override void Write(NetWriter w)
    {
        w.Write(Time);
        w.Write((sbyte)BulletId);
        w.Write((short)ContainerType);
        StartingPos.Write(w);
        w.Write(Angle);
    }
}

/// <summary>
/// Reports that a projectile hit us. The server accepts this with no distance or timing check, so
/// being hit is entirely client-authoritative.
/// </summary>
public sealed class PlayerHitPacket : ClientPacket
{
    public override PacketId Id => PacketId.PlayerHit;

    public byte BulletId;

    /// <summary>The projectile owner's entity id, not ours.</summary>
    public int ObjectId;

    public override void Write(NetWriter w)
    {
        w.Write((sbyte)BulletId);
        w.Write(ObjectId);
    }
}

/// <summary>
/// Reports that one of our projectiles hit an enemy.
/// </summary>
/// <remarks>
/// Damage was fixed when the projectile was created, so <see cref="Killed"/> is the only thing we
/// influence — it tells the server to drop the entity on the next Update. The bullet id must still
/// be live in the server's 256-slot ring, otherwise the whole packet is silently ignored.
/// </remarks>
public sealed class EnemyHitPacket : ClientPacket
{
    public override PacketId Id => PacketId.EnemyHit;

    public int Time;
    public byte BulletId;
    public int TargetId;
    public bool Killed;

    public override void Write(NetWriter w)
    {
        w.Write(Time);
        w.Write((sbyte)BulletId);
        w.Write(TargetId);
        w.Write(Killed);
    }
}

/// <summary>
/// Reports a projectile hitting something that is neither us nor an enemy we own the shot against.
/// The server's handler is empty; kept for fidelity.
/// </summary>
public sealed class OtherHitPacket : ClientPacket
{
    public override PacketId Id => PacketId.OtherHit;

    public int Time;
    public byte BulletId;

    /// <summary>The projectile owner's entity id.</summary>
    public int ObjectId;

    public int TargetId;

    public override void Write(NetWriter w)
    {
        w.Write(Time);
        w.Write((sbyte)BulletId);
        w.Write(ObjectId);
        w.Write(TargetId);
    }
}

/// <summary>
/// Reports a projectile terminating against a wall or the edge of the map. The server's handler is
/// empty; kept for fidelity.
/// </summary>
public sealed class SquareHitPacket : ClientPacket
{
    public override PacketId Id => PacketId.SquareHit;

    public int Time;
    public byte BulletId;

    /// <summary>The projectile owner's entity id.</summary>
    public int ObjectId;

    public override void Write(NetWriter w)
    {
        w.Write(Time);
        w.Write((sbyte)BulletId);
        w.Write(ObjectId);
    }
}

/// <summary>
/// Reports standing on a damaging tile.
/// </summary>
/// <remarks>
/// Ground damage exists only because we report it — the server's own per-tick ground handling is
/// commented out. It rolls the shared RNG to pick the damage, so we must roll ours in the same
/// place to keep the two streams aligned. The AS3 client rate-limits this to one report per tile
/// per 500 ms.
/// </remarks>
public sealed class GroundDamagePacket : ClientPacket
{
    public override PacketId Id => PacketId.GroundDamage;

    public int Time;
    public WorldPos Position;

    public override void Write(NetWriter w)
    {
        w.Write(Time);
        Position.Write(w);
    }
}

/// <summary>Requests a condition effect. The server's handler is an unimplemented stub.</summary>
public sealed class SetConditionPacket : ClientPacket
{
    public override PacketId Id => PacketId.SetCondition;

    public ConditionEffectIndex Effect;
    public float DurationSeconds;

    public override void Write(NetWriter w)
    {
        w.Write((sbyte)Effect);
        w.Write(DurationSeconds);
    }
}
