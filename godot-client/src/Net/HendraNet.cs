using System;
using System.Collections.Generic;
using Godot;

namespace Hendra.Net;

/// <summary>Why the server refused a connection.</summary>
/// <remarks>
/// Mirrors <c>RejectReason</c> in <c>crates/net/src/message.rs</c>. The numbering is on the wire, so
/// a new reason goes on the end and nothing is renumbered.
/// </remarks>
public enum RejectReason
{
    VersionMismatch = 0,
    BadToken = 1,
    AlreadyPlaying = 2,
    Full = 3,
    Banned = 4,
    NoSuchCharacter = 5,
}

/// <summary>Where a connection currently stands.</summary>
public enum LinkStatus
{
    Idle = 0,
    Connecting = 1,
    Connected = 2,
    Failed = 3,
    Closed = 4,
}

/// <summary>
/// The client's view of the world, as the extension decoded it.
/// </summary>
/// <remarks>
/// Parallel arrays rather than a list of objects, because that is how they cross from Rust: one
/// marshalled block per field instead of one allocation per entity. Entities are in the same order
/// in every array, and <see cref="Count"/> applies to all of them.
/// </remarks>
public readonly struct WorldView
{
    public readonly int[] Ids;
    public readonly int[] Types;

    /// <summary>Interleaved x, y — two entries per entity.</summary>
    public readonly float[] Positions;

    public readonly int[] Hp;
    public readonly int[] MaxHp;

    public WorldView(int[] ids, int[] types, float[] positions, int[] hp, int[] maxHp)
    {
        Ids = ids;
        Types = types;
        Positions = positions;
        Hp = hp;
        MaxHp = maxHp;
    }

    public int Count => Ids?.Length ?? 0;

    public Vector2 PositionOf(int index) => new(Positions[index * 2], Positions[index * 2 + 1]);
}

/// <summary>
/// The network connection, as a typed C# API over the native extension.
/// </summary>
/// <remarks>
/// The protocol itself lives in Rust — <c>crates/net</c> — and is linked into both this client and
/// the server, so the two cannot disagree about the wire format. Nothing in this file parses bytes;
/// it converts already-decoded values into shapes the rest of the client prefers, and it exists
/// mainly so no other file has to address the extension through <see cref="ClassDB"/> by name.
///
/// The extension is polled rather than signalled. <see cref="Poll"/> must be called once per frame:
/// it drains the events that arrived since the last one and raises them, and it never blocks —
/// everything that could wait happens on the extension's own worker thread.
/// </remarks>
public sealed partial class HendraNet : Node
{
    /// <summary>The class the extension registers. Referenced once, here.</summary>
    private const string NativeClass = "HendraConnection";

    private GodotObject _native;

    /// <summary>Raised once the QUIC handshake has completed.</summary>
    public event Action Connected;

    /// <summary>Raised when the server accepts us into a world.</summary>
    public event Action<int, uint, string> Welcomed;

    /// <summary>Raised when the server refuses the connection.</summary>
    public event Action<RejectReason> Rejected;

    /// <summary>Raised for chat, with the speaker and the line.</summary>
    public event Action<string, string> Chatted;

    /// <summary>Raised once the connection has ended, with whatever explanation there was.</summary>
    public event Action<string> Disconnected;

    /// <summary>Raised when the world changed, at most once per frame.</summary>
    public event Action<WorldView> WorldChanged;

    /// <summary>Whether the native library loaded at all.</summary>
    /// <remarks>
    /// Worth checking before anything else: a missing or mismatched extension shows up here as a
    /// clear answer rather than as a null reference somewhere later.
    /// </remarks>
    public static bool ExtensionAvailable => ClassDB.ClassExists(NativeClass);

    /// <summary>The protocol revision the extension speaks.</summary>
    public int ProtocolVersion => _native is null ? 0 : (int)_native.Call("protocol_version");

    public LinkStatus Status =>
        _native is null ? LinkStatus.Idle : (LinkStatus)(int)(long)_native.Call("status");

    /// <summary>Whether the handshake has completed and the link is usable.</summary>
    /// <remarks>
    /// Named for the server rather than just "connected" because <see cref="GodotObject"/> already
    /// has an <c>IsConnected</c> that asks about signal connections, and the two would be easy to
    /// confuse at a call site.
    /// </remarks>
    public bool IsConnectedToServer =>
        _native is not null && (bool)_native.Call("is_connected_to_server");

    /// <summary>Round-trip time in milliseconds, as QUIC estimates it.</summary>
    public int RoundTripMs => _native is null ? 0 : (int)(long)_native.Call("rtt_ms");

    private ulong _lastRevision;

    public override void _Ready()
    {
        if (!ExtensionAvailable)
        {
            GD.PushError(
                $"Hendra: the native extension is not loaded. Build it with "
                    + "`cargo build -p hendra-godot --release` and copy the library into res://bin/."
            );
            return;
        }

        _native = ClassDB.Instantiate(NativeClass).As<GodotObject>();
        if (_native is Node node)
        {
            AddChild(node);
        }
    }

    /// <summary>
    /// Opens a connection and sends the opening message.
    /// </summary>
    /// <param name="token">
    /// A session token from the app server, obtained over HTTPS. No password ever reaches the game
    /// socket.
    /// </param>
    /// <param name="allowAnyCertificate">
    /// Turns off server verification. Development only — with it set, anything that can answer the
    /// address can impersonate the server.
    /// </param>
    public bool Connect(
        string host,
        int port,
        string token,
        int characterId,
        bool allowAnyCertificate = false
    )
    {
        if (_native is null)
        {
            return false;
        }

        return (bool)
            _native.Call("connect_to_server", host, port, token, characterId, allowAnyCertificate);
    }

    public void Disconnect() => _native?.Call("disconnect_from_server");

    /// <summary>
    /// Drains everything that arrived since the last frame and raises it.
    /// </summary>
    /// <remarks>
    /// Events are raised in the order they arrived. The world is read at most once per call, and
    /// only when its revision has moved, so a frame in which nothing happened costs one integer
    /// comparison.
    /// </remarks>
    public void Poll()
    {
        if (_native is null)
        {
            return;
        }

        foreach (Godot.Collections.Dictionary entry in
            _native.Call("poll").AsGodotArray<Godot.Collections.Dictionary>())
        {
            Raise(entry);
        }

        ulong revision = (ulong)(long)_native.Call("world_revision");
        if (revision != _lastRevision)
        {
            _lastRevision = revision;
            WorldChanged?.Invoke(ReadWorld());
        }
    }

    private void Raise(Godot.Collections.Dictionary entry)
    {
        string kind = entry["kind"].AsString();
        switch (kind)
        {
            case "connected":
                Connected?.Invoke();
                break;

            case "welcome":
                Welcomed?.Invoke(
                    entry["player"].AsInt32(),
                    (uint)entry["tick"].AsInt64(),
                    entry["world"].AsString()
                );
                break;

            case "rejected":
                Rejected?.Invoke((RejectReason)entry["reason"].AsInt32());
                break;

            case "chat":
                Chatted?.Invoke(entry["from"].AsString(), entry["text"].AsString());
                break;

            case "disconnected":
                Disconnected?.Invoke(entry["reason"].AsString());
                break;

            default:
                // An extension newer than this client can send a kind we do not know. Ignoring it
                // is correct; crashing on it is not.
                GD.PushWarning($"Hendra: ignoring unknown network event '{kind}'");
                break;
        }
    }

    /// <summary>Reads the current world out of the extension.</summary>
    public WorldView ReadWorld()
    {
        if (_native is null)
        {
            return new WorldView(Array.Empty<int>(), Array.Empty<int>(), Array.Empty<float>(),
                Array.Empty<int>(), Array.Empty<int>());
        }

        return new WorldView(
            _native.Call("entity_ids").AsInt32Array(),
            _native.Call("entity_types").AsInt32Array(),
            _native.Call("entity_positions").AsFloat32Array(),
            _native.Call("entity_hp").AsInt32Array(),
            _native.Call("entity_max_hp").AsInt32Array()
        );
    }

    /// <summary>
    /// Reports where the player believes it is, and acknowledges the newest snapshot held.
    /// </summary>
    /// <remarks>
    /// The acknowledgement is what the server encodes its next snapshot against, so this wants
    /// sending every tick even when the player has not moved. The extension tracks which tick to
    /// acknowledge; the caller only supplies the position.
    /// </remarks>
    public void SendInput(Vector2 position, long clientTimeMs) =>
        _native?.Call("send_input", position.X, position.Y, clientTimeMs);

    public void SendChat(string text) => _native?.Call("send_chat", text);

    public void UsePortal(int entityId) => _native?.Call("use_portal", entityId);

    public override void _ExitTree()
    {
        Disconnect();
        _native = null;
    }
}
