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

        // The title art again, filling the window and dimmed, so signing in and picking a
        // character read as the same place as the title rather than as a form on flat black.
        var art = new TextureRect
        {
            Texture = ServiceLocator.Assets?.GetImage("TitleScreen"),
            ExpandMode = TextureRect.ExpandModeEnum.IgnoreSize,
            StretchMode = TextureRect.StretchModeEnum.KeepAspectCovered,
            MouseFilter = MouseFilterEnum.Ignore,
        };
        art.SetAnchorsPreset(LayoutPreset.FullRect);
        AddChild(art);

        var wash = new ColorRect { Color = new Color(0.03f, 0.03f, 0.05f, 0.72f) };
        wash.SetAnchorsPreset(LayoutPreset.FullRect);
        wash.MouseFilter = MouseFilterEnum.Ignore;
        AddChild(wash);

        var centre = new CenterContainer();
        centre.SetAnchorsPreset(LayoutPreset.FullRect);
        AddChild(centre);

        var stack = new VBoxContainer();
        stack.AddThemeConstantOverride("separation", 14);
        centre.AddChild(stack);

        var heading = new Label
        {
            Text = "Sign in",
            HorizontalAlignment = HorizontalAlignment.Center,
        };
        heading.AddThemeFontSizeOverride("font_size", 26);
        heading.AddThemeColorOverride("font_color", new Color(0.93f, 0.9f, 0.84f));
        stack.AddChild(heading);
        _heading = heading;

        // The sign-in form and the character list share the screen's centre, one replacing the
        // other, the way the original moves from its account screen to its character screen.
        _signInPanel = NewPanel();
        stack.AddChild(_signInPanel);

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 10);
        _signInPanel.AddChild(column);

        _guid = AddField(column, "Account", string.Empty);
        _password = AddField(column, "Password", string.Empty);
        _password.Secret = true;

        _signIn = new Button { Text = "Sign in", CustomMinimumSize = new Vector2(0, 34) };
        _signIn.Pressed += OnSignInPressed;
        column.AddChild(_signIn);

        // Registering is the same two fields, so it is a second button rather than a third page.
        // The server creates the account and the client signs straight in with it.
        _register = new Button { Text = "Create an account with these details" };
        _register.Pressed += OnRegisterPressed;
        column.AddChild(_register);

        var back = new Button { Text = "Back" };
        back.Pressed += () => BackPressed?.Invoke();
        column.AddChild(back);

        _status = new Label
        {
            AutowrapMode = TextServer.AutowrapMode.WordSmart,
            HorizontalAlignment = HorizontalAlignment.Center,
            CustomMinimumSize = new Vector2(CharacterBoxWidth, 0),
        };
        _status.AddThemeColorOverride("font_color", new Color(0.9f, 0.7f, 0.5f));
        stack.AddChild(_status);

        // A fresh account has no name until it picks one, and the world server refuses to let it
        // in until it has. The original puts this on its character screen too.
        _namePanel = NewPanel();
        _namePanel.Visible = false;
        stack.AddChild(_namePanel);

        var nameColumn = new VBoxContainer();
        nameColumn.AddThemeConstantOverride("separation", 8);
        _namePanel.AddChild(nameColumn);

        nameColumn.AddChild(new Label
        {
            Text = "Choose the name other players will see.\nThis is separate from your account, and you only get one.",
            AutowrapMode = TextServer.AutowrapMode.WordSmart,
            CustomMinimumSize = new Vector2(CharacterBoxWidth, 0),
        });

        _name = AddField(nameColumn, "Name", string.Empty);

        _setName = new Button { Text = "Take this name", CustomMinimumSize = new Vector2(0, 32) };
        _setName.Pressed += OnSetNamePressed;
        nameColumn.AddChild(_setName);

        _name.TextSubmitted += _ => OnSetNamePressed();

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
        foreach (var field in new[] { _guid, _password })
            field.TextSubmitted += _ => OnSignInPressed();

        _guid.CallDeferred(Control.MethodName.GrabFocus);
    }

    /// <summary>
    /// The standing sprite for a class, at the size the boxes want it.
    /// </summary>
    /// <remarks>
    /// The original's character boxes are built from SWF graphics with the sprite composited into
    /// them. Those are compiled artwork rather than loose files and cannot be pulled out, so the
    /// box is drawn here instead and the sprite that goes in it is the real one — the same
    /// eight-pixel frame the game draws when the character is standing still.
    /// </remarks>
    private static Control Portrait(ushort objectType, int size)
    {
        var holder = new Control
        {
            CustomMinimumSize = new Vector2(size, size),
            MouseFilter = MouseFilterEnum.Ignore,
        };

        var desc = ServiceLocator.Data?.GetObject(objectType);
        if (desc?.Texture == null || ServiceLocator.Assets == null)
            return holder;

        var resolved = new Assets.TextureResolver(ServiceLocator.Assets).Resolve(desc.Texture);
        var sprite = resolved.Animated != null
            ? resolved.Animated.Frame(0f, 0f, Assets.CharAction.Stand, 0f).Sprite
            : resolved.Still;

        if (!sprite.IsValid)
            return holder;

        var view = new TextureRect
        {
            Texture = new AtlasTexture { Atlas = sprite.Sheet, Region = sprite.Region },
            StretchMode = TextureRect.StretchModeEnum.KeepAspectCentered,

            // Nearest, or an eight-pixel sprite blown up to forty is a smear.
            TextureFilter = CanvasItem.TextureFilterEnum.Nearest,
            MouseFilter = MouseFilterEnum.Ignore,
        };

        view.SetAnchorsPreset(LayoutPreset.FullRect);
        holder.AddChild(view);
        return holder;
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
            CustomMinimumSize = new Vector2(CharacterBoxWidth, 82),
            TooltipText = "Play this character",
        };

        box.Pressed += play;

        var margin = new MarginContainer { MouseFilter = MouseFilterEnum.Ignore };
        margin.SetAnchorsPreset(LayoutPreset.FullRect);
        foreach (string side in new[] { "margin_left", "margin_top", "margin_right", "margin_bottom" })
            margin.AddThemeConstantOverride(side, 8);
        box.AddChild(margin);

        var across = new HBoxContainer { MouseFilter = MouseFilterEnum.Ignore };
        across.AddThemeConstantOverride("separation", 10);
        margin.AddChild(across);
        across.AddChild(Portrait(character.ObjectType, 46));

        var rows = new VBoxContainer { MouseFilter = MouseFilterEnum.Ignore };
        rows.AddThemeConstantOverride("separation", 2);
        rows.SizeFlagsHorizontal = SizeFlags.ExpandFill;
        across.AddChild(rows);

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

    private Button _register;
    private UI.CutEdgePanel _namePanel;
    private LineEdit _name;
    private Button _setName;

    /// <summary>Raised when the player wants to go back to the title screen.</summary>
    public event Action BackPressed;

    private Label _heading;
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

    /// <summary>
    /// Claims a display name, then reloads the character list.
    /// </summary>
    /// <remarks>
    /// The server refuses duplicates and anything it considers invalid, and says which in the
    /// response, so its words are shown rather than a guess at what went wrong.
    /// </remarks>
    private async void OnSetNamePressed()
    {
        _setName.Disabled = true;
        _status.Text = "Claiming the name...";

        try
        {
            using var client = new AppEngineClient(ServerConfig.AppServer);
            await client.PostAsync("/account/setName", new Dictionary<string, string>
            {
                ["guid"] = _guid.Text,
                ["password"] = _password.Text,
                ["name"] = _name.Text,
            });

            OnSignInPressed();
        }
        catch (Exception ex)
        {
            _status.Text = ex.Message;
        }
        finally
        {
            _setName.Disabled = false;
        }
    }

    /// <summary>
    /// Creates an account, then signs in with it.
    /// </summary>
    /// <remarks>
    /// The endpoint wants the *current* credentials as well as the new ones: it doubles as a rename
    /// for a guest account, so an empty pair means "make a fresh one" rather than "no credentials".
    /// </remarks>
    private async void OnRegisterPressed()
    {
        _register.Disabled = true;
        _status.Text = "Creating the account...";

        try
        {
            using var client = new AppEngineClient(ServerConfig.AppServer);
            await client.PostAsync("/account/register", new Dictionary<string, string>
            {
                ["guid"] = string.Empty,
                ["password"] = string.Empty,
                ["newGUID"] = _guid.Text,
                ["newPassword"] = _password.Text,
                ["eliteAccount"] = "0",
            });

            OnSignInPressed();
        }
        catch (Exception ex)
        {
            _status.Text = ex.Message;
        }
        finally
        {
            _register.Disabled = false;
        }
    }

    /// <summary>Fills the form and signs in. Development only.</summary>
    public void PrefillForTesting(string account, string password)
    {
        _guid.Text = account;
        _password.Text = password;
        OnSignInPressed();
    }

    private async void OnSignInPressed()
    {
        _signIn.Disabled = true;
        _status.Text = "Signing in...";
        ClearCharacters();

        string baseUrl = ServerConfig.AppServer;

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


    private void ShowCharacters()
    {
        _servers.Clear();
        foreach (var server in _charList.Servers)
        {
            string load = server.IsFull ? "full" : server.IsCrowded ? "busy" : "open";
            _servers.AddItem($"{server.Name} — {load}");
        }

        _signInPanel.Visible = false;

        // Nothing else on this screen matters until the account has a name.
        bool needsName = !_charList.Account.NameChosen;
        _namePanel.Visible = needsName;
        _charactersPanel.Visible = !needsName;
        _heading.Text = needsName ? "Choose your name" : "Choose a character";

        if (needsName)
        {
            _status.Text = string.Empty;
            _name.CallDeferred(Control.MethodName.GrabFocus);
            return;
        }

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

        _characters.AddChild(new Control { CustomMinimumSize = new Vector2(0, 6) });
        _characters.AddChild(new Label
        {
            Text = onlyOption ? "No characters yet. Create one:" : "Or create a new character:",
        });

        var grid = new GridContainer { Columns = 5 };
        grid.AddThemeConstantOverride("h_separation", 6);
        grid.AddThemeConstantOverride("v_separation", 6);
        _characters.AddChild(grid);

        foreach (var playerClass in classes)
        {
            ushort classType = playerClass.Type;

            var button = new Button
            {
                CustomMinimumSize = new Vector2(126, 74),
                TooltipText = playerClass.DisplayId ?? playerClass.Id,
            };
            button.Pressed += () => RequestCreate(classType);
            grid.AddChild(button);

            var stack = new VBoxContainer { MouseFilter = MouseFilterEnum.Ignore };
            stack.AddThemeConstantOverride("separation", 2);
            stack.SetAnchorsPreset(LayoutPreset.FullRect);
            button.AddChild(stack);

            var art = Portrait(classType, 34);
            art.SizeFlagsHorizontal = SizeFlags.ExpandFill;
            stack.AddChild(art);

            stack.AddChild(new Label
            {
                Text = playerClass.DisplayId ?? playerClass.Id ?? "?",
                HorizontalAlignment = HorizontalAlignment.Center,
                MouseFilter = MouseFilterEnum.Ignore,
            });
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
