using System;
using System.Collections.Generic;
using System.Linq;
using Godot;
using Hendra.App;

namespace Hendra.UI;

/// <summary>
/// The options screen: four tabs of settings drawn in the game's own chrome.
/// </summary>
/// <remarks>
/// <para>
/// Custom-drawn rather than assembled out of engine widgets, for the same reason the rest of the
/// interface is: a checkbox and a dropdown from the engine's default theme sit in a pixel-art game
/// like a form in a spreadsheet. A row here is a label plate and a control plate, the control being
/// a green ON, a red OFF, a value with a triangle, a track with a square handle, or the name of a
/// key -- which is what the original's screen is made of.
/// </para>
/// <para>
/// The full list from the original is here, which means a number of rows are remembered but have
/// nothing behind them yet -- the ones for pets, summons, titles, quest portraits, trade and guild
/// invite panels, cursors, and the parts of the log and the fame notifications this fork has not
/// built. They persist, so a player who sets them keeps them, and each is a single wiring change
/// away from working the day the feature underneath it exists. What none of them do is lie about
/// something that is already there.
/// </para>
/// <para>
/// Changes apply as they are made. A volume you cannot hear, or a shadow you cannot see, until the
/// panel closes is a setting you cannot judge.
/// </para>
/// </remarks>
public partial class OptionsView : Control
{
    private const float PanelWidth = 720f;
    private const float PanelHeight = 560f;

    private const float TitleHeight = 52f;
    private const float TabHeight = 30f;
    private const float RowHeight = 30f;
    private const float RowGap = 3f;
    private const float HeadingHeight = 34f;

    /// <summary>Where the control plate starts, as a fraction of the body's width.</summary>
    private const float SplitAt = 0.55f;

    private static readonly string[] TabNames = { "Controls", "Gameplay", "Video", "Sound" };

    private Settings _settings;
    private Body _body;
    private Control _panel;

    private int _tab;

    /// <summary>Raised whenever a value changes, so the caller can apply and save it.</summary>
    public event Action Changed;

    public bool IsOpen => _panel is { Visible: true };

    public void Configure(Settings settings)
    {
        _settings = settings;
        _body?.Rebuild();
    }

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;
        this.FillScreen();

        _panel = new Chrome(this) { Visible = false, MouseFilter = MouseFilterEnum.Stop };
        AddChild(_panel);

        _body = new Body(this) { MouseFilter = MouseFilterEnum.Stop };
        _panel.AddChild(_body);

        Resized += Fit;
        Fit();
    }

    private void Fit()
    {
        if (_panel == null)
            return;

        var centre = (Size - new Vector2(PanelWidth, PanelHeight)) / 2f;
        _panel.Position = new Vector2(Mathf.Round(centre.X), Mathf.Round(centre.Y));
        _panel.Size = new Vector2(PanelWidth, PanelHeight);

        float top = TitleHeight + TabHeight;
        _body.Position = new Vector2(8f, top);
        _body.Size = new Vector2(PanelWidth - 16f, PanelHeight - top - 8f);

        // Deliberately not a rebuild: a resize -- which changing the window mode causes -- would
        // otherwise throw the rows away and put the player back at the top of the list.
        if (_body.IsEmpty)
            _body.Rebuild();

        _body.QueueRedraw();
    }

    /// <summary>Opens the panel, or closes it if it is already open.</summary>
    /// <summary>
    /// Which page the panel opens on, by name.
    /// </summary>
    /// <remarks>
    /// For the command line, which is the only caller: an unattended run that wants a picture of
    /// the video settings has no hand on the keyboard to click the tab with.
    /// </remarks>
    public void ShowTab(string name)
    {
        int at = Array.FindIndex(TabNames, t => string.Equals(t, name, StringComparison.OrdinalIgnoreCase));
        if (at >= 0)
            _tab = at;
    }

    public void Toggle()
    {
        if (_panel == null)
            return;

        // A rebinding left half-finished would otherwise swallow the next key pressed in the world.
        _body?.StopListening();

        _panel.Visible = !_panel.Visible;

        if (_panel.Visible)
            _body?.Rebuild();
    }

    private void Apply()
    {
        Changed?.Invoke();
        _body?.QueueRedraw();
    }

    // ==========================================================================================
    // Chrome: the frame, the title, the tabs and the close button
    // ==========================================================================================

    private sealed partial class Chrome : Control
    {
        private readonly OptionsView _owner;

        private Rect2 _close;
        private int _hoveredTab = -1;
        private bool _overClose;
        private bool _overReset;

        public Chrome(OptionsView owner) => _owner = owner;

        public override void _Ready() => MouseExited += () =>
        {
            _hoveredTab = -1;
            _overClose = false;
            _overReset = false;
            QueueRedraw();
        };

        private Rect2 TabAt(int index)
        {
            float width = (Size.X - 16f) / TabNames.Length;
            return new Rect2(8f + index * width, TitleHeight - 2f, width - 4f, TabHeight);
        }

        private Rect2 ResetAt() =>
            new(180f, 14f, Style.Measure("reset to defaults", Style.FontSmall) + 8f, 20f);

        public override void _GuiInput(InputEvent @event)
        {
            switch (@event)
            {
                case InputEventMouseMotion motion:
                {
                    int over = -1;
                    for (int i = 0; i < TabNames.Length; i++)
                    {
                        if (TabAt(i).HasPoint(motion.Position))
                            over = i;
                    }

                    bool close = _close.HasPoint(motion.Position);
                    bool reset = ResetAt().HasPoint(motion.Position);

                    if (over != _hoveredTab || close != _overClose || reset != _overReset)
                    {
                        _hoveredTab = over;
                        _overClose = close;
                        _overReset = reset;
                        QueueRedraw();
                    }

                    return;
                }

                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left } click:
                {
                    AcceptEvent();

                    if (_close.HasPoint(click.Position))
                    {
                        _owner.Toggle();
                        return;
                    }

                    if (ResetAt().HasPoint(click.Position))
                    {
                        KeyBindings.ResetAll(_owner._settings);
                        _owner._body?.Rebuild();
                        return;
                    }

                    for (int i = 0; i < TabNames.Length; i++)
                    {
                        if (!TabAt(i).HasPoint(click.Position))
                            continue;

                        _owner._tab = i;
                        _owner._body?.StopListening();
                        _owner._body?.Rebuild();
                        return;
                    }

                    return;
                }
            }
        }

        public override void _Draw()
        {
            var full = new Rect2(Vector2.Zero, Size);

            DrawRect(full, Style.PanelEdge);
            DrawRect(full.Grow(-2f), Style.PanelSolid);

            // The title, and the small grey affordance beside it that puts the keys back.
            this.DrawText(new Vector2(16f, 38f), "Options", 28, Style.Text);

            var reset = ResetAt();
            this.DrawText(new Vector2(reset.Position.X + 4f, 28f), "reset to defaults",
                Style.FontSmall, _overReset ? Style.Text : Style.TextDim);

            // Close, in the original's red.
            _close = new Rect2(Size.X - 116f, 12f, 100f, 26f);
            DrawRect(_close, _overClose ? new Color("e0453f") : new Color("c62f2a"));
            DrawRect(_close, Style.PanelEdge, filled: false, width: 1f);
            this.DrawText(
                new Vector2(_close.Position.X + (_close.Size.X - Style.Measure("Close", Style.FontBody)) / 2f,
                    _close.Position.Y + 18f), "Close", Style.FontBody, Style.Text);

            for (int i = 0; i < TabNames.Length; i++)
            {
                var box = TabAt(i);
                bool active = i == _owner._tab;

                DrawRect(box, active ? Style.ButtonFace : Style.TabIdle);

                if (_hoveredTab == i && !active)
                    DrawRect(box, Style.ButtonHover);

                DrawRect(box, Style.PanelEdge, filled: false, width: 1f);

                // The active tab is joined to the body below it by covering its own bottom edge.
                if (active)
                    DrawRect(new Rect2(box.Position.X + 1f, box.End.Y - 1f, box.Size.X - 2f, 2f), Style.ButtonFace);

                float text = box.Position.X + (box.Size.X - Style.Measure(TabNames[i], Style.FontBody)) / 2f;
                this.DrawText(new Vector2(text, box.Position.Y + 20f), TabNames[i], Style.FontBody,
                    active ? Style.Text : Style.TextDim);
            }
        }
    }

    // ==========================================================================================
    // Rows
    // ==========================================================================================

    private enum RowKind
    {
        Heading,
        Toggle,
        Choice,
        Slider,
        Key,
    }

    private sealed class Row
    {
        public RowKind Kind;
        public string Label;

        public Func<bool> GetBool;
        public Action<bool> SetBool;

        public string[] Choices;
        public Func<int> GetChoice;
        public Action<int> SetChoice;

        public Func<float> GetValue;
        public Action<float> SetValue;

        public string Action;

        public float Height => Kind == RowKind.Heading ? HeadingHeight : RowHeight;
    }

    /// <summary>Where a value sits in its range, as nought to one.</summary>
    private static float Fraction(float value, float low, float high) =>
        Mathf.Clamp((value - low) / (high - low), 0f, 1f);

    /// <summary>The value at that point in the range, rounded to something a slider can land on.</summary>
    private static float Lerp(float fraction, float low, float high) =>
        Mathf.Round((low + (high - low) * Mathf.Clamp(fraction, 0f, 1f)) * 100f) / 100f;

    /// <summary>Frame rate ceilings offered, with zero meaning none.</summary>
    private static readonly List<int> FpsChoices = new() { 0, 30, 60, 120, 144, 240 };

    private static readonly string[] FpsLabels = { "Unlimited", "30", "60", "120", "144", "240" };

    /// <summary>The chat sizes the original offers, in pixels.</summary>
    private static readonly List<int> ChatSizes = new() { 9, 11, 12, 14, 16 };

    // ==========================================================================================
    // Body: the scrolling list of rows
    // ==========================================================================================

    private sealed partial class Body : Control
    {
        private readonly OptionsView _owner;
        private readonly List<Row> _rows = new();

        private float _scroll;
        private int _hovered = -1;
        private bool _dragging;
        private int _draggingSlider = -1;

        /// <summary>The row waiting for a key press, or -1 when nothing is being rebound.</summary>
        private int _listening = -1;

        public Body(OptionsView owner) => _owner = owner;

        public override void _Ready()
        {
            ClipContents = true;
            MouseExited += () => { _hovered = -1; QueueRedraw(); };
        }

        private Settings Options => _owner._settings;

        /// <summary>Whether the rows have yet to be built, so a relayout knows to build them.</summary>
        public bool IsEmpty => _rows.Count == 0;

        /// <summary>Moves the list, clamped to what there is to see.</summary>
        public void Scroll(float by)
        {
            float was = _scroll;
            _scroll = Mathf.Clamp(_scroll + by, 0f, MaxScroll());

            if (!Mathf.IsEqualApprox(was, _scroll))
                QueueRedraw();
        }

        public void StopListening()
        {
            if (_listening < 0)
                return;

            _listening = -1;
            QueueRedraw();
        }

        public void Rebuild()
        {
            _rows.Clear();
            _scroll = 0f;
            _listening = -1;

            if (Options != null)
            {
                switch (_owner._tab)
                {
                    case 0: BuildControls(); break;
                    case 1: BuildGameplay(); break;
                    case 2: BuildVideo(); break;
                    default: BuildSound(); break;
                }
            }

            QueueRedraw();
        }

        private void Heading(string text) => _rows.Add(new Row { Kind = RowKind.Heading, Label = text });

        private void Toggle(string label, Func<bool> get, Action<bool> set) =>
            _rows.Add(new Row { Kind = RowKind.Toggle, Label = label, GetBool = get, SetBool = set });

        private void Choice(string label, string[] choices, Func<int> get, Action<int> set) =>
            _rows.Add(new Row
            {
                Kind = RowKind.Choice, Label = label, Choices = choices, GetChoice = get, SetChoice = set,
            });

        private void Slider(string label, Func<float> get, Action<float> set) =>
            _rows.Add(new Row { Kind = RowKind.Slider, Label = label, GetValue = get, SetValue = set });

        private void BuildControls()
        {
            string group = null;
            foreach (var binding in KeyBindings.All)
            {
                if (binding.Group != group)
                {
                    group = binding.Group;
                    Heading(group);
                }

                _rows.Add(new Row { Kind = RowKind.Key, Label = binding.Label, Action = binding.Action });
            }
        }

        private void BuildGameplay()
        {
            Heading("General");
            Choice("User Interface", new[] { "Classic" }, () => 0, _ => { });
            Toggle("Allow Camera Rotation", () => Options.AllowCameraRotation, on => Options.AllowCameraRotation = on);
            Toggle("Allow Minimap Rotation", () => Options.MinimapRotation, on => Options.MinimapRotation = on);
            Toggle("Switch Item to/from Backpack", () => Options.SwapWithBackpack, on => Options.SwapWithBackpack = on);
            Toggle("Keep the player centred", () => Options.CenterOnPlayer, on => Options.CenterOnPlayer = on);

            Heading("Opacity");
            Slider("Percentage", () => Options.Opacity, value => Options.Opacity = Mathf.Max(0.1f, value));
            Toggle("Player on top", () => Options.PlayerOnTop, on => Options.PlayerOnTop = on);
            Toggle("Guild Members", () => Options.FadeGuildMembers, on => Options.FadeGuildMembers = on);
            Toggle("Players", () => Options.FadePlayers, on => Options.FadePlayers = on);
            Toggle("Projectiles", () => Options.FadeProjectiles, on => Options.FadeProjectiles = on);

            Heading("Accessibility");
            Choice("Camera Rotation Speed", new[] { "Slow", "Normal", "Fast" },
                () => Options.CameraRotationSpeed, value => Options.CameraRotationSpeed = value);
            Choice("Chat Font Size", new[] { "Tiny", "Small", "Default", "Large", "Big" },
                () => ChatSizes.IndexOf(Options.ChatFontSize) is var at && at >= 0 ? at : 2,
                value => Options.ChatFontSize = ChatSizes[value]);
            Toggle("Dynamic HP Colors on GUI", () => Options.DynamicHpGui, on => Options.DynamicHpGui = on);
            Toggle("Dynamic HP Colors on Player", () => Options.DynamicHpPlayer, on => Options.DynamicHpPlayer = on);
            Toggle("Dynamic HP Colors on Boss", () => Options.DynamicHpBoss, on => Options.DynamicHpBoss = on);
            Toggle("Small Condition Icons", () => Options.SmallConditionIcons, on => Options.SmallConditionIcons = on);

            Heading("Social");
            Toggle("Hide Chat Window", () => Options.HideChat, on => Options.HideChat = on);
            Toggle("Player Chat", () => Options.PlayerChat, on => Options.PlayerChat = on);
            Toggle("Whisper Chat", () => Options.WhisperChat, on => Options.WhisperChat = on);
            Toggle("Guild Chat", () => Options.GuildChatShown, on => Options.GuildChatShown = on);
            Toggle("Show Player Titles", () => Options.ShowPlayerTitles, on => Options.ShowPlayerTitles = on);
        }

        private void BuildVideo()
        {
            Heading("Rendering");

            // The preset writes the two below it. It is not stored anywhere -- it is read back
            // from what they say, so moving either one on its own is not overridden the next time
            // this page is opened, and lands on "Custom" if the pair matches no preset.
            Choice("Quality", QualityNames, () => QualityOf(Options), value => ApplyQuality(Options, value));

            Choice("Render Scale", ScaleLabels,
                () => Nearest(ScaleChoices, Options.RenderScale),
                value => Options.RenderScale = ScaleChoices[value]);

            Choice("Anti-Aliasing", new[] { "Off", "FXAA", "MSAA 2x", "MSAA 4x", "MSAA 8x + FXAA" },
                () => Options.AntiAliasing, value => Options.AntiAliasing = value);

            Choice("Interface Scale", HudScaleLabels,
                () => Nearest(HudScaleChoices, Options.HudScale),
                value => Options.HudScale = HudScaleChoices[value]);

            Heading("Display");
            Choice("Window Mode", new[] { "Fullscreen", "Windowed" },
                () => Options.Windowed ? 1 : 0, value => Options.Windowed = value == 1);

            // Shown as a share of the range rather than as a raw multiplier, so the slider reads
            // the way every other slider on this screen does.
            Slider("View Distance", () => 1f - Fraction(Options.CameraZoom, 0.5f, 2f),
                value => Options.CameraZoom = Lerp(1f - value, 0.5f, 2f));
            Slider("Bag Size", () => Fraction(Options.BagSize, 0.5f, 2.5f),
                value => Options.BagSize = Lerp(value, 0.5f, 2.5f));
            Choice("Frame Rate Limit", FpsLabels,
                () => FpsChoices.IndexOf(Options.MaxFps) is var at && at >= 0 ? at : 0,
                value => Options.MaxFps = FpsChoices[value]);

            Choice("V-Sync", new[] { "Off", "On", "Adaptive" },
                () => Options.VSync, value => Options.VSync = value);

            Choice("Default Camera Angle", new[] { "0°", "45°" },
                () => Options.DefaultCameraAngle == 45 ? 1 : 0,
                value => Options.DefaultCameraAngle = value == 1 ? 45 : 0);

            Heading("Interface");
            Toggle("Show Ally Buff Icons", () => Options.ShowAllyBuffIcons, on => Options.ShowAllyBuffIcons = on);
            Toggle("Show Boss HP Bars", () => Options.ShowBossHpBars, on => Options.ShowBossHpBars = on);
            Toggle("Show Tier Level", () => Options.ShowTierLevel, on => Options.ShowTierLevel = on);
            Toggle("Expand Log", () => Options.ExpandLog, on => Options.ExpandLog = on);
            Toggle("Show Fame Gain", () => Options.ShowFameGain, on => Options.ShowFameGain = on);
            Choice("Toggle Fame and HP/MP Text", new[] { "Off", "Fame", "HP/MP", "All" },
                () => Options.BarText, value => Options.BarText = value);
            Choice("HP Bars", new[] { "Off", "Enemy", "Ally", "All" },
                () => Options.HealthBars, value => Options.HealthBars = value);

            Heading("Quality");
            Toggle("Particles Master", () => Options.Particles, on => Options.Particles = on);
            Choice("Particle Effect", new[] { "Low", "Medium", "High" },
                () => Options.ParticleDetail, value => Options.ParticleDetail = value);
            Choice("Draw Gameview Shadows", new[] { "Off", "Low", "High" },
                () => Options.Shadows, value => Options.Shadows = value);
            Toggle("Enemy Particles", () => Options.EnemyParticles, on => Options.EnemyParticles = on);
            Toggle("Player Hit Particles", () => Options.PlayerHitParticles, on => Options.PlayerHitParticles = on);
            Toggle("AOE Particles", () => Options.AoeParticles, on => Options.AoeParticles = on);
            Toggle("Draw Text Bubbles", () => Options.TextBubbles, on => Options.TextBubbles = on);
            Toggle("Ally Notifications", () => Options.AllyNotifications, on => Options.AllyNotifications = on);
            Toggle("Enemy Damage Text", () => Options.EnemyDamageText, on => Options.EnemyDamageText = on);
            Toggle("Ally Damage Text", () => Options.AllyDamageText, on => Options.AllyDamageText = on);
            Choice("Always Show EXP", new[] { "Off", "On", "Self" },
                () => Options.AlwaysShowExp, value => Options.AlwaysShowExp = value);
            Toggle("Curse Indication", () => Options.CurseIndication, on => Options.CurseIndication = on);
            Choice("Ally Shoot", new[] { "Show All", "Hide Projectiles", "Hide All" },
                () => Options.AllyShoot, value => Options.AllyShoot = value);
        }

        /// <summary>
        /// The presets, as the pair of values each one stands for.
        /// </summary>
        /// <remarks>
        /// Supersampling is where the quality is, so the ladder is mostly render scale: half again
        /// at High and double at Ultra. Low turns everything off for a machine that is struggling,
        /// which on this game is a real case -- a full realm is a few thousand sprites.
        /// </remarks>
        private static readonly (string Name, int Scale, int Aa)[] Presets =
        {
            ("Low", 100, 0),
            ("Medium", 100, 1),
            ("High", 150, 1),
            ("Ultra", 200, 4),
        };

        private static readonly string[] QualityNames =
            Presets.Select(p => p.Name).Append("Custom").ToArray();

        private static readonly int[] ScaleChoices = { 75, 100, 125, 150, 175, 200 };

        /// <summary>Zero is "fit to the window", which is what the interface has always done.</summary>
        private static readonly int[] HudScaleChoices = { 0, 100, 125, 150, 175, 200, 250 };

        private static readonly string[] HudScaleLabels =
            { "Fit to Window", "100%", "125%", "150%", "175%", "200%", "250%" };

        private static readonly string[] ScaleLabels =
            ScaleChoices.Select(v => $"{v}%").ToArray();

        /// <summary>Which preset the current pair of values is, or Custom.</summary>
        private static int QualityOf(App.Settings options)
        {
            for (int i = 0; i < Presets.Length; i++)
                if (Presets[i].Scale == options.RenderScale && Presets[i].Aa == options.AntiAliasing)
                    return i;

            return Presets.Length;
        }

        private static void ApplyQuality(App.Settings options, int preset)
        {
            if (preset < 0 || preset >= Presets.Length)
                return;

            options.RenderScale = Presets[preset].Scale;
            options.AntiAliasing = Presets[preset].Aa;
        }

        /// <summary>The index of the nearest offered value, so a hand-edited file still shows.</summary>
        private static int Nearest(int[] choices, int value)
        {
            int best = 0;
            for (int i = 1; i < choices.Length; i++)
                if (Mathf.Abs(choices[i] - value) < Mathf.Abs(choices[best] - value))
                    best = i;

            return best;
        }

        private void BuildSound()
        {
            Heading("Volume");
            Slider("Master Volume", () => Options.MasterVolume, value => Options.MasterVolume = value);
            Slider("Sound Effects Volume", () => Options.EffectVolume, value =>
            {
                Options.EffectVolume = value;

                // Played on release so the slider demonstrates the level it has just been set to.
                ServiceLocator.Audio?.PlayEffect("button_click");
            });
            Slider("Music Volume", () => Options.MusicVolume, value => Options.MusicVolume = value);

            Heading("General");
            Toggle("Play Weapon Sounds", () => Options.WeaponSounds, on => Options.WeaponSounds = on);
        }

        // ---- geometry ----

        private float Content()
        {
            float total = 0f;
            foreach (var row in _rows)
                total += row.Height + RowGap;
            return total;
        }

        private float MaxScroll() => HudScrollbar.MaxOffset(Size, Content());

        private Rect2 RowAt(int index)
        {
            float y = -_scroll;
            for (int i = 0; i < index; i++)
                y += _rows[i].Height + RowGap;

            return new Rect2(0f, y, Size.X - HudScrollbar.Width - 4f, _rows[index].Height);
        }

        private int RowUnder(Vector2 at)
        {
            for (int i = 0; i < _rows.Count; i++)
            {
                if (_rows[i].Kind != RowKind.Heading && RowAt(i).HasPoint(at))
                    return i;
            }

            return -1;
        }

        /// <summary>The control plate on the right of a row.</summary>
        private Rect2 ControlAt(int index)
        {
            var row = RowAt(index);
            float left = Mathf.Round(row.Size.X * SplitAt);
            return new Rect2(row.Position.X + left, row.Position.Y, row.Size.X - left, row.Size.Y);
        }

        // ---- input ----

        public override void _GuiInput(InputEvent @event)
        {
            switch (@event)
            {
                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelDown }:
                    Scroll(RowHeight * 3f);
                    AcceptEvent();
                    return;

                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelUp }:
                    Scroll(-RowHeight * 3f);
                    AcceptEvent();
                    return;

                case InputEventMouseButton { Pressed: false, ButtonIndex: MouseButton.Left }:
                    _dragging = false;
                    _draggingSlider = -1;
                    return;

                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left } click:
                    AcceptEvent();
                    Press(click.Position);
                    return;

                case InputEventMouseMotion motion:
                    if (_dragging)
                    {
                        _scroll = HudScrollbar.OffsetForThumbTop(Size, Content(), motion.Position.Y);
                        QueueRedraw();
                        return;
                    }

                    if (_draggingSlider >= 0)
                    {
                        DragSlider(_draggingSlider, motion.Position);
                        return;
                    }

                    int over = RowUnder(motion.Position);
                    if (over != _hovered)
                    {
                        _hovered = over;
                        QueueRedraw();
                    }

                    return;
            }
        }

        private void Press(Vector2 at)
        {
            // The scrollbar first: it overlays the right edge of every row.
            if (at.X >= Size.X - HudScrollbar.Width)
            {
                switch (HudScrollbar.Test(Size, _scroll, Content(), at))
                {
                    case HudScrollbar.Part.Up:
                        _scroll = Mathf.Max(0f, _scroll - RowHeight);
                        break;
                    case HudScrollbar.Part.Down:
                        _scroll = Mathf.Min(MaxScroll(), _scroll + RowHeight);
                        break;
                    case HudScrollbar.Part.Thumb:
                        _dragging = true;
                        break;
                    case HudScrollbar.Part.TrackAbove:
                        _scroll = Mathf.Max(0f, _scroll - Size.Y);
                        break;
                    case HudScrollbar.Part.TrackBelow:
                        _scroll = Mathf.Min(MaxScroll(), _scroll + Size.Y);
                        break;
                }

                QueueRedraw();
                return;
            }

            int index = RowUnder(at);
            if (index < 0)
                return;

            var row = _rows[index];

            switch (row.Kind)
            {
                case RowKind.Toggle:
                    row.SetBool(!row.GetBool());
                    _owner.Apply();
                    break;

                case RowKind.Choice:
                    // Cycling rather than a popup list: every choice here is two to five values, and
                    // a menu that opens over a panel that is already scrolling is more machinery
                    // than the thing it selects.
                    row.SetChoice((row.GetChoice() + 1) % row.Choices.Length);
                    _owner.Apply();
                    break;

                case RowKind.Slider:
                    _draggingSlider = index;
                    DragSlider(index, at);
                    break;

                case RowKind.Key:
                    // A mouse-bound action is shown but not caught here; see KeyBindings.BoundTo.
                    if (!KeyBindings.IsMouse(row.Action))
                        _listening = index;
                    break;
            }

            QueueRedraw();
        }

        private void DragSlider(int index, Vector2 at)
        {
            var track = SliderTrack(ControlAt(index));
            float value = Mathf.Clamp((at.X - track.Position.X) / Mathf.Max(track.Size.X, 1f), 0f, 1f);

            _rows[index].SetValue(Mathf.Round(value * 20f) / 20f);
            _owner.Apply();
        }

        /// <summary>
        /// Catches the next key for whichever binding is listening.
        /// </summary>
        /// <remarks>
        /// Taken as unhandled input rather than through the row, because the point is to catch keys
        /// the panel has no interest in -- including the ones that would otherwise close it.
        /// </remarks>
        public override void _Input(InputEvent @event)
        {
            if (@event is not InputEventKey { Pressed: true, Echo: false } key)
                return;

            // The keyboard moves the list too, which matters on the Controls tab: it is long enough
            // that reaching the bottom of it by wheel is a chore.
            if (_listening < 0)
            {
                switch (key.Keycode)
                {
                    case Key.Pagedown: Scroll(Size.Y - RowHeight); break;
                    case Key.Pageup: Scroll(-(Size.Y - RowHeight)); break;
                    case Key.Down: Scroll(RowHeight); break;
                    case Key.Up: Scroll(-RowHeight); break;
                    case Key.Home: Scroll(-MaxScroll()); break;
                    case Key.End: Scroll(MaxScroll()); break;
                    default: return;
                }

                GetViewport().SetInputAsHandled();
                return;
            }

            GetViewport().SetInputAsHandled();

            // Escape abandons the rebinding rather than binding Escape, which is the one key a
            // player cannot afford to lose and the one they will press to get out of this.
            if (key.Keycode != Key.Escape)
                KeyBindings.Bind(_rows[_listening].Action, key.Keycode, Options);

            _listening = -1;
            QueueRedraw();
        }

        // ---- drawing ----

        private static Rect2 SliderTrack(Rect2 control) =>
            new(control.Position.X + 8f, control.Position.Y + control.Size.Y / 2f - 4f,
                control.Size.X - 70f, 8f);

        public override void _Draw()
        {
            DrawRect(new Rect2(Vector2.Zero, Size), Style.PanelSolid);

            for (int i = 0; i < _rows.Count; i++)
            {
                var row = _rows[i];
                var box = RowAt(i);

                // Off the top or the bottom: nothing to draw, and the geometry above is cheap.
                if (box.End.Y < 0f || box.Position.Y > Size.Y)
                    continue;

                if (row.Kind == RowKind.Heading)
                {
                    this.DrawText(new Vector2(4f, box.Position.Y + 24f), row.Label, 18, Style.Text);
                    continue;
                }

                // The label plate, and the control plate beside it.
                var control = ControlAt(i);
                var label = new Rect2(box.Position, new Vector2(control.Position.X - box.Position.X - 4f, box.Size.Y));

                DrawRect(label, _hovered == i ? Style.Panel : Style.PanelInset);
                DrawRect(control, _hovered == i ? Style.Panel : Style.PanelInset);

                this.DrawText(new Vector2(label.Position.X + 12f, label.Position.Y + 20f),
                    row.Label, Style.FontBody, Style.TextDim);

                switch (row.Kind)
                {
                    case RowKind.Toggle: DrawToggle(control, row.GetBool()); break;
                    case RowKind.Choice: DrawChoice(control, row.Choices[Mathf.Clamp(row.GetChoice(), 0, row.Choices.Length - 1)]); break;
                    case RowKind.Slider: DrawSlider(control, row.GetValue()); break;
                    case RowKind.Key: DrawKey(control, row, i == _listening); break;
                }
            }

            if (Content() > Size.Y)
                HudScrollbar.Draw(this, Size, _scroll, Content(), _dragging);
        }

        private void DrawToggle(Rect2 control, bool on)
        {
            string text = on ? "ON" : "OFF";
            var pill = new Rect2(
                control.Position.X + (control.Size.X - 54f) / 2f, control.Position.Y + 6f, 54f, control.Size.Y - 12f);

            DrawRect(pill, on ? new Color("4caf50") : new Color("c62f2a"));
            DrawRect(pill, Style.PanelEdge, filled: false, width: 1f);

            this.DrawText(
                new Vector2(pill.Position.X + (pill.Size.X - Style.Measure(text, Style.FontTag)) / 2f,
                    pill.Position.Y + pill.Size.Y - 5f), text, Style.FontTag, Style.Text);
        }

        private void DrawChoice(Rect2 control, string value)
        {
            this.DrawText(new Vector2(control.Position.X + 12f, control.Position.Y + 20f),
                value, Style.FontBody, Style.Text);

            // The triangle at the right edge, which is what says the value can be changed.
            var at = new Vector2(control.End.X - 20f, control.Position.Y + control.Size.Y / 2f - 2f);
            DrawColoredPolygon(
                new[] { at, at + new Vector2(10f, 0f), at + new Vector2(5f, 6f) }, Style.TextDim);
        }

        private void DrawSlider(Rect2 control, float value)
        {
            var track = SliderTrack(control);

            DrawRect(track, Style.BarTrack);
            DrawRect(track, Style.BarEdge, filled: false, width: 1f);

            var handle = new Rect2(
                Mathf.Round(track.Position.X + (track.Size.X - 10f) * Mathf.Clamp(value, 0f, 1f)),
                control.Position.Y + 6f, 10f, control.Size.Y - 12f);

            DrawRect(handle, Style.Text);
            DrawRect(handle, Style.PanelEdge, filled: false, width: 1f);

            string percent = $"{Mathf.RoundToInt(value * 100f)}%";
            this.DrawText(new Vector2(control.End.X - 8f - Style.Measure(percent, Style.FontSmall),
                control.Position.Y + 20f), percent, Style.FontSmall, Style.Text);
        }

        private void DrawKey(Rect2 control, Row row, bool listening)
        {
            string text = listening ? "press a key…" : KeyBindings.BoundTo(row.Action);

            var colour = listening
                ? Style.FameFill
                : KeyBindings.IsMouse(row.Action) ? Style.TextDim
                : KeyBindings.IsChanged(row.Action) ? Style.TierSpecial
                : Style.Text;

            this.DrawText(
                new Vector2(control.Position.X + (control.Size.X - Style.Measure(text, Style.FontBody)) / 2f,
                    control.Position.Y + 20f), text, Style.FontBody, colour);
        }
    }
}
