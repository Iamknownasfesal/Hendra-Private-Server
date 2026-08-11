using System;
using Godot;
using Hendra.Account;
using Hendra.Net;
using Hendra.Net.Packets;
using Hendra.Render;
using Hendra.UI;
using Hendra.World;

namespace Hendra.App;

/// <summary>
/// A session in progress: the 3D world, the controller driving it, and the connection behind both.
/// </summary>
/// <remarks>
/// Also handles being told to move: a Reconnect means tearing this down and standing it back up
/// against a different world, echoing the key the server issued. Entering a portal, dying and
/// returning to the Nexus all arrive that way.
/// </remarks>
public partial class GameScene : Node
{
    private WorldRoot _world;
    private WorldOverlay _overlay;
    private HudView _hud;
    private ChatView _chat;
    private MinimapView _minimap;
    private TradeView _trade;
    private WorldController _controller;
    private GameSession _session;

    private string _host;
    private int _port;
    private string _guid;
    private string _password;
    private int _characterId;

    /// <summary>Starts with fire held down. Set from the command line for unattended runs.</summary>
    public bool Autofire { get; set; }

    /// <summary>
    /// Lines to send once in the world, in order. Set from the command line.
    /// </summary>
    /// <remarks>
    /// One queue for the session, handed to each controller in turn, so a script survives a change
    /// of world instead of restarting in the new one.
    /// </remarks>
    public System.Collections.Generic.Queue<string> ScriptedLines { get; set; }

    /// <summary>Raised when the session ends, with a reason to show the player.</summary>
    public event Action<string> Ended;

    /// <summary>Raised when our character dies. Carries the packet and the level it reached.</summary>
    public event Action<DeathPacket, int> Died;

    public override void _Ready()
    {
        _world = new WorldRoot();
        AddChild(_world);

        // The interface lives on a CanvasLayer for two reasons. A Control only sizes itself against
        // the viewport when its parent is a Viewport or a CanvasLayer -- hang one off a plain Node
        // and it silently stays zero-sized, laying everything out on top of itself. And a layer
        // above zero draws over the 3D world regardless of node order.
        var ui = new CanvasLayer { Layer = 1 };
        AddChild(ui);

        _overlay = new WorldOverlay();
        ui.AddChild(_overlay);

        _hud = new HudView();
        _hud.Configure(ServiceLocator.Assets, ServiceLocator.Data);
        ui.AddChild(_hud);

        _chat = new ChatView();
        ui.AddChild(_chat);

        _minimap = new MinimapView();
        ui.AddChild(_minimap);

        _trade = new TradeView();
        ui.AddChild(_trade);

        _controller = new WorldController();
        AddChild(_controller);
    }

    /// <summary>Connects and loads an existing character.</summary>
    public void Play(ServerInfo server, string guid, string password, int characterId)
    {
        _host = server.Address;
        _port = server.Port;
        _guid = guid;
        _password = password;
        _characterId = characterId;

        StartSession(session => session.ConnectToLoadAsync(
            _host, _port, _guid, _password, GameIds.Nexus, _characterId));
    }

    /// <summary>Connects and creates a new character of the given class.</summary>
    public void CreateAndPlay(
        ServerInfo server, string guid, string password, int characterId, ushort classType, ushort skinType)
    {
        _host = server.Address;
        _port = server.Port;
        _guid = guid;
        _password = password;
        _characterId = characterId;

        StartSession(session => session.ConnectToCreateAsync(
            _host, _port, _guid, _password, GameIds.Nexus, _characterId, classType, skinType));
    }

    private async void StartSession(Func<GameSession, System.Threading.Tasks.Task> connect)
    {
        _session = ServiceLocator.BeginSession();
        Subscribe(_session);

        _controller.Begin(_session, ServiceLocator.Data, ServiceLocator.Assets, _world, ServiceLocator.Clock, _overlay, _hud, _chat, _minimap);
        _controller.AutofireOnStart = Autofire;
        _controller.ScriptedLines = ScriptedLines;
        _trade.Configure(_controller.Trading, ServiceLocator.Assets, ServiceLocator.Data);
        _controller.Trading.Requested += who =>
            _controller.Chat?.AddSystem($"{who} wants to trade. Type /trade {who} to accept.");
        _controller.Trading.Ended += message => _controller.Chat?.AddSystem(message);
        _controller.NexusRequested += () =>
            Reconnect(string.Empty, _port, GameIds.Nexus, 0, System.Array.Empty<byte>(), false);
        _controller.Died += OnCharacterDied;

        try
        {
            await connect(_session);
        }
        catch (Exception ex)
        {
            OnDisconnected($"Could not reach {_host}:{_port} — {ex.Message}");
        }
    }

    private void Subscribe(GameSession session)
    {
        session.Disconnected += OnDisconnected;
        session.Failed += OnFailed;
        session.ReconnectRequested += OnReconnectRequested;
    }

    private void Unsubscribe(GameSession session)
    {
        if (session == null)
            return;

        session.Disconnected -= OnDisconnected;
        session.Failed -= OnFailed;
        session.ReconnectRequested -= OnReconnectRequested;
    }

    private void OnFailed(FailurePacket failure)
    {
        // Raised on the receive thread, so it is bounced onto the main one before touching the tree.
        CallDeferred(nameof(ReportFailure), failure.ErrorId, failure.ErrorDescription ?? string.Empty);
    }

    private void ReportFailure(int errorId, string description)
    {
        string message = errorId switch
        {
            FailureCode.IncorrectVersion =>
                $"The server rejected this client's version. It expects {ProtocolKeys.BuildVersion}.",
            FailureCode.BadKey => "That reconnect key was not accepted.",
            FailureCode.EmailVerificationNeeded => "This account needs its email verified first.",
            _ => string.IsNullOrEmpty(description) ? $"The server refused the connection (code {errorId})." : description,
        };

        GD.PushWarning($"[game] {message}");
        Ended?.Invoke(message);
    }

    /// <summary>
    /// Our character died.
    /// </summary>
    /// <remarks>
    /// Permanent: the character is gone and the session is over. A zombie death is different — the
    /// server hands back a new object to keep playing as — but that path is not implemented, so it
    /// is reported the same way rather than silently doing nothing.
    /// </remarks>
    private void OnCharacterDied(DeathPacket death) => Died?.Invoke(death, _controller.PlayerLevel);

    private void OnDisconnected(string reason) =>
        CallDeferred(nameof(ReportDisconnect), reason ?? "Connection lost.");

    private void ReportDisconnect(string reason)
    {
        GD.Print($"[game] disconnected: {reason}");
        Ended?.Invoke(reason);
    }

    /// <summary>
    /// Moves to another world at the server's request.
    /// </summary>
    /// <remarks>
    /// The key and game id have to be echoed back verbatim in the next Hello — the server checks
    /// both and disconnects on a mismatch. An empty host means the same machine.
    /// </remarks>
    private void OnReconnectRequested(ReconnectPacket packet) => CallDeferred(nameof(Reconnect),
        packet.Host ?? string.Empty, packet.Port, packet.GameId, packet.KeyTime, packet.Key, packet.IsFromArena);

    private async void Reconnect(string host, int port, int gameId, int keyTime, byte[] key, bool fromArena)
    {
        if (!string.IsNullOrEmpty(host))
            _host = host;
        if (port > 0)
            _port = port;

        // Detach before tearing the old session down. Closing it raises Disconnected, and if that
        // still reached us it would be reported as the session ending and bounce the player back to
        // the login screen -- so every portal, every death and every trip to the Nexus would look
        // like a dropped connection.
        Unsubscribe(_session);

        _controller?.QueueFree();
        ServiceLocator.EndSession("Moving to another world.");

        _controller = new WorldController();
        AddChild(_controller);

        _session = ServiceLocator.BeginSession();
        Subscribe(_session);
        _controller.Begin(_session, ServiceLocator.Data, ServiceLocator.Assets, _world, ServiceLocator.Clock, _overlay, _hud, _chat, _minimap);
        _controller.AutofireOnStart = Autofire;
        _controller.ScriptedLines = ScriptedLines;
        _trade.Configure(_controller.Trading, ServiceLocator.Assets, ServiceLocator.Data);
        _controller.Trading.Requested += who =>
            _controller.Chat?.AddSystem($"{who} wants to trade. Type /trade {who} to accept.");
        _controller.Trading.Ended += message => _controller.Chat?.AddSystem(message);
        _controller.NexusRequested += () =>
            Reconnect(string.Empty, _port, GameIds.Nexus, 0, System.Array.Empty<byte>(), false);
        _controller.Died += OnCharacterDied;

        try
        {
            await _session.ConnectToLoadAsync(
                _host, _port, _guid, _password, gameId, _characterId, fromArena, keyTime, key);
        }
        catch (Exception ex)
        {
            ReportDisconnect($"Could not reconnect to {_host}:{_port} — {ex.Message}");
        }
    }

    public override void _ExitTree()
    {
        Unsubscribe(_session);
        ServiceLocator.EndSession("Left the game.");
    }
}

/// <summary>
/// The special world identifiers Hello understands. Anything else is a specific world instance.
/// </summary>
public static class GameIds
{
    public const int Tutorial = -1;
    public const int Nexus = -2;
    public const int RandomRealm = -3;
    public const int MapTest = -6;
}
