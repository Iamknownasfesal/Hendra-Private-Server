using System;
using System.Collections.Generic;
using Hendra.Data;

namespace Hendra.Net.Packets;

/// <summary>
/// Opens the session. Everything the server needs to authenticate us and decide which world to
/// place us in.
/// </summary>
/// <remarks>
/// Two traps here. <see cref="BuildVersion"/> must equal <see cref="ProtocolKeys.BuildVersion"/>
/// exactly — on a mismatch the server's handler returns without writing anything, so the socket
/// stays open and silent forever rather than reporting an error. And <see cref="MapJson"/> is the
/// only string in the protocol with a 32-bit length prefix; every other string is 16-bit.
/// </remarks>
public sealed class HelloPacket : ClientPacket
{
    public override PacketId Id => PacketId.Hello;

    public string BuildVersion = ProtocolKeys.BuildVersion;
    public int GameId;

    /// <summary>Account id. Encrypted on write — assign the plaintext.</summary>
    public string Guid = string.Empty;

    /// <summary>Account password. Encrypted on write — assign the plaintext.</summary>
    public string Password = string.Empty;

    /// <summary>Always empty in this fork, but still RSA-encrypted (an empty string stays empty).</summary>
    public string Secret = string.Empty;

    /// <summary>-1 for a fresh connection; echoed from a Reconnect otherwise.</summary>
    public int KeyTime = -1;

    /// <summary>Empty for a fresh connection; the 16 bytes from a Reconnect otherwise.</summary>
    public byte[] Key = Array.Empty<byte>();

    /// <summary>Only meaningful for map-test worlds; empty elsewhere.</summary>
    public string MapJson = string.Empty;

    public override void Write(NetWriter w)
    {
        w.WriteUtf(BuildVersion);
        w.Write(GameId);
        w.WriteUtf(ProtocolKeys.EncryptCredential(Guid));
        w.WriteUtf(ProtocolKeys.EncryptCredential(Password));
        w.WriteUtf(ProtocolKeys.EncryptCredential(Secret));
        w.Write(KeyTime);
        w.Write((short)Key.Length);
        w.Write(Key);
        w.Write32Utf(MapJson);
    }
}

/// <summary>Loads an existing character. Only accepted while the server is in the Handshaked state.</summary>
public sealed class LoadPacket : ClientPacket
{
    public override PacketId Id => PacketId.Load;

    public int CharId;
    public bool IsFromArena;

    public override void Write(NetWriter w)
    {
        w.Write(CharId);
        w.Write(IsFromArena);
    }
}

/// <summary>Creates a new character. Only accepted while the server is in the Handshaked state.</summary>
public sealed class CreatePacket : ClientPacket
{
    public override PacketId Id => PacketId.Create;

    public ushort ClassType;
    public ushort SkinType;

    public override void Write(NetWriter w)
    {
        w.Write(ClassType);
        w.Write(SkinType);
    }
}

/// <summary>
/// The movement heartbeat: exactly one per NewTick, echoing that tick's id.
/// </summary>
/// <remarks>
/// The server dequeues an expected tick id for every Move and disconnects on a mismatch, on an id
/// ahead of its own counter, or on a second Move for the same tick. <see cref="Time"/> must be the
/// per-frame clock snapshot, not a live reading. A paused player sends (-1, -1), which the server
/// reads as "unchanged".
/// </remarks>
public sealed class MovePacket : ClientPacket
{
    public override PacketId Id => PacketId.Move;

    public int ObjectId;
    public int TickId;
    public int Time;
    public WorldPos NewPosition;
    public List<MoveRecord> Records = new();

    public override void Write(NetWriter w)
    {
        w.Write(ObjectId);
        w.Write(TickId);
        w.Write(Time);
        NewPosition.Write(w);
        w.Write((short)Records.Count);
        foreach (var record in Records)
            record.Write(w);
    }
}

/// <summary>
/// Acknowledges an Update. Empty body; the server only counts these, and both over-sending and
/// letting one lapse for 12 seconds are disconnects.
/// </summary>
public sealed class UpdateAckPacket : ClientPacket
{
    public override PacketId Id => PacketId.UpdateAck;

    public override void Write(NetWriter w)
    {
        // No body.
    }
}

/// <summary>
/// Acknowledges a Goto. One is owed for every Goto received — including the ones generated when
/// *another* player teleports, since the server broadcasts those to the whole world.
/// </summary>
public sealed class GotoAckPacket : ClientPacket
{
    public override PacketId Id => PacketId.GotoAck;

    public int Time;

    public override void Write(NetWriter w) => w.Write(Time);
}

/// <summary>Replies to an in-world Ping. The server does not check the serial here, but we echo it anyway.</summary>
public sealed class PongPacket : ClientPacket
{
    public override PacketId Id => PacketId.Pong;

    public int Serial;
    public int Time;

    public override void Write(NetWriter w)
    {
        w.Write(Serial);
        w.Write(Time);
    }
}

/// <summary>
/// Replies to a QueuePing while waiting in the connection queue. Unlike Pong, the serial *is*
/// checked — a mismatched one does not refresh the timer and we get dropped after 15 seconds.
/// </summary>
public sealed class QueuePongPacket : ClientPacket
{
    public override PacketId Id => PacketId.QueuePong;

    public int Serial;
    public int Time;

    public override void Write(NetWriter w)
    {
        w.Write(Serial);
        w.Write(Time);
    }
}

/// <summary>
/// Reports whether a shot the server told us about could be spawned locally.
/// </summary>
/// <remarks>
/// A <see cref="Time"/> of -1 is the distinct "could not spawn it" signal, sent when the shooting
/// entity is missing or already dead. The server's handler is currently an empty body, so nothing
/// observable depends on this, but it is cheap to keep faithful.
/// </remarks>
public sealed class ShootAckPacket : ClientPacket
{
    public override PacketId Id => PacketId.ShootAck;

    public int Time;

    public override void Write(NetWriter w) => w.Write(Time);
}

/// <summary>Acknowledges an area-of-effect blast. The server's handler is a no-op.</summary>
public sealed class AoeAckPacket : ClientPacket
{
    public override PacketId Id => PacketId.AoeAck;

    public int Time;
    public WorldPos Position;

    public override void Write(NetWriter w)
    {
        w.Write(Time);
        Position.Write(w);
    }
}

/// <summary>
/// Returns to the Nexus.
/// </summary>
/// <remarks>
/// Present for completeness but deliberately never sent. This fork's client performs a local
/// reconnect to game id -2 instead, and the server's handler hard-disconnects anyone who sends
/// Escape while already in the Nexus.
/// </remarks>
public sealed class EscapePacket : ClientPacket
{
    public override PacketId Id => PacketId.Escape;

    public override void Write(NetWriter w)
    {
        // No body.
    }
}
