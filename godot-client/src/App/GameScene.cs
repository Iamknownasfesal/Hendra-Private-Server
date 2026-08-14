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
    private OptionsView _options;
    private DebugOverlay _debug;
    private GuildView _guild;
    private CharacterPanel _character;
    private AccountPanel _account;
    private SystemMenu _menu;

    /// <summary>The world's own furniture: health bars and markers, in the world's coordinates.</summary>
    private CanvasLayer _ui;

    /// <summary>The HUD, on its own scaled canvas so it can be written in reference pixels.</summary>
    private HudLayer _hudLayer;

    private CanvasLayer _loadingLayer;
    private MapLoadingView _loading;
    private WorldController _controller;
    private GameSession _session;

    private string _host;
    private int _port;
    private string _guid;
    private string _password;
    private int _characterId;

    /// <summary>Starts with fire held down. Set from the command line for unattended runs.</summary>
    public bool Autofire { get; set; }

    /// <summary>Uses the ability on a loop. Set from the command line for unattended runs.</summary>
    public bool AutoAbility { get; set; }

    /// <summary>Walks in a circle. Set from the command line for unattended runs.</summary>
    public bool AutoWalk { get; set; }

    /// <summary>Opens the character sheet once in the world. Set from the command line.</summary>
    public bool OpenCharacterPanel { get; set; }

    /// <summary>Opens the account panel once in the world. Set from the command line.</summary>
    public bool OpenAccountPanel { get; set; }

    /// <summary>
    /// Holds the vault panel open without standing on the chest. Set from the command line.
    /// </summary>
    /// <remarks>
    /// For screenshots. The panel is otherwise only reachable by walking a character onto the
    /// access object, which an unattended run has no way to do.
    /// </remarks>
    public bool OpenVault { get; set; }

    /// <summary>Opens the options page once in the world. Set from the command line.</summary>
    public bool OpenOptions { get; set; }

    /// <summary>Which options tab to open on, or null for the first. Set from the command line.</summary>
    public string OptionsTab { get; set; }

    /// <summary>Opens the Escape menu once in the world. Set from the command line.</summary>
    public bool OpenMenu { get; set; }

    /// <summary>Overrides the starting camera heading, in radians. Set from the command line.</summary>
    public float? StartingCameraAngle { get; set; }

    /// <summary>The default camera angle as it stood last time the options were applied.</summary>
    private int _cameraAngleWas;

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
        _ui = new CanvasLayer { Layer = 1 };
        AddChild(_ui);

        _overlay = new WorldOverlay();
        _ui.AddChild(_overlay);

        // The three clusters of the HUD share a canvas of their own. Its scale is what lets them be
        // laid out in the reference resolution's pixels while the screens either side of the game
        // keep the project's own base; see UI.HudLayer.
        _hudLayer = new HudLayer { Layer = 2 };
        AddChild(_hudLayer);

        // Above everything, because it covers the whole screen including the HUD while a world is
        // being entered.
        _loadingLayer = new CanvasLayer { Layer = 5 };
        AddChild(_loadingLayer);

        _loading = new MapLoadingView { Visible = false };
        _loadingLayer.AddChild(_loading);

        _hud = new HudView();
        _hud.Configure(ServiceLocator.Assets, ServiceLocator.Data);
        _hudLayer.AddChild(_hud);

        _chat = new ChatView();
        _hudLayer.AddChild(_chat);

        _minimap = new MinimapView();
        _hudLayer.AddChild(_minimap);

        // Last onto the HUD canvas, so it draws over the clusters it docks beside while still being
        // laid out in the same reference pixels they are.
        _character = new CharacterPanel();
        _character.Configure(ServiceLocator.Data, ServiceLocator.Assets);
        _hudLayer.AddChild(_character);

        _account = new AccountPanel();
        _hudLayer.AddChild(_account);

        // These three used to sit on a canvas of their own, one layer above the HUD. That put them
        // outside the only scaled canvas in the client, and Style.Sharpness -- which is global and
        // set from the HUD's scale -- then had them rasterising their text at the HUD's factor and
        // drawing it under the inverse of a transform their canvas was not applying. On any window
        // where that factor is not one, every word on them came out resampled. They are ordinary
        // HUD children now, added after the panels they open over, which is all the layering they
        // ever needed.
        _trade = new TradeView();
        _hudLayer.AddChild(_trade);

        _guild = new GuildView();
        _hudLayer.AddChild(_guild);

        // On the HUD's own layer, so it is laid out in the same space as the player card it sits
        // under and is scaled with the rest of the interface rather than beside it.
        _debug = new DebugOverlay();
        _hudLayer.AddChild(_debug);

        _options = new OptionsView();
        _options.Configure(ServiceLocator.Settings);
        _options.Changed += OnOptionsChanged;
        _hudLayer.AddChild(_options);

        // Last, so it draws over everything else on the canvas: it is the way out of all of them.
        _menu = new SystemMenu();
        _menu.OptionsRequested += () => _options.Toggle();
        _menu.NexusRequested += () =>
            Reconnect(string.Empty, _port, GameIds.Nexus, 0, System.Array.Empty<byte>(), false);
        _menu.CharactersRequested += () => Ended?.Invoke(string.Empty);
        _menu.QuitRequested += () => GetTree().Quit();
        _hudLayer.AddChild(_menu);

        _hudLayer.Refit();

        // Seeded, so the first change the player makes is seen as a change rather than as the
        // baseline and swallowed.
        _cameraAngleWas = ServiceLocator.Settings?.DefaultCameraAngle ?? 0;

        // The card's buttons go straight to the panels they open, wired once, here.
        //
        // They used to travel out to the world controller and back as events, and the controller is
        // rebuilt every time you change world while the interface is not -- so the wiring aged: old
        // controllers kept answering, new ones added themselves beside them, and a single click
        // arrived at the panel more than once. Guarding the dead ones made the count right and did
        // not make the route sensible. This object owns the HUD and it owns the panels; there is
        // nothing in between for a world change to invalidate.
        //
        // The keys that do the same thing still come through the controller, because that is where
        // input is read. One handler each, on a controller that is new every time.
        WireCardButtons();

        _controller = new WorldController();
        AddChild(_controller);
    }

    /// <summary>Opens each panel from its own button, for as long as this scene exists.</summary>
    private void WireCardButtons()
    {
        _hud.StatsPressed += ShowCharacter;
        _hud.AccountPressed += ShowAccount;
        _hud.OptionsPressed += () => _options.Toggle();
    }

    /// <summary>The two share a slot on the screen, so opening one closes the other.</summary>
    private void ShowCharacter()
    {
        _account.Close();
        _character.Toggle();
    }

    private void ShowAccount()
    {
        _character.Close();
        _account.Toggle();
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

    /// <summary>
    /// Feeds the character sheet, which is the one panel that reads the player rather than a packet.
    /// </summary>
    /// <remarks>
    /// Pushed from here rather than pulled inside the panel, so the panel keeps the property every
    /// other piece of the interface has: it is handed what it draws and never reaches into the
    /// world for it.
    /// </remarks>
    public override void _Process(double delta) =>
        _character?.Refresh(_controller?.Map?.Player, delta);

    /// <summary>
    /// Shows or hides everything drawn over the world.
    /// </summary>
    /// <remarks>
    /// Two layers rather than one now that the HUD has a canvas of its own, which is what the key
    /// that hides the interface for a screenshot has to reach. The world keeps drawing underneath.
    /// </remarks>
    private void ShowInterface(bool shown)
    {
        _ui.Visible = shown;
        _hudLayer.Visible = shown;
    }

    /// <summary>
    /// Escape, once nothing else has claimed it.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Unhandled input reaches children before their parent, so every panel built on
    /// <see cref="ModalPanel"/> has already had its chance: if one was open it closed itself and
    /// marked the press handled, and this never runs. What is left is Escape pressed with nothing
    /// in the way, which is the request for the menu.
    /// </para>
    /// <para>
    /// Two exceptions have to be caught here because neither is a modal panel. The chat box reads
    /// Escape by polling from the world controller, so the key would otherwise both close the box
    /// and open the menu; and the options page draws its own chrome rather than using the shell, so
    /// it has no Escape of its own to consume.
    /// </para>
    /// </remarks>
    public override void _UnhandledKeyInput(InputEvent @event)
    {
        if (@event is not InputEventKey { Pressed: true, Echo: false, Keycode: Key.Escape })
            return;

        if (_chat is { IsTyping: true })
            return;

        GetViewport().SetInputAsHandled();

        if (_options is { IsOpen: true })
        {
            _options.Toggle();
            return;
        }

        if (_menu == null)
            return;

        _menu.CanReturnToNexus = _controller is { InNexus: false };
        _menu.Toggle();
    }

    /// <summary>Applies a changed setting immediately and writes it out.</summary>
    private void OnOptionsChanged()
    {
        ServiceLocator.ApplySettings();

        // Interface Scale is read by the canvas rather than pushed to it, and the canvas only reads
        // it when it is built or when the window changes shape. Without this the row moves, saves
        // and does nothing you can see until the next launch, which reads as a setting that does
        // not work.
        _hudLayer?.Refit();

        // Only when the angle itself moved. Every row on the page comes through here, and swinging
        // the camera because someone dragged the music slider would undo whatever they had turned
        // the view to.
        int angle = ServiceLocator.Settings?.DefaultCameraAngle ?? 0;
        if (angle != _cameraAngleWas)
        {
            _cameraAngleWas = angle;
            _controller?.ResetCamera();
        }

        if (_controller != null)
            _controller.CenterOnPlayer = ServiceLocator.Settings.CenterOnPlayer;

        if (_chat != null)
            _chat.Visible = !ServiceLocator.Settings.HideChat;
    }

    private async void StartSession(Func<GameSession, System.Threading.Tasks.Task> connect)
    {
        _session = ServiceLocator.BeginSession();
        Subscribe(_session);

        _controller.Begin(_session, ServiceLocator.Data, ServiceLocator.Assets, _world, ServiceLocator.Clock, _overlay, _hud, _chat, _minimap);
        _controller.AutofireOnStart = Autofire;
        _controller.AutoAbility = AutoAbility;
        _controller.AutoWalk = AutoWalk;
        _controller.StartingCameraAngle = StartingCameraAngle;
        _controller.CenterOnPlayer = ServiceLocator.Settings?.CenterOnPlayer ?? true;
        _controller.HoldVaultOpen = OpenVault;

        _controller.HudVisibilityChanged += hidden => ShowInterface(!hidden);
        _controller.WorldEntering += (name, difficulty) => _loading?.Show(name, difficulty);
        _controller.WorldEntered += () =>
        {
            _loading?.Finish();

            if (OpenCharacterPanel && !_character.IsOpen)
                _character.Toggle();

            if (OpenAccountPanel && !_account.IsOpen)
                _account.Toggle();

            if (OpenOptions && !_options.IsOpen)
            {
                _options.ShowTab(OptionsTab);
                _options.Toggle();
            }

            if (OpenMenu && !_menu.IsOpen)
            {
                _menu.CanReturnToNexus = !_controller.InNexus;
                _menu.Toggle();
            }
        };
        _controller.OptionsToggled += () => _options.Toggle();
        _controller.DebugToggled += () => _debug.Toggle();
        _debug.Session = () => _session;
        _debug.EntityCount = () => _controller.EntityCount;
        _debug.ProjectileCount = () => _controller.ProjectileCount;
        _debug.WorldName = () => _controller.CurrentWorldName;
        _debug.PlayerAt = () => _controller.PlayerAt;
        _debug.Phases = _controller.Phases;
        _debug.SpriteSurfaces = () => _controller.SpriteSurfaces;
        _controller.GuildToggled += () =>
        {
            // Read at the moment it opens: the name arrives as a stat after the panel is built.
            _guild.AccountName = _controller.Map?.Player?.Name;
            _guild.Toggle();
        };
        _controller.CharacterToggled += ShowCharacter;

        _account.Connect($"http://{_host}:8888", _guid, _password);
        _character.Connect($"http://{_host}:8888", _guid, _password, _characterId);

        // The character sheet is deliberately absent from this list. It opens beside the world
        // rather than over it, and the player keeps playing while it is up.
        _controller.OptionsAreOpen = () => _options.IsOpen || _guild.IsOpen || _menu.IsOpen;
        _guild.Configure(_session, $"http://{_host}:8888", _guid, _password);
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
        // The server's own words win whenever it sent any. Its failure codes are broader than they
        // look -- a refused login and a refused reconnect both arrive as BadKey -- so a canned
        // message per code told players their reconnect key was bad when they had simply typed the
        // wrong password.
        if (!string.IsNullOrWhiteSpace(description))
        {
            // The account is named because the server's "Bad Login" covers both a wrong password
            // and an account that does not exist, so the message alone never says whose fault it is.
            GD.PushWarning($"[game] {_guid}: {description}");
            Ended?.Invoke(description);
            return;
        }

        string message = errorId switch
        {
            FailureCode.IncorrectVersion =>
                $"The server rejected this client's version. It expects {ProtocolKeys.BuildVersion}.",
            FailureCode.BadKey => "Those credentials were not accepted.",
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
        _controller.AutoAbility = AutoAbility;
        _controller.AutoWalk = AutoWalk;
        _controller.StartingCameraAngle = StartingCameraAngle;
        _controller.CenterOnPlayer = ServiceLocator.Settings?.CenterOnPlayer ?? true;
        _controller.HoldVaultOpen = OpenVault;

        _controller.HudVisibilityChanged += hidden => ShowInterface(!hidden);
        _controller.WorldEntering += (name, difficulty) => _loading?.Show(name, difficulty);
        _controller.WorldEntered += () =>
        {
            _loading?.Finish();

            if (OpenCharacterPanel && !_character.IsOpen)
                _character.Toggle();

            if (OpenAccountPanel && !_account.IsOpen)
                _account.Toggle();

            if (OpenOptions && !_options.IsOpen)
            {
                _options.ShowTab(OptionsTab);
                _options.Toggle();
            }

            if (OpenMenu && !_menu.IsOpen)
            {
                _menu.CanReturnToNexus = !_controller.InNexus;
                _menu.Toggle();
            }
        };
        _controller.OptionsToggled += () => _options.Toggle();
        _controller.DebugToggled += () => _debug.Toggle();
        _debug.Session = () => _session;
        _debug.EntityCount = () => _controller.EntityCount;
        _debug.ProjectileCount = () => _controller.ProjectileCount;
        _debug.WorldName = () => _controller.CurrentWorldName;
        _debug.PlayerAt = () => _controller.PlayerAt;
        _debug.Phases = _controller.Phases;
        _debug.SpriteSurfaces = () => _controller.SpriteSurfaces;
        _controller.GuildToggled += () =>
        {
            // Read at the moment it opens: the name arrives as a stat after the panel is built.
            _guild.AccountName = _controller.Map?.Player?.Name;
            _guild.Toggle();
        };
        _controller.CharacterToggled += ShowCharacter;

        _account.Connect($"http://{_host}:8888", _guid, _password);
        _character.Connect($"http://{_host}:8888", _guid, _password, _characterId);

        // The character sheet is deliberately absent from this list. It opens beside the world
        // rather than over it, and the player keeps playing while it is up.
        _controller.OptionsAreOpen = () => _options.IsOpen || _guild.IsOpen || _menu.IsOpen;
        _guild.Configure(_session, $"http://{_host}:8888", _guid, _password);
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
