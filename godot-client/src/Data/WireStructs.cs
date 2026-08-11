using Hendra.Net;

namespace Hendra.Data;

/// <summary>A position in world (tile) coordinates. wServer calls this Position.</summary>
public struct WorldPos
{
    public float X;
    public float Y;

    public WorldPos(float x, float y)
    {
        X = x;
        Y = y;
    }

    public static WorldPos Read(ref NetReader r) => new(r.ReadSingle(), r.ReadSingle());

    public void Write(NetWriter w)
    {
        w.Write(X);
        w.Write(Y);
    }

    public override string ToString() => $"({X}, {Y})";
}

/// <summary>A position stamped with the client clock. Used for the Move packet's history buffer.</summary>
public struct TimedPosition
{
    public int Time;
    public WorldPos Position;

    public TimedPosition(int time, float x, float y)
    {
        Time = time;
        Position = new WorldPos(x, y);
    }

    public static TimedPosition Read(ref NetReader r) => new()
    {
        Time = r.ReadInt32(),
        Position = WorldPos.Read(ref r),
    };

    public void Write(NetWriter w)
    {
        w.Write(Time);
        Position.Write(w);
    }
}

/// <summary>
/// Identifies an inventory slot on some entity. wServer calls this ObjectSlot.
/// </summary>
/// <remarks>
/// The signedness here is asymmetric in the original and must be preserved. The slot id is written
/// as a signed byte but read as unsigned, which is how the two potion slots — 254 (health) and 255
/// (magic) — travel as -2 and -1. Reading them back as signed would put them out of range.
/// </remarks>
public struct SlotObject
{
    public int ObjectId;
    public byte SlotId;
    public int ObjectType;

    public SlotObject(int objectId, byte slotId, int objectType)
    {
        ObjectId = objectId;
        SlotId = slotId;
        ObjectType = objectType;
    }

    public static SlotObject Read(ref NetReader r) => new()
    {
        ObjectId = r.ReadInt32(),
        SlotId = r.ReadByte(),
        ObjectType = r.ReadInt32(),
    };

    public void Write(NetWriter w)
    {
        w.Write(ObjectId);
        w.Write((sbyte)SlotId);
        w.Write(ObjectType);
    }

    public override string ToString() => $"{{Object {ObjectId}, slot {SlotId}, type {ObjectType}}}";
}

/// <summary>A colour as four separate bytes, in A-R-G-B order on the wire.</summary>
public struct Argb
{
    public byte A;
    public byte R;
    public byte G;
    public byte B;

    public Argb(uint argb)
    {
        A = (byte)(argb >> 24);
        R = (byte)(argb >> 16);
        G = (byte)(argb >> 8);
        B = (byte)argb;
    }

    public readonly uint ToUInt32() => ((uint)A << 24) | ((uint)R << 16) | ((uint)G << 8) | B;

    public static Argb Read(ref NetReader r) => new()
    {
        A = r.ReadByte(),
        R = r.ReadByte(),
        G = r.ReadByte(),
        B = r.ReadByte(),
    };

    public void Write(NetWriter w)
    {
        w.Write(A);
        w.Write(R);
        w.Write(G);
        w.Write(B);
    }
}

/// <summary>One line of a trade window.</summary>
public struct TradeItem
{
    public int Item;
    public int SlotType;
    public bool Tradeable;
    public bool Included;

    public static TradeItem Read(ref NetReader r) => new()
    {
        Item = r.ReadInt32(),
        SlotType = r.ReadInt32(),
        Tradeable = r.ReadBoolean(),
        Included = r.ReadBoolean(),
    };

    public void Write(NetWriter w)
    {
        w.Write(Item);
        w.Write(SlotType);
        w.Write(Tradeable);
        w.Write(Included);
    }
}

/// <summary>A single tile in an Update packet.</summary>
public struct GroundTile
{
    public short X;
    public short Y;
    public ushort Type;

    public static GroundTile Read(ref NetReader r) => new()
    {
        X = r.ReadInt16(),
        Y = r.ReadInt16(),
        Type = r.ReadUInt16(),
    };

    public void Write(NetWriter w)
    {
        w.Write(X);
        w.Write(Y);
        w.Write(Type);
    }
}

/// <summary>An item offered on the player market.</summary>
public struct MarketOffer
{
    public int Price;
    public SlotObject Slot;

    public static MarketOffer Read(ref NetReader r) => new()
    {
        Price = r.ReadInt32(),
        Slot = SlotObject.Read(ref r),
    };

    public void Write(NetWriter w)
    {
        w.Write(Price);
        Slot.Write(w);
    }
}

/// <summary>A listing returned by a market query.</summary>
public struct PlayerShopItem
{
    public uint Id;
    public ushort ItemId;
    public int Price;
    public int InsertTime;
    public int Count;
    public bool IsLast;

    public static PlayerShopItem Read(ref NetReader r) => new()
    {
        Id = r.ReadUInt32(),
        ItemId = r.ReadUInt16(),
        Price = r.ReadInt32(),
        InsertTime = r.ReadInt32(),
        Count = r.ReadInt32(),
        IsLast = r.ReadBoolean(),
    };
}
