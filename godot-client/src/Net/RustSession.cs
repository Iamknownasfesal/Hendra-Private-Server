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
    /// <remarks>
    /// Taken from the same configuration every other request reads, so <c>--app-port</c> reaches
    /// the sign-in as well as the panels. A literal here means the one request that carries a
    /// password is the one request that ignores where the server was said to be.
    /// </remarks>
    public string AppServer { get; set; } = App.ServerConfig.AppServer;

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
        _net.ContainerFilled += OnContainerFilled;
        _net.VaultUpdated += OnVaultUpdated;
        _net.Shot += OnShot;
        _net.Damage += OnDamage;
        _net.Repositioned += OnRepositioned;

        // The world we are already in sounding different: the /music command and the behaviours
        // that rescore a dungeon as it turns. Straight through to the packet the legacy path fed.
        _net.MusicSwitched += music => PacketReceived?.Invoke(new SwitchMusicPacket
        {
            Music = music,
        });

        // What the quest arrow points at, and what the camera follows. Both name an entity, and
        // both are drawn by handlers the client already has.
        _net.QuestTargeted += target => PacketReceived?.Invoke(new QuestObjIdPacket
        {
            ObjectId = target,
        });

        _net.Focused += target => PacketReceived?.Invoke(new SetFocusPacket
        {
            ObjectId = target,
        });

        // Who is on one of the account's lists, by name. The server enforces either way; this is
        // what puts the marker beside the name.
        _net.Listed += (list, names) => PacketReceived?.Invoke(new AccountListPacket
        {
            ListId = list == 1 ? AccountListId.LockedOut : AccountListId.Ignored,
            AccountIds = names,
            LockAction = -1,
        });

        // Straight through to the renderer the legacy path already fed: the effect numbers are the
        // original's (`Structures.cs:133-151`) and the client draws all nineteen of them, so there
        // is nothing here to translate.
        _net.ShowEffect += (effect, target, x1, y1, x2, y2, color) =>
            PacketReceived?.Invoke(new ShowEffectPacket
            {
                EffectType = (ShowEffectType)effect,
                TargetObjectId = target,
                Pos1 = new WorldPos(x1, y1),
                Pos2 = new WorldPos(x2, y2),
                Color = new Argb((uint)color),
            });

        // Straight through to the overlay the legacy path already fed. The message is either a
        // literal or a LineBuilder blob, which is the original's own ambiguity and which the
        // handler already resolves.
        _net.StatusText += (objectId, text, color) =>
            PacketReceived?.Invoke(new NotificationPacket
            {
                ObjectId = objectId,
                Message = text,
                Color = new Argb((uint)color),
            });

        // Marked as already applied: this server takes the health itself and reports it as an
        // ordinary hit, so what arrives here is the ring rather than an instruction to damage
        // ourselves.
        _net.Blast += (x, y, radius, damage, effect, duration, origType) =>
            PacketReceived?.Invoke(new AoePacket
            {
                Position = new WorldPos(x, y),
                Radius = radius,
                Damage = (ushort)damage,
                Effect = (ConditionEffectIndex)effect,
                DurationSeconds = duration,
                OrigType = (ushort)origType,
                ServerApplied = true,
            });

        _net.InvitedToGuild += (name, guild) =>
            PacketReceived?.Invoke(new InvitedToGuildPacket
            {
                Name = name,
                GuildName = guild,
            });

        // Commands rather than sentences, which is what GlobalNotification carries: whether the
        // gift chest has anything in it, a key colour, the key panel. The world controller reads
        // the five it knows and shows the rest.
        _net.Notified += text => PacketReceived?.Invoke(new GlobalNotificationPacket
        {
            Text = text,
        });

        _net.Notice += text => PacketReceived?.Invoke(new TextPacket
        {
            Name = "",
            ObjectId = -1,
            Text = text,
            Recipient = "",
            CleanText = text,
        });
        // What a command reports, under the name the original's `SendInfo` speaks in
        // (`Player.Chat.cs:110-119`), which the log draws in yellow. A refusal is a different thing
        // and takes a different route: `SendError` (`:132-141`) differs from `SendInfo` in nothing
        // but the sender name, so the server sends one as ordinary chat spoken by `*Error*` and it
        // arrives above, through <see cref="OnChatted"/>, already the right colour.
        _net.Refused += text => PacketReceived?.Invoke(new TextPacket
        {
            Name = "",
            ObjectId = -1,
            Text = text,
            Recipient = "",
            CleanText = text,
        });

        // The death screen goes up from here, and nowhere else: the world's own snapshot only ever
        // says a body stopped being mentioned, which is what walking out of sight looks like too.
        // ZombieId is minus one because nothing hands back a body to keep playing as -- the same
        // value the original writes for every ordinary death.
        _net.Died += (character, killedBy, fame) => PacketReceived?.Invoke(new DeathPacket
        {
            // The death screen asks the app server for the tally with both of these, and the app
            // server refuses a character that does not belong to the account named. Left empty it
            // asked for character 41 on behalf of nobody and got "Invalid character" where the
            // bonuses should have been.
            AccountId = _accountId,
            CharId = character,
            KilledBy = killedBy,
            ZombieType = 0,
            ZombieId = -1,
            FinalFame = fame,
        });
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

    /// <summary>Ids the server said outright were killed, since the last frame was applied.</summary>
    /// <remarks>
    /// Held rather than acted on immediately because the world is rebuilt once per frame, and a
    /// removal handed to it between frames would be undone by the next snapshot.
    /// </remarks>
    private readonly HashSet<int> _killed = new();

    private readonly List<ObjectStats> _statuses = new();
    private readonly List<GroundTile> _tiles = new();

    /// <summary>Names of unhandled sends already reported, so each is mentioned once.</summary>
    private readonly HashSet<string> _reported = new();

    /// <summary>
    /// What the player is carrying and wearing, by slot.
    /// </summary>
    /// <remarks>
    /// Containers arrive whole, on their own event, rather than in the snapshot — a container
    /// changes when a player moves something, not twenty times a second. The game layer reads them
    /// as stats on the player's own entity, so they are held here and put back on the next status.
    /// </remarks>
    private readonly Dictionary<int, int> _carried = new();
    private bool _containersChanged;

    private int _tickTime;

    /// <summary>
    /// When the previous snapshot was applied, so a tick can report how long it spanned.
    /// </summary>
    /// <remarks>
    /// <see cref="_tickTime"/> is a clock reading, not a duration. Entities divide the distance they
    /// moved by the span to get a movement vector, so handing them the reading yields a vector
    /// smaller by the age of the session and leaves facing, <c>IsMoving</c> and the while-moving
    /// animation reading from noise.
    /// </remarks>
    private int _lastSnapshotMs = -1;

    private bool _entered;

    /// <summary>
    /// How far the client and the server may disagree about the player before the server wins.
    /// </summary>
    /// <remarks>
    /// Loose enough that ordinary prediction — the client moving a frame ahead of the tick that
    /// confirms it — is never fought over, and tight enough that a real divergence is corrected
    /// long before it looks like a client claiming to have crossed the map.
    /// </remarks>
    private const float MaxDriftSquared = 4f;

    public int PlayerObjectId { get; private set; } = -1;

    /// <summary>
    /// The shared damage generator, which this protocol does not have.
    /// </summary>
    /// <remarks>
    /// The legacy client predicted its own damage from a generator stepped in lockstep with the
    /// server, and every extra or missing draw desynchronised everything after it. Here the server
    /// rolls a player's own damage alone and reports what it took, so there is nothing to predict
    /// and nothing to keep in step. Null is the answer, and both call sites already read it that
    /// way.
    /// </remarks>
    /// <remarks>
    /// Damage the player <em>takes</em> is still predicted, because it needs no shared generator:
    /// the amount rides on the shot itself, exactly as the original's two shot packets carry it.
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
    /// The character is made over HTTP before the socket is opened, because the class has to be
    /// said somewhere and the game socket has nowhere to say it: its login carries a token and a
    /// character id and nothing else. Connecting without creating first reaches a server that finds
    /// no such character and makes one of whatever class it defaults to, which is how asking for a
    /// wizard produced something else entirely.
    /// </remarks>
    public async System.Threading.Tasks.Task ConnectToCreateAsync(
        string host, int port, string guid, string password, int gameId, int charId,
        ushort classType, ushort skinType)
    {
        string token = await SignInAsync(guid, password);
        if (token is null)
            return;

        int created = await CreateCharacterAsync(token, classType);
        if (created < 0)
            return;

        Connect(host, port, token, created);
    }

    /// <summary>
    /// Asks the account server for a new character of a class, and returns its id.
    /// </summary>
    /// <remarks>
    /// Returns -1 having already reported why, so the caller only has to know whether to carry on.
    /// A refusal here is a class the account has not unlocked, and the server's own wording says
    /// which class has to be levelled first.
    /// </remarks>
    /// <remarks>
    /// A class is all that is sent, and no name. The character wears the account's, which is the
    /// name the player already chose and the only one a character has.
    /// </remarks>
    private async System.Threading.Tasks.Task<int> CreateCharacterAsync(string token, ushort classType)
    {
        string body = System.Text.Json.JsonSerializer.Serialize(new { @class = classType });

        try
        {
            using var http = new System.Net.Http.HttpClient
            {
                Timeout = TimeSpan.FromSeconds(15),
            };
            http.DefaultRequestHeaders.Authorization =
                new System.Net.Http.Headers.AuthenticationHeaderValue("Bearer", token);

            using var content = new System.Net.Http.StringContent(
                body, System.Text.Encoding.UTF8, "application/json");

            var answer = await http.PostAsync($"{AppServer}/characters", content);
            string text = await answer.Content.ReadAsStringAsync();

            using var parsed = System.Text.Json.JsonDocument.Parse(text);

            if (!answer.IsSuccessStatusCode)
            {
                // The refusal carries prose meant to be read: "that class is locked; reach level
                // five with an Archer" is the whole of what the player needs.
                string why = parsed.RootElement.TryGetProperty("error", out var error)
                    ? error.GetString()
                    : $"The character could not be created ({(int)answer.StatusCode}).";
                Fail((int)answer.StatusCode, why);
                return -1;
            }

            if (!parsed.RootElement.TryGetProperty("id", out var id))
            {
                Fail(-1, "The account server made a character without saying which.");
                return -1;
            }

            return id.GetInt32();
        }
        catch (Exception ex)
        {
            Fail(-1, $"Cannot reach the account server at {AppServer} — {ex.Message}");
            return -1;
        }
    }

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

    /// <summary>Which account this session signed in as, taken from its own token.</summary>
    /// <remarks>
    /// The token reads <c>account.character.expiry.signature</c>, so the account is the first field
    /// of it. Read here rather than carried alongside because every way in — the login screen, the
    /// command line, a character just created — arrives at <see cref="Connect"/> with a token and
    /// nothing else in common.
    /// </remarks>
    private string _accountId = string.Empty;

    /// <summary>Opens the connection. The token comes from the app server over HTTP.</summary>
    public bool Connect(string host, int port, string token, int character, bool allowAnyCertificate = true)
    {
        _known.Clear();
        PlayerObjectId = -1;
        _entered = false;

        int dot = token?.IndexOf('.') ?? -1;
        _accountId = dot > 0 ? token.Substring(0, dot) : string.Empty;

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

    private void OnWelcomed(Welcome welcome)
    {
        PlayerObjectId = welcome.Player;
        WorldName = welcome.World;

        // A welcome is a new world, and everything held about the last one is void: its entities
        // are gone, and — the part that disconnects you — its coordinates are meaningless here.
        // Claiming a nexus position inside a realm is a claim to have crossed a thousand tiles in
        // a tick, which is the one refusal the server is right to treat as a lie.
        _known.Clear();
        _tiles.Clear();
        _entered = false;
        HasPlayerPosition = false;

        // The extent comes with the welcome rather than being inferred from the rows, because the
        // map and the minimap are sized before the first row lands. The rest of it is what the
        // world says about itself: the marks on the loading screen, the sky, whether a teleport may
        // be offered, and what to play.
        MapLoaded?.Invoke(new MapInfoPacket
        {
            Name = welcome.World,
            DisplayName = welcome.World,
            Width = welcome.Width,
            Height = welcome.Height,
            Seed = 0,
            Background = welcome.Background,
            Difficulty = welcome.Difficulty,
            AllowPlayerTeleport = welcome.AllowTeleport,
            ShowDisplays = welcome.ShowDisplays,
            Music = welcome.Music,
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

    private void OnChatted(int speaker, string from, string text)
    {
        // A line with a body behind it is drawn over that body as well as written in the log; one
        // the server spoke in its own voice has no body, and a bubble over nobody is nothing.
        PacketReceived?.Invoke(new TextPacket
        {
            Name = from,
            ObjectId = speaker > 0 ? speaker : -1,
            BubbleTime = speaker > 0 ? BubbleSeconds : (byte)0,
            Text = text,
            Recipient = "",
            CleanText = text,
        });
    }

    /// <summary>How long a spoken line stays over the speaker's head, from ChatManager.cs.</summary>
    private const byte BubbleSeconds = 5;

    /// <summary>Takes a container's whole contents, to be reported as slots on the player.</summary>
    private void OnContainerFilled(int container, int[] slots, int[] items)
    {
        // 0 is what the player carries and 1 what they wear; the client lays both out in one run of
        // slots, equipment first. The vault is a screen of its own and not part of this.
        if (container > 1)
            return;

        int first = container == 1 ? 0 : CarriedFirstSlot;
        int count = container == 1 ? WornSlots : CarriedSlots;

        // Emptied first. Only the occupied slots come over the wire, so a slot the player has just
        // emptied is described by its absence, and keeping the previous contents would leave the
        // item showing in a place it no longer is.
        for (int i = 0; i < count; i++)
            _carried[first + i] = Empty;

        // Parallel arrays: slots[i] says where, items[i] says what.
        for (int i = 0; i < slots.Length && i < items.Length; i++)
        {
            if (slots[i] >= 0 && slots[i] < count)
                _carried[first + slots[i]] = items[i];
        }

        _containersChanged = true;
    }

    /// <summary>
    /// Where the carried run begins in the flat slot numbering the game reads.
    /// </summary>
    /// <remarks>
    /// Eight, not four. Only four of the worn slots are ever drawn, but the numbering leaves room
    /// for eight, and the carried items begin after the gap rather than after the last drawn slot.
    /// </remarks>
    private const int CarriedFirstSlot = 8;

    /// <summary>How many slots a character wears, and how many they carry.</summary>
    private const int WornSlots = 4;
    private const int CarriedSlots = 8;

    /// <summary>An empty slot. Minus one, because zero is a real object type.</summary>
    private const int Empty = -1;

    /// <summary>
    /// The whole vault, which is the only thing the vault panel is drawn from.
    /// </summary>
    /// <remarks>
    /// Its own message rather than a container: the panel needs the version to quote back on the
    /// next move, the capacity to stop offering more chests, and the gifts, and a container says
    /// none of those. The store this reaches applies it whole, so a client that missed one update
    /// is corrected by the next rather than drifting.
    /// </remarks>
    private void OnVaultUpdated(
        int version, int chestCount, int maxChests, int nextChestPrice, int[] slots, int[] gifts)
    {
        PacketReceived?.Invoke(new VaultUpdatePacket
        {
            Version = version,
            ChestCount = chestCount,
            MaxChests = maxChests,
            NextChestPrice = nextChestPrice,
            Slots = Types(slots),
            Gifts = Types(gifts),
        });
    }

    /// <summary>Narrows what the engine's integer arrays carry back to the item types they hold.</summary>
    private static ushort[] Types(int[] values)
    {
        if (values == null || values.Length == 0)
            return Array.Empty<ushort>();

        var types = new ushort[values.Length];
        for (int i = 0; i < values.Length; i++)
            types[i] = (ushort)values[i];

        return types;
    }

    /// <summary>A projectile somebody fired, which the client draws and animates itself.</summary>
    private void OnShot(
        int projectile, int owner, int objectType, float x, float y, float angle, float speed, int lifetimeMs,
        int damage)
    {
        // A shot of ours that the server bothered to tell us about is one we could not have drawn
        // ourselves: the server leaves us out of everything our own trigger produced, and sends
        // back only what it authored somewhere we are not standing — a nova's ring, which starts at
        // the cursor. Those carry their own descriptor and their own starting point, which is what
        // the server-authored path reads and what the enemy path, keyed on the shooter's own
        // descriptor, cannot: a player has no projectiles of their own to look the bullet up in.
        if (owner == PlayerObjectId)
        {
            PacketReceived?.Invoke(new ServerPlayerShootPacket
            {
                BulletId = (byte)projectile,
                OwnerId = owner,
                ContainerType = objectType,
                StartingPos = new WorldPos(x, y),
                Angle = angle,

                // What the bullet is worth, carried on the shot as the original's own two shot
                // packets carry it (`ServerPlayerShoot.as:20-27`, `EnemyShoot.as:23-35`). Whoever
                // it lands on is the one client the server does not tell when it lands, so this is
                // what the number over their own head is worked out from.
                Damage = (short)Math.Clamp(damage, 0, short.MaxValue),
            });
            return;
        }

        PacketReceived?.Invoke(new EnemyShootPacket
        {
            BulletId = (byte)projectile,
            OwnerId = owner,
            BulletType = 0,
            StartingPos = new WorldPos(x, y),
            Angle = angle,
            Damage = (short)Math.Clamp(damage, 0, short.MaxValue),
            NumShots = 1,
            AngleInc = 0f,
        });
    }

    /// <summary>The server moved a body somewhere it did not walk to.</summary>
    /// <remarks>
    /// The only thing that can move the local player, and the only thing that makes another body
    /// appear somewhere rather than travel there. A snapshot cannot say either: this client owns
    /// its own position and glides everything else towards the position a snapshot gives it, so
    /// without this a teleport is a walk the server refuses and the client undoes.
    /// </remarks>
    private void OnRepositioned(int objectId, float x, float y)
    {
        // Published before the packet goes out, because the next input claims these fields and a
        // claim made from where we used to be is the server's own teleport being walked back.
        if (objectId == PlayerObjectId)
        {
            PlayerX = x;
            PlayerY = y;
            HasPlayerPosition = true;
        }

        Repositioned?.Invoke(new GotoPacket
        {
            ObjectId = objectId,
            Position = new WorldPos(x, y),
        });
    }

    /// <summary>A hit the server resolved, and whether it killed.</summary>
    /// <remarks>
    /// The snapshot carries health, so this is not where the number comes from. It is the only
    /// thing on this wire that separates a body that died from one that walked out of sight, and
    /// without it the world has to guess from a snapshot diff — which leaves corpses standing
    /// until whatever they were doing happens to stop.
    /// </remarks>
    private void OnDamage(int target, int[] effects, int amount, bool kill, int bullet, int owner)
    {
        if (kill)
            _killed.Add(target);

        var indices = new ConditionEffectIndex[effects.Length];
        for (int i = 0; i < effects.Length; i++)
            indices[i] = (ConditionEffectIndex)effects[i];

        PacketReceived?.Invoke(new DamagePacket
        {
            TargetId = target,
            Effects = indices,
            DamageAmount = (ushort)Math.Clamp(amount, 0, ushort.MaxValue),
            Kill = kill,
            BulletId = (byte)bullet,
            ObjectId = owner,
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

            // No correction while simply walking. The server clamps a claim it will not grant and
            // the client walks on from where it believes it is; snapping to the server's answer
            // every frame is a fight between the two, and the fight is what a player feels as
            // jitter. Arriving in a world is the exception, and it is handled where an entity
            // first appears; being moved by the server is the other, and it arrives as its own
            // message rather than being guessed at from a snapshot.
            _statuses.Add(stats);
        }

        // Anything the server said outright was killed goes first and goes immediately, whether or
        // not this snapshot still mentions it. That is the one case the diff below cannot decide:
        // a body missing from a snapshot has either died or walked out of sight, and only the
        // server's own `Kill` tells the two apart.
        foreach (int id in _killed)
        {
            if (_known.Remove(id))
                _departed.Add(id);
        }
        _killed.Clear();

        // Everything else the snapshot stopped mentioning has walked out of sight, or died without
        // a hit anybody could see.
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
            int arrived = _clock?.NowMs ?? _tickTime;
            int span = _lastSnapshotMs < 0 ? 0 : arrived - _lastSnapshotMs;
            _lastSnapshotMs = arrived;

            Ticked?.Invoke(new NewTickPacket
            {
                TickId = 0,
                TickTime = span,
                Statuses = _statuses.ToArray(),
            });
        }
    }

    /// <summary>Bit zero of an entity's marks: the account behind it is an administrator.</summary>
    private const int AdminMark = 1;

    /// <summary>Bit one: this character owns the eight extra carried slots.</summary>
    private const int BackpackMark = 2;

    /// <summary>Bit two: this account picked its own name rather than being handed one.</summary>
    private const int NameChosenMark = 4;

    /// <summary>Bit three: this portal refuses to be entered.</summary>
    private const int PortalUnusableMark = 8;

    /// <summary>
    /// Which entities were last sent a dye, so washing one off is sent once rather than never.
    /// </summary>
    /// <remarks>
    /// The same reasoning as <see cref="_glowing"/>: zero is the value nearly every entity has
    /// forever, and remembering who was painted is what separates "wears no dye" from "has just had
    /// one taken off".
    /// </remarks>
    private readonly HashSet<int> _dyed = new();

    /// <summary>
    /// Which entities were last sent a halo, so putting one out is sent once rather than never.
    /// </summary>
    /// <remarks>
    /// A stat is only worth sending when it says something, and zero is the value nearly every
    /// entity has forever. Remembering who was lit is what separates "has no halo" from "has just
    /// had one taken off", which is the difference between <c>/glow 0</c> working and not.
    /// </remarks>
    private readonly HashSet<int> _glowing = new();

    /// <summary>One entity's position and stats, in the shape <c>StatApplier</c> reads.</summary>
    private ObjectStats StatusOf(in WorldView view, int index)
    {
        // A player's maxima are what its eight stats total to, and everything else's are its own.
        // One or the other, never both: the game applies these in order and the last write wins, so
        // a value taken from two places shows whichever happened to be appended second.
        var own = view.Ids[index] == PlayerObjectId ? view.StatsOf(index) : null;
        bool totalled = own is { Length: WorldView.StatCount };

        var stats = new List<StatData>(16)
        {
            new() { Type = StatsType.Hp, IntValue = view.Hp[index] },
            new() { Type = StatsType.MaxHp, IntValue = totalled ? own[0] : view.MaxHp[index] },
            new() { Type = StatsType.Size, IntValue = view.Sizes[index] },
            new() { Type = StatsType.AltTextureIndex, IntValue = view.Textures[index] },

            // The skin, which is a different sheet entirely rather than a frame within one. The
            // original carries it as its own stat for the same reason (`Stats.cs:81`,
            // `Player.cs:303`).
            new() { Type = StatsType.Skin, IntValue = view.Skins[index] },
            new() { Type = StatsType.Condition, IntValue = (int)view.ConditionsLow[index] },
            new() { Type = StatsType.Condition2, IntValue = (int)(view.ConditionsLow[index] >> 32) },
        };

        if (!string.IsNullOrEmpty(view.Names[index]))
            stats.Add(new StatData { Type = StatsType.Name, StringValue = view.Names[index] });

        // The rating beside a name, and the mark that recolours it. The original draws them into
        // the name plate rather than writing them out -- one star per fame threshold each of this
        // account's classes has passed, coloured by the total and by one colour of its own for an
        // administrator (Player.as:750-756). The stars are an account's, so nothing that happens in
        // a world moves them.
        int marks = view.MarksOf(index);
        if (view.Stars[index] > 0 || (marks & AdminMark) != 0)
        {
            stats.Add(new StatData { Type = StatsType.NumStars, IntValue = view.Stars[index] });
            stats.Add(new StatData { Type = StatsType.Admin, IntValue = (marks & AdminMark) != 0 ? 1 : 0 });
        }

        // What a dye put on. The clothing dye and the accessory dye of one colour carry the same
        // number and differ only in which layer they paint, so both travel and neither is guessed
        // at from the value (Player.cs:301-302).
        var dyes = view.DyesOf(index);
        if (dyes.Length == 2 && (dyes[0] != 0 || dyes[1] != 0 || _dyed.Contains(view.Ids[index])))
        {
            stats.Add(new StatData { Type = StatsType.Tex1, IntValue = dyes[0] });
            stats.Add(new StatData { Type = StatsType.Tex2, IntValue = dyes[1] });

            if (dyes[0] != 0 || dyes[1] != 0)
                _dyed.Add(view.Ids[index]);
            else
                _dyed.Remove(view.Ids[index]);
        }

        // Which neighbours a wall or a fence joins onto. Sent only where there is a join, since
        // every other entity in the world would otherwise pay a stat to say "none"
        // (ConnectedObject.cs:114-118).
        int connection = view.ConnectionOf(index);
        if (connection != 0)
            stats.Add(new StatData { Type = StatsType.ObjectConnection, IntValue = connection });

        // Whether a portal will let anybody through. The original defaults it to true and
        // PortalMonitor closes it when a realm ends (Portal.cs:15, PortalMonitor.cs:166); the
        // packed marks carry the closed case, so this says so only when it is shut.
        if ((marks & PortalUnusableMark) != 0)
            stats.Add(new StatData { Type = StatsType.PortalActive, IntValue = 0 });

        // The halo /glow sets, which the original paints as a coloured ring around the sprite
        // (GameObject.setGlow -> GlowRedrawer.as:19-46). Zero for everybody who has not asked for
        // one, and sent even then so taking it off puts the ring out.
        int glow = view.GlowOf(index);
        if (glow != 0 || _glowing.Contains(view.Ids[index]))
        {
            stats.Add(new StatData { Type = StatsType.GlowColor, IntValue = glow });

            if (glow != 0)
                _glowing.Add(view.Ids[index]);
            else
                _glowing.Remove(view.Ids[index]);
        }

        // The guild written under a player's own name, and the rank that colours it. ExportStats
        // sends both on every update (Player.cs:291-292) and the client draws them over the head
        // and in the tooltip; without them nobody can see who is standing with them.
        string guild = view.GuildOf(index);
        if (!string.IsNullOrEmpty(guild))
        {
            stats.Add(new StatData { Type = StatsType.GuildName, StringValue = guild });
            stats.Add(new StatData { Type = StatsType.GuildRank, IntValue = view.GuildRankOf(index) });
        }

        // What a loot bag, a chest or a vault is holding. The original writes the eight slots into
        // the container's stats (Container.cs:76-90), and the panel that opens at the player's feet
        // is drawn from them -- without these every bag on the ground looks empty.
        var contents = view.ContentsOf(index);
        for (int slot = 0; slot < contents.Length; slot++)
        {
            if (contents[slot] < 0)
                continue;

            stats.Add(new StatData
            {
                Type = (StatsType)((int)StatsType.Inventory0 + slot),
                IntValue = contents[slot],
            });
        }

        // What a vendor is offering. SellableObject and Merchant write these five into the entity's
        // stats (SellableObject.cs:72-78, Merchant.cs:46-52); without them a merchant is drawn as a
        // placeholder and the panel that buys never opens, which leaves every shop in the game
        // reachable only by typing.
        var stall = view.MerchandiseOf(index);
        if (stall.Length == WorldView.MerchandiseFields && stall[0] >= 0)
        {
            stats.Add(new StatData { Type = StatsType.MerchandiseType, IntValue = stall[0] });
            stats.Add(new StatData { Type = StatsType.MerchandisePrice, IntValue = stall[1] });
            stats.Add(new StatData { Type = StatsType.MerchandiseCurrency, IntValue = stall[2] });
            stats.Add(new StatData { Type = StatsType.MerchandiseCount, IntValue = stall[3] });
            stats.Add(new StatData { Type = StatsType.MerchandiseRankReq, IntValue = stall[4] });
        }

        // The eight stats and the rest of what a HUD reads belong to the player alone; everything
        // else sends zeroes, and adding them for every entity would be per-frame work for values
        // nothing displays.
        if (view.Ids[index] == PlayerObjectId)
        {
            // What the character is wearing and carrying, which the game reads as slots on the
            // player rather than as a container of its own.
            if (_containersChanged)
            {
                foreach (var (slot, item) in _carried)
                {
                    var type = slot < 16
                        ? (StatsType)((int)StatsType.Inventory0 + slot)
                        : (StatsType)((int)StatsType.Backpack0 + slot - 16);

                    stats.Add(new StatData { Type = type, IntValue = item });
                }
            }

            // The order the original writes them in, from Player.cs:330 onwards. The maximum health
            // is already in the list above, taken from the same totals these come from.
            if (totalled)
            {
                stats.Add(new StatData { Type = StatsType.MaxMp, IntValue = own[1] });
                stats.Add(new StatData { Type = StatsType.Attack, IntValue = own[2] });
                stats.Add(new StatData { Type = StatsType.Defense, IntValue = own[3] });
                stats.Add(new StatData { Type = StatsType.Speed, IntValue = own[4] });
                stats.Add(new StatData { Type = StatsType.Dexterity, IntValue = own[5] });
                stats.Add(new StatData { Type = StatsType.Vitality, IntValue = own[6] });
                stats.Add(new StatData { Type = StatsType.Wisdom, IntValue = own[7] });

                // The three the eight-stat character sheet has no row for. DamageMin and DamageMax
                // are the equipped weapon's own bounds, which is what a shot rolls between, and
                // Luck is the private-drop bonus. ExportStats sends all three (Player.cs:339-341)
                // and nothing in this content ever moves the last one off zero.
                stats.Add(new StatData { Type = StatsType.DamageMin, IntValue = own[8] });
                stats.Add(new StatData { Type = StatsType.DamageMax, IntValue = own[9] });
                stats.Add(new StatData { Type = StatsType.Luck, IntValue = own[10] });

                // What equipment and running boosts add to each of those, which is what the sheet
                // draws in green beside them. ExportStats sends the whole Boost array
                // (Player.cs:342-352); a total on its own cannot say how much of an attack is the
                // ring, so a player has no way to see what taking an item off would cost.
                var boosts = view.BoostsOf(index);
                if (boosts.Length == WorldView.StatCount)
                {
                    stats.Add(new StatData { Type = StatsType.MaxHpBoost, IntValue = boosts[0] });
                    stats.Add(new StatData { Type = StatsType.MaxMpBoost, IntValue = boosts[1] });
                    stats.Add(new StatData { Type = StatsType.AttackBoost, IntValue = boosts[2] });
                    stats.Add(new StatData { Type = StatsType.DefenseBoost, IntValue = boosts[3] });
                    stats.Add(new StatData { Type = StatsType.SpeedBoost, IntValue = boosts[4] });
                    stats.Add(new StatData { Type = StatsType.DexterityBoost, IntValue = boosts[5] });
                    stats.Add(new StatData { Type = StatsType.VitalityBoost, IntValue = boosts[6] });
                    stats.Add(new StatData { Type = StatsType.WisdomBoost, IntValue = boosts[7] });

                    // And the three the sheet has no row for, which are boosted like any other:
                    // ExportStats sends DamageMinBonus, DamageMaxBonus and LuckBonus beside the
                    // eight (Player.cs:350-352). Without them the damage line cannot say how much
                    // of a shot is the weapon and how much is what is worn around it.
                    stats.Add(new StatData { Type = StatsType.DamageMinBonus, IntValue = boosts[8] });
                    stats.Add(new StatData { Type = StatsType.DamageMaxBonus, IntValue = boosts[9] });
                    stats.Add(new StatData { Type = StatsType.LuckBonus, IntValue = boosts[10] });
                }
            }
            else
            {
                stats.Add(new StatData { Type = StatsType.MaxMp, IntValue = view.MaxMp[index] });
            }

            stats.Add(new StatData { Type = StatsType.Mp, IntValue = view.Mp[index] });
            stats.Add(new StatData { Type = StatsType.Breath, IntValue = view.Oxygen[index] });

            // Levelling, which the HUD's top bar and the character sheet are both drawn from. The
            // experience is already the figure within the current level, as the original sends it
            // (Player.cs:287), so the bar is a fraction without the client knowing the curve.
            stats.Add(new StatData { Type = StatsType.Level, IntValue = view.Levels[index] });
            stats.Add(new StatData { Type = StatsType.Exp, IntValue = view.Experience[index] });
            stats.Add(new StatData { Type = StatsType.NextLevelExp, IntValue = view.ExperienceGoals[index] });
            stats.Add(new StatData { Type = StatsType.Fame, IntValue = view.Fame[index] });

            // The account's purse, which is a different thing from the fame above: Fame is what
            // this character has earned and CurrentFame is what the account may spend
            // (Player.cs:291-292). Every shop in the game prices in one of these three, so without
            // them the HUD shows an empty purse and no merchant will sell anything.
            var purse = view.PurseOf(index);
            if (purse.Length == 3)
            {
                stats.Add(new StatData { Type = StatsType.Credits, IntValue = purse[0] });
                stats.Add(new StatData { Type = StatsType.CurrentFame, IntValue = purse[1] });
                stats.Add(new StatData { Type = StatsType.Prestige, IntValue = purse[2] });
            }

            // The eight extra carried slots, which the HUD shows a tab for only when the character
            // owns them. The server refuses a move into them for one that does not
            // (InvSwapHandler.cs:177), so the two ends have to agree.
            stats.Add(new StatData
            {
                Type = StatsType.HasBackpack,
                IntValue = (marks & BackpackMark) != 0 ? 1 : 0,
            });

            stats.Add(new StatData
            {
                Type = StatsType.NameChosen,
                IntValue = (marks & NameChosenMark) != 0 ? 1 : 0,
            });

            // The ceiling the fame bar fills towards. Without it the bar has a numerator and no
            // denominator (Player.cs:293).
            stats.Add(new StatData
            {
                Type = StatsType.NextClassQuestFame,
                IntValue = view.FameGoalOf(index),
            });

            // The three boost clocks, in whole seconds as ExportStats sends them
            // (Player.cs:356-358). The XpBoosted flag is derived rather than sent, exactly as the
            // original derives it: `(XPBoostTime != 0) ? 1 : 0` (Player.cs:355).
            var clocks = view.BoostTimeOf(index);
            if (clocks.Length == 3)
            {
                stats.Add(new StatData { Type = StatsType.XpBoosted, IntValue = clocks[0] != 0 ? 1 : 0 });
                stats.Add(new StatData { Type = StatsType.XpTimer, IntValue = clocks[0] });
                stats.Add(new StatData { Type = StatsType.LdTimer, IntValue = clocks[1] });
                stats.Add(new StatData { Type = StatsType.LtTimer, IntValue = clocks[2] });
            }
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
                MoveItem(swap.Slot1, swap.Slot2);
                break;

            case InvDropPacket drop:
                _net.DropItem(drop.Slot.SlotId);
                break;

            // The vault panel speaks for itself: a chest and a slot at each end, and the version it
            // last saw. Not an InvSwap, because a vault slot is not an entity's slot and the panel
            // has to be told when it loses a race.
            case VaultMovePacket move:
                _net.VaultMove(
                    move.Version, move.FromChest, move.FromSlot, move.ToChest, move.ToSlot);
                break;

            case VaultBuyPacket buy:
                _net.VaultBuy(buy.ChestCount);
                break;

            // The merchant and nothing else. The quantity the original's packet carries is read
            // and then ignored by the server (BuyHandler.cs:16), so one press buys one item.
            case BuyPacket purchase:
                _net.Buy(purchase.ObjectId);
                break;

            // The container, the slot and where it was aimed. All three of what the slot object
            // carries, because a slot alone cannot say whose: `UseItemHandler` passes the object id
            // and the slot id on together (UseItemHandler.cs:22), which is what lets a potion be
            // drunk out of a bag on the ground. The use type is dropped because the server does not
            // read that either, so a held ability is two ordinary uses to it.
            case UseItemPacket use:
                _net.UseItem(
                    use.Slot.ObjectId, use.Slot.SlotId, use.ItemUsePos.X, use.ItemUsePos.Y);
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
    /// Sends a swap, which is one message of a different shape for each pair of ends it can join.
    /// </summary>
    /// <remarks>
    /// <para>
    /// A legacy swap names an entity and a slot at each end, and the entity is the whole of what
    /// says which container is meant. Here the player's own slots are named by tag — a client
    /// cannot address somebody else's pack — and a bag on the ground is named by its entity,
    /// because a bag belongs to the world rather than to a player.
    /// </para>
    /// <para>
    /// The slot numbers are handed over exactly as the game counts them: worn from nought with room
    /// for eight, carried from eight, the backpack from sixteen. What that number means on the
    /// wire is worked out in one place, <c>hendra_net::slot</c>, which the server reads the same
    /// numbers through — so a drag lands on the square the player dragged to.
    /// </para>
    /// <para>
    /// A swap between two bags is not a move this server has: neither end is the player's, and
    /// there is nothing to say. Nothing is sent rather than something addressed at random.
    /// </para>
    /// </remarks>
    private void MoveItem(SlotObject from, SlotObject to)
    {
        bool fromPlayer = from.ObjectId == PlayerObjectId;
        bool toPlayer = to.ObjectId == PlayerObjectId;

        if (fromPlayer && toPlayer)
            _net.MoveItem(from.SlotId, to.SlotId);
        else if (fromPlayer)
            _net.PutInBag(from.SlotId, to.ObjectId, to.SlotId);
        else if (toPlayer)
            _net.TakeFromBag(from.ObjectId, from.SlotId, to.SlotId);
    }
}
