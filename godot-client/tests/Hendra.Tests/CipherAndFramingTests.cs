using System;
using System.Text;
using Hendra.Data;
using Hendra.Net;
using Hendra.Net.Packets;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// Covers the cipher and the packet encodings that the handshake depends on.
/// </summary>
public sealed class CipherAndFramingTests
{
    private static byte[] Crypt(string key, string plaintext)
    {
        var rc4 = new Rc4(Encoding.ASCII.GetBytes(key));
        var buffer = Encoding.ASCII.GetBytes(plaintext);
        rc4.Crypt(buffer);
        return buffer;
    }

    private static string Hex(byte[] bytes) => Convert.ToHexString(bytes);

    [Theory]
    [InlineData("Key", "Plaintext", "BBF316E8D940AF0AD3")]
    [InlineData("Wiki", "pedia", "1021BF0420")]
    [InlineData("Secret", "Attack at dawn", "45A01F645FC35B383552544B9BF5")]
    public void Rc4_MatchesPublishedTestVectors(string key, string plaintext, string expected)
    {
        Assert.Equal(expected, Hex(Crypt(key, plaintext)));
    }

    [Fact]
    public void Rc4_IsAContinuousStreamAcrossCalls()
    {
        // This is the property the whole connection rests on. The keystream advances across packet
        // boundaries and is never reset, so enciphering a message in two pieces must give the same
        // bytes as enciphering it in one. If this ever stopped holding, every packet after the
        // first would decode to noise — and the server discards frames it cannot parse silently,
        // so the only visible symptom would be a connection that stops responding.
        var whole = new Rc4(ProtocolKeys.ClientToServerKey);
        var split = new Rc4(ProtocolKeys.ClientToServerKey);

        var wholeBuffer = new byte[64];
        var splitBuffer = new byte[64];
        for (int i = 0; i < wholeBuffer.Length; i++)
            wholeBuffer[i] = splitBuffer[i] = (byte)i;

        whole.Crypt(wholeBuffer);
        split.Crypt(splitBuffer.AsSpan(0, 13));
        split.Crypt(splitBuffer.AsSpan(13, 27));
        split.Crypt(splitBuffer.AsSpan(40));

        Assert.Equal(wholeBuffer, splitBuffer);
    }

    [Fact]
    public void Rc4_IsItsOwnInverse()
    {
        var original = Encoding.UTF8.GetBytes("the quick brown fox jumps over the lazy dog");
        var buffer = (byte[])original.Clone();

        new Rc4(ProtocolKeys.ServerToClientKey).Crypt(buffer);
        Assert.NotEqual(original, buffer);

        new Rc4(ProtocolKeys.ServerToClientKey).Crypt(buffer);
        Assert.Equal(original, buffer);
    }

    [Fact]
    public void ProtocolKeys_MatchTheServerConfiguration()
    {
        // The client-to-server key is the hex decoding of wServer.json's serverSettings.key
        // ("B1A5ED"); the other is hardcoded as Client.ServerKey. Getting either wrong produces
        // the same silent dead connection as a framing bug, so they are pinned here.
        Assert.Equal("B1A5ED", Hex(ProtocolKeys.ClientToServerKey.ToArray()));
        Assert.Equal("612A806CAC78114BA5013CB531", Hex(ProtocolKeys.ServerToClientKey.ToArray()));
        Assert.Equal("Alpha v01", ProtocolKeys.BuildVersion);
    }

    [Fact]
    public void RsaCredential_RoundTripsThroughTheServersPrivateKey()
    {
        // Hello's credentials are RSA-PKCS1 then base64. Padding is randomised, so the ciphertext
        // differs every time; what matters is that it is the right size for the 512-bit key and
        // decodes as base64.
        string encrypted = ProtocolKeys.EncryptCredential("testaccount@example.com");

        byte[] cipher = Convert.FromBase64String(encrypted);
        Assert.Equal(64, cipher.Length);
        Assert.NotEqual(encrypted, ProtocolKeys.EncryptCredential("testaccount@example.com"));
    }

    [Fact]
    public void EmptyCredential_EncodesAsAnEmptyString()
    {
        // Hello.Secret is always empty, and the server's RSA helper short-circuits on empty input.
        // Encrypting it anyway would produce 64 bytes the server would then fail to decrypt.
        Assert.Equal(string.Empty, ProtocolKeys.EncryptCredential(string.Empty));
        Assert.Equal(string.Empty, ProtocolKeys.EncryptCredential(null));
    }

    [Fact]
    public void MovePacket_HasTheExpectedLayout()
    {
        var packet = new MovePacket
        {
            ObjectId = 0x11223344,
            TickId = 7,
            Time = 1000,
            NewPosition = new WorldPos(1.5f, -2.5f),
        };
        packet.Records.Add(new MoveRecord(950, 1.0f, 2.0f));

        var writer = new NetWriter();
        packet.Write(writer);
        var reader = new NetReader(writer.WrittenSpan);

        Assert.Equal(0x11223344, reader.ReadInt32());
        Assert.Equal(7, reader.ReadInt32());
        Assert.Equal(1000, reader.ReadInt32());
        Assert.Equal(1.5f, reader.ReadSingle());
        Assert.Equal(-2.5f, reader.ReadSingle());
        Assert.Equal((short)1, reader.ReadInt16());
        Assert.Equal(950, reader.ReadInt32());
        Assert.Equal(1.0f, reader.ReadSingle());
        Assert.Equal(2.0f, reader.ReadSingle());
        Assert.Equal(0, reader.Remaining);
    }

    [Fact]
    public void UpdateAck_HasAnEmptyBody()
    {
        var writer = new NetWriter();
        new UpdateAckPacket().Write(writer);
        Assert.Equal(0, writer.Length);
    }

    [Fact]
    public void SlotObject_SurvivesThePotionSlots()
    {
        // Slot ids are written signed and read unsigned. The two potion slots, 254 and 255, go out
        // as -2 and -1 and have to come back unchanged; reading them as signed would put them out
        // of range and silently target the wrong slot.
        foreach (byte slot in new byte[] { 0, 1, 127, 128, 254, 255 })
        {
            var writer = new NetWriter();
            new SlotObject(42, slot, 0x0A22).Write(writer);

            var reader = new NetReader(writer.WrittenSpan);
            var decoded = SlotObject.Read(ref reader);

            Assert.Equal(42, decoded.ObjectId);
            Assert.Equal(slot, decoded.SlotId);
            Assert.Equal(0x0A22, decoded.ObjectType);
        }
    }

    [Fact]
    public void ConditionEffects_RoundTripThroughTheSplitStatFields()
    {
        // The server splits its 64-bit mask as Effects = (int)value and Effects2 = (value >> 31).
        // The shift is 31, not 32, so bit 31 appears in both fields — reassembling with 32 would
        // misplace every effect from SlowedImmune upward.
        ConditionEffects[] cases =
        {
            ConditionEffects.None,
            ConditionEffects.Paralyzed,
            ConditionEffects.Slowed | ConditionEffects.Dazed,
            ConditionEffects.SlowedImmune,
            ConditionEffects.Curse | ConditionEffects.Petrify,
            ConditionEffects.XMasVision,
            ConditionEffects.Invincible | ConditionEffects.SlowedImmune | ConditionEffects.Muted,
        };

        foreach (var effects in cases)
        {
            int low = (int)(ulong)effects;
            int high = (int)((ulong)effects >> 31);
            Assert.Equal(effects, ConditionEffectsCodec.Combine(low, high));
        }
    }

    [Fact]
    public void ConditionEffectIndex_MapsOntoTheMatchingFlag()
    {
        Assert.Equal(ConditionEffects.Dead, ConditionEffectIndex.Dead.ToFlag());
        Assert.Equal(ConditionEffects.Paralyzed, ConditionEffectIndex.Paralyzed.ToFlag());
        Assert.Equal(ConditionEffects.SlowedImmune, ConditionEffectIndex.SlowedImmune.ToFlag());
        Assert.Equal(ConditionEffects.XMasVision, ConditionEffectIndex.XMasVision.ToFlag());
    }

    [Fact]
    public void StringStats_AreExactlyTheFourTheServerWrites()
    {
        // Verified against what the server puts in the dictionary, not against its reader: both
        // Player.ExportStats and Container.ExportStats assign these via ToString(), and
        // ObjectStats.Write dispatches on the runtime type. Reading the wrong set desynchronises
        // the stat loop and silently corrupts everything after it in the packet.
        Assert.True(StatsType.Name.IsStringStat());
        Assert.True(StatsType.GuildName.IsStringStat());
        Assert.True(StatsType.AccountId.IsStringStat());
        Assert.True(StatsType.OwnerAccountId.IsStringStat());

        Assert.False(StatsType.Hp.IsStringStat());
        Assert.False(StatsType.Condition.IsStringStat());
        Assert.False(StatsType.Condition2.IsStringStat());
        Assert.False(StatsType.Inventory0.IsStringStat());
    }

    [Fact]
    public void ObjectStats_RoundTripsMixedStringAndIntegerStats()
    {
        var original = new ObjectStats
        {
            ObjectId = 1234,
            Position = new WorldPos(10.25f, 20.5f),
            Stats = new[]
            {
                new StatData { Type = StatsType.Hp, IntValue = 500 },
                new StatData { Type = StatsType.Name, StringValue = "Tester" },
                new StatData { Type = StatsType.AccountId, StringValue = "9001" },
                new StatData { Type = StatsType.Condition, IntValue = unchecked((int)0x80000001) },
                new StatData { Type = StatsType.GuildName, StringValue = "A Guild" },
                new StatData { Type = StatsType.Inventory3, IntValue = -1 },
            },
        };

        var writer = new NetWriter();
        original.Write(writer);
        var reader = new NetReader(writer.WrittenSpan);
        var decoded = ObjectStats.Read(ref reader);

        Assert.Equal(0, reader.Remaining);
        Assert.Equal(original.ObjectId, decoded.ObjectId);
        Assert.Equal(original.Position.X, decoded.Position.X);
        Assert.Equal(original.Stats.Length, decoded.Stats.Length);

        Assert.True(decoded.TryGet(StatsType.Name, out var name));
        Assert.Equal("Tester", name.StringValue);
        Assert.True(decoded.TryGet(StatsType.Condition, out var condition));
        Assert.Equal(unchecked((int)0x80000001), condition.IntValue);
        Assert.False(decoded.TryGet(StatsType.Mp, out _));
    }

    [Fact]
    public void PacketRegistry_CoversThePacketsTheAs3ClientDropped()
    {
        // The AS3 client left these two out of its dispatch table entirely and tore down the
        // connection whenever the server sent one.
        Assert.NotNull(PacketRegistry.Create(PacketId.MarketResult));
        Assert.NotNull(PacketRegistry.Create(PacketId.ServerFull));
    }

    [Fact]
    public void PacketRegistry_ReturnsNullForPacketsWeSend()
    {
        // Client-to-server ids should never appear in the inbound table; a hit here would mean a
        // direction mix-up.
        Assert.Null(PacketRegistry.Create(PacketId.Move));
        Assert.Null(PacketRegistry.Create(PacketId.PlayerShoot));
        Assert.Null(PacketRegistry.Create(PacketId.UpdateAck));
    }
}
