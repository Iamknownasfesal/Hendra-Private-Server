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
    /// <param name="remove">
    /// Asks to delete this character. Wired to a cross in the heading rather than to the box, and
    /// it opens a confirmation rather than doing it -- a character is weeks of play and the server
    /// has no undo.
    /// </param>
    private static Control NewCharacterBox(
        string className, Account.CharacterInfo character, Action play, Action remove)
    {
        // A click target, not a keyboard widget -- the original's are graphics you click, and
        // CardButton keeps itself out of the focus chain for the same reason. Godot gives the first
        // focusable control focus on its own, and a focused Button is activated by ui_accept, which
        // is Enter, Space *and joypad button 0* by default.
        var box = new UI.CardButton(UI.Style.FameFill)
        {
            CustomMinimumSize = new Vector2(CharacterBoxWidth, 86),
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

        // The class and the level are the two things you compare between characters, so the level
        // gets a badge of its own rather than trailing the name as more words.
        var headingRow = new HBoxContainer { MouseFilter = MouseFilterEnum.Ignore };
        headingRow.AddThemeConstantOverride("separation", 7);
        rows.AddChild(headingRow);

        var heading = new Label
        {
            Text = className,
            MouseFilter = MouseFilterEnum.Ignore,
            SizeFlagsHorizontal = SizeFlags.ExpandFill,
        };
        heading.Typeset(UI.Style.FontName, UI.Style.Text);
        headingRow.AddChild(heading);

        // Stars, on the original's thresholds: one for each of 20, 150, 400, 800 and 2000 fame.
        // They are what the original rates an account by, and a character's own count is the part
        // of that rating it contributes.
        int stars = UI.Fame.Stars(character.CurrentFame);
        if (stars > 0)
        {
            var starRow = new HBoxContainer
            {
                MouseFilter = Control.MouseFilterEnum.Ignore,
                SizeFlagsVertical = SizeFlags.ShrinkCenter,
            };
            starRow.AddThemeConstantOverride("separation", 1);

            var colour = UI.Fame.Colour(stars, 1);
            for (int i = 0; i < stars; i++)
                starRow.AddChild(new UI.StarIcon(colour, 13));

            headingRow.AddChild(starRow);
        }

        headingRow.AddChild(new UI.LevelBadge(character.Level)
        {
            MouseFilter = Control.MouseFilterEnum.Ignore,
            SizeFlagsVertical = SizeFlags.ShrinkCenter,
        });

        // A Control with Stop inside a row of Ignores: the row does not swallow the click and the
        // cross is picked before the card behind it, so deleting and playing stay separate presses.
        var scrap = new UI.HudIconButton(UI.HudIcons.Cross, "Delete this character", inset: 6f)
        {
            Tint = UI.Style.TextDim,
            CustomMinimumSize = new Vector2(20, 20),
            SizeFlagsVertical = SizeFlags.ShrinkCenter,
        };
        scrap.Pressed += remove;
        headingRow.AddChild(scrap);

        var vitals = new Label
        {
            Text = $"{character.HitPoints}/{character.MaxHitPoints} HP    " +
                   $"{character.MagicPoints}/{character.MaxMagicPoints} MP    " +
                   $"{character.CurrentFame:N0} fame",
            MouseFilter = MouseFilterEnum.Ignore,
        };
        vitals.Typeset(UI.Style.FontBody, UI.Style.Text);
        rows.AddChild(vitals);

        var stats = new Label
        {
            Text = $"ATT {character.Attack}  DEF {character.Defense}  SPD {character.Speed}  " +
                   $"DEX {character.Dexterity}  VIT {character.Vitality}  WIS {character.Wisdom}",
            MouseFilter = MouseFilterEnum.Ignore,
        };
        stats.Typeset(UI.Style.FontSmall, UI.Style.TextDim);
        rows.AddChild(stats);

        return box;
    }

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
            var doomed = character;

            _characters.AddChild(NewCharacterBox(name, character,
                () => RequestPlay(characterId),
                () => ConfirmDelete(name, doomed)));
        }

        if (living.Count < Math.Max(_charList.MaxCharacters, 1))
            AddClassPicker(living.Count == 0);
        else
            AddSlotOffer();
    }

    /// <summary>Puts the confirmation panel up for one character.</summary>
    private void ConfirmDelete(string className, Account.CharacterInfo character)
    {
        _deleting = character.CharacterId;

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

    /// <summary>
    /// The way out of a full character list.
    /// </summary>
    /// <remarks>
    /// Only when every slot is taken, because that is the only time it is the answer. The price and
    /// the currency both come down with the character list, so a server that prices slots in gold
    /// rather than fame, or does not sell them at all, says so itself.
    /// </remarks>
    private void AddSlotOffer()
    {
        var account = _charList.Account;

        string used = $"All {_charList.MaxCharacters} of your character slots are in use.";

        // The balance goes in the sentence, not in a tooltip. A greyed-out button that only explains
        // itself on hover looks like a fault; one under a line saying what you have and what it
        // costs looks like a price.
        string note = account.NextSlotPrice <= 0
            ? $"{used} Delete one to make room."
            : $"{used} You have {account.SlotBalance:N0} {account.SlotCurrencyName}, and another " +
              $"slot costs {account.NextSlotPrice:N0}.";

        _characters.AddChild(new Control { CustomMinimumSize = new Vector2(0, 6) });
        _characters.AddChild(new Label
        {
            Text = note,
            AutowrapMode = TextServer.AutowrapMode.WordSmart,
            CustomMinimumSize = new Vector2(CharacterBoxWidth, 0),
        }.Typeset(UI.Style.FontBody, UI.Style.TextDim));

        // A server can decline to sell them at all, which it says by pricing them at nothing.
        if (account.NextSlotPrice <= 0)
            return;

        var buy = new UI.GameButton(
            $"Buy another slot — {account.NextSlotPrice:N0} {account.SlotCurrencyName}", compact: true)
        {
            Disabled = account.SlotBalance < account.NextSlotPrice,
        };

        buy.Pressed += () => OnBuySlotPressed(buy);
        _characters.AddChild(buy);
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

    private async void OnBuySlotPressed(Button buy)
    {
        buy.Disabled = true;
        _status.Text = "Buying a character slot...";

        try
        {
            using var client = new AppEngineClient(ServerConfig.AppServer);
            await client.PostAsync("/account/purchaseCharSlot", new Dictionary<string, string>
            {
                ["guid"] = _guid.Text,
                ["password"] = _password.Text,
            });

            // The list is rebuilt from here, so the button this ran from is on its way out and is
            // deliberately not re-enabled.
            OnSignInPressed();
        }
        catch (Exception ex)
        {
            _status.Text = $"The slot was not bought: {ex.Message}";

            if (IsInstanceValid(buy))
                buy.Disabled = false;
        }
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

            // Click only, for the reason the character boxes are: creating a character by accident
            // costs a slot, and CardButton stays out of the focus chain.
            // No accent: a class you have not made yet is not one of your characters, and the amber
            // stripe is what marks the ones that are.
            var button = new UI.CardButton(UI.Style.SlotBorderHi)
            {
                CustomMinimumSize = new Vector2(126, 78),
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
