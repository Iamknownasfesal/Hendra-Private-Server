using System;
using System.Collections.Generic;
using Godot;
using Hendra.Account;

namespace Hendra.App;

/// <summary>
/// Sign in, pick a character, and enter the world.
/// </summary>
/// <remarks>
/// <para>
/// The original spread this across a title screen, an account screen, a server screen and a
/// character-selection screen, wired together by a dozen signals and commands. They are one screen
/// here because the flow is short and the data all arrives in a single request: despite its name,
/// <c>/char/list</c> returns the characters, the account *and* the server list.
/// </para>
/// <para>
/// The host is editable rather than hardcoded. The original compiled the address in and shipped
/// seven near-identical configuration classes to vary it.
/// </para>
/// </remarks>
public partial class LoginScreen : Control
{
    private LineEdit _host;
    private LineEdit _guid;
    private LineEdit _password;
    private Button _signIn;
    private Label _status;
    private VBoxContainer _characters;
    private OptionButton _servers;

    private CharListResult _charList;

    /// <summary>Raised with everything needed to load an existing character.</summary>
    public event Action<ServerInfo, string, string, int> PlayRequested;

    /// <summary>Raised with everything needed to create one: server, credentials, id, class.</summary>
    public event Action<ServerInfo, string, string, int, ushort> CreateRequested;

    public override void _Ready()
    {
        SetAnchorsPreset(LayoutPreset.FullRect);

        // Without this the node has no size, so the centre of it is the top-left corner and every
        // centred child lands in the corner with it.
        UI.ScreenFit.FillScreen(this);

        // A dark ground behind everything, so the screen reads as a screen rather than as a form
        // floating on the engine's default grey.
        var backdrop = new ColorRect { Color = new Color(0.06f, 0.055f, 0.055f) };
        backdrop.SetAnchorsPreset(LayoutPreset.FullRect);
        backdrop.MouseFilter = MouseFilterEnum.Ignore;
        AddChild(backdrop);

        var centre = new CenterContainer();
        centre.SetAnchorsPreset(LayoutPreset.FullRect);
        AddChild(centre);

        var stack = new VBoxContainer();
        stack.AddThemeConstantOverride("separation", 14);
        centre.AddChild(stack);

        var title = new Label
        {
            Text = "Hendra",
            HorizontalAlignment = HorizontalAlignment.Center,
        };
        title.AddThemeFontSizeOverride("font_size", 44);
        title.AddThemeColorOverride("font_color", new Color(0.93f, 0.85f, 0.55f));
        stack.AddChild(title);

        // The sign-in form and the character list share the screen's centre, one replacing the
        // other, the way the original moves from its account screen to its character screen.
        _signInPanel = NewPanel();
        stack.AddChild(_signInPanel);

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 10);
        _signInPanel.AddChild(column);

        _host = AddField(column, "Server", "127.0.0.1:8888");
        _guid = AddField(column, "Account", string.Empty);
        _password = AddField(column, "Password", string.Empty);
        _password.Secret = true;

        _signIn = new Button { Text = "Sign in", CustomMinimumSize = new Vector2(0, 34) };
        _signIn.Pressed += OnSignInPressed;
        column.AddChild(_signIn);

        _status = new Label
        {
            AutowrapMode = TextServer.AutowrapMode.WordSmart,
            HorizontalAlignment = HorizontalAlignment.Center,
            CustomMinimumSize = new Vector2(CharacterBoxWidth, 0),
        };
        _status.AddThemeColorOverride("font_color", new Color(0.9f, 0.7f, 0.5f));
        stack.AddChild(_status);

        _charactersPanel = NewPanel();
        _charactersPanel.Visible = false;
        stack.AddChild(_charactersPanel);

        var characterColumn = new VBoxContainer();
        characterColumn.AddThemeConstantOverride("separation", 8);
        _charactersPanel.AddChild(characterColumn);

        _servers = new OptionButton { Visible = false };
        characterColumn.AddChild(_servers);

        _characters = new VBoxContainer();
        _characters.AddThemeConstantOverride("separation", 6);
        characterColumn.AddChild(_characters);

        // Enter submits from any field, which is how anyone actually uses a login form.
        foreach (var field in new[] { _host, _guid, _password })
            field.TextSubmitted += _ => OnSignInPressed();

        _guid.CallDeferred(Control.MethodName.GrabFocus);
    }

    /// <summary>
    /// One character on the list: class and level over its vital and stat lines.
    /// </summary>
    /// <remarks>
    /// A box rather than a line of text on a button, which is what the original shows — the whole
    /// point of the screen is comparing characters at a glance, and a sentence per character makes
    /// that a reading exercise.
    /// </remarks>
    private static Control NewCharacterBox(string className, Account.CharacterInfo character, Action play)
    {
        var box = new Button
        {
            CustomMinimumSize = new Vector2(CharacterBoxWidth, 62),
            TooltipText = "Play this character",
        };

        box.Pressed += play;

        var margin = new MarginContainer { MouseFilter = MouseFilterEnum.Ignore };
        margin.SetAnchorsPreset(LayoutPreset.FullRect);
        foreach (string side in new[] { "margin_left", "margin_top", "margin_right", "margin_bottom" })
            margin.AddThemeConstantOverride(side, 8);
        box.AddChild(margin);

        var rows = new VBoxContainer { MouseFilter = MouseFilterEnum.Ignore };
        rows.AddThemeConstantOverride("separation", 2);
        margin.AddChild(rows);

        var heading = new Label
        {
            Text = $"{className}   Level {character.Level}",
            MouseFilter = MouseFilterEnum.Ignore,
        };
        heading.AddThemeFontSizeOverride("font_size", 17);
        rows.AddChild(heading);

        var vitals = new Label
        {
            Text = $"{character.HitPoints}/{character.MaxHitPoints} HP    " +
                   $"{character.MagicPoints}/{character.MaxMagicPoints} MP    " +
                   $"{character.CurrentFame:N0} fame",
            MouseFilter = MouseFilterEnum.Ignore,
        };
        vitals.AddThemeColorOverride("font_color", new Color(0.72f, 0.72f, 0.72f));
        rows.AddChild(vitals);

        var stats = new Label
        {
            Text = $"ATT {character.Attack}  DEF {character.Defense}  SPD {character.Speed}  " +
                   $"DEX {character.Dexterity}  VIT {character.Vitality}  WIS {character.Wisdom}",
            MouseFilter = MouseFilterEnum.Ignore,
        };
        stats.AddThemeColorOverride("font_color", new Color(0.62f, 0.62f, 0.62f));
        rows.AddChild(stats);

        return box;
    }

    /// <summary>Width of a character entry, wide enough for a class name and its stats.</summary>
    private const int CharacterBoxWidth = 420;

    private UI.CutEdgePanel _signInPanel;
    private UI.CutEdgePanel _charactersPanel;

    /// <summary>A panel in the game's own shape, so the menu belongs to the same game as the HUD.</summary>
    private static UI.CutEdgePanel NewPanel()
    {
        var panel = new UI.CutEdgePanel
        {
            Background = UI.CutEdgePanel.PanelBackground,
            Border = new Color(0.42f, 0.42f, 0.42f),
        };

        panel.Padded(18);
        return panel;
    }

    private static LineEdit AddField(Control parent, string label, string initial)
    {
        var row = new HBoxContainer();
        row.AddThemeConstantOverride("separation", 8);
        parent.AddChild(row);

        row.AddChild(new Label { Text = label, CustomMinimumSize = new Vector2(90, 0) });

        var edit = new LineEdit
        {
            Text = initial,
            CustomMinimumSize = new Vector2(280, 30),
            SizeFlagsHorizontal = SizeFlags.ExpandFill,
        };
        row.AddChild(edit);
        return edit;
    }

    private async void OnSignInPressed()
    {
        _signIn.Disabled = true;
        _status.Text = "Signing in...";
        ClearCharacters();

        string baseUrl = NormalizeHost(_host.Text);

        try
        {
            using var client = new AppEngineClient(baseUrl);
            string xml = await client.PostAsync("/char/list", new Dictionary<string, string>
            {
                ["guid"] = _guid.Text,
                ["password"] = _password.Text,
            });

            _charList = CharListResult.Parse(xml);
            ShowCharacters();
        }
        catch (AppEngineException ex)
        {
            _status.Text = ex.IsFatal ? $"Sign-in failed: {ex.Message}" : $"{ex.Message}";
        }
        catch (Exception ex)
        {
            _status.Text = $"Could not reach {baseUrl}: {ex.Message}";
        }
        finally
        {
            _signIn.Disabled = false;
        }
    }

    /// <summary>
    /// Accepts a bare host, a host and port, or a full URL, and produces something addressable.
    /// </summary>
    private static string NormalizeHost(string text)
    {
        string value = (text ?? string.Empty).Trim();
        if (value.Length == 0)
            return "http://127.0.0.1:8888";

        if (value.StartsWith("http://", StringComparison.OrdinalIgnoreCase) ||
            value.StartsWith("https://", StringComparison.OrdinalIgnoreCase))
            return value;

        // The app server's default port, which differs from the world server's.
        return value.Contains(':') ? $"http://{value}" : $"http://{value}:8888";
    }

    private void ShowCharacters()
    {
        _servers.Clear();
        foreach (var server in _charList.Servers)
        {
            string load = server.IsFull ? "full" : server.IsCrowded ? "busy" : "open";
            _servers.AddItem($"{server.Name} — {load}");
        }

        _signInPanel.Visible = false;
        _charactersPanel.Visible = true;
        _servers.Visible = _charList.Servers.Count > 0;
        if (_servers.Visible)
        {
            var chosen = _charList.ChooseServer();
            int index = _charList.Servers.IndexOf(chosen);
            _servers.Selected = Math.Max(index, 0);
        }

        var account = _charList.Account;
        _status.Text = _charList.Servers.Count == 0
            ? "Signed in, but the server list is empty — is the world server running and registered?"
            : $"Signed in as {(string.IsNullOrEmpty(account.Name) ? "guest" : account.Name)}.";

        var living = _charList.Characters.FindAll(c => !c.Dead);

        foreach (var character in living)
        {
            var desc = ServiceLocator.Data?.GetObject(character.ObjectType);
            string name = desc?.DisplayId ?? desc?.Id ?? $"Type {character.ObjectType}";

            int characterId = character.CharacterId;
            _characters.AddChild(NewCharacterBox(name, character, () => RequestPlay(characterId)));
        }

        if (living.Count < Math.Max(_charList.MaxCharacters, 1))
            AddClassPicker(living.Count == 0);
    }

    /// <summary>
    /// Offers the classes this account may create.
    /// </summary>
    /// <remarks>
    /// Every known class is offered rather than trying to predict which are unlocked: the server
    /// decides, and answers a refusal with a message saying why.
    /// </remarks>
    private void AddClassPicker(bool onlyOption)
    {
        var classes = ServiceLocator.Data?.PlayerClasses;
        if (classes == null || classes.Count == 0)
            return;

        _characters.AddChild(new Label
        {
            Text = onlyOption ? "No characters yet. Create one:" : "Or create a new character:",
        });

        var grid = new GridContainer { Columns = 3 };
        _characters.AddChild(grid);

        foreach (var playerClass in classes)
        {
            var button = new Button { Text = playerClass.DisplayId ?? playerClass.Id ?? "?" };
            ushort classType = playerClass.Type;
            button.Pressed += () => RequestCreate(classType);
            grid.AddChild(button);
        }
    }

    private void RequestCreate(ushort classType)
    {
        var server = SelectedServer();
        if (server == null)
            return;

        // The id the server told us to use next; it allocates the real one on success.
        CreateRequested?.Invoke(server, _guid.Text, _password.Text, _charList.NextCharacterId, classType);
    }

    private void RequestPlay(int characterId)
    {
        var server = SelectedServer();
        if (server == null)
            return;

        PlayRequested?.Invoke(server, _guid.Text, _password.Text, characterId);
    }

    private ServerInfo SelectedServer()
    {
        if (_charList == null || _charList.Servers.Count == 0)
        {
            _status.Text = "No server to connect to.";
            return null;
        }

        int index = Mathf.Clamp(_servers.Selected, 0, _charList.Servers.Count - 1);
        return _charList.Servers[index];
    }

    private void ClearCharacters()
    {
        foreach (var child in _characters.GetChildren())
            child.QueueFree();
    }
}
