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
    private WorldController _controller;
    private GameSession _session;

    private string _host;
    private int _port;
    private string _guid;
    private string _password;
    private int _characterId;

    /// <summary>Starts with fire held down. Set from the command line for unattended runs.</summary>
    public bool Autofire { get; set; }

    /// <summary>Raised when the session ends, with a reason to show the player.</summary>
    public event Action<string> Ended;

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
        _session.Disconnected += OnDisconnected;
        _session.Failed += OnFailed;
        _session.ReconnectRequested += OnReconnectRequested;

        _controller.Begin(_session, ServiceLocator.Data, ServiceLocator.Assets, _world, ServiceLocator.Clock, _overlay, _hud, _chat, _minimap);
        _controller.AutofireOnStart = Autofire;
        _controller.NexusRequested += () =>
            Reconnect(string.Empty, _port, GameIds.Nexus, 0, System.Array.Empty<byte>(), false);

        try
        {
            await connect(_session);
        }
        catch (Exception ex)
        {
            OnDisconnected($"Could not reach {_host}:{_port} — {ex.Message}");
        }
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

        _controller?.QueueFree();
        ServiceLocator.EndSession("Moving to another world.");

        _controller = new WorldController();
        AddChild(_controller);

        _session = ServiceLocator.BeginSession();
        _session.Disconnected += OnDisconnected;
        _session.Failed += OnFailed;
        _session.ReconnectRequested += OnReconnectRequested;
        _controller.Begin(_session, ServiceLocator.Data, ServiceLocator.Assets, _world, ServiceLocator.Clock, _overlay, _hud, _chat, _minimap);
        _controller.AutofireOnStart = Autofire;
        _controller.NexusRequested += () =>
            Reconnect(string.Empty, _port, GameIds.Nexus, 0, System.Array.Empty<byte>(), false);

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

    public override void _ExitTree() => ServiceLocator.EndSession("Left the game.");
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
