using System;
using System.Collections.Generic;
using System.Linq;
using Godot;
using Hendra.Account;
using Hendra.UI;

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
            // The manifest's key. See TitleScreen: "TitleScreen" matches nothing and returns null,
            // which is why this screen has been a form on flat black rather than on the artwork.
            Texture = ServiceLocator.Assets?.GetImage("OriginalTitleScreen"),
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

        // The same sky as the title screen, so moving between them does not change worlds.
        AddChild(new UI.Starfield());
        AddChild(new UI.Vignette(0.16f));

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
        }.Typeset(UI.Style.FontTitle, UI.Style.Text);
        stack.AddChild(heading);
        _heading = heading;

        // The sign-in form and the character list share the screen's centre, one replacing the
        // other, the way the original moves from its account screen to its character screen.
        _signInPanel = NewPanel();
        stack.AddChild(_signInPanel);

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 10);
        _signInPanel.AddChild(column);

        var saved = ServiceLocator.Settings;

        _guid = AddField(column, "Account", saved?.Account ?? string.Empty);
        _guid.PlaceholderText = "you@example.com";
        _password = AddField(column, "Password", saved?.Password ?? string.Empty);
        _password.Secret = true;

        _remember = new CheckBox { Text = "Remember me", ButtonPressed = saved?.RememberMe ?? true };
        _remember.Toggled += on =>
        {
            if (ServiceLocator.Settings == null)
                return;

            // Turning it off forgets what is already saved rather than only declining to save next
            // time, which is what someone unticking it on a shared machine is asking for.
            ServiceLocator.Settings.RememberMe = on;
            ServiceLocator.ApplySettings();
        };
        column.AddChild(_remember);

        _signIn = new UI.GameButton("Sign in", primary: true, compact: true) { CustomMinimumSize = new Vector2(0, 38) };
        _signIn.Pressed += OnSignInPressed;
        column.AddChild(_signIn);

        // Registering is the same two fields, so it is a second button rather than a third page.
        // The server creates the account and the client signs straight in with it.
        _register = new UI.GameButton("Create an account with these details", compact: true);
        _register.Pressed += OnRegisterPressed;
        column.AddChild(_register);

        var back = new UI.GameButton("Back", compact: true);
        back.Pressed += () => BackPressed?.Invoke();
        column.AddChild(back);

        _status = new Label
        {
            AutowrapMode = TextServer.AutowrapMode.WordSmart,
            HorizontalAlignment = HorizontalAlignment.Center,
            CustomMinimumSize = new Vector2(CharacterBoxWidth, 0),
        };
        _status.Typeset(UI.Style.FontBody, UI.Style.StatLabel);
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
            Text = "Choose the name other players will see. Three to fifteen letters, " +
                   "no digits or spaces.\nThis is separate from your account, and you only get one.",
            AutowrapMode = TextServer.AutowrapMode.WordSmart,
            CustomMinimumSize = new Vector2(CharacterBoxWidth, 0),
        });

        _name = AddField(nameColumn, "Name", string.Empty);
        _name.MaxLength = 15;

        _setName = new UI.GameButton("Take this name", primary: true, compact: true) { CustomMinimumSize = new Vector2(0, 36) };
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

        BuildCharacterScreens();

        // Deleting takes over the screen rather than opening a box over it. It is the one thing here
        // that cannot be undone, and a panel you have to read your way out of is the point.
        _deletePanel = NewPanel();
        _deletePanel.Visible = false;
        stack.AddChild(_deletePanel);

        var deleteColumn = new VBoxContainer();
        deleteColumn.AddThemeConstantOverride("separation", 10);
        _deletePanel.AddChild(deleteColumn);

        _deletePrompt = new Label
        {
            AutowrapMode = TextServer.AutowrapMode.WordSmart,
            CustomMinimumSize = new Vector2(CharacterBoxWidth, 0),
        }.Typeset(UI.Style.FontBody, UI.Style.Text);
        deleteColumn.AddChild(_deletePrompt);

        // The safe way out takes the accent. The destructive one is the plain plate, and it is the
        // second button rather than the first.
        var keep = new UI.GameButton("Keep this character", primary: true, compact: true)
        {
            CustomMinimumSize = new Vector2(0, 36),
        };
        keep.Pressed += () => { _deleting = -1; ShowCharacters(); };
        deleteColumn.AddChild(keep);

        _deleteConfirm = new UI.GameButton("Delete it permanently", compact: true);
        _deleteConfirm.Pressed += OnDeleteConfirmed;
        deleteColumn.AddChild(_deleteConfirm);

        // Enter submits from any field, which is how anyone actually uses a login form.
        foreach (var field in new[] { _guid, _password })
            field.TextSubmitted += _ => OnSignInPressed();

        if (!string.IsNullOrEmpty(saved?.Account) && !string.IsNullOrEmpty(saved.Password) && saved.RememberMe)
            CallDeferred(nameof(OnSignInPressed));
        else
            _guid.CallDeferred(Control.MethodName.GrabFocus);
    }

    /// <summary>
    /// Builds the two screens the character list is really made of.
    /// </summary>
    /// <remarks>
    /// Both are laid out in the interface's own 1920 by 1080 reference pixels, which the rest of
    /// this page is not — it is centred against the project's base resolution and stretched. They go
    /// on a <see cref="UI.HudLayer"/> of their own for that reason, which is the same canvas the HUD
    /// uses in the world, so the panel is the same size and shape here as it is once you are in.
    /// </remarks>
    private void BuildCharacterScreens()
    {
        _screens = new UI.HudLayer { Layer = 4 };
        AddChild(_screens);

        _picker = new UI.CharactersPanel();
        _picker.PlayRequested += RequestPlay;
        _picker.DeleteRequested += id => ConfirmDelete(id);
        _picker.BuySlotRequested += OnBuySlotPressed;
        _picker.NewCharacterRequested += ShowCreatePage;
        _picker.Loaded += roster => _create?.Show(roster);
        _screens.AddChild(_picker);

        _create = new UI.CreateCharacterScreen();
        _create.PlayRequested += RequestCreate;
        _create.Closed += () =>
        {
            if (_charList != null)
                _picker.Open();
        };
        _screens.AddChild(_create);
    }

    private void ShowCreatePage()
    {
        _picker.Close();
        _create.Show(_picker.Roster);
        _create.Open();
    }

    private UI.HudLayer _screens;
    private UI.CharactersPanel _picker;
    private UI.CreateCharacterScreen _create;

    /// <summary>Goes straight to the create page once signed in. Set from the command line.</summary>
    public bool OpenCreatePage { get; set; }

    /// <summary>Which of the panel's tabs to open on. Set from the command line.</summary>
    public string CharactersTab { get; set; }

    /// <summary>Width of a character entry, wide enough for a class name and its stats.</summary>
    private const int CharacterBoxWidth = 420;

    private Button _register;
    private CheckBox _remember;
    private PanelContainer _namePanel;
    private LineEdit _name;
    private Button _setName;

    /// <summary>Raised when the player wants to go back to the title screen.</summary>
    public event Action BackPressed;

    private Label _heading;
    private PanelContainer _signInPanel;
    private PanelContainer _charactersPanel;

    private PanelContainer _deletePanel;
    private Label _deletePrompt;
    private Button _deleteConfirm;

    /// <summary>The character the confirmation panel is asking about, or -1 when it is closed.</summary>
    private int _deleting = -1;

    /// <summary>
    /// A panel in the game's own shape, so the menu belongs to the same game as the HUD.
    /// </summary>
    /// <remarks>
    /// An opaque grey plate with a one-pixel dark edge and square corners, which is what every
    /// panel in the interface is. It used to be a cut-cornered box in the older palette, and the
    /// player met it about four seconds before meeting the HUD's version of the same idea.
    /// </remarks>
    private static PanelContainer NewPanel()
    {
        var plate = UI.Style.Plate(UI.Style.Panel, UI.Style.PanelEdge);
        plate.ContentMarginLeft = 18;
        plate.ContentMarginRight = 18;
        plate.ContentMarginTop = 18;
        plate.ContentMarginBottom = 18;

        var panel = new PanelContainer();
        panel.AddThemeStyleboxOverride("panel", plate);
        return panel;
    }

    private static LineEdit AddField(Control parent, string label, string initial)
    {
        var row = new HBoxContainer();
        row.AddThemeConstantOverride("separation", 8);
        parent.AddChild(row);

        row.AddChild(new Label
        {
            Text = label,
            CustomMinimumSize = new Vector2(90, 0),
            VerticalAlignment = VerticalAlignment.Center,
        }.Typeset(UI.Style.FontBody, UI.Style.TextDim));

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
        // The server's rule, checked here so a refusal is instant and says what is wrong rather
        // than coming back from a round trip as a bare "Invalid name".
        string name = _name.Text ?? string.Empty;
        if (name.Length < 3 || name.Length > 15 || !name.All(char.IsLetter))
        {
            _status.Text = "A name must be three to fifteen letters, with no digits or spaces.";
            return;
        }

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
        // The server takes an email address as the account name and refuses anything else.
        if (!_guid.Text.Contains('@') || !_guid.Text.Contains('.'))
        {
            _status.Text = "The account name has to be an email address.";
            return;
        }

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
            var credentials = new Dictionary<string, string>
            {
                ["guid"] = _guid.Text,
                ["password"] = _password.Text,
            };

            // Verify first, and only then fetch. /char/list is a character fetch that happens to
            // tolerate strangers: an account it has never heard of gets a *guest* built for it on
            // the spot -- AccountId 0, a name off a fixed list, no NameChosen -- and answers 200.
            // Taking that for a successful sign-in is what stranded players on the name page, since
            // every endpoint that checks credentials properly then answered "Bad Login".
            // /account/verify passes nothing but LoginStatus.OK.
            await client.PostAsync("/account/verify", credentials);

            string xml = await client.PostAsync("/char/list", credentials);
            _charList = CharListResult.Parse(xml);

            // Belt and braces: a guest is never persisted, so it can only have come from the path
            // above. Its character list would look playable right up until the world server refused
            // the login.
            if (_charList.Account.AccountId is null or "" or "0")
                throw new AppEngineException("Bad Login", isFatal: false);

            // Saved only once the server has accepted them; storing what was typed would keep a
            // wrong password and quietly fail the auto sign-in every time from then on.
            if (ServiceLocator.Settings != null)
            {
                ServiceLocator.Settings.RememberMe = _remember.ButtonPressed;
                ServiceLocator.Settings.Account = _guid.Text;
                ServiceLocator.Settings.Password = _password.Text;
                ServiceLocator.ApplySettings();
            }
            ShowCharacters();
        }
        catch (AppEngineException ex)
        {
            // The server answers "Bad Login" for a wrong password and for an account that does not
            // exist alike -- LoginStatus.GetInfo() returns the same string for both -- so the reply
            // says what to do about it instead of repeating a phrase that rules nothing out.
            _status.Text = ex.Message.Contains("Bad Login", StringComparison.OrdinalIgnoreCase)
                ? "That email and password do not match an account. Check them, or press Register to create one."
                : ex.IsFatal ? $"Sign-in failed: {ex.Message}" : ex.Message;
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
        // Called to come back from the delete confirmation as well as after a fetch, so it clears
        // rather than assuming it is being run against an empty list.
        ClearCharacters();

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
        _deletePanel.Visible = false;
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

        // The server row is the only thing left in the middle of the page once the panel is up, and
        // with one server to choose between it is a dropdown with nothing to say. A successful sign
        // in is reported by the panel appearing; only a failure gets words.
        _charactersPanel.Visible = _charList.Servers.Count > 1;
        _status.Text = _charList.Servers.Count == 0
            ? "Signed in, but the server list is empty — is the world server running and registered?"
            : string.Empty;

        // The list itself is the same panel the world opens over itself, so the player meets it
        // once and it does not change shape when they sign in.
        _heading.Visible = false;
        _picker.Connect(ServerConfig.AppServer, _guid.Text, _password.Text);
        _picker.CurrentCharacterId = -1;

        if (OpenCreatePage)
        {
            _picker.Refresh();
            ShowCreatePage();
            return;
        }

        _picker.ShowTab(CharactersTab);

        if (!_create.IsOpen)
            _picker.Open();
        else
            _picker.Refresh();
    }

    /// <summary>Puts the confirmation panel up for one character, found by its id.</summary>
    private void ConfirmDelete(int characterId)
    {
        var character = _charList?.Characters.Find(c => c.CharacterId == characterId);
        if (character == null)
            return;

        ConfirmDelete(ClassName(character), character);
    }

    private static string ClassName(Account.CharacterInfo character)
    {
        var desc = ServiceLocator.Data?.GetObject(character.ObjectType);
        return desc?.DisplayId ?? desc?.Id ?? $"Type {character.ObjectType}";
    }

    /// <summary>Puts the confirmation panel up for one character.</summary>
    private void ConfirmDelete(string className, Account.CharacterInfo character)
    {
        _deleting = character.CharacterId;
        _picker.Close();
        _create.Close();
        _heading.Visible = true;

        // Named by what it cost to get, because that is what the answer turns on. A level 1 Wizard
        // rolled by accident and a level 20 with two thousand fame both reach this panel, and the
        // panel should not read the same for both.
        _deletePrompt.Text =
            $"Delete this level {character.Level} {className}, with {character.CurrentFame:N0} fame?" +
            "\n\nIt is gone for good, and no fame is banked for it. The slot it frees can be used " +
            "for a new character.";

        _charactersPanel.Visible = false;
        _deletePanel.Visible = true;
        _heading.Text = "Delete a character";
        _status.Text = string.Empty;
    }

    private async void OnDeleteConfirmed()
    {
        if (_deleting < 0)
            return;

        int charId = _deleting;
        _deleteConfirm.Disabled = true;
        _status.Text = "Deleting the character...";

        try
        {
            using var client = new AppEngineClient(ServerConfig.AppServer);
            await client.PostAsync("/char/delete", new Dictionary<string, string>
            {
                ["guid"] = _guid.Text,
                ["password"] = _password.Text,
                ["charId"] = charId.ToString(),
            });

            // Refetched rather than removed from the copy on hand: the slot count and the next
            // character id both moved, and the server is the only thing that knows what to.
            _deleting = -1;
            OnSignInPressed();
        }
        catch (Exception ex)
        {
            _deleting = -1;
            ShowCharacters();
            _status.Text = $"The character was not deleted: {ex.Message}";
        }
        finally
        {
            _deleteConfirm.Disabled = false;
        }
    }

    private async void OnBuySlotPressed()
    {
        _status.Text = "Buying a character slot...";

        try
        {
            using var client = new AppEngineClient(ServerConfig.AppServer);
            await client.PostAsync("/account/purchaseCharSlot", new Dictionary<string, string>
            {
                ["guid"] = _guid.Text,
                ["password"] = _password.Text,
            });

            // Refetched rather than adjusted in place: the slot count and the balance both moved,
            // and the server is the only thing that knows what to.
            OnSignInPressed();
        }
        catch (Exception ex)
        {
            _status.Text = $"The slot was not bought: {ex.Message}";
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

    /// <summary>
    /// Empties the character list.
    /// </summary>
    /// <remarks>
    /// Detached as well as freed. <c>QueueFree</c> alone does not take a node out of the tree until
    /// the end of the frame, so a rebuild that happens in the same frame -- coming back from the
    /// delete confirmation does -- would append its boxes below the ones it meant to replace.
    /// </remarks>
    private void ClearCharacters()
    {
        foreach (var child in _characters.GetChildren())
        {
            _characters.RemoveChild(child);
            child.QueueFree();
        }
    }
}
