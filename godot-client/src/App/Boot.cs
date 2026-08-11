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
    private GameScene _game;
    private LaunchOptions _options;

    private double _elapsed;
    private bool _screenshotTaken;

    public override void _Ready()
    {
        SetAnchorsPreset(LayoutPreset.FullRect);

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

        if (_options.CanAutoConnect)
        {
            GD.Print($"[boot] auto-connecting to {_options.Host}:{_options.Port} as character {_options.CharacterId}");
            StartGame(_options.ToServer(), _options.Guid, _options.Password ?? string.Empty, _options.CharacterId);
        }
        else
        {
            ShowLogin();
        }
    }

    private void ShowLogin()
    {
        if (_game != null)
        {
            _game.QueueFree();
            _game = null;
        }

        _login = new LoginScreen();
        _login.PlayRequested += StartGame;
        AddChild(_login);
    }

    private void StartGame(ServerInfo server, string guid, string password, int characterId)
    {
        if (_login != null)
        {
            _login.QueueFree();
            _login = null;
        }

        _game = new GameScene { Autofire = _options?.Autofire ?? false };
        _game.Ended += OnSessionEnded;

        // AddChild runs the scene's _Ready synchronously, so the world exists by the time this
        // returns and Play can be called straight away.
        AddChild(_game);
        _game.Play(server, guid, password, characterId);
    }

    private void OnSessionEnded(string reason)
    {
        ShowLogin();
        _status.Visible = true;
        _status.Text = reason;
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
