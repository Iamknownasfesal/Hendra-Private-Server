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
        ShowLogin();
    }

    private void ShowLogin()
    {
        if (_game != null)
        {
            _game.QueueFree();
            _game = null;
        }

        _login = new LoginScreen();
        _login.PlayRequested += OnPlayRequested;
        AddChild(_login);
    }

    private void OnPlayRequested(ServerInfo server, string guid, string password, int characterId)
    {
        _login.QueueFree();
        _login = null;

        _game = new GameScene();
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
}
