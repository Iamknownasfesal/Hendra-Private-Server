using Hendra.Resources;

namespace Hendra.World;

/// <summary>
/// One tile of the world.
/// </summary>
/// <remarks>
/// Two distinct kinds of "nothing" meet here and they behave the same way but arrive differently.
/// <see cref="GroundDesc.Type"/> <c>0xFF</c> is a real terrain type named "Empty" — the void the
/// server paints outside a dungeon's rooms. <see cref="NotReceived"/> is the client's own marker for
/// a tile the server has not streamed yet. Both block movement completely, which is what keeps a
/// player inside the region they can actually see.
/// </remarks>
public sealed class Square
{
    /// <summary>Terrain type for a tile the server has not sent. Not a value that appears on the wire.</summary>
    public const ushort NotReceived = 0xFFFF;

    /// <summary>The "Empty" terrain type, which is the void rather than absent data.</summary>
    public const ushort EmptyTerrain = 0x00FF;

    public ushort TileType = NotReceived;

    /// <summary>Null until the tile is received, or if its type is not in the loaded data.</summary>
    public GroundDesc Desc;

    /// <summary>
    /// The single static object standing here, if any. Only static objects are tracked per tile;
    /// everything else is found through the entity table.
    /// </summary>
    public Entity StaticObject;

    /// <summary>When this tile last dealt damage, for its half-second cooldown.</summary>
    public int LastDamageMs = int.MinValue;

    /// <summary>How deep a sprite standing here sinks, in pixels.</summary>
    public int Sink;

    /// <summary>Whether the tile exists as far as the client is concerned.</summary>
    public bool IsKnown => TileType != NotReceived && Desc != null;

    /// <summary>Whether anything can stand here.</summary>
    public bool IsWalkable =>
        IsKnown
        && TileType != EmptyTerrain
        && !Desc.NoWalk
        && (StaticObject?.Desc == null || !StaticObject.Desc.OccupySquare);

    /// <summary>
    /// Whether the tile blocks completely, pushing back on movement in neighbouring tiles too.
    /// </summary>
    public bool IsFullOccupy =>
        !IsKnown
        || TileType == EmptyTerrain
        || (StaticObject?.Desc != null && StaticObject.Desc.FullOccupy);

    /// <summary>Whether a projectile terminates on reaching this tile.</summary>
    public bool StopsProjectiles => !IsKnown || TileType == EmptyTerrain;
}
