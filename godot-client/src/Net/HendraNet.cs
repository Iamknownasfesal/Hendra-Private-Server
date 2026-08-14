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
    public readonly int[] Mp;
    public readonly int[] MaxMp;

    /// <summary>
    /// Condition masks, split into halves.
    /// </summary>
    /// <remarks>
    /// The mask is 128 bits wide and the engine's integer is 64, so it crosses as two arrays.
    /// The game uses 51 effects today; the room above them is what a truncating read would lose
    /// later without anything failing at the time.
    /// </remarks>
    public readonly long[] ConditionsLow;
    public readonly long[] ConditionsHigh;

    /// <summary>Rendered size in percent, where 100 is the object's natural size.</summary>
    public readonly int[] Sizes;

    /// <summary>Which sprite to draw, for things that change appearance without changing type.</summary>
    public readonly int[] Textures;

    /// <summary>Names in entity order, empty where an entity has none.</summary>
    public readonly string[] Names;

    /// <summary>
    /// The eight stats per entity, laid end to end: entity <c>n</c> occupies <c>n * 8</c> onward.
    /// Meaningful only for the player's own entity.
    /// </summary>
    public readonly int[] Stats;

    public readonly int[] Stars;

    /// <summary>Air remaining, from 100 down to 0. Full everywhere but a drowning world.</summary>
    public readonly int[] Oxygen;

    public WorldView(
        int[] ids, int[] types, float[] positions, int[] hp, int[] maxHp,
        int[] mp, int[] maxMp, long[] conditionsLow, long[] conditionsHigh,
        int[] sizes, int[] textures, string[] names, int[] stats, int[] stars, int[] oxygen)
    {
        Ids = ids;
        Types = types;
        Positions = positions;
        Hp = hp;
        MaxHp = maxHp;
        Mp = mp;
        MaxMp = maxMp;
        ConditionsLow = conditionsLow;
        ConditionsHigh = conditionsHigh;
        Sizes = sizes;
        Textures = textures;
        Names = names;
        Stats = stats;
        Stars = stars;
        Oxygen = oxygen;
    }

    public static WorldView Empty => new(
        Array.Empty<int>(), Array.Empty<int>(), Array.Empty<float>(), Array.Empty<int>(),
        Array.Empty<int>(), Array.Empty<int>(), Array.Empty<int>(), Array.Empty<long>(),
        Array.Empty<long>(), Array.Empty<int>(), Array.Empty<int>(), Array.Empty<string>(),
        Array.Empty<int>(), Array.Empty<int>(), Array.Empty<int>());

    public int Count => Ids?.Length ?? 0;

    public Vector2 PositionOf(int index) => new(Positions[index * 2], Positions[index * 2 + 1]);

    /// <summary>The eight stats belonging to one entity.</summary>
    public ReadOnlySpan<int> StatsOf(int index) =>
        Stats is null || Stats.Length < (index + 1) * 8
            ? ReadOnlySpan<int>.Empty
            : Stats.AsSpan(index * 8, 8);

    /// <summary>Whether an entity carries a condition, by its wire index.</summary>
    public bool HasCondition(int index, int effect)
    {
        if (ConditionsLow is null || index >= ConditionsLow.Length)
            return false;

        return effect < 64
            ? (ConditionsLow[index] & (1L << effect)) != 0
            : (ConditionsHigh[index] & (1L << (effect - 64))) != 0;
    }
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

    /// <summary>One row of the map: the row, where it starts, and the tiles along it.</summary>
    public event Action<int, int, int[]> TerrainRow;

    /// <summary>Scenery in one row: the row, and the x, object and size of each piece.</summary>
    public event Action<int, int[], int[], int[]> SceneryRow;

    /// <summary>Squares whose ground changed, as parallel x, y and tile arrays.</summary>
    public event Action<int[], int[], int[]> GroundChanged;

    /// <summary>The whole contents of one container: which, then slot and item pairs.</summary>
    public event Action<int, int[]> ContainerFilled;

    /// <summary>Something the player asked for was refused, with a line to show them.</summary>
    public event Action<string> Refused;

    /// <summary>Something the world wants shown rather than said.</summary>
    public event Action<string> Notice;

    /// <summary>The server is full: where we stand in the line, and how many are waiting.</summary>
    public event Action<int, int> Queued;

    /// <summary>How many of each stacking potion the character holds.</summary>
    public event Action<int, int> Stacks;

    /// <summary>This character died: which, what killed it, and the fame it earned.</summary>
    public event Action<int, string, int> Died;

    /// <summary>A projectile was fired; its whole flight follows from these.</summary>
    public event Action<int, int, int, float, float, float, float, int> Shot;

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

            case "terrain":
                TerrainRow?.Invoke(
                    entry["y"].AsInt32(),
                    entry["x"].AsInt32(),
                    entry["tiles"].AsInt32Array()
                );
                break;

            case "scenery":
                SceneryRow?.Invoke(
                    entry["y"].AsInt32(),
                    entry["x"].AsInt32Array(),
                    entry["objects"].AsInt32Array(),
                    entry["sizes"].AsInt32Array()
                );
                break;

            case "ground":
                GroundChanged?.Invoke(
                    entry["x"].AsInt32Array(),
                    entry["y"].AsInt32Array(),
                    entry["tiles"].AsInt32Array()
                );
                break;

            case "container":
                ContainerFilled?.Invoke(
                    entry["container"].AsInt32(),
                    entry["slots"].AsInt32Array()
                );
                break;

            case "refused":
                Refused?.Invoke(entry["text"].AsString());
                break;

            case "notice":
                Notice?.Invoke(entry["text"].AsString());
                break;

            case "queued":
                Queued?.Invoke(entry["place"].AsInt32(), entry["waiting"].AsInt32());
                break;

            case "stacks":
                Stacks?.Invoke(entry["health"].AsInt32(), entry["magic"].AsInt32());
                break;

            case "died":
                Died?.Invoke(
                    entry["character"].AsInt32(),
                    entry["killed_by"].AsString(),
                    entry["fame"].AsInt32()
                );
                break;

            case "shot":
                Shot?.Invoke(
                    entry["projectile"].AsInt32(),
                    entry["owner"].AsInt32(),
                    entry["object_type"].AsInt32(),
                    (float)entry["x"].AsDouble(),
                    (float)entry["y"].AsDouble(),
                    (float)entry["angle"].AsDouble(),
                    (float)entry["speed"].AsDouble(),
                    entry["lifetime_ms"].AsInt32()
                );
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
            return WorldView.Empty;

        return new WorldView(
            _native.Call("entity_ids").AsInt32Array(),
            _native.Call("entity_types").AsInt32Array(),
            _native.Call("entity_positions").AsFloat32Array(),
            _native.Call("entity_hp").AsInt32Array(),
            _native.Call("entity_max_hp").AsInt32Array(),
            _native.Call("entity_mp").AsInt32Array(),
            _native.Call("entity_max_mp").AsInt32Array(),
            _native.Call("entity_conditions_low").AsInt64Array(),
            _native.Call("entity_conditions_high").AsInt64Array(),
            _native.Call("entity_sizes").AsInt32Array(),
            _native.Call("entity_textures").AsInt32Array(),
            _native.Call("entity_names").AsStringArray(),
            _native.Call("entity_stats").AsInt32Array(),
            _native.Call("entity_stars").AsInt32Array(),
            _native.Call("entity_oxygen").AsInt32Array()
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

    /// <summary>Fires in the given direction, in radians. Only the aim is sent.</summary>
    public void Shoot(float angle) => _native?.Call("shoot", angle);

    /// <summary>
    /// Asks to move an item between two slots.
    /// </summary>
    /// <remarks>
    /// Containers are named by tag rather than by entity, so a client cannot address somebody
    /// else's inventory: 0 is what you carry, 1 what you wear, 2 the vault.
    /// </remarks>
    public void MoveItem(long fromContainer, int fromSlot, long toContainer, int toSlot) =>
        _native?.Call("move_item", fromContainer, fromSlot, toContainer, toSlot);

    /// <summary>Takes an item out of a bag on the ground.</summary>
    public void PickUp(int bag, int slot) => _native?.Call("pick_up", bag, slot);

    /// <summary>Drops a carried item at the player's feet.</summary>
    public void DropItem(int slot) => _native?.Call("drop_item", slot);

    public override void _ExitTree()
    {
        Disconnect();
        _native = null;
    }
}
