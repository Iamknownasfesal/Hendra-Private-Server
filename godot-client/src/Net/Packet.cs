namespace Hendra.Net;

/// <summary>
/// Base for every packet. Packets are named and typed from the *client's* point of view, which is
/// the inverse of the server's naming: what wServer calls an OutgoingMessage is a
/// <see cref="ServerPacket"/> here, and what it calls an IncomingMessage is a
/// <see cref="ClientPacket"/>.
/// </summary>
public abstract class Packet
{
    public abstract PacketId Id { get; }
}

/// <summary>
/// A packet the server sends and we decode. Mirrors wServer's OutgoingMessage set.
/// </summary>
public abstract class ServerPacket : Packet
{
    public abstract void Read(ref NetReader r);
}

/// <summary>
/// A packet we encode and send. Mirrors wServer's IncomingMessage set.
/// </summary>
public abstract class ClientPacket : Packet
{
    public abstract void Write(NetWriter w);
}
