using System;
using System.Buffers.Binary;
using System.Text;

namespace Hendra.Net;

/// <summary>
/// Big-endian writer over a growable buffer, mirroring common/NWriter.cs on the server.
///
/// A single instance is reused for every outbound packet (see <see cref="Reset"/>), so steady-state
/// sending allocates nothing.
/// </summary>
public sealed class NetWriter
{
    private byte[] _buffer;
    private int _pos;

    public NetWriter(int initialCapacity = 1024)
    {
        _buffer = new byte[initialCapacity];
        _pos = 0;
    }

    public int Length => _pos;
    public ReadOnlySpan<byte> WrittenSpan => _buffer.AsSpan(0, _pos);

    /// <summary>
    /// The written bytes as a mutable span, so the caller can encipher in place instead of copying
    /// into a second buffer. Only valid until the next write or <see cref="Reset"/>.
    /// </summary>
    public Span<byte> WrittenSpanForCrypt() => _buffer.AsSpan(0, _pos);

    public void Reset() => _pos = 0;

    private Span<byte> Advance(int count)
    {
        if (_pos + count > _buffer.Length)
        {
            int capacity = _buffer.Length == 0 ? 1024 : _buffer.Length;
            while (capacity < _pos + count)
                capacity *= 2;
            Array.Resize(ref _buffer, capacity);
        }
        var slice = _buffer.AsSpan(_pos, count);
        _pos += count;
        return slice;
    }

    public void Write(byte value) => Advance(1)[0] = value;
    public void Write(sbyte value) => Write((byte)value);
    public void Write(bool value) => Write((byte)(value ? 1 : 0));

    public void Write(short value) => BinaryPrimitives.WriteInt16BigEndian(Advance(2), value);
    public void Write(ushort value) => BinaryPrimitives.WriteUInt16BigEndian(Advance(2), value);
    public void Write(int value) => BinaryPrimitives.WriteInt32BigEndian(Advance(4), value);
    public void Write(uint value) => BinaryPrimitives.WriteUInt32BigEndian(Advance(4), value);
    public void Write(long value) => BinaryPrimitives.WriteInt64BigEndian(Advance(8), value);
    public void Write(ulong value) => BinaryPrimitives.WriteUInt64BigEndian(Advance(8), value);
    public void Write(float value) => BinaryPrimitives.WriteSingleBigEndian(Advance(4), value);
    public void Write(double value) => BinaryPrimitives.WriteDoubleBigEndian(Advance(8), value);

    public void Write(ReadOnlySpan<byte> bytes) => bytes.CopyTo(Advance(bytes.Length));

    /// <summary>int16 byte-length prefix, then UTF-8. Null is written as a zero-length string.</summary>
    public void WriteUtf(string value)
    {
        if (string.IsNullOrEmpty(value))
        {
            Write((ushort)0);
            return;
        }
        int len = Encoding.UTF8.GetByteCount(value);
        Write((ushort)len);
        Encoding.UTF8.GetBytes(value, Advance(len));
    }

    /// <summary>int32 byte-length prefix, then UTF-8.</summary>
    public void Write32Utf(string value)
    {
        if (string.IsNullOrEmpty(value))
        {
            Write(0);
            return;
        }
        int len = Encoding.UTF8.GetByteCount(value);
        Write(len);
        Encoding.UTF8.GetBytes(value, Advance(len));
    }
}
