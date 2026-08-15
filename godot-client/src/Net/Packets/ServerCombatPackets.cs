using System;
using System.Collections.Generic;
using Hendra.Data;

namespace Hendra.Net.Packets;

/// <summary>
/// Visual effect kinds carried by <see cref="ShowEffectPacket"/>.
/// </summary>
/// <remarks>
/// Named after the AS3 client's constants, because those describe what actually gets drawn.
/// wServer's own <c>EffectType</c> enum uses different names for the same numbers (Potion, Dead,
/// Trail, Diffuse, Trap, Concentrate, BlastWave, Earthquake, Flashing, BeachBall) and stops at 16;
/// the client recognises three more above that.
/// </remarks>
public enum ShowEffectType : byte
{
    Unknown = 0,
    Heal = 1,
    Teleport = 2,
    Stream = 3,
    Throw = 4,

    /// <summary>Radius is carried in <c>Pos1.X</c>.</summary>
    Nova = 5,

    Poison = 6,
    Line = 7,

    /// <summary>Radius is the distance between Pos1 and Pos2.</summary>
    Burst = 8,

    Flow = 9,

    /// <summary>Radius is carried in <c>Pos1.X</c>.</summary>
    Ring = 10,

    /// <summary>Particle count is carried in <c>Pos2.X</c>.</summary>
    Lightning = 11,

    /// <summary>Radius is the distance between Pos1 and Pos2.</summary>
    Collapse = 12,

    /// <summary>Origin is Pos1; radius is carried in <c>Pos2.X</c>.</summary>
    ConeBlast = 13,

    /// <summary>Shakes the camera.</summary>
    Jitter = 14,

    /// <summary>Period in <c>Pos1.X</c>, cycle count in <c>Pos1.Y</c>.</summary>
    Flash = 15,

    ThrowProjectile = 16,
    Shocker = 17,
    Shockee = 18,

    /// <summary>Duration in seconds is carried in <c>Pos1.X</c>.</summary>
    RisingFury = 19,
}

/// <summary>
/// Applies damage and status effects to an entity.
/// </summary>
/// <remarks>
/// The effects list is a count followed by that many condition-effect *indices*, not a bitmask.
/// When <see cref="ObjectId"/> is non-negative and <see cref="BulletId"/> is non-zero, the named
/// projectile should be retired locally (unless it is multi-hit).
/// </remarks>
public sealed class DamagePacket : ServerPacket
{
    public override PacketId Id => PacketId.Damage;

    public int TargetId;
    public ConditionEffectIndex[] Effects = Array.Empty<ConditionEffectIndex>();
    public ushort DamageAmount;
    public bool Kill;
    public byte BulletId;

    /// <summary>The projectile owner's entity id.</summary>
    public int ObjectId;

    public override void Read(ref NetReader r)
    {
        TargetId = r.ReadInt32();

        int count = r.ReadByte();
        Effects = count == 0 ? Array.Empty<ConditionEffectIndex>() : new ConditionEffectIndex[count];
        for (int i = 0; i < count; i++)
            Effects[i] = (ConditionEffectIndex)r.ReadByte();

        DamageAmount = r.ReadUInt16();
        Kill = r.ReadBoolean();
        BulletId = r.ReadByte();
        ObjectId = r.ReadInt32();
    }
}

/// <summary>
/// An area-of-effect blast. We decide locally whether it hit us and must answer with an AoeAck
/// either way — including when we have no player yet, in which case the ack carries (0, 0).
/// </summary>
public sealed class AoePacket : ServerPacket
{
    public override PacketId Id => PacketId.Aoe;

    public WorldPos Position;
    public float Radius;
    public ushort Damage;
    public ConditionEffectIndex Effect;
    public float DurationSeconds;

    /// <summary>Type of the entity that produced the blast, for the damage attribution text.</summary>
    public ushort OrigType;

    /// <summary>
    /// Whether the server has already taken the health, making this a telegraph rather than an
    /// instruction.
    /// </summary>
    /// <remarks>
    /// Never set by the wire read below, which is the original's format: that protocol leaves the
    /// hit to the client. Set by the bridge for a server that decides its own blasts and reports
    /// them as ordinary damage.
    /// </remarks>
    public bool ServerApplied;

    public override void Read(ref NetReader r)
    {
        Position = WorldPos.Read(ref r);
        Radius = r.ReadSingle();
        Damage = r.ReadUInt16();
        Effect = (ConditionEffectIndex)r.ReadByte();
        DurationSeconds = r.ReadSingle();
        OrigType = r.ReadUInt16();
    }
}

/// <summary>
/// An enemy fired. One packet can describe a whole volley.
/// </summary>
/// <remarks>
/// Spawn <see cref="NumShots"/> projectiles, the i-th at
/// <c>Angle + AngleInc * i</c> with bullet id <c>(BulletId + i) % 256</c>. If the owner is missing
/// or dead, answer <c>ShootAck(-1)</c> and spawn nothing.
///
/// The AS3 client read the last two fields conditionally, treating them as an optional tail with
/// defaults of 1 and 0. This server build always writes them, but the guard costs nothing.
/// </remarks>
public sealed class EnemyShootPacket : ServerPacket
{
    public override PacketId Id => PacketId.EnemyShoot;

    public byte BulletId;
    public int OwnerId;
    public byte BulletType;
    public WorldPos StartingPos;
    public float Angle;
    public short Damage;
    public byte NumShots = 1;
    public float AngleInc;

    public override void Read(ref NetReader r)
    {
        BulletId = r.ReadByte();
        OwnerId = r.ReadInt32();
        BulletType = r.ReadByte();
        StartingPos = WorldPos.Read(ref r);
        Angle = r.ReadSingle();
        Damage = r.ReadInt16();

        if (r.Remaining > 0)
        {
            NumShots = r.ReadByte();
            AngleInc = r.ReadSingle();
        }
        else
        {
            NumShots = 1;
            AngleInc = 0f;
        }
    }
}

/// <summary>
/// Another player fired. Cosmetic only — no ack, no damage, and the projectile spawns from the
/// owner's current position rather than a position on the wire.
/// </summary>
public sealed class AllyShootPacket : ServerPacket
{
    public override PacketId Id => PacketId.AllyShoot;

    public byte BulletId;
    public int OwnerId;

    /// <summary>16-bit here, unlike the 32-bit field of the same name in ServerPlayerShoot.</summary>
    public ushort ContainerType;

    public float Angle;

    public override void Read(ref NetReader r)
    {
        BulletId = r.ReadByte();
        OwnerId = r.ReadInt32();
        ContainerType = r.ReadUInt16();
        Angle = r.ReadSingle();
    }
}

/// <summary>
/// A server-authored projectile — ability shots and novas. Unlike AllyShoot this is a real
/// projectile with a position and damage, and it needs a ShootAck when we own it.
/// </summary>
public sealed class ServerPlayerShootPacket : ServerPacket
{
    public override PacketId Id => PacketId.ServerPlayerShoot;

    public byte BulletId;
    public int OwnerId;

    /// <summary>32-bit here, unlike the 16-bit field of the same name in AllyShoot.</summary>
    public int ContainerType;

    public WorldPos StartingPos;
    public float Angle;
    public short Damage;

    public override void Read(ref NetReader r)
    {
        BulletId = r.ReadByte();
        OwnerId = r.ReadInt32();
        ContainerType = r.ReadInt32();
        StartingPos = WorldPos.Read(ref r);
        Angle = r.ReadSingle();
        Damage = r.ReadInt16();
    }
}

/// <summary>
/// Spawns a visual effect. <see cref="Pos1"/> and <see cref="Pos2"/> are overloaded per effect
/// type — see the notes on <see cref="ShowEffectType"/>.
/// </summary>
public sealed class ShowEffectPacket : ServerPacket
{
    public override PacketId Id => PacketId.ShowEffect;

    public ShowEffectType EffectType;
    public int TargetObjectId;
    public WorldPos Pos1;
    public WorldPos Pos2;
    public Argb Color;

    public override void Read(ref NetReader r)
    {
        EffectType = (ShowEffectType)r.ReadByte();
        TargetObjectId = r.ReadInt32();
        Pos1 = WorldPos.Read(ref r);
        Pos2 = WorldPos.Read(ref r);
        Color = Argb.Read(ref r);
    }
}

/// <summary>Plays one of an entity's XML-declared sounds.</summary>
public sealed class PlaySoundPacket : ServerPacket
{
    public override PacketId Id => PacketId.PlaySound;

    public int OwnerId;
    public byte SoundId;

    public override void Read(ref NetReader r)
    {
        OwnerId = r.ReadInt32();
        SoundId = r.ReadByte();
    }
}

/// <summary>
/// Floating text above an entity. <see cref="Message"/> is either a literal string or a JSON
/// localisation blob — see the text builder for the disambiguation rule.
/// </summary>
public sealed class NotificationPacket : ServerPacket
{
    public override PacketId Id => PacketId.Notification;

    public int ObjectId;
    public string Message;
    public Argb Color;

    public override void Read(ref NetReader r)
    {
        ObjectId = r.ReadInt32();
        Message = r.ReadUtf();
        Color = Argb.Read(ref r);
    }
}

/// <summary>
/// A server-wide announcement or a UI toggle. <see cref="Type"/> is an int on the wire but the
/// client dispatches on <see cref="Text"/>, which carries values like "yellow", "showKeyUI" or
/// "giftChestOccupied".
/// </summary>
public sealed class GlobalNotificationPacket : ServerPacket
{
    public override PacketId Id => PacketId.GlobalNotification;

    public int Type;
    public string Text;

    public override void Read(ref NetReader r)
    {
        Type = r.ReadInt32();
        Text = r.ReadUtf();
    }
}
