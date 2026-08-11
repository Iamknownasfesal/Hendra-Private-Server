using System;
using System.IO;
using System.Text;
using Hendra.Net;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// Checks the serialisation primitives against <c>common/NReader.cs</c> and
/// <c>common/NWriter.cs</c> compiled straight out of the server tree.
///
/// The client's versions are a deliberate rewrite — span-based rather than stream-based, so that
/// decoding a packet allocates nothing but its strings — which makes it worth proving the rewrite
/// produces identical bytes rather than assuming it.
/// </summary>
public sealed class WireFormatParityTests
{
    private static byte[] ServerWrite(Action<common.NWriter> write)
    {
        using var stream = new MemoryStream();
        var writer = new common.NWriter(stream);
        write(writer);
        writer.Flush();
        return stream.ToArray();
    }

    private static byte[] ClientWrite(Action<NetWriter> write)
    {
        var writer = new NetWriter();
        write(writer);
        return writer.WrittenSpan.ToArray();
    }

    [Fact]
    public void Integers_AreWrittenIdentically()
    {
        short[] shorts = { 0, 1, -1, short.MinValue, short.MaxValue, 0x1234 };
        int[] ints = { 0, 1, -1, int.MinValue, int.MaxValue, 0x12345678 };
        uint[] uints = { 0u, 1u, uint.MaxValue, 0x80000000u, 0xDEADBEEFu };

        foreach (short value in shorts)
            Assert.Equal(ServerWrite(w => w.Write(value)), ClientWrite(w => w.Write(value)));

        foreach (int value in ints)
            Assert.Equal(ServerWrite(w => w.Write(value)), ClientWrite(w => w.Write(value)));

        foreach (uint value in uints)
            Assert.Equal(ServerWrite(w => w.Write(value)), ClientWrite(w => w.Write(value)));
    }

    [Fact]
    public void Floats_AreWrittenIdentically()
    {
        // Positions and angles are floats, and they are byte-reversed rather than written through
        // a network-order helper, so this is worth pinning down explicitly.
        float[] values = { 0f, 1f, -1f, 0.5f, 3.14159f, -273.15f, float.MaxValue, float.MinValue };

        foreach (float value in values)
            Assert.Equal(ServerWrite(w => w.Write(value)), ClientWrite(w => w.Write(value)));
    }

    [Fact]
    public void Strings_AreWrittenIdentically()
    {
        string[] values = { "", "a", "Alpha v01", "a name with spaces", "unicode: é中文" };

        foreach (string value in values)
        {
            Assert.Equal(ServerWrite(w => w.WriteUTF(value)), ClientWrite(w => w.WriteUtf(value)));
            Assert.Equal(ServerWrite(w => w.Write32UTF(value)), ClientWrite(w => w.Write32Utf(value)));
        }
    }

    [Fact]
    public void NullString_IsWrittenAsZeroLength()
    {
        // The server special-cases null in WriteUTF; matching it means a missing optional string
        // never desynchronises the reader.
        Assert.Equal(ServerWrite(w => w.WriteUTF(null)), ClientWrite(w => w.WriteUtf(null)));
    }

    [Fact]
    public void Reader_MatchesServerWriter()
    {
        byte[] payload = ServerWrite(w =>
        {
            w.Write((byte)0xAB);
            w.Write(true);
            w.Write((short)-1234);
            w.Write((ushort)54321);
            w.Write(0x12345678);
            w.Write(0xDEADBEEFu);
            w.Write(1.5f);
            w.WriteUTF("hello");
            w.Write32UTF("wide");
        });

        var reader = new NetReader(payload);
        Assert.Equal(0xAB, reader.ReadByte());
        Assert.True(reader.ReadBoolean());
        Assert.Equal((short)-1234, reader.ReadInt16());
        Assert.Equal((ushort)54321, reader.ReadUInt16());
        Assert.Equal(0x12345678, reader.ReadInt32());
        Assert.Equal(0xDEADBEEFu, reader.ReadUInt32());
        Assert.Equal(1.5f, reader.ReadSingle());
        Assert.Equal("hello", reader.ReadUtf());
        Assert.Equal("wide", reader.Read32Utf());
        Assert.Equal(0, reader.Remaining);
    }

    [Fact]
    public void Reader_RejectsATruncatedBody()
    {
        // A short read must throw rather than return silence. The receive loop treats this as
        // fatal, because a body it could not parse means it can no longer trust its position in
        // the RC4 keystream.
        var reader = new NetReader(new byte[] { 0x00, 0x01 });
        Assert.Throws<PacketFormatException>(() =>
        {
            var r = new NetReader(new byte[] { 0x00, 0x01 });
            r.ReadInt32();
        });
        Assert.Equal(2, reader.Remaining);
    }

    [Fact]
    public void ReadDotNetString_MatchesBinaryWriter()
    {
        // KeyInfoResponse is written with BinaryWriter.Write(string) rather than WriteUTF, because
        // NWriter does not override it. The AS3 client read those fields as ordinary protocol
        // strings and misparsed the packet; we decode them the way the server actually writes them.
        foreach (string value in new[] { "", "Key", new string('x', 200) })
        {
            using var stream = new MemoryStream();
            var writer = new common.NWriter(stream);
            writer.Write(value);
            writer.Flush();

            var reader = new NetReader(stream.ToArray());
            Assert.Equal(value, reader.ReadDotNetString());
            Assert.Equal(0, reader.Remaining);
        }
    }

    [Fact]
    public void LongStrings_UseAMultiByteDotNetLengthPrefix()
    {
        // Guards the loop in ReadDotNetString: anything past 127 bytes needs a second length byte.
        string value = new('y', 5000);
        using var stream = new MemoryStream();
        var writer = new common.NWriter(stream);
        writer.Write(value);
        writer.Flush();

        var reader = new NetReader(stream.ToArray());
        Assert.Equal(value, reader.ReadDotNetString());
    }

    [Fact]
    public void Writer_GrowsPastItsInitialCapacity()
    {
        // The send path reuses one writer for every packet, so its buffer has to grow on demand
        // and still produce correct bytes afterwards.
        var writer = new NetWriter(initialCapacity: 8);
        var expected = new byte[4096];
        for (int i = 0; i < expected.Length; i++)
            expected[i] = (byte)i;

        writer.Write(expected);
        Assert.Equal(expected, writer.WrittenSpan.ToArray());

        writer.Reset();
        Assert.Equal(0, writer.Length);
    }

    [Fact]
    public void Encoding_IsUtf8NotUtf16()
    {
        // Flash's writeUTF and .NET's UTF8 agree on byte length only if both treat the string as
        // UTF-8. A regression to UTF-16 here would double every string length silently.
        const string value = "abc";
        byte[] written = ClientWrite(w => w.WriteUtf(value));
        Assert.Equal(2 + Encoding.UTF8.GetByteCount(value), written.Length);
    }
}
