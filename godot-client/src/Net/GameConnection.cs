using System;
using System.Buffers.Binary;
using System.Collections.Concurrent;
using System.IO;
using System.Net.Sockets;
using System.Threading;
using System.Threading.Tasks;

namespace Hendra.Net;

/// <summary>
/// The TCP connection to the world server: framing, RC4, and the thread boundary.
/// </summary>
/// <remarks>
/// Frame layout, matching wServer's CommHandler:
/// <code>
/// [int32 big-endian totalLength = bodyLength + 5][uint8 packetId][RC4-enciphered body]
/// </code>
/// The length prefix and the id byte are plaintext; only the body passes through the cipher.
///
/// The cipher is the reason this class is careful about ordering. Each direction has one
/// continuous RC4 keystream for the whole session, so bytes must be enciphered in exactly the order
/// they hit the wire. Sending is therefore serialised end to end under one lock — splitting
/// "encipher" from "write" would let two concurrent sends interleave and desynchronise the stream
/// permanently. The failure mode is silent: the server discards frames it cannot parse without
/// closing the connection or logging anything the client can see.
///
/// Receiving runs on a background task and hands decoded packets to the main thread through a
/// queue, so <see cref="Poll"/> is the only place game state gets touched.
/// </remarks>
public sealed class GameConnection : IDisposable
{
    /// <summary>
    /// Largest frame the server can put on the wire, above which this is a framing error.
    /// </summary>
    /// <remarks>
    /// A megabyte, which is the size of the server's outgoing buffer -- <c>SendToken.Data</c> in
    /// its Server.cs. This used to be 0x20000, taken from the server's <i>receive</i> buffer, which
    /// is the wrong side of the connection: it bounds what a client may send, not what it may be
    /// sent. An Update carrying a few hundred entities at once sails past that, and because a
    /// framing error is unrecoverable -- the RC4 keystream position depends on having consumed
    /// exactly the right bytes -- the connection was dropped rather than merely stuttering.
    /// </remarks>
    private const int MaxFrameLength = 0x100000;

    private const int HeaderLength = 5;

    private readonly ConcurrentQueue<ServerPacket> _inbound = new();
    private readonly object _sendLock = new();
    private readonly NetWriter _sendBody = new();
    private readonly byte[] _sendHeader = new byte[HeaderLength];

    private Socket _socket;
    private NetworkStream _stream;
    private Rc4 _sendCipher;
    private Rc4 _receiveCipher;
    private CancellationTokenSource _cancellation;
    private volatile bool _connected;

    /// <summary>Raised on the background thread when the connection ends, cleanly or otherwise.</summary>
    public event Action<string> Disconnected;

    public bool IsConnected => _connected;

    /// <summary>
    /// Opens the socket and installs both ciphers. The ciphers must be in place before the first
    /// byte is sent, since Hello is already enciphered.
    /// </summary>
    public async Task ConnectAsync(string host, int port, CancellationToken cancellationToken = default)
    {
        if (_connected)
            throw new InvalidOperationException("Already connected.");

        _socket = new Socket(SocketType.Stream, ProtocolType.Tcp) { NoDelay = true };
        await _socket.ConnectAsync(host, port, cancellationToken).ConfigureAwait(false);

        _stream = new NetworkStream(_socket, ownsSocket: false);
        _sendCipher = new Rc4(ProtocolKeys.ClientToServerKey);
        _receiveCipher = new Rc4(ProtocolKeys.ServerToClientKey);
        _cancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        _connected = true;

        _ = Task.Run(() => ReceiveLoopAsync(_cancellation.Token));
    }

    /// <summary>
    /// Serialises, enciphers and writes a packet. Blocking, but only for as long as it takes to
    /// hand the bytes to the socket buffer.
    /// </summary>
    public void Send(ClientPacket packet)
    {
        if (!_connected)
            return;

        lock (_sendLock)
        {
            _sendBody.Reset();
            packet.Write(_sendBody);

            int bodyLength = _sendBody.Length;
            int frameLength = bodyLength + HeaderLength;
            if (frameLength > MaxFrameLength)
                throw new InvalidOperationException(
                    $"{packet.Id} serialises to {frameLength} bytes, over the {MaxFrameLength}-byte frame limit.");

            // The body buffer is enciphered in place, so the writer must not be reused until the
            // bytes are on the wire. That is guaranteed by the lock.
            var body = _sendBody.WrittenSpanForCrypt();
            _sendCipher.Crypt(body);

            BinaryPrimitives.WriteInt32BigEndian(_sendHeader, frameLength);
            _sendHeader[4] = (byte)packet.Id;

            try
            {
                _stream.Write(_sendHeader, 0, HeaderLength);
                if (bodyLength > 0)
                    _stream.Write(body);
            }
            catch (Exception ex)
            {
                Drop($"Send failed: {ex.Message}");
            }
        }
    }

    /// <summary>
    /// Hands over everything decoded since the last call. Call once per frame on the main thread;
    /// nothing else touches game state.
    /// </summary>
    public bool Poll(out ServerPacket packet) => _inbound.TryDequeue(out packet);

    private async Task ReceiveLoopAsync(CancellationToken cancellationToken)
    {
        var header = new byte[HeaderLength];

        try
        {
            while (!cancellationToken.IsCancellationRequested)
            {
                await _stream.ReadExactlyAsync(header, 0, HeaderLength, cancellationToken).ConfigureAwait(false);

                int frameLength = BinaryPrimitives.ReadInt32BigEndian(header);
                var id = (PacketId)header[4];

                if (frameLength < HeaderLength || frameLength > MaxFrameLength)
                {
                    // Unrecoverable: we no longer know where the next frame starts, and the RC4
                    // keystream position is tied to having consumed exactly the right bytes.
                    Drop($"Malformed frame length {frameLength} for packet id {(byte)id}.");
                    return;
                }

                int bodyLength = frameLength - HeaderLength;
                var body = bodyLength == 0 ? Array.Empty<byte>() : new byte[bodyLength];
                if (bodyLength > 0)
                    await _stream.ReadExactlyAsync(body, 0, bodyLength, cancellationToken).ConfigureAwait(false);

                // Every body must go through the cipher, including ones we do not decode —
                // skipping one would leave the keystream offset by that many bytes.
                _receiveCipher.Crypt(body);

                var packet = PacketRegistry.Create(id);
                if (packet == null)
                    continue;

                if (!TryDecode(packet, body, out string decodeError))
                {
                    // The body is written out before the connection goes, because a decode failure
                    // is the one fault that cannot be reproduced by staring at the code: whatever
                    // is on the wire disagrees with what both sides believe, and only the bytes say
                    // how. Deleted on the next successful run of the same packet id.
                    DumpBody(id, body);
                    Drop($"Could not decode {id}: {decodeError}");
                    return;
                }

                _inbound.Enqueue(packet);
            }
        }
        catch (OperationCanceledException)
        {
            // Expected on a deliberate disconnect.
        }
        catch (EndOfStreamException)
        {
            Drop("Server closed the connection.");
        }
        catch (Exception ex)
        {
            Drop($"Receive failed: {ex.Message}");
        }
    }

    /// <summary>
    /// Decodes a body into <paramref name="packet"/>.
    /// </summary>
    /// <remarks>
    /// Split out of the receive loop because <see cref="NetReader"/> is a ref struct and cannot
    /// live across an await point, so it may not appear in an async method at all.
    /// </remarks>
    private static bool TryDecode(ServerPacket packet, byte[] body, out string error)
    {
        try
        {
            var reader = new NetReader(body);
            packet.Read(ref reader);
            error = null;
            return true;
        }
        catch (Exception ex)
        {
            // Anything at all, not just a format error. A reader that trips over an unexpected
            // length throws whatever the framework felt like -- an overflow from allocating a
            // negative array, most memorably -- and catching only the tidy exception meant those
            // surfaced as an unattributed "receive failed" with no clue which packet did it.
            error = $"{ex.GetType().Name}: {ex.Message}";
            return false;
        }
    }

    /// <summary>Writes a body that would not decode next to the log, for picking apart offline.</summary>
    private static void DumpBody(PacketId id, byte[] body)
    {
        try
        {
            string path = System.IO.Path.Combine(
                Godot.ProjectSettings.GlobalizePath("user://"), $"bad-{id}-{body.Length}.bin");

            System.IO.File.WriteAllBytes(path, body);
            Godot.GD.PushWarning($"[net] wrote {body.Length} undecodable bytes to {path}");
        }
        catch (Exception ex)
        {
            Godot.GD.PushWarning($"[net] could not dump body: {ex.Message}");
        }
    }

    private void Drop(string reason)
    {
        if (!_connected)
            return;

        _connected = false;
        try
        {
            _cancellation?.Cancel();
            _socket?.Shutdown(SocketShutdown.Both);
        }
        catch
        {
            // The socket is already gone; nothing useful to do.
        }

        Disconnected?.Invoke(reason);
    }

    /// <summary>Closes the connection deliberately.</summary>
    public void Close(string reason = "Closed by client.") => Drop(reason);

    public void Dispose()
    {
        Drop("Disposed.");
        _cancellation?.Dispose();
        _stream?.Dispose();
        _socket?.Dispose();
    }
}
