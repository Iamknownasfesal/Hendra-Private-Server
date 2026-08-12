using System;
using Godot;
using Hendra.Account;

namespace Hendra.App;

/// <summary>
/// The client's entry point: loads content, then swaps between the login screen and a session.
/// </summary>
/// <remarks>
/// Content loading fails loudly here rather than later. A missing manifest otherwise surfaces as an
/// object with no sprite, halfway into a dungeon.
/// </remarks>
public partial class Boot : Control
{
    private Label _status;
    private LoginScreen _login;
    private UI.TitleScreen _title;
    private GameScene _game;
    private UI.DeathScreen _death;
    private CanvasLayer _deathLayer;
    private LaunchOptions _options;

    /// <summary>The app server the current session's world server belongs to.</summary>
    private string _appServerUrl;

    private double _elapsed;
    private bool _screenshotTaken;

    public override void _Ready()
    {
        SetAnchorsPreset(LayoutPreset.FullRect);

        // One theme for the window, so every control the port creates picks it up -- including the
        // ones it does not draw itself, like the dropdown and the text fields, which otherwise
        // arrive looking like an editor's.
        GetTree().Root.Theme = UI.Style.Build();

        // The interface is laid out against 1920 by 1080 and scaled from there, so it holds its
        // proportions from a laptop panel to an ultrawide instead of shrinking to a corner on one
        // and swallowing the screen on the other. Clamped, because past about half again the
        // chrome starts costing more play area than it is worth.
        UI.Style.ApplyScale(GetTree().Root);
        GetTree().Root.SizeChanged += () => UI.Style.ApplyScale(GetTree().Root);

        _status = new Label
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        };
        _status.SetAnchorsPreset(LayoutPreset.FullRect);
        AddChild(_status);

        _options = LaunchOptions.Parse();

        try
        {
            ServiceLocator.LoadContent();
        }
        catch (Exception ex)
        {
            _status.Text = $"Could not load game content.\n\n{ex.Message}";
            GD.PushError($"[boot] {ex.Message}");
            return;
        }

        _status.Visible = false;

        if (_options.CanAutoCreate)
        {
            GD.Print($"[boot] auto-creating a character of class {_options.CreateClassType}");
            CreateCharacter(_options.ToServer(), _options.Guid, _options.Password ?? string.Empty,
                _options.CharacterId >= 0 ? _options.CharacterId : 0, (ushort)_options.CreateClassType);
        }
        else if (_options.CanAutoConnect)
        {
            GD.Print($"[boot] auto-connecting to {_options.Host}:{_options.Port} as character {_options.CharacterId}");
            StartGame(_options.ToServer(), _options.Guid, _options.Password ?? string.Empty, _options.CharacterId);
        }
        else
        {
            ShowTitle();
        }
    }

    /// <summary>
    /// The title screen, which is where the original starts and where Play leads on from.
    /// </summary>
    private void ShowTitle()
    {
        CloseScreens();

        _title = new UI.TitleScreen();
        _title.PlayPressed += ShowLogin;

        // Play and Account both lead to the same page: it signs you in and then offers your
        // characters, which is what both of them are for.
        _title.AccountPressed += ShowLogin;
        _title.QuitPressed += () => GetTree().Quit();
        AddChild(_title);
    }

    /// <summary>Tears down whichever menu or session is on screen.</summary>
    private void CloseScreens()
    {
        foreach (Node screen in new Node[] { _title, _login, _deathLayer, _game })
            screen?.QueueFree();

        _title = null;
        _login = null;
        _deathLayer = null;
        _death = null;
        _game = null;
    }

    private void ShowLogin()
    {
        CloseScreens();

        _login = new LoginScreen();
        _login.BackPressed += ShowTitle;
        _login.PlayRequested += StartGame;
        _login.CreateRequested += CreateCharacter;
        AddChild(_login);
    }

    private void StartGame(ServerInfo server, string guid, string password, int characterId)
    {
        // One game at a time. Every route into here goes through a button, and a second one
        // arriving while a session is already up would leave the first running unreferenced --
        // still connected, still acking -- with the server dropping both for the double login.
        if (_game != null)
            return;

        if (_login != null)
        {
            _login.QueueFree();
            _login = null;
        }

        // Fired and forgotten: the session does not wait on it, and keys render as themselves
        // until it lands.
        _appServerUrl = $"http://{server.Address}:8888";
        _ = ServiceLocator.LoadLanguageAsync(_appServerUrl);
        ServiceLocator.Audio?.Configure(_appServerUrl);

        // Fired and forgotten, like the language table: the objects that use these are rare, and
        // waiting on a download before showing the world would be a poor trade.
        _ = Assets.RemoteTextures.LoadAsync(_appServerUrl, ServiceLocator.Assets, ServiceLocator.Data);

        _game = new GameScene { Autofire = _options?.Autofire ?? false, AutoAbility = _options?.AutoAbility ?? false, AutoWalk = _options?.AutoWalk ?? false,
            StartingCameraAngle = _options?.CameraAngleDegrees * Mathf.Pi / 180f, ScriptedLines = new System.Collections.Generic.Queue<string>(_options?.Say ?? new System.Collections.Generic.List<string>()) };
        _game.Ended += OnSessionEnded;
        _game.Died += OnCharacterDied;

        // AddChild runs the scene's _Ready synchronously, so the world exists by the time this
        // returns and Play can be called straight away.
        AddChild(_game);
        _game.Play(server, guid, password, characterId);
    }

    private void CreateCharacter(ServerInfo server, string guid, string password, int characterId, ushort classType)
    {
        // One game at a time. Every route into here goes through a button, and a second one
        // arriving while a session is already up would leave the first running unreferenced --
        // still connected, still acking -- with the server dropping both for the double login.
        if (_game != null)
            return;

        if (_login != null)
        {
            _login.QueueFree();
            _login = null;
        }

        _appServerUrl = $"http://{server.Address}:8888";
        _ = ServiceLocator.LoadLanguageAsync(_appServerUrl);
        ServiceLocator.Audio?.Configure(_appServerUrl);

        // Fired and forgotten, like the language table: the objects that use these are rare, and
        // waiting on a download before showing the world would be a poor trade.
        _ = Assets.RemoteTextures.LoadAsync(_appServerUrl, ServiceLocator.Assets, ServiceLocator.Data);

        _game = new GameScene { Autofire = _options?.Autofire ?? false, AutoAbility = _options?.AutoAbility ?? false, AutoWalk = _options?.AutoWalk ?? false,
            StartingCameraAngle = _options?.CameraAngleDegrees * Mathf.Pi / 180f, ScriptedLines = new System.Collections.Generic.Queue<string>(_options?.Say ?? new System.Collections.Generic.List<string>()) };
        _game.Ended += OnSessionEnded;
        _game.Died += OnCharacterDied;
        AddChild(_game);

        // Skin zero is the class's default appearance.
        _game.CreateAndPlay(server, guid, password, characterId, classType, skinType: 0);
    }

    /// <summary>
    /// Puts the death screen over the game scene, leaving the world visible behind it.
    /// </summary>
    /// <remarks>
    /// The scene is deliberately left in the tree until the player dismisses this. Its last frame is
    /// the one they died on, and tearing it down to show a summary of what just happened would take
    /// away the picture of it happening.
    /// </remarks>
    private void OnCharacterDied(Net.Packets.DeathPacket death, int level)
    {
        if (_death != null)
            return;

        // On its own layer, above the game scene's interface. A Control parented straight to this
        // node would be drawn underneath the HUD, which is on a CanvasLayer of its own.
        _deathLayer = new CanvasLayer { Layer = 10 };
        AddChild(_deathLayer);

        _death = new UI.DeathScreen();
        _death.Dismissed += ReturnToLogin;
        _deathLayer.AddChild(_death);
        _death.Show(death, _appServerUrl, ServiceLocator.Data, level);
    }

    private void OnSessionEnded(string reason)
    {
        // The server drops the connection immediately after a death, so this arrives while the
        // summary is still on screen. Letting it through would replace the summary with the login
        // screen before it could be read.
        if (_death != null)
            return;

        ShowLogin();
        _status.Visible = !string.IsNullOrEmpty(reason);
        _status.Text = reason;
    }

    /// <summary>Dismisses the death screen and goes back to the character list.</summary>
    private void ReturnToLogin()
    {
        _death = null;
        ShowLogin();
        _status.Visible = false;
    }

    public override void _Process(double delta)
    {
        if (_options?.ScreenshotPath == null || _screenshotTaken)
            return;

        _elapsed += delta;
        if (_elapsed < _options.ScreenshotDelaySeconds)
            return;

        _screenshotTaken = true;
        CaptureScreenshot(_options.ScreenshotPath);

        if (_options.QuitAfterScreenshot)
            GetTree().Quit();
    }

    /// <summary>
    /// Writes what is currently on screen to a file, so the renderer can be inspected without
    /// anyone watching it.
    /// </summary>
    private void CaptureScreenshot(string path)
    {
        var image = GetViewport().GetTexture().GetImage();
        var error = image.SavePng(path);

        if (error != Error.Ok)
            GD.PushError($"[boot] could not write {path}: {error}");
        else
            GD.Print($"[boot] screenshot written to {path}");
    }
}
