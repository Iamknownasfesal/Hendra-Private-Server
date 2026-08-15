using System;
using System.Collections.Generic;
using System.Linq;
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

        // The same sky as the title screen, so moving between them does not change worlds.
        AddChild(new UI.Starfield());
        AddChild(new UI.Vignette(0.45f));

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
        // The server creates the account and the client signs straight in with it. Labelled with
        // the original's word for it -- "Register" throughout WebLoginDialog.as:48 and
        // RegisterPromptDialog.as:14-16 -- which is also the word the failed sign-in message uses
        // to point at this button.
        _register = new UI.GameButton("Register", compact: true);
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
    /// <param name="available">
    /// False for a class the account may not play, which draws the sprite as a half-transparent
    /// black silhouette. That is the original's own treatment, applied in
    /// <c>SavedCharacter.getImage</c> (SavedCharacter.as:53-55) through the colour transform at
    /// SavedCharacter.as:28, which multiplies the colour away and the alpha by a half.
    /// </param>
    private static Control Portrait(ushort objectType, int size, bool available = true)
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
            Modulate = available ? Colors.White : new Color(0f, 0f, 0f, 0.5f),
        };

        view.SetAnchorsPreset(LayoutPreset.FullRect);
        holder.AddChild(view);
        return holder;
    }

    /// <summary>
    /// The padlock stamped on a class the account may not play.
    /// </summary>
    /// <remarks>
    /// The original's own sprite, not a drawn one: <c>lofiInterface2</c> index 5, which is what
    /// CharacterBox.as:116 puts on a locked box and what the skin list and the player menu use for
    /// the same idea. It is a white silhouette on the sheet, so it takes whatever colour it is
    /// modulated to.
    /// </remarks>
    private static Control Padlock(int size)
    {
        var sprite = ServiceLocator.Assets?.GetSprite("lofiInterface2", LockSpriteIndex) ?? default;
        if (!sprite.IsValid)
            return null;

        var view = new TextureRect
        {
            Texture = new AtlasTexture { Atlas = sprite.Sheet, Region = sprite.Region },
            StretchMode = TextureRect.StretchModeEnum.KeepAspectCentered,
            TextureFilter = CanvasItem.TextureFilterEnum.Nearest,
            MouseFilter = MouseFilterEnum.Ignore,
            CustomMinimumSize = new Vector2(size, size),
            Size = new Vector2(size, size),
            Modulate = new Color(0.92f, 0.9f, 0.86f),
        };

        return view;
    }

    /// <summary>Where the padlock sits on the <c>lofiInterface2</c> sheet. See CharacterBox.as:116.</summary>
    private const int LockSpriteIndex = 5;

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
        // A click target, not a keyboard widget -- the original's are graphics you click, and
        // CardButton keeps itself out of the focus chain for the same reason. Godot gives the first
        // focusable control focus on its own, and a focused Button is activated by ui_accept, which
        // is Enter, Space *and joypad button 0* by default.
        var box = new UI.CardButton(UI.Style.Gold)
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
        heading.AddThemeFontSizeOverride("font_size", 18);
        heading.AddThemeColorOverride("font_color", UI.Style.Text);
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
    private CheckBox _remember;
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

    /// <summary>
    /// The name to claim when the account reaches the naming step, if a run has one in mind.
    /// </summary>
    /// <remarks>
    /// Set by an unattended run rather than by anything a player does. Naming happens partway
    /// through signing in, so there is no moment outside this screen at which it could be supplied.
    /// </remarks>
    public string ClaimName { get; set; }

    /// <summary>The class to create once the character list is up, by object type, or -1 for none.</summary>
    public int PickClassType { get; set; } = -1;

    /// <summary>Fills the form and signs in, as pressing the button does.</summary>
    public void PrefillForTesting(string account, string password)
    {
        _guid.Text = account;
        _password.Text = password;
        OnSignInPressed();
    }

    /// <summary>Fills the form and registers, as pressing Register does.</summary>
    public void RegisterForTesting(string account, string password)
    {
        _guid.Text = account;
        _password.Text = password;
        OnRegisterPressed();
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

            // Kept for the world, which reads the account's best level in each class to tell a
            // level-up that unlocked something from one that did not.
            ServiceLocator.Account = _charList.Account;

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

            // Before the list is drawn, so the class boxes know which of them are locked the first
            // time they are built rather than correcting themselves a moment later.
            _offers = await FetchClassOffersAsync(baseUrl, _guid.Text, _password.Text);

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
        // Cleared here rather than only when a sign-in begins, so that drawing the list is
        // idempotent. Two sign-ins can be in flight at once — a remembered account signs itself in
        // and the player may press the button before it answers — and each clears before it waits
        // on the server, so whichever answers second would otherwise append its characters below
        // the first's and show everything twice.
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
        _heading.Text = needsName ? "Choose your name" : "Choose a character";

        if (needsName)
        {
            _status.Text = string.Empty;
            _name.CallDeferred(Control.MethodName.GrabFocus);

            // An unattended run types the name and presses the button, rather than stopping here
            // for a keyboard that is not there.
            if (!string.IsNullOrEmpty(ClaimName))
            {
                _name.Text = ClaimName;
                ClaimName = null;
                OnSetNamePressed();
            }
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

        // The same call the class card's own handler makes, for a run with nobody to click it.
        if (PickClassType >= 0)
        {
            ushort classType = (ushort)PickClassType;
            PickClassType = -1;
            RequestCreate(classType);
        }
    }

    /// <summary>
    /// Offers the classes this account may create, showing the locked ones as locked.
    /// </summary>
    /// <remarks>
    /// A locked class is drawn the way the original draws it: the portrait blacked out to a
    /// half-transparent silhouette, a padlock stamped on the box, the word LOCKED in red, and the
    /// requirement plus the buy price in the tooltip — CharacterBox.as:86-124 and
    /// ClassToolTip.as:64-120. Thirteen of the fourteen are locked on a fresh account, so offering
    /// them all alike makes almost every first click a refusal.
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
            string name = playerClass.DisplayId ?? playerClass.Id ?? "?";

            // No entry at all means the account server never answered, in which case every class is
            // offered as before rather than the whole picker being struck out on a failed request.
            _offers.TryGetValue(classType, out var offer);
            bool locked = offer.Locked != null;

            // Click only, for the reason the character boxes are: creating a character by accident
            // costs a slot, and CardButton stays out of the focus chain.
            var button = new UI.CardButton(locked ? UI.Style.Steel : UI.Style.Gold)
            {
                CustomMinimumSize = new Vector2(126, 92),
                TooltipText = locked ? LockedTooltip(name, offer) : name,
            };

            if (locked)
            {
                // The original puts a buy button on a locked box. There is no shop here, so the
                // click says what the box is waiting for instead of sending a create the server is
                // certain to refuse.
                string reason = LockedTooltip(name, offer);
                button.Pressed += () => _status.Text = reason;
            }
            else
            {
                button.Pressed += () => RequestCreate(classType);
            }

            grid.AddChild(button);

            var stack = new VBoxContainer { MouseFilter = MouseFilterEnum.Ignore };
            stack.AddThemeConstantOverride("separation", 1);
            stack.SetAnchorsPreset(LayoutPreset.FullRect);
            button.AddChild(stack);

            var art = Portrait(classType, 34, available: !locked);
            art.SizeFlagsHorizontal = SizeFlags.ExpandFill;
            stack.AddChild(art);

            if (locked)
            {
                var status = new Label
                {
                    Text = "LOCKED",
                    HorizontalAlignment = HorizontalAlignment.Center,
                    MouseFilter = MouseFilterEnum.Ignore,
                };

                // The original's own red, from the status field at CharacterBox.as:244.
                status.AddThemeColorOverride("font_color", new Color(1f, 0f, 0f));
                status.AddThemeFontSizeOverride("font_size", 14);
                stack.AddChild(status);
            }

            var label = new Label
            {
                Text = name,
                HorizontalAlignment = HorizontalAlignment.Center,
                MouseFilter = MouseFilterEnum.Ignore,
            };

            if (locked)
                label.AddThemeColorOverride("font_color", new Color(0.55f, 0.55f, 0.55f));

            stack.AddChild(label);

            // Stamped over the box rather than laid out in it, which is where the original puts it
            // -- top left, over the portrait (CharacterBox.as:116-121).
            var padlock = locked ? Padlock(16) : null;
            if (padlock != null)
            {
                padlock.Position = new Vector2(5f, 5f);
                button.AddChild(padlock);
            }
        }
    }

    /// <summary>
    /// What a locked box says when the pointer rests on it.
    /// </summary>
    /// <remarks>
    /// The original's three parts, in the original's order: the heading, the requirement, and the
    /// price to skip it — ClassToolTip.as:87-119.
    /// </remarks>
    private static string LockedTooltip(string name, ClassOffer offer)
    {
        var text = new System.Text.StringBuilder();
        text.Append(name).Append('\n').Append("TO UNLOCK\n").Append(Article(offer.Locked));

        if (offer.Cost > 0)
            text.Append("\nor buy now for ").Append(offer.Cost).Append(" Gold");

        return text.ToString();
    }

    /// <summary>
    /// Fixes the indefinite article in a sentence the server wrote, and gives it a capital.
    /// </summary>
    /// <remarks>
    /// The reason arrives as prose assembled around a class name the server does not inspect, so it
    /// says "a Archer" and "a Assassin". The original never had the problem — it built the sentence
    /// from a format string and the class name separately (ClassToolTip.as:102-105) — and a tooltip
    /// is the last place a player should be reading around a grammar mistake.
    /// </remarks>
    private static string Article(string sentence)
    {
        if (string.IsNullOrEmpty(sentence))
            return sentence;

        int at = sentence.IndexOf(" a ", StringComparison.Ordinal);
        if (at >= 0 && at + 3 < sentence.Length && "AEIOUaeiou".IndexOf(sentence[at + 3]) >= 0)
            sentence = sentence[..(at + 2)] + "n" + sentence[(at + 2)..];

        return char.ToUpperInvariant(sentence[0]) + sentence[1..];
    }

    /// <summary>What the account server says about one class.</summary>
    private readonly struct ClassOffer
    {
        /// <summary>Why this class cannot be played, or null when it can.</summary>
        public string Locked { get; init; }

        /// <summary>What unlocking it outright costs, or zero when it is not for sale.</summary>
        public int Cost { get; init; }
    }

    /// <summary>
    /// Which classes this account may play, by object type.
    /// </summary>
    /// <remarks>
    /// Empty until a sign-in fills it, and left empty if the request fails, which offers every class
    /// rather than locking the picker shut on a network error.
    /// </remarks>
    private Dictionary<ushort, ClassOffer> _offers = new();

    /// <summary>
    /// Asks the account server which classes this account has unlocked.
    /// </summary>
    /// <remarks>
    /// <para>
    /// A second request rather than a second field on <c>/char/list</c>: the account server already
    /// resolves this exactly — the per-class rule against the account's best level in each class —
    /// and answers it on <c>/classes</c>, while <c>/char/list</c> carries only living characters and
    /// so cannot say what a dead one reached.
    /// </para>
    /// <para>
    /// That route is bearer-authenticated, so the credentials are exchanged for a token first. Any
    /// failure returns an empty map and is not reported: the picker degrades to offering everything,
    /// which is what it did before, rather than a sign-in failing over a decoration.
    /// </para>
    /// </remarks>
    private static async System.Threading.Tasks.Task<Dictionary<ushort, ClassOffer>> FetchClassOffersAsync(
        string baseUrl, string guid, string password)
    {
        var offers = new Dictionary<ushort, ClassOffer>();

        try
        {
            using var http = new System.Net.Http.HttpClient { Timeout = TimeSpan.FromSeconds(10) };

            string credentials = System.Text.Json.JsonSerializer.Serialize(new { name = guid, password });
            using var body = new System.Net.Http.StringContent(
                credentials, System.Text.Encoding.UTF8, "application/json");

            var signIn = await http.PostAsync($"{baseUrl}/login", body);
            if (!signIn.IsSuccessStatusCode)
                return offers;

            using var session = System.Text.Json.JsonDocument.Parse(await signIn.Content.ReadAsStringAsync());
            if (!session.RootElement.TryGetProperty("token", out var token))
                return offers;

            http.DefaultRequestHeaders.Authorization =
                new System.Net.Http.Headers.AuthenticationHeaderValue("Bearer", token.GetString());

            var answer = await http.GetAsync($"{baseUrl}/classes");
            if (!answer.IsSuccessStatusCode)
                return offers;

            using var listed = System.Text.Json.JsonDocument.Parse(await answer.Content.ReadAsStringAsync());

            foreach (var entry in listed.RootElement.EnumerateArray())
            {
                if (!entry.TryGetProperty("object_type", out var type))
                    continue;

                string locked = entry.TryGetProperty("locked", out var why)
                                && why.ValueKind == System.Text.Json.JsonValueKind.String
                    ? why.GetString()
                    : null;

                int cost = entry.TryGetProperty("cost", out var price)
                           && price.ValueKind == System.Text.Json.JsonValueKind.Number
                    ? price.GetInt32()
                    : 0;

                offers[(ushort)type.GetUInt32()] = new ClassOffer { Locked = locked, Cost = cost };
            }
        }
        catch (Exception ex)
        {
            GD.PushWarning($"[login] could not read class unlocks: {ex.Message}");
        }

        return offers;
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

    /// <summary>Empties the character list before it is rebuilt.</summary>
    /// <remarks>
    /// Detached here and freed afterwards, rather than only queued for freeing. A queued node is
    /// still a child until the end of the frame, so a second sign-in that lands in the same frame
    /// as the first — which is what a remembered account does, since it signs in by itself and the
    /// player may press the button before it finishes — rebuilds the list underneath the old one
    /// and shows every character twice.
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
