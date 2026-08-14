using System;
using System.Collections.Generic;
using Godot;
using Hendra.Core;
using Hendra.Data;
using Hendra.Net.Packets;

namespace Hendra.Net;

/// <summary>
/// The Rust server, presented in the shape the rest of the client already reads.
/// </summary>
/// <remarks>
/// <para>
/// The game layer — <c>WorldController</c>, the HUD, the inventory, the renderer — is written
/// against the legacy packets: an <see cref="UpdatePacket"/> of tiles, arrivals and departures, and
/// a <see cref="NewTickPacket"/> of per-entity stats. The Rust protocol says the same things in a
/// different shape: a snapshot of everything visible, plus events for the ground and for what a
/// player owns.
/// </para>
/// <para>
/// This translates one into the other, so three thousand lines of game code move across without
/// being rewritten. It is a translation and not a protocol: nothing here parses bytes, and the
/// legacy packet classes it fills in are used as plain records.
/// </para>
/// <para>
/// Three kinds of thing the old client sent have no counterpart and are dropped here rather than
/// faked. Acknowledgements — <c>ShootAck</c>, <c>AoeAck</c>, the move ledger — exist because that
/// server audited a ledger, and this one does not keep one. Hit claims — <c>EnemyHit</c>,
/// <c>PlayerHit</c>, <c>SquareHit</c>, <c>GroundDamage</c> — are refused by design: this server
/// resolves every hit itself and there is no message for a client to claim one. Anything else that
/// arrives unhandled is logged once, so a gap shows up as a line rather than as silence.
/// </para>
/// </remarks>
/// <remarks>
/// A plain object rather than a node, as the session it replaces was: it is created by the service
/// locator and driven by the world controller's frame, not by the scene tree. The extension
/// underneath it *is* a node — it holds a native object and wants freeing with the tree — so it is
/// parented to the root here.
/// </remarks>
public sealed class RustSession : IDisposable
{
    private readonly HendraNet _net;
    private readonly Core.GameClock _clock;

    /// <summary>Where the app server answers. It is the only thing that ever sees a password.</summary>
    public string AppServer { get; set; } = "http://127.0.0.1:8080";

    public RustSession(Core.GameClock clock)
    {
        _clock = clock;

        _net = new HendraNet();

        // Deferred, because a session is made while the scene that asked for it is still building
        // its own children and the tree refuses a new one mid-setup.
        if (Engine.GetMainLoop() is SceneTree tree)
            tree.Root.CallDeferred(Node.MethodName.AddChild, _net);

        _net.Welcomed += OnWelcomed;
        _net.Rejected += OnRejected;
        _net.Disconnected += reason => Disconnected?.Invoke(reason);
        _net.Chatted += OnChatted;
        _net.TerrainRow += OnTerrainRow;
        _net.GroundChanged += OnGroundChanged;
        _net.WorldChanged += OnWorldChanged;
    }

    public void Dispose()
    {
        _net?.QueueFree();
    }

    /// <summary>What the world held last frame, so arrivals and departures can be worked out.</summary>
    private readonly Dictionary<int, int> _known = new();

    /// <summary>Reused between frames so a steady world allocates nothing.</summary>
    private readonly List<ObjectDef> _arrived = new();
    private readonly List<int> _departed = new();
    private readonly List<ObjectStats> _statuses = new();
    private readonly List<GroundTile> _tiles = new();

    /// <summary>Names of unhandled sends already reported, so each is mentioned once.</summary>
    private readonly HashSet<string> _reported = new();

    private int _tickTime;
    private bool _entered;

    public int PlayerObjectId { get; private set; } = -1;

    /// <summary>
    /// The shared damage generator, which this protocol does not have.
    /// </summary>
    /// <remarks>
    /// The legacy client predicted its own damage from a generator stepped in lockstep with the
    /// server, and every extra or missing draw desynchronised everything after it. Here the server
    /// is authoritative and health arrives in the snapshot, so there is nothing to predict and
    /// nothing to keep in step. Null is the answer, and both call sites already read it that way.
    /// </remarks>
    public MinstdRandom Random => null;

    /// <summary>
    /// The move ledger the old server audited, which this one does not keep.
    /// </summary>
    /// <remarks>
    /// Kept as an always-empty record so the combat code's staleness guard reads false rather than
    /// needing a branch for which server it is talking to.
    /// </remarks>
    public MoveRecords MoveRecords { get; } = new();

    /// <summary>Where the player believes it is. Written by the world controller each frame.</summary>
    public float PlayerX { get; set; }
    public float PlayerY { get; set; }
    public bool HasPlayerPosition { get; set; }

    /// <summary>Whether the player is paused.</summary>
    public bool PlayerPaused { get; set; }

    /// <summary>Round-trip time, which QUIC measures for us.</summary>
    public int PingDelayMs => _net?.RoundTripMs ?? 0;

    /// <summary>How long since anything arrived.</summary>
    public int SinceLastPacketMs { get; private set; }

    /// <summary>Where the connection stands, for the debug overlay.</summary>
    public LinkStatus State => _net?.Status ?? LinkStatus.Idle;

    /// <summary>The world the server put us in.</summary>
    public string WorldName { get; private set; } = "";

    // The same events GameSession raises, so subscribers do not care which one they have.
    public event Action<MapInfoPacket> MapLoaded;
    public event Action<CreateSuccessPacket> Entered;
    public event Action<UpdatePacket> WorldUpdated;
    public event Action<NewTickPacket> Ticked;
    public event Action<GotoPacket> Repositioned;
    public event Action<ServerPacket> PacketReceived;
    public event Action<FailurePacket> Failed;
    public event Action<string> Disconnected;

    /// <summary>Raised when the server asks the client to reconnect elsewhere.</summary>
    public event Action<ReconnectPacket> ReconnectRequested;

    /// <summary>
    /// Signs in and joins a world.
    /// </summary>
    /// <remarks>
    /// Two steps, and the split is the point: the app server is the only thing that ever sees a
    /// password, and it answers with a signed token. The world server takes the token and checks
    /// the signature, so no credential ever reaches the game socket.
    /// </remarks>
    public async System.Threading.Tasks.Task ConnectToLoadAsync(
        string host, int port, string guid, string password, int gameId, int charId,
        bool fromArena = false, int keyTime = -1, byte[] key = null)
    {
        string token = await SignInAsync(guid, password);
        if (token is null)
            return;

        Connect(host, port, token, charId);
    }

    /// <summary>
    /// The same, for a character that does not exist yet.
    /// </summary>
    /// <remarks>
    /// The world server makes one on first connection when the character is not found, so this
    /// differs from loading only in which id is asked for.
    /// </remarks>
    public System.Threading.Tasks.Task ConnectToCreateAsync(
        string host, int port, string guid, string password, int gameId, int charId,
        ushort classType, ushort skinType) =>
        ConnectToLoadAsync(host, port, guid, password, gameId, charId);

    /// <summary>Exchanges a name and password for a session token, or reports why not.</summary>
    /// <remarks>
    /// A plain HTTP client rather than Godot's <c>HttpRequest</c>, which is a node and refuses to
    /// work until it is inside the scene tree — and signing in happens before the session is
    /// parented.
    /// </remarks>
    private async System.Threading.Tasks.Task<string> SignInAsync(string guid, string password)
    {
        string body = System.Text.Json.JsonSerializer.Serialize(new
        {
            name = guid,
            password,
        });

        try
        {
            using var http = new System.Net.Http.HttpClient
            {
                Timeout = TimeSpan.FromSeconds(15),
            };

            using var content = new System.Net.Http.StringContent(
                body, System.Text.Encoding.UTF8, "application/json");

            var answer = await http.PostAsync($"{AppServer}/login", content);
            string text = await answer.Content.ReadAsStringAsync();

            if (!answer.IsSuccessStatusCode)
            {
                Fail((int)answer.StatusCode, answer.StatusCode == System.Net.HttpStatusCode.Unauthorized
                    ? "Wrong name or password."
                    : $"Sign-in failed ({(int)answer.StatusCode}).");
                return null;
            }

            using var parsed = System.Text.Json.JsonDocument.Parse(text);
            if (!parsed.RootElement.TryGetProperty("token", out var token))
            {
                Fail(-1, "The account server answered without a token.");
                return null;
            }

            return token.GetString();
        }
        catch (Exception ex)
        {
            Fail(-1, $"Cannot reach the account server at {AppServer} — {ex.Message}");
            return null;
        }
    }

    private void Fail(int code, string description) =>
        Failed?.Invoke(new FailurePacket { ErrorId = code, ErrorDescription = description });

    /// <summary>Opens the connection. The token comes from the app server over HTTP.</summary>
    public bool Connect(string host, int port, string token, int character, bool allowAnyCertificate = true)
    {
        _known.Clear();
        PlayerObjectId = -1;
        _entered = false;

        return _net.Connect(host, port, token, character, allowAnyCertificate);
    }

    public void Disconnect() => _net?.Disconnect();

    /// <summary>Ends the session, for whatever reason the caller has.</summary>
    public void Close(string reason)
    {
        GD.Print($"[rust] closing: {reason}");
        Disconnect();
    }

    /// <summary>
    /// Notes where the player believes it is, for the input this frame.
    /// </summary>
    /// <remarks>
    /// The legacy protocol sent one Move per tick against a ledger the server audited. Here the
    /// claim is advisory and the server decides where the player actually ends up, so this records
    /// and <see cref="Poll"/> sends.
    /// </remarks>
    public void RecordPosition()
    {
        // The controller has already written the position; this exists because the legacy session
        // sampled here for the ledger, and there is no ledger to sample for.
    }

    /// <summary>
    /// Drains what arrived and reports where the player is. Called once per frame.
    /// </summary>
    public void Poll()
    {
        _tickTime = _clock?.NowMs ?? _tickTime;
        _net?.Poll();

        if (HasPlayerPosition)
            _net?.SendInput(new Vector2(PlayerX, PlayerY), _tickTime);
    }



    // ----------------------------------------------------------------------------------------
    // Arriving
    // ----------------------------------------------------------------------------------------

    private void OnWelcomed(int player, uint tick, string world, int width, int height)
    {
        PlayerObjectId = player;
        WorldName = world;

        // A welcome is a new world, and everything held about the last one is void: its entities
        // are gone, and — the part that disconnects you — its coordinates are meaningless here.
        // Claiming a nexus position inside a realm is a claim to have crossed a thousand tiles in
        // a tick, which is the one refusal the server is right to treat as a lie.
        _known.Clear();
        _tiles.Clear();
        _entered = false;
        HasPlayerPosition = false;

        // The extent comes with the welcome rather than being inferred from the rows, because the
        // map and the minimap are sized before the first row lands.
        MapLoaded?.Invoke(new MapInfoPacket
        {
            Name = world,
            DisplayName = world,
            Width = width,
            Height = height,
            Seed = 0,
            Background = 0,
            Difficulty = 0,
            AllowPlayerTeleport = true,
            ShowDisplays = true,
            Music = "",
        });
    }

    private void OnRejected(RejectReason reason)
    {
        Failed?.Invoke(new FailurePacket
        {
            ErrorId = (int)reason,
            ErrorDescription = reason switch
            {
                RejectReason.VersionMismatch => "This client is a different version to the server.",
                RejectReason.BadToken => "That sign-in has expired. Log in again.",
                RejectReason.AlreadyPlaying => "That character is already in a world.",
                RejectReason.Full => "The server is full.",
                RejectReason.Banned => "This account is banned.",
                RejectReason.NoSuchCharacter => "No such character.",
                _ => "The server refused the connection.",
            },
        });
    }

    private void OnChatted(string from, string text)
    {
        PacketReceived?.Invoke(new TextPacket
        {
            Name = from,
            ObjectId = -1,
            Text = text,
            Recipient = "",
            CleanText = text,
        });
    }

    // ----------------------------------------------------------------------------------------
    // The ground
    // ----------------------------------------------------------------------------------------

    private void OnTerrainRow(int y, int x, int[] tiles)
    {
        for (int i = 0; i < tiles.Length; i++)
            _tiles.Add(new GroundTile { X = (short)(x + i), Y = (short)y, Type = (ushort)tiles[i] });
    }

    private void OnGroundChanged(int[] xs, int[] ys, int[] tiles)
    {
        for (int i = 0; i < tiles.Length; i++)
            _tiles.Add(new GroundTile { X = (short)xs[i], Y = (short)ys[i], Type = (ushort)tiles[i] });
    }

    // ----------------------------------------------------------------------------------------
    // The world, once per frame
    // ----------------------------------------------------------------------------------------

    private void OnWorldChanged(WorldView view)
    {
        _arrived.Clear();
        _departed.Clear();
        _statuses.Clear();

        for (int i = 0; i < view.Count; i++)
        {
            int id = view.Ids[i];
            var stats = StatusOf(view, i);

            if (_known.TryAdd(id, view.Types[i]))
            {
                _arrived.Add(new ObjectDef
                {
                    ObjectType = (ushort)view.Types[i],
                    Stats = stats,
                });

                // Where the server put us, which is the only thing that can tell the client where
                // it is in a world it has just arrived in. The client owns its position from here
                // on; this is the one moment it does not.
                if (id == PlayerObjectId)
                {
                    var at = view.PositionOf(i);
                    PlayerX = at.X;
                    PlayerY = at.Y;
                    HasPlayerPosition = true;

                    Repositioned?.Invoke(new GotoPacket
                    {
                        ObjectId = id,
                        Position = new WorldPos(at.X, at.Y),
                    });
                }
                continue;
            }

            _statuses.Add(stats);
        }

        // Anything the snapshot stopped mentioning has either died or walked out of sight, and
        // nothing on this wire tells the two apart either.
        foreach (var (id, _) in _known)
        {
            bool present = false;
            for (int i = 0; i < view.Count && !present; i++)
                present = view.Ids[i] == id;

            if (!present)
                _departed.Add(id);
        }

        foreach (int id in _departed)
            _known.Remove(id);

        if (_tiles.Count > 0 || _arrived.Count > 0 || _departed.Count > 0)
        {
            WorldUpdated?.Invoke(new UpdatePacket
            {
                Tiles = _tiles.ToArray(),
                NewObjects = _arrived.ToArray(),
                Drops = _departed.ToArray(),
            });
            _tiles.Clear();
        }

        // The first snapshot is what "we are in" means here: there is no separate acceptance
        // message, because being sent a world is the acceptance.
        if (!_entered && PlayerObjectId >= 0)
        {
            _entered = true;
            Entered?.Invoke(new CreateSuccessPacket
            {
                ObjectId = PlayerObjectId,
                CharId = 0,
            });
        }

        if (_statuses.Count > 0)
        {
            Ticked?.Invoke(new NewTickPacket
            {
                TickId = 0,
                TickTime = _tickTime,
                Statuses = _statuses.ToArray(),
            });
        }
    }

    /// <summary>One entity's position and stats, in the shape <c>StatApplier</c> reads.</summary>
    private ObjectStats StatusOf(in WorldView view, int index)
    {
        var stats = new List<StatData>(12)
        {
            new() { Type = StatsType.Hp, IntValue = view.Hp[index] },
            new() { Type = StatsType.MaxHp, IntValue = view.MaxHp[index] },
            new() { Type = StatsType.Size, IntValue = view.Sizes[index] },
            new() { Type = StatsType.AltTextureIndex, IntValue = view.Textures[index] },
            new() { Type = StatsType.Condition, IntValue = (int)view.ConditionsLow[index] },
            new() { Type = StatsType.Condition2, IntValue = (int)(view.ConditionsLow[index] >> 32) },
        };

        if (!string.IsNullOrEmpty(view.Names[index]))
            stats.Add(new StatData { Type = StatsType.Name, StringValue = view.Names[index] });

        // The eight stats and the rest of what a HUD reads belong to the player alone; everything
        // else sends zeroes, and adding them for every entity would be per-frame work for values
        // nothing displays.
        if (view.Ids[index] == PlayerObjectId)
        {
            var own = view.StatsOf(index);
            if (own.Length == 8)
            {
                stats.Add(new StatData { Type = StatsType.MaxHp, IntValue = own[0] });
                stats.Add(new StatData { Type = StatsType.MaxMp, IntValue = own[1] });
                stats.Add(new StatData { Type = StatsType.Attack, IntValue = own[2] });
                stats.Add(new StatData { Type = StatsType.Defense, IntValue = own[3] });
                stats.Add(new StatData { Type = StatsType.Speed, IntValue = own[4] });
                stats.Add(new StatData { Type = StatsType.Dexterity, IntValue = own[5] });
                stats.Add(new StatData { Type = StatsType.Vitality, IntValue = own[6] });
                stats.Add(new StatData { Type = StatsType.Wisdom, IntValue = own[7] });
            }

            stats.Add(new StatData { Type = StatsType.Mp, IntValue = view.Mp[index] });
            stats.Add(new StatData { Type = StatsType.MaxMp, IntValue = view.MaxMp[index] });
            stats.Add(new StatData { Type = StatsType.Breath, IntValue = view.Oxygen[index] });
        }

        return new ObjectStats
        {
            ObjectId = view.Ids[index],
            Position = new WorldPos(view.PositionOf(index).X, view.PositionOf(index).Y),
            Stats = stats.ToArray(),
        };
    }

    // ----------------------------------------------------------------------------------------
    // Going out
    // ----------------------------------------------------------------------------------------

    /// <summary>Carries out what a legacy packet was asking for, where this protocol has it.</summary>
    public void Send(ClientPacket packet)
    {
        switch (packet)
        {
            case PlayerTextPacket text:
                _net.SendChat(text.Text);
                break;

            case PlayerShootPacket shot:
                _net.Shoot(shot.Angle);
                break;

            case UsePortalPacket portal:
                _net.UsePortal(portal.ObjectId);
                break;

            case InvSwapPacket swap:
                _net.MoveItem(
                    ContainerOf(swap.Slot1.ObjectId), swap.Slot1.SlotId,
                    ContainerOf(swap.Slot2.ObjectId), swap.Slot2.SlotId);
                break;

            case InvDropPacket drop:
                _net.DropItem(drop.Slot.SlotId);
                break;

            // Acknowledgements the old server audited a ledger for, and hit claims this one
            // refuses by design. Both are silently correct to drop.
            case ShootAckPacket:
            case AoeAckPacket:
            case EnemyHitPacket:
            case PlayerHitPacket:
            case OtherHitPacket:
            case SquareHitPacket:
            case GroundDamagePacket:
                break;

            default:
                if (_reported.Add(packet.GetType().Name))
                    GD.Print($"[rust] nothing carries {packet.GetType().Name} yet");
                break;
        }
    }

    /// <summary>
    /// Which container a slot belongs to: 0 what you carry, 1 what you wear, 2 the vault.
    /// </summary>
    /// <remarks>
    /// The legacy slot names an entity and a slot index, and containers here are named by tag so a
    /// client cannot address somebody else's inventory. Our own object id is what we are carrying.
    /// </remarks>
    private long ContainerOf(int objectId) => objectId == PlayerObjectId ? 0 : 2;
}
