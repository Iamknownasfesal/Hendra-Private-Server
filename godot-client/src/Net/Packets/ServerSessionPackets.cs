using System;
using Hendra.Data;

namespace Hendra.Net.Packets;

/// <summary>Known values of <see cref="FailurePacket.ErrorId"/>.</summary>
public static class FailureCode
{
    public const int IncorrectVersion = 4;
    public const int BadKey = 5;
    public const int InvalidTeleportTarget = 6;
    public const int EmailVerificationNeeded = 7;

    /// <summary>The description is a JSON dialog spec: <c>{build, title, description}</c>.</summary>
    public const int JsonDialog = 8;
}

/// <summary>A fatal or advisory error. Some codes end the session, others are informational.</summary>
public sealed class FailurePacket : ServerPacket
{
    public override PacketId Id => PacketId.Failure;

    public int ErrorId;
    public string ErrorDescription;

    public override void Read(ref NetReader r)
    {
        ErrorId = r.ReadInt32();
        ErrorDescription = r.ReadUtf();
    }
}

/// <summary>
/// Describes the world we are entering. The first packet the server sends after Hello.
/// </summary>
/// <remarks>
/// <see cref="Seed"/> is the shared PRNG seed — reseed the damage generator from it on every
/// MapInfo, since it changes on every map. Receiving this also starts a 15-second window in which
/// we must send Load or Create.
/// </remarks>
public sealed class MapInfoPacket : ServerPacket
{
    public override PacketId Id => PacketId.MapInfo;

    public int Width;
    public int Height;
    public string Name;
    public string DisplayName;

    /// <summary>Seed for the synced PRNG. Matches the server's <c>client.Random</c>.</summary>
    public uint Seed;

    public int Background;
    public int Difficulty;
    public bool AllowPlayerTeleport;
    public bool ShowDisplays;

    /// <summary>Per-map object/ground XML additions, merged into the loaded libraries.</summary>
    public string[] ClientXml = Array.Empty<string>();

    public string[] ExtraXml = Array.Empty<string>();
    public string Music;

    public override void Read(ref NetReader r)
    {
        Width = r.ReadInt32();
        Height = r.ReadInt32();
        Name = r.ReadUtf();
        DisplayName = r.ReadUtf();
        Seed = r.ReadUInt32();
        Background = r.ReadInt32();
        Difficulty = r.ReadInt32();
        AllowPlayerTeleport = r.ReadBoolean();
        ShowDisplays = r.ReadBoolean();

        ClientXml = new string[r.ReadInt16()];
        for (int i = 0; i < ClientXml.Length; i++)
            ClientXml[i] = r.Read32Utf();

        ExtraXml = new string[r.ReadInt16()];
        for (int i = 0; i < ExtraXml.Length; i++)
            ExtraXml[i] = r.Read32Utf();

        Music = r.ReadUtf();
    }
}

/// <summary>
/// Confirms our character is in the world. Until this arrives the server has no Player object for
/// us, and several of its handlers dereference it without a null check — so nothing beyond the
/// handshake may be sent before this.
/// </summary>
public sealed class CreateSuccessPacket : ServerPacket
{
    public override PacketId Id => PacketId.CreateSuccess;

    /// <summary>Our own entity id, used to recognise ourselves in Update and NewTick.</summary>
    public int ObjectId;

    public int CharId;

    public override void Read(ref NetReader r)
    {
        ObjectId = r.ReadInt32();
        CharId = r.ReadInt32();
    }
}

/// <summary>
/// A delta of the visible world: new terrain, entities entering view, and entities leaving.
/// Every one of these owes exactly one UpdateAck.
/// </summary>
public sealed class UpdatePacket : ServerPacket
{
    public override PacketId Id => PacketId.Update;

    public GroundTile[] Tiles = Array.Empty<GroundTile>();
    public ObjectDef[] NewObjects = Array.Empty<ObjectDef>();

    /// <summary>Entity ids that have left view or been destroyed.</summary>
    public int[] Drops = Array.Empty<int>();

    public override void Read(ref NetReader r)
    {
        Tiles = new GroundTile[r.ReadInt16()];
        for (int i = 0; i < Tiles.Length; i++)
            Tiles[i] = GroundTile.Read(ref r);

        NewObjects = new ObjectDef[r.ReadInt16()];
        for (int i = 0; i < NewObjects.Length; i++)
            NewObjects[i] = ObjectDef.Read(ref r);

        Drops = new int[r.ReadInt16()];
        for (int i = 0; i < Drops.Length; i++)
            Drops[i] = r.ReadInt32();
    }
}

/// <summary>
/// The per-tick world state delta. Arrives roughly every 166 ms and must be answered with exactly
/// one Move carrying the same <see cref="TickId"/>.
/// </summary>
public sealed class NewTickPacket : ServerPacket
{
    public override PacketId Id => PacketId.NewTick;

    public int TickId;

    /// <summary>Milliseconds the server measured for this tick; drives remote interpolation.</summary>
    public int TickTime;

    public ObjectStats[] Statuses = Array.Empty<ObjectStats>();

    public override void Read(ref NetReader r)
    {
        TickId = r.ReadInt32();
        TickTime = r.ReadInt32();

        Statuses = new ObjectStats[r.ReadInt16()];
        for (int i = 0; i < Statuses.Length; i++)
            Statuses[i] = ObjectStats.Read(ref r);
    }
}

/// <summary>
/// An authoritative position correction — the only thing that repositions our own player, since
/// NewTick never overwrites it. Broadcast to the whole world on any teleport, so these arrive for
/// other players' movements too, and each owes a GotoAck.
/// </summary>
public sealed class GotoPacket : ServerPacket
{
    public override PacketId Id => PacketId.Goto;

    public int ObjectId;
    public WorldPos Position;

    public override void Read(ref NetReader r)
    {
        ObjectId = r.ReadInt32();
        Position = WorldPos.Read(ref r);
    }
}

/// <summary>In-world keepalive, every 3 seconds. Answer with Pong within 12 seconds.</summary>
public sealed class PingPacket : ServerPacket
{
    public override PacketId Id => PacketId.Ping;

    public int Serial;

    public override void Read(ref NetReader r) => Serial = r.ReadInt32();
}

/// <summary>
/// Queue keepalive, every 3 seconds while waiting for a slot. The reply must carry this exact
/// serial or it does not count.
/// </summary>
public sealed class QueuePingPacket : ServerPacket
{
    public override PacketId Id => PacketId.QueuePing;

    public int Serial;
    public int Position;
    public int Count;

    public override void Read(ref NetReader r)
    {
        Serial = r.ReadInt32();
        Position = r.ReadInt32();
        Count = r.ReadInt32();
    }
}

/// <summary>Our position in the connection queue.</summary>
/// <remarks>
/// The AS3 client never registered a handler for this id and would tear down the connection on
/// receipt. Handled properly here.
/// </remarks>
public sealed class ServerFullPacket : ServerPacket
{
    public override PacketId Id => PacketId.ServerFull;

    public int Position;
    public int Count;

    public override void Read(ref NetReader r)
    {
        Position = r.ReadInt32();
        Count = r.ReadInt32();
    }
}

/// <summary>
/// Instructs us to open a new connection, to another world or another host.
/// </summary>
/// <remarks>
/// <see cref="Key"/> and <see cref="GameId"/> must be echoed verbatim in the next Hello; the server
/// checks both and disconnects on a mismatch. An empty <see cref="Host"/> means "same server".
/// </remarks>
public sealed class ReconnectPacket : ServerPacket
{
    public override PacketId Id => PacketId.Reconnect;

    public string Name;
    public string Host;
    public int Port;
    public int GameId;
    public int KeyTime;
    public bool IsFromArena;
    public byte[] Key = Array.Empty<byte>();

    public override void Read(ref NetReader r)
    {
        Name = r.ReadUtf();
        Host = r.ReadUtf();
        Port = r.ReadInt32();
        GameId = r.ReadInt32();
        KeyTime = r.ReadInt32();
        IsFromArena = r.ReadBoolean();
        Key = r.ReadBytes(r.ReadInt16());
    }
}

/// <summary>Our character died. <c>ZombieId != -1</c> means we come back as a zombie.</summary>
public sealed class DeathPacket : ServerPacket
{
    public override PacketId Id => PacketId.Death;

    public string AccountId;
    public int CharId;
    public string KilledBy;
    public int ZombieType;
    public int ZombieId;

    public bool IsZombie => ZombieId != -1;

    public override void Read(ref NetReader r)
    {
        AccountId = r.ReadUtf();
        CharId = r.ReadInt32();
        KilledBy = r.ReadUtf();
        ZombieType = r.ReadInt32();
        ZombieId = r.ReadInt32();
    }
}

/// <summary>Marks an entity as the current quest objective, for the on-screen arrow.</summary>
public sealed class QuestObjIdPacket : ServerPacket
{
    public override PacketId Id => PacketId.QuestObjId;

    public int ObjectId;

    public override void Read(ref NetReader r) => ObjectId = r.ReadInt32();
}

/// <summary>Points the camera at an entity other than our player (spectator and cinematic use).</summary>
public sealed class SetFocusPacket : ServerPacket
{
    public override PacketId Id => PacketId.SetFocus;

    public int ObjectId;

    public override void Read(ref NetReader r) => ObjectId = r.ReadInt32();
}

/// <summary>Changes the background music track.</summary>
public sealed class SwitchMusicPacket : ServerPacket
{
    public override PacketId Id => PacketId.SwitchMusic;

    public string Music;

    public override void Read(ref NetReader r) => Music = r.ReadUtf();
}

/// <summary>An account-level telemetry stat. The AS3 web account discarded these.</summary>
public sealed class ClientStatPacket : ServerPacket
{
    public override PacketId Id => PacketId.ClientStat;

    public string Name;
    public int Value;

    public override void Read(ref NetReader r)
    {
        Name = r.ReadUtf();
        Value = r.ReadInt32();
    }
}

/// <summary>A file the server wants us to save locally.</summary>
public sealed class FilePacket : ServerPacket
{
    public override PacketId Id => PacketId.File;

    public string Name;
    public byte[] Bytes = Array.Empty<byte>();

    public override void Read(ref NetReader r)
    {
        Name = r.ReadUtf();
        Bytes = r.ReadBytes(r.ReadInt32());
    }
}

/// <summary>A raw 32-bit ARGB image to display as an overlay.</summary>
public sealed class PicPacket : ServerPacket
{
    public override PacketId Id => PacketId.Pic;

    public int Width;
    public int Height;

    /// <summary>Width * Height * 4 bytes, in the order the server's BitmapData struct writes them.</summary>
    public byte[] Pixels = Array.Empty<byte>();

    public override void Read(ref NetReader r)
    {
        Width = r.ReadInt32();
        Height = r.ReadInt32();
        Pixels = r.ReadBytes(Width * Height * 4);
    }
}

/// <summary>Prompts for email verification. No body.</summary>
public sealed class VerifyEmailPacket : ServerPacket
{
    public override PacketId Id => PacketId.VerifyEmail;

    public override void Read(ref NetReader r)
    {
        // No body.
    }
}

/// <summary>Prompts for a password reset. Status 2 = prompt, 3 = forced, 4 = registration.</summary>
public sealed class PasswordPromptPacket : ServerPacket
{
    public override PacketId Id => PacketId.PasswordPrompt;

    public int CleanPasswordStatus;

    public override void Read(ref NetReader r) => CleanPasswordStatus = r.ReadInt32();
}
