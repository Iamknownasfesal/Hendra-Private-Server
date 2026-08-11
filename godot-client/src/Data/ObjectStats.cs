using System.Collections.Generic;
using Hendra.Net;

namespace Hendra.Data;

/// <summary>
/// A single stat value. Most stats are int32; the four listed in
/// <see cref="StatsTypeExtensions.IsStringStat"/> carry a length-prefixed string instead.
/// </summary>
public struct StatData
{
    public StatsType Type;
    public int IntValue;
    public string StringValue;

    public readonly bool IsString => Type.IsStringStat();

    public static StatData Read(ref NetReader r)
    {
        var stat = new StatData { Type = (StatsType)r.ReadByte() };
        if (stat.Type.IsStringStat())
            stat.StringValue = r.ReadUtf();
        else
            stat.IntValue = r.ReadInt32();
        return stat;
    }

    public void Write(NetWriter w)
    {
        w.Write((sbyte)Type);
        if (IsString)
            w.WriteUtf(StringValue);
        else
            w.Write(IntValue);
    }

    public override string ToString() => IsString ? $"{Type}={StringValue}" : $"{Type}={IntValue}";
}

/// <summary>
/// An entity's identity, position and a delta of changed stats. Appears in NewTick (as a
/// per-tick delta) and inside <see cref="ObjectDef"/> in Update (as the full initial state).
/// </summary>
public struct ObjectStats
{
    public int ObjectId;
    public WorldPos Position;
    public StatData[] Stats;

    public static ObjectStats Read(ref NetReader r)
    {
        var result = new ObjectStats
        {
            ObjectId = r.ReadInt32(),
            Position = WorldPos.Read(ref r),
        };

        int count = r.ReadInt16();
        result.Stats = count == 0 ? System.Array.Empty<StatData>() : new StatData[count];
        for (int i = 0; i < count; i++)
            result.Stats[i] = StatData.Read(ref r);

        return result;
    }

    public void Write(NetWriter w)
    {
        w.Write(ObjectId);
        Position.Write(w);
        var stats = Stats ?? System.Array.Empty<StatData>();
        w.Write((short)stats.Length);
        foreach (var stat in stats)
            stat.Write(w);
    }

    /// <summary>Convenience lookup; returns false if the stat is absent from this delta.</summary>
    public readonly bool TryGet(StatsType type, out StatData value)
    {
        if (Stats != null)
        {
            foreach (var stat in Stats)
            {
                if (stat.Type == type)
                {
                    value = stat;
                    return true;
                }
            }
        }
        value = default;
        return false;
    }
}

/// <summary>A newly visible entity: its type plus its initial <see cref="ObjectStats"/>.</summary>
public struct ObjectDef
{
    public ushort ObjectType;
    public ObjectStats Stats;

    public static ObjectDef Read(ref NetReader r) => new()
    {
        ObjectType = r.ReadUInt16(),
        Stats = ObjectStats.Read(ref r),
    };

    public void Write(NetWriter w)
    {
        w.Write(ObjectType);
        Stats.Write(w);
    }
}

/// <summary>
/// One sample of the local player's position, sent in the Move packet so the server can audit
/// sub-tick movement.
/// </summary>
/// <remarks>
/// Write-only: the server parses the array but never acts on it, and never sends one back.
/// The bucketing rules that decide which samples get sent live in <c>MoveRecords</c>.
/// </remarks>
public struct MoveRecord
{
    public int Time;
    public float X;
    public float Y;

    public MoveRecord(int time, float x, float y)
    {
        Time = time;
        X = x;
        Y = y;
    }

    public void Write(NetWriter w)
    {
        w.Write(Time);
        w.Write(X);
        w.Write(Y);
    }
}
