using System;
using System.Buffers.Binary;
using System.Text;

namespace Hendra.Net;

/// <summary>
/// Big-endian reader over a packet body.
///
/// Mirrors common/NReader.cs on the server, which is a BinaryReader subclass that byte-swaps every
/// scalar. We read straight out of a span instead so that decoding a packet allocates nothing but
/// the strings it actually contains — the server wraps each body in a fresh MemoryStream, which is
/// the sort of per-packet garbage worth not reproducing.
/// </summary>
public ref struct NetReader
{
    private readonly ReadOnlySpan<byte> _data;
    private int _pos;

    public NetReader(ReadOnlySpan<byte> data)
    {
        _data = data;
        _pos = 0;
    }

    public int Position => _pos;
    public int Remaining => _data.Length - _pos;

    private ReadOnlySpan<byte> Take(int count)
    {
        if (count < 0 || _pos + count > _data.Length)
            throw new PacketFormatException(
                $"Read of {count} byte(s) at offset {_pos} runs past the {_data.Length}-byte body.");
        var slice = _data.Slice(_pos, count);
        _pos += count;
        return slice;
    }

    public byte ReadByte() => Take(1)[0];
    public sbyte ReadSByte() => (sbyte)ReadByte();

    /// <summary>
    /// Booleans are a single byte, non-zero being true. The server writes them via BinaryWriter,
    /// which emits 0 or 1.
    /// </summary>
    public bool ReadBoolean() => ReadByte() != 0;

    public short ReadInt16() => BinaryPrimitives.ReadInt16BigEndian(Take(2));
    public ushort ReadUInt16() => BinaryPrimitives.ReadUInt16BigEndian(Take(2));
    public int ReadInt32() => BinaryPrimitives.ReadInt32BigEndian(Take(4));
    public uint ReadUInt32() => BinaryPrimitives.ReadUInt32BigEndian(Take(4));
    public long ReadInt64() => BinaryPrimitives.ReadInt64BigEndian(Take(8));
    public ulong ReadUInt64() => BinaryPrimitives.ReadUInt64BigEndian(Take(8));
    public float ReadSingle() => BinaryPrimitives.ReadSingleBigEndian(Take(4));
    public double ReadDouble() => BinaryPrimitives.ReadDoubleBigEndian(Take(8));

    /// <summary>int16 byte-length prefix, then UTF-8. The protocol's default string form.</summary>
    public string ReadUtf()
    {
        int len = ReadUInt16();
        return len == 0 ? string.Empty : Encoding.UTF8.GetString(Take(len));
    }

    /// <summary>
    /// int32 byte-length prefix, then UTF-8. Used only where a string can exceed 64 KB — the
    /// per-map XML blobs in MapInfo and Hello.MapJSON.
    /// </summary>
    public string Read32Utf()
    {
        int len = ReadInt32();
        return len == 0 ? string.Empty : Encoding.UTF8.GetString(Take(len));
    }

    /// <summary>
    /// A string in .NET's <c>BinaryWriter.Write(string)</c> format: a 7-bit-encoded length prefix
    /// (one byte for anything under 128 bytes) followed by UTF-8.
    /// </summary>
    /// <remarks>
    /// This is not the protocol's string format, and it exists for exactly one packet.
    /// <c>KeyInfoResponse</c> on the server writes its three fields with <c>wtr.Write(...)</c>
    /// instead of <c>wtr.WriteUTF(...)</c>, and <c>NWriter</c> does not override
    /// <c>Write(string)</c> — so those fields go out length-prefixed the .NET way while the AS3
    /// client reads them with <c>readUTF</c> and misparses. Decoding it the way the server actually
    /// writes it is the only way to be compatible with the server as it stands.
    /// </remarks>
    public string ReadDotNetString()
    {
        int len = 0;
        int shift = 0;
        while (true)
        {
            if (shift == 5 * 7)
                throw new PacketFormatException("7-bit encoded string length is malformed.");
            byte b = ReadByte();
            len |= (b & 0x7F) << shift;
            shift += 7;
            if ((b & 0x80) == 0)
                break;
        }
        return len == 0 ? string.Empty : Encoding.UTF8.GetString(Take(len));
    }

    public byte[] ReadBytes(int count) => Take(count).ToArray();

    /// <summary>Reads whatever is left. Used by the raw pixel blob in the Pic packet.</summary>
    public byte[] ReadRemainingBytes() => Take(Remaining).ToArray();
}

/// <summary>
/// Thrown when a packet body does not decode. The caller should treat this as fatal for the
/// connection: the RC4 keystreams are positional, so a body we could not parse means we can no
/// longer trust our place in the stream.
/// </summary>
public sealed class PacketFormatException : Exception
{
    public PacketFormatException(string message) : base(message) { }
}
