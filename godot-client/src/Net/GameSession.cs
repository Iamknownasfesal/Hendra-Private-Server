using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Hendra.Core;
using Hendra.Data;
using Hendra.Net.Packets;

namespace Hendra.Net;

/// <summary>Where we are in the world server's handshake.</summary>
public enum SessionState
{
    Disconnected,

    /// <summary>Socket open, Hello sent, waiting for MapInfo.</summary>
    Handshaking,

    /// <summary>Waiting for a slot. Only QueuePing/QueuePong traffic flows.</summary>
    Queued,

    /// <summary>MapInfo received; Load or Create sent; waiting for CreateSuccess.</summary>
    Entering,

    /// <summary>In the world. Everything is permitted.</summary>
    Ready,
}

/// <summary>
/// Drives the world-server protocol: the handshake, the keepalives, and — most importantly — the
/// acknowledgement obligations the server counts.
/// </summary>
/// <remarks>
/// The server maintains a strict ledger and disconnects on any imbalance, in either direction:
/// <list type="bullet">
/// <item>exactly one Move per NewTick, in order, echoing that tick's id;</item>
/// <item>exactly one UpdateAck per Update;</item>
/// <item>exactly one GotoAck per Goto — including the Gotos broadcast when *other* players
/// teleport, which is easy to miss;</item>
/// <item>a Pong within 12 s of each Ping, and a QueuePong with the *matching serial* within 15 s of
/// each QueuePing.</item>
/// </list>
/// So every one of those replies is sent from here, before the corresponding event is raised. If a
/// subscriber throws while handling an event, the acknowledgement has already gone out and the
/// connection survives; doing it the other way round would turn a rendering bug into a disconnect.
///
/// Several server handlers dereference our player without a null check, so nothing beyond the
/// handshake may be sent before <see cref="SessionState.Ready"/>.
/// </remarks>
public sealed class GameSession : IDisposable
{
    private readonly GameConnection _connection = new();
    private readonly GameClock _clock;
    private readonly MovePacket _movePacket = new();
    private readonly UpdateAckPacket _updateAck = new();
    private readonly GotoAckPacket _gotoAck = new();
    private readonly PongPacket _pong = new();
    private readonly QueuePongPacket _queuePong = new();

    private int _gameId;
    private int _charId;
    private bool _createCharacter;
    private ushort _createClassType;
    private ushort _createSkinType;
    private bool _isFromArena;
    private int _keyTime = -1;
    private byte[] _key = Array.Empty<byte>();

    public GameSession(GameClock clock)
    {
        _clock = clock;
        _connection.Disconnected += reason => Disconnected?.Invoke(reason);
    }

    public SessionState State { get; private set; } = SessionState.Disconnected;

    /// <summary>Our own entity id, valid once <see cref="SessionState.Ready"/>. -1 before that.</summary>
    public int PlayerObjectId { get; private set; } = -1;

    /// <summary>The tick id of the most recent NewTick.</summary>
    public int LastTickId { get; private set; }

    /// <summary>
    /// The damage generator, shared with the server. Reseeded on every MapInfo. Null until the
    /// first one arrives.
    /// </summary>
    public MinstdRandom Random { get; private set; }

    public MoveRecords MoveRecords { get; } = new();

    /// <summary>
    /// The world layer publishes the local player's state here each frame; the Move packet reads it.
    /// A paused player reports (-1, -1), which the server interprets as "unchanged".
    /// </summary>
    public float PlayerX;

    public float PlayerY;
    public bool PlayerPaused;

    public event Action<string> Disconnected;
    public event Action<FailurePacket> Failed;
    public event Action<MapInfoPacket> MapLoaded;
    public event Action<CreateSuccessPacket> Entered;
    public event Action<UpdatePacket> WorldUpdated;
    public event Action<NewTickPacket> Ticked;
    public event Action<GotoPacket> Repositioned;
    public event Action<ReconnectPacket> ReconnectRequested;
    public event Action<QueuePingPacket> QueueUpdated;

    /// <summary>Everything not handled above, for the world and UI layers to dispatch on.</summary>
    public event Action<ServerPacket> PacketReceived;

    /// <summary>Raised after a Move goes out, so the world can refresh tile-derived movement state.</summary>
    public event Action MoveSent;

    /// <summary>Opens a connection and sends Hello for an existing character.</summary>
    public Task ConnectToLoadAsync(string host, int port, string guid, string password, int gameId, int charId,
        bool isFromArena = false, int keyTime = -1, byte[] key = null)
    {
        _createCharacter = false;
        _charId = charId;
        _isFromArena = isFromArena;
        return ConnectAsync(host, port, guid, password, gameId, keyTime, key);
    }

    /// <summary>Opens a connection and sends Hello, creating a new character on arrival.</summary>
    public Task ConnectToCreateAsync(string host, int port, string guid, string password, int gameId, int charId,
        ushort classType, ushort skinType)
    {
        _createCharacter = true;
        _charId = charId;
        _createClassType = classType;
        _createSkinType = skinType;
        _isFromArena = false;
        return ConnectAsync(host, port, guid, password, gameId, -1, null);
    }

    private async Task ConnectAsync(string host, int port, string guid, string password, int gameId,
        int keyTime, byte[] key)
    {
        _gameId = gameId;
        _keyTime = keyTime;
        _key = key ?? Array.Empty<byte>();

        PlayerObjectId = -1;
        LastTickId = 0;
        MoveRecords.Reset();

        await _connection.ConnectAsync(host, port).ConfigureAwait(false);
        State = SessionState.Handshaking;

        _connection.Send(new HelloPacket
        {
            GameId = _gameId,
            Guid = guid,
            Password = password,
            Secret = string.Empty,
            KeyTime = _keyTime,
            Key = _key,
            MapJson = string.Empty,
        });
    }

    /// <summary>
    /// Drains everything the receive thread has decoded. Call once per frame, after
    /// <see cref="GameClock.BeginFrame"/> and after the world has published the player's position.
    /// </summary>
    public void Poll()
    {
        while (_connection.Poll(out var packet))
            Handle(packet);
    }

    /// <summary>Sends a packet. Silently ignored before the session is <see cref="SessionState.Ready"/>.</summary>
    public void Send(ClientPacket packet)
    {
        if (State != SessionState.Ready)
            return;
        _connection.Send(packet);
    }

    private void Handle(ServerPacket packet)
    {
        switch (packet)
        {
            case MapInfoPacket mapInfo:
                OnMapInfo(mapInfo);
                return;

            case CreateSuccessPacket createSuccess:
                State = SessionState.Ready;
                PlayerObjectId = createSuccess.ObjectId;
                _charId = createSuccess.CharId;
                MoveRecords.Clear(_clock.FrameMs);
                Entered?.Invoke(createSuccess);
                return;

            case UpdatePacket update:
                // Acknowledge before doing any work with the contents.
                _connection.Send(_updateAck);
                WorldUpdated?.Invoke(update);
                return;

            case NewTickPacket tick:
                OnNewTick(tick);
                return;

            case GotoPacket got0:
                _gotoAck.Time = _clock.FrameMs;
                _connection.Send(_gotoAck);
                Repositioned?.Invoke(got0);
                return;

            case PingPacket ping:
                _pong.Serial = ping.Serial;
                _pong.Time = _clock.FrameMs;
                _connection.Send(_pong);
                return;

            case QueuePingPacket queuePing:
                // The serial is checked here, unlike Pong — a mismatch does not refresh the timer.
                State = SessionState.Queued;
                _queuePong.Serial = queuePing.Serial;
                _queuePong.Time = _clock.FrameMs;
                _connection.Send(_queuePong);
                QueueUpdated?.Invoke(queuePing);
                return;

            case ReconnectPacket reconnect:
                ReconnectRequested?.Invoke(reconnect);
                return;

            case FailurePacket failure:
                Failed?.Invoke(failure);
                return;

            default:
                PacketReceived?.Invoke(packet);
                return;
        }
    }

    private void OnMapInfo(MapInfoPacket mapInfo)
    {
        // The seed changes with every map, and both sides must restart their sequence together.
        Random = new MinstdRandom(mapInfo.Seed);

        MapLoaded?.Invoke(mapInfo);

        // The server gives us 15 seconds from here to claim a character, and only accepts these
        // two packets while it is in the handshaked state.
        State = SessionState.Entering;
        if (_createCharacter)
        {
            _connection.Send(new CreatePacket
            {
                ClassType = _createClassType,
                SkinType = _createSkinType,
            });
        }
        else
        {
            _connection.Send(new LoadPacket
            {
                CharId = _charId,
                IsFromArena = _isFromArena,
            });
        }
    }

    private void OnNewTick(NewTickPacket tick)
    {
        LastTickId = tick.TickId;

        // Send the Move before applying the tick's contents: it is the reply the server is
        // counting, and it must carry this tick's id.
        _movePacket.ObjectId = PlayerObjectId;
        _movePacket.TickId = tick.TickId;
        _movePacket.Time = _clock.FrameMs;
        _movePacket.NewPosition = PlayerPaused
            ? new WorldPos(-1f, -1f)
            : new WorldPos(PlayerX, PlayerY);

        MoveRecords.TakeForMove(_movePacket.Time, _movePacket.Records);
        _connection.Send(_movePacket);
        MoveSent?.Invoke();

        Ticked?.Invoke(tick);
    }

    /// <summary>Records this frame's player position into the Move history. Call once per frame.</summary>
    public void RecordPosition()
    {
        if (State == SessionState.Ready)
            MoveRecords.AddRecord(_clock.FrameMs, PlayerX, PlayerY);
    }

    public void Close(string reason = "Closed by client.")
    {
        State = SessionState.Disconnected;
        PlayerObjectId = -1;
        MoveRecords.Reset();
        _connection.Close(reason);
    }

    public void Dispose()
    {
        _connection.Dispose();
    }
}
