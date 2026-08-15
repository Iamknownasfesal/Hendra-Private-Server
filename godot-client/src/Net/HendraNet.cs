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
/// Everything the server says about the world we have just been let into.
/// </summary>
/// <remarks>
/// The server's half of <c>MapInfo</c>: the extent, the marks the loading screen draws, the sky the
/// map is laid on, whether the menu over another player should offer a teleport, and what to play.
/// Only the extent is read before the first terrain row lands; the rest is what makes one world
/// look and sound different from the next.
/// </remarks>
public struct Welcome
{
    public int Player;
    public uint Tick;
    public string World;
    public int Width;
    public int Height;
    public int Background;
    public int Difficulty;
    public bool AllowTeleport;
    public bool ShowDisplays;
    public string Music;
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

    /// <summary>
    /// The skin each player is wearing, as the skin object's own type, and zero for none.
    /// </summary>
    /// <remarks>
    /// A whole different animated sheet rather than a frame within the one the object type names,
    /// which is what <see cref="Textures"/> picks.
    /// </remarks>
    public readonly int[] Skins;

    /// <summary>Names in entity order, empty where an entity has none.</summary>
    public readonly string[] Names;

    /// <summary>
    /// The eight stats per entity, laid end to end: entity <c>n</c> occupies <c>n * 8</c> onward.
    /// Meaningful only for the player's own entity.
    /// </summary>
    public readonly int[] Stats;

    /// <summary>
    /// What equipment and running boosts add to each of those eleven, laid out the same way.
    /// </summary>
    /// <remarks>
    /// The character sheet draws these in green beside the totals, and in red where an item takes a
    /// stat down. A total on its own cannot say how much of an attack is the player.
    /// </remarks>
    public readonly int[] Boosts;

    /// <summary>Each player's guild, in entity order, and empty for anybody in none.</summary>
    public readonly string[] Guilds;

    /// <summary>Rank within that guild: 0 initiate, 10 member, 20 officer, 40 founder.</summary>
    public readonly int[] GuildRanks;

    public readonly int[] Stars;

    /// <summary>Air remaining, from 100 down to 0. Full everywhere but a drowning world.</summary>
    public readonly int[] Oxygen;

    /// <summary>The level reached. Zero for anything that is not a player.</summary>
    public readonly int[] Levels;

    /// <summary>Experience earned since the current level began, which is what the bar fills.</summary>
    public readonly int[] Experience;

    /// <summary>Experience the current level needs before the next, the bar's ceiling.</summary>
    public readonly int[] ExperienceGoals;

    /// <summary>Fame banked by this character, its lifetime experience divided by a thousand.</summary>
    public readonly int[] Fame;

    /// <summary>
    /// The eight container slots per entity, laid end to end: entity <c>n</c> occupies <c>n * 8</c>
    /// onward. Meaningful only for a loot bag, a chest or a vault; -1 for an empty slot and for
    /// anything that is not a container, since zero is a real object type.
    /// </summary>
    public readonly int[] Contents;

    /// <summary>
    /// The five merchandise stats per entity, laid end to end: entity <c>n</c> occupies
    /// <c>n * 5</c> onward, in the order item type, price, currency, count, rank requirement.
    /// </summary>
    /// <remarks>
    /// An item type of -1 means the entity sells nothing, which is almost all of them. Currency is
    /// the game's own CurrencyType: zero gold, one fame. A count of -1 is stock that never runs
    /// out, which is every shop the map stands.
    /// </remarks>
    public readonly int[] Merchandise;

    /// <summary>
    /// What the account can spend, three per entity laid end to end: entity <c>n</c> occupies
    /// <c>n * 3</c> onward, in the order gold, fame, prestige.
    /// </summary>
    /// <remarks>
    /// The fame here is the account's, which is what every shop charges against; what the character
    /// has earned is in <see cref="Fame"/>. Meaningful only for the player's own entity.
    /// </remarks>
    public readonly int[] Purse;

    /// <summary>
    /// The halo colour per entity, as a packed <c>0xRRGGBB</c>. Zero is no halo, which is nearly
    /// everybody.
    /// </summary>
    /// <remarks>
    /// <c>StatsType.Glow</c> (<c>Player.cs:302</c>), which the original paints as a ring around the
    /// sprite (<c>GlowRedrawer.as:19-46</c>). It is what <c>/glow</c> sets.
    /// </remarks>
    public readonly int[] Glow;

    /// <summary>
    /// The rarely-changing booleans, one integer per entity: bit zero an administrator, bit one a
    /// character that owns a backpack, bit two an account that chose its own name, bit three a
    /// portal that refuses to be entered.
    /// </summary>
    public readonly int[] Marks;

    /// <summary>
    /// The two dyes per entity laid end to end, cloth then accessory.
    /// </summary>
    /// <remarks>
    /// <c>StatsType.Texture1</c>/<c>Texture2</c> (<c>Player.cs:301-302</c>). The top byte is a type
    /// and the low twenty-four its argument: <c>1</c> a solid <c>0xRRGGBB</c>, and <c>4</c>,
    /// <c>5</c>, <c>9</c> or <c>10</c> an index into the <c>textile{n}x{n}</c> sheet of that size
    /// (<c>TextureRedrawer.as:161-186</c>).
    /// </remarks>
    public readonly int[] Dyes;

    /// <summary>Which neighbours each piece of scenery joins onto, as <c>ConnectionInfo.Bits</c>.</summary>
    public readonly int[] Connection;

    /// <summary>
    /// What is left of the three boost clocks per entity, in seconds: experience, loot drop, loot
    /// tier, laid end to end.
    /// </summary>
    public readonly int[] BoostTime;

    /// <summary>The fame the next class quest asks for. Zero once every star has been earned.</summary>
    public readonly int[] FameGoal;

    /// <summary>How many numbers one entity's merchandise takes in <see cref="Merchandise"/>.</summary>
    public const int MerchandiseFields = 5;

    public WorldView(
        int[] ids, int[] types, float[] positions, int[] hp, int[] maxHp,
        int[] mp, int[] maxMp, long[] conditionsLow, long[] conditionsHigh,
        int[] sizes, int[] textures, int[] skins, string[] names, int[] stats, int[] stars,
        int[] oxygen,
        int[] levels, int[] experience, int[] experienceGoals, int[] fame, int[] contents,
        int[] merchandise = null, int[] purse = null,
        int[] boosts = null, string[] guilds = null, int[] guildRanks = null,
        int[] glow = null, int[] marks = null, int[] dyes = null, int[] connection = null,
        int[] boostTime = null, int[] fameGoal = null)
    {
        Glow = glow;
        Marks = marks;
        Dyes = dyes;
        Connection = connection;
        BoostTime = boostTime;
        FameGoal = fameGoal;
        Boosts = boosts;
        Guilds = guilds;
        GuildRanks = guildRanks;
        Contents = contents;
        Merchandise = merchandise;
        Purse = purse;
        Levels = levels;
        Experience = experience;
        ExperienceGoals = experienceGoals;
        Fame = fame;
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
        Skins = skins;
        Names = names;
        Stats = stats;
        Stars = stars;
        Oxygen = oxygen;
    }

    public static WorldView Empty => new(
        Array.Empty<int>(), Array.Empty<int>(), Array.Empty<float>(), Array.Empty<int>(),
        Array.Empty<int>(), Array.Empty<int>(), Array.Empty<int>(), Array.Empty<long>(),
        Array.Empty<long>(), Array.Empty<int>(), Array.Empty<int>(), Array.Empty<int>(),
        Array.Empty<string>(),
        Array.Empty<int>(), Array.Empty<int>(), Array.Empty<int>(), Array.Empty<int>(),
        Array.Empty<int>(), Array.Empty<int>(), Array.Empty<int>(), Array.Empty<int>());

    public int Count => Ids?.Length ?? 0;

    public Vector2 PositionOf(int index) => new(Positions[index * 2], Positions[index * 2 + 1]);

    /// <summary>The eight container slots belonging to one entity, or empty when it holds nothing.</summary>
    public ReadOnlySpan<int> ContentsOf(int index) =>
        Contents == null || (index + 1) * 8 > Contents.Length
            ? ReadOnlySpan<int>.Empty
            : Contents.AsSpan(index * 8, 8);

    /// <summary>What the account behind one entity can spend, or empty when it is not a player.</summary>
    public ReadOnlySpan<int> PurseOf(int index) =>
        Purse == null || (index + 1) * 3 > Purse.Length
            ? ReadOnlySpan<int>.Empty
            : Purse.AsSpan(index * 3, 3);

    /// <summary>What one entity is selling, or empty when it sells nothing.</summary>
    public ReadOnlySpan<int> MerchandiseOf(int index) =>
        Merchandise == null || (index + 1) * MerchandiseFields > Merchandise.Length
            ? ReadOnlySpan<int>.Empty
            : Merchandise.AsSpan(index * MerchandiseFields, MerchandiseFields);

    /// <summary>The halo colour of one entity, or zero for no halo.</summary>
    public int GlowOf(int index) => Glow != null && index < Glow.Length ? Glow[index] : 0;

    /// <summary>The packed marks of one entity, or zero for something that carries none.</summary>
    public int MarksOf(int index) => Marks != null && index < Marks.Length ? Marks[index] : 0;

    /// <summary>The two dyes one entity wears, or empty when it wears none.</summary>
    public ReadOnlySpan<int> DyesOf(int index) =>
        Dyes == null || (index + 1) * 2 > Dyes.Length
            ? ReadOnlySpan<int>.Empty
            : Dyes.AsSpan(index * 2, 2);

    /// <summary>Which neighbours one piece of scenery joins onto, or zero for none.</summary>
    public int ConnectionOf(int index) =>
        Connection != null && index < Connection.Length ? Connection[index] : 0;

    /// <summary>The three boost clocks of one entity in seconds, or empty when it has none.</summary>
    public ReadOnlySpan<int> BoostTimeOf(int index) =>
        BoostTime == null || (index + 1) * 3 > BoostTime.Length
            ? ReadOnlySpan<int>.Empty
            : BoostTime.AsSpan(index * 3, 3);

    /// <summary>The fame the next class quest asks of one entity.</summary>
    public int FameGoalOf(int index) =>
        FameGoal != null && index < FameGoal.Length ? FameGoal[index] : 0;

    /// <summary>
    /// The eleven stats belonging to one entity, in the game's own stat numbering: the eight a
    /// class declares, then DamageMin, DamageMax and Luck.
    /// </summary>
    public ReadOnlySpan<int> StatsOf(int index) =>
        Stats is null || Stats.Length < (index + 1) * StatCount
            ? ReadOnlySpan<int>.Empty
            : Stats.AsSpan(index * StatCount, StatCount);

    /// <summary>What equipment and boosts add to one entity's eleven stats, in the same order.</summary>
    public ReadOnlySpan<int> BoostsOf(int index) =>
        Boosts is null || Boosts.Length < (index + 1) * StatCount
            ? ReadOnlySpan<int>.Empty
            : Boosts.AsSpan(index * StatCount, StatCount);

    /// <summary>One entity's guild, or empty when they are in none.</summary>
    public string GuildOf(int index) =>
        Guilds is null || index >= Guilds.Length ? string.Empty : Guilds[index];

    /// <summary>One entity's rank in that guild, or zero.</summary>
    public int GuildRankOf(int index) =>
        GuildRanks is null || index >= GuildRanks.Length ? 0 : GuildRanks[index];

    /// <summary>How many stats each entity carries. <c>StatsManager.NumStatTypes</c>.</summary>
    public const int StatCount = 11;

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

    /// <summary>
    /// Raised when the server accepts us into a world, with everything it says about that world.
    /// </summary>
    public event Action<Welcome> Welcomed;

    /// <summary>Raised when the world we are already in changes what it is playing.</summary>
    public event Action<string> MusicSwitched;

    /// <summary>Raised with the entity our quest arrow should point at.</summary>
    public event Action<int> QuestTargeted;

    /// <summary>Raised with the entity our camera should follow. Our own id gives it back.</summary>
    public event Action<int> Focused;

    /// <summary>
    /// Raised with one of the account's lists: 0 ignored, 1 locked out, and the names on it.
    /// </summary>
    public event Action<int, string[]> Listed;

    /// <summary>Raised when the server refuses the connection.</summary>
    public event Action<RejectReason> Rejected;

    /// <summary>Raised for chat, with the speaker and the line.</summary>
    public event Action<int, string, string> Chatted;

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

    /// <summary>
    /// The whole contents of one container: which it is, then parallel slot and item arrays.
    /// </summary>
    public event Action<int, int[], int[]> ContainerFilled;

    /// <summary>
    /// The whole vault: version, chest count, the ceiling, the next price, the slots and the gifts.
    /// </summary>
    /// <remarks>
    /// Sent whole rather than as a delta, so a client that misses one is corrected by the next
    /// rather than drifting. Slot entries of <c>0xffff</c> are empty.
    /// </remarks>
    public event Action<int, int, int, int, int[], int[]> VaultUpdated;

    /// <summary>Something the player asked for was refused, with a line to show them.</summary>
    public event Action<string> Refused;

    /// <summary>Something the world wants shown rather than said.</summary>
    public event Action<string> Notice;

    /// <summary>
    /// A word the client acts on rather than reads: whether a gift is waiting, a key colour, the
    /// key panel. <c>GlobalNotification</c> in the original.
    /// </summary>
    public event Action<string> Notified;

    /// <summary>The server is full: where we stand in the line, and how many are waiting.</summary>
    public event Action<int, int> Queued;

    /// <summary>How many of each stacking potion the character holds.</summary>
    public event Action<int, int> Stacks;

    /// <summary>This character died: which, what killed it, and the fame it earned.</summary>
    public event Action<int, string, int> Died;

    /// <summary>A projectile was fired; its whole flight follows from these.</summary>
    /// <remarks>
    /// The last field is what it takes off whatever it hits, before that body's defence. Whoever
    /// it hits is the one client the server does not tell when it lands, so this is what the
    /// number over their own head is worked out from.
    /// </remarks>
    public event Action<int, int, int, float, float, float, float, int, int> Shot;

    /// <summary>A body is somewhere it did not walk to: object, x, y.</summary>
    /// <remarks>
    /// The one thing a snapshot cannot say. A client owns the position of its own player and
    /// glides every other body towards the position the snapshot gives it, so neither can express
    /// "stop, you are here now" — which is what a teleport is.
    /// </remarks>
    public event Action<int, float, float> Repositioned;

    /// <summary>
    /// Something took a hit: target, effect indices, amount, whether it killed, which bullet, and
    /// who dealt it.
    /// </summary>
    /// <remarks>
    /// The snapshot already carries health, so this is not where the number comes from. It is the
    /// only thing on the wire that separates a body that died from one that walked out of sight.
    /// </remarks>
    public event Action<int, int[], int, bool, int, int> Damage;

    /// <summary>
    /// Something to draw: effect kind, the body it hangs off, two positions, and a colour.
    /// </summary>
    /// <remarks>
    /// The positions are overloaded per effect — a radius, a particle size, a flash period — and
    /// only the renderer knows which, so nothing is interpreted here.
    /// </remarks>
    public event Action<int, int, float, float, float, float, int> ShowEffect;

    /// <summary>
    /// A line of text to float off a body: the object it belongs to, what to say, and the colour.
    /// </summary>
    /// <remarks>
    /// The only channel that says something about one body rather than to one connection. Every
    /// heal, every mana refill, every fame gain and both quest completions arrive here.
    /// </remarks>
    public event Action<int, string, int> StatusText;

    /// <summary>
    /// A blast went off: where, how far it reached, what it took, what it left, and what threw it.
    /// </summary>
    /// <remarks>
    /// The damage has already been applied by the server and reported as an ordinary hit. This is
    /// the ring, and the ring alone.
    /// </remarks>
    public event Action<float, float, float, int, int, float, int> Blast;

    /// <summary>Somebody has asked this player into their guild: who, and which guild.</summary>
    public event Action<string, string> InvitedToGuild;

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

    public override void _Ready() => EnsureNative();

    /// <summary>
    /// Creates the native object if it does not exist yet.
    /// </summary>
    /// <remarks>
    /// On demand rather than only in <c>_Ready</c>, because a session is built and connected before
    /// the node carrying it has been parented — the scene that asks for one is still setting up its
    /// own children at the time, so the tree defers the add by a frame and the connection would
    /// find nothing here.
    /// </remarks>
    private bool EnsureNative()
    {
        if (_native is not null)
            return true;

        if (!ExtensionAvailable)
        {
            GD.PushError(
                $"Hendra: the native extension is not loaded. Build it with "
                    + "`cargo build -p hendra-godot --release` and copy the library into res://bin/."
            );
            return false;
        }

        _native = ClassDB.Instantiate(NativeClass).As<GodotObject>();
        if (_native is Node node)
        {
            AddChild(node);
        }

        return _native is not null;
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
        EnsureNative();
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
        EnsureNative();
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
                Welcomed?.Invoke(new Welcome
                {
                    Player = entry["player"].AsInt32(),
                    Tick = (uint)entry["tick"].AsInt64(),
                    World = entry["world"].AsString(),
                    Width = entry["width"].AsInt32(),
                    Height = entry["height"].AsInt32(),
                    Background = entry["background"].AsInt32(),
                    Difficulty = entry["difficulty"].AsInt32(),
                    AllowTeleport = entry["allow_teleport"].AsBool(),
                    ShowDisplays = entry["show_displays"].AsBool(),
                    Music = entry["music"].AsString(),
                });
                break;

            case "switch_music":
                MusicSwitched?.Invoke(entry["music"].AsString());
                break;

            case "quest_target":
                QuestTargeted?.Invoke(entry["target"].AsInt32());
                break;

            case "set_focus":
                Focused?.Invoke(entry["target"].AsInt32());
                break;

            case "account_list":
                Listed?.Invoke(entry["list"].AsInt32(), entry["names"].AsStringArray());
                break;

            case "rejected":
                Rejected?.Invoke((RejectReason)entry["reason"].AsInt32());
                break;

            case "chat":
                Chatted?.Invoke(
                    entry["speaker"].AsInt32(),
                    entry["from"].AsString(),
                    entry["text"].AsString()
                );
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
                    entry["slots"].AsInt32Array(),
                    entry["items"].AsInt32Array()
                );
                break;

            case "vault":
                VaultUpdated?.Invoke(
                    entry["version"].AsInt32(),
                    entry["chest_count"].AsInt32(),
                    entry["max_chests"].AsInt32(),
                    entry["next_chest_price"].AsInt32(),
                    entry["slots"].AsInt32Array(),
                    entry["gifts"].AsInt32Array()
                );
                break;

            case "refused":
                Refused?.Invoke(entry["message"].AsString());
                break;

            case "notice":
                Notice?.Invoke(entry["text"].AsString());
                break;

            case "notification":
                Notified?.Invoke(entry["text"].AsString());
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
                    entry["lifetime_ms"].AsInt32(),
                    entry["damage"].AsInt32()
                );
                break;

            case "goto":
                Repositioned?.Invoke(
                    entry["object_id"].AsInt32(),
                    (float)entry["x"].AsDouble(),
                    (float)entry["y"].AsDouble()
                );
                break;

            case "show_effect":
                ShowEffect?.Invoke(
                    entry["effect"].AsInt32(),
                    entry["target"].AsInt32(),
                    (float)entry["x1"].AsDouble(),
                    (float)entry["y1"].AsDouble(),
                    (float)entry["x2"].AsDouble(),
                    (float)entry["y2"].AsDouble(),
                    entry["color"].AsInt32()
                );
                break;

            case "status_text":
                StatusText?.Invoke(
                    entry["object_id"].AsInt32(),
                    entry["text"].AsString(),
                    entry["color"].AsInt32()
                );
                break;

            case "aoe":
                Blast?.Invoke(
                    (float)entry["x"].AsDouble(),
                    (float)entry["y"].AsDouble(),
                    (float)entry["radius"].AsDouble(),
                    entry["damage"].AsInt32(),
                    entry["effect"].AsInt32(),
                    (float)entry["duration"].AsDouble(),
                    entry["orig_type"].AsInt32()
                );
                break;

            case "invited_to_guild":
                InvitedToGuild?.Invoke(entry["name"].AsString(), entry["guild"].AsString());
                break;

            case "damage":
                Damage?.Invoke(
                    entry["target"].AsInt32(),
                    entry["effects"].AsInt32Array(),
                    entry["amount"].AsInt32(),
                    entry["kill"].AsBool(),
                    entry["bullet"].AsInt32(),
                    entry["owner"].AsInt32()
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
            _native.Call("entity_skins").AsInt32Array(),
            _native.Call("entity_names").AsStringArray(),
            _native.Call("entity_stats").AsInt32Array(),
            _native.Call("entity_stars").AsInt32Array(),
            _native.Call("entity_oxygen").AsInt32Array(),
            _native.Call("entity_levels").AsInt32Array(),
            _native.Call("entity_experience").AsInt32Array(),
            _native.Call("entity_experience_goals").AsInt32Array(),
            _native.Call("entity_fame").AsInt32Array(),
            _native.Call("entity_contents").AsInt32Array(),
            _native.Call("entity_merchandise").AsInt32Array(),
            _native.Call("entity_purse").AsInt32Array(),
            _native.Call("entity_boosts").AsInt32Array(),
            _native.Call("entity_guilds").AsStringArray(),
            _native.Call("entity_guild_ranks").AsInt32Array(),
            _native.Call("entity_glow").AsInt32Array(),
            _native.Call("entity_marks").AsInt32Array(),
            _native.Call("entity_dyes").AsInt32Array(),
            _native.Call("entity_connection").AsInt32Array(),
            _native.Call("entity_boost_time").AsInt32Array(),
            _native.Call("entity_fame_goal").AsInt32Array()
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
    /// Asks to move an item between two of the player's own slots.
    /// </summary>
    /// <remarks>
    /// Slots are flat numbers, as the inventory counts them: worn from nought with room for eight,
    /// carried from eight, the backpack from sixteen. The extension turns each into the container
    /// and the index the wire names, which is the only place that translation is written down.
    /// </remarks>
    public void MoveItem(int fromSlot, int toSlot) =>
        _native?.Call("move_item", fromSlot, toSlot);

    /// <summary>
    /// Moves an item within the vault, or between the vault and the player's own inventory.
    /// </summary>
    /// <remarks>
    /// Both ends are a chest and a slot, because the operation is a swap and is symmetric. Minus one
    /// is the player's own pack, minus two the gifts waiting, minus three a potion stack. The
    /// version is the vault as this client last saw it, and a move quoting one the server has moved
    /// past is refused and answered with the truth.
    /// </remarks>
    public void VaultMove(int version, int fromChest, int fromSlot, int toChest, int toSlot) =>
        _native?.Call("vault_move", version, fromChest, fromSlot, toChest, toSlot);

    /// <summary>Buys one more vault chest, naming the count we believe we own.</summary>
    public void VaultBuy(int chestCount) => _native?.Call("vault_buy", chestCount);

    /// <summary>Buys what the merchant standing in front of the player is selling.</summary>
    /// <remarks>
    /// Names the merchant and no more: what it sells and what it costs are the server's to know.
    /// </remarks>
    public void Buy(int merchant) => _native?.Call("buy", merchant);

    /// <summary>Takes an item out of a bag on the ground and into one of our own slots.</summary>
    public void TakeFromBag(int bag, int bagSlot, int intoSlot) =>
        _native?.Call("take_from_bag", bag, bagSlot, intoSlot);

    /// <summary>Puts one of our own items into a bag on the ground.</summary>
    /// <remarks>
    /// The bag's slot says where it was aimed and no more: a bag is shared, and the server puts the
    /// item wherever the bag has room.
    /// </remarks>
    public void PutInBag(int fromSlot, int bag, int bagSlot) =>
        _native?.Call("put_in_bag", fromSlot, bag, bagSlot);

    /// <summary>Drops what is in one of our own slots at the player's feet.</summary>
    public void DropItem(int slot) => _native?.Call("drop_item", slot);

    /// <summary>Uses what is in a slot of some container, aimed at a point in the world.</summary>
    /// <remarks>
    /// The slot rather than the item: the server knows what is in a slot, and a client naming an
    /// item it does not hold would be a claim rather than a fact. The container is the entity whose
    /// slot is meant — zero, and the player's own id, both mean their own slots, and anything else
    /// is a bag or a chest standing in the world.
    /// </remarks>
    public void UseItem(int container, int slot, float x, float y) =>
        _native?.Call("use_item", container, slot, x, y);

    public override void _ExitTree()
    {
        Disconnect();
        _native = null;
    }
}
