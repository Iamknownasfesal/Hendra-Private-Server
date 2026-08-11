using System;

namespace Hendra.Net;

/// <summary>
/// RC4 keystream, one instance per direction.
///
/// The important property is that this is a *continuous* stream for the whole connection, not a
/// per-packet cipher: the keystream position advances across packet boundaries and is never reset.
/// That makes framing errors unrecoverable — if we ever mis-slice a frame, or drop one, or encrypt
/// a byte we should not have, every subsequent packet in that direction decodes to garbage. The
/// server responds to a malformed frame by silently discarding it, so the symptom is a connection
/// that simply stops working rather than any kind of error.
///
/// Only the packet body is enciphered. The 4-byte length prefix and the 1-byte id are plaintext.
/// </summary>
public sealed class Rc4
{
    private readonly byte[] _state = new byte[256];
    private byte _i;
    private byte _j;

    public Rc4(ReadOnlySpan<byte> key)
    {
        if (key.Length == 0)
            throw new ArgumentException("RC4 key must not be empty.", nameof(key));

        for (int n = 0; n < 256; n++)
            _state[n] = (byte)n;

        byte j = 0;
        for (int n = 0; n < 256; n++)
        {
            j = (byte)(j + _state[n] + key[n % key.Length]);
            (_state[n], _state[j]) = (_state[j], _state[n]);
        }

        _i = 0;
        _j = 0;
    }

    /// <summary>XORs <paramref name="buffer"/> with the next bytes of the keystream, in place.</summary>
    public void Crypt(Span<byte> buffer)
    {
        byte i = _i;
        byte j = _j;
        var s = _state;

        for (int n = 0; n < buffer.Length; n++)
        {
            i = (byte)(i + 1);
            j = (byte)(j + s[i]);
            (s[i], s[j]) = (s[j], s[i]);
            buffer[n] ^= s[(byte)(s[i] + s[j])];
        }

        _i = i;
        _j = j;
    }
}
