using System;
using System.Collections.Generic;
using System.Globalization;
using Godot;
using Hendra.Assets;
using Hendra.Resources;
using Hendra.World;

namespace Hendra.UI;

/// <summary>
/// The in-game panel: vital bars, stats, equipment and inventory.
/// </summary>
/// <remarks>
/// <para>
/// Laid out down the right-hand edge like the original, and built from ordinary Control nodes. The
/// original drew all of this by hand into bitmaps — every bar, every slot border, every number —
/// and re-rasterised text whenever a value changed. Here the layout containers do the work and the
/// panel resizes with the window instead of assuming an 800x600 stage.
/// </para>
/// <para>
/// Reads straight off the player entity each frame rather than subscribing to change notifications.
/// The values arrive as stat deltas many times a second anyway, so a pull is both simpler and
/// fewer moving parts than the signal-per-stat arrangement it replaces.
/// </para>
/// </remarks>
public partial class HudView : Control
{
    /// <summary>
    /// Width of the column, in pixels.
    /// </summary>
    /// <remarks>
    /// The original's: its stage was eight hundred wide and the HUD started at six hundred. The
    /// slot size follows from it — four across with margins between.
    /// </remarks>
    private const int PanelWidth = 200;
    /// <summary>Vertical space the minimap occupies at the top of the column.</summary>
    private const int MinimapAllowance = 196;

    private const int SlotSize = 40;
    private const int SlotsPerRow = 4;

    /// <summary>Worn equipment, which the character screen shows on its own row.</summary>
    private const int EquipmentSlots = 4;

    /// <summary>Carried items, the backpack aside.</summary>
    private const int InventorySlots = 8;

    /// <summary>The extra carried slots a backpack grants, at indices 16 to 23.</summary>
    private const int BackpackSlots = 8;

    /// <summary>Loot bags carry eight; vaults carry more, but eight is what fits the panel.</summary>
    private const int ContainerSlots = 8;

    private readonly List<SlotView> _equipment = new();
    private readonly List<SlotView> _inventory = new();
    private readonly List<SlotView> _container = new();
    private readonly List<SlotView> _backpack = new();

    /// <summary>The backpack section, shown only once the character owns one.</summary>
    private Control _backpackPanel;

    private Control _inventoryPanel;
    private HBoxContainer _tabs;
    private Button _inventoryTab;
    private Button _statsTab;
    private Button _backpackTab;
    private CutEdgePanel _statsPanel;
    private StatRow[] _statRows;

    /// <summary>
    /// Which page the strip over the carried grids is showing.
    /// </summary>
    /// <remarks>
    /// The original's TabStripModel names four -- Main Inventory, Stats, Backpack and Pets. Pets
    /// are not in this fork, so the strip carries the other three.
    /// </remarks>
    private enum Page { Inventory, Stats, Backpack }

    private Page _page = Page.Inventory;

    /// <summary>Which of the two carried grids the strip is showing.</summary>

    /// <summary>Raised with the slot's index in the player's 24-entry equipment array.</summary>
    public event Action<int> SlotActivated;

    /// <summary>Raised with the slot's index in the open container.</summary>
    public event Action<int> ContainerSlotActivated;

    /// <summary>Raised when an item is dragged from one slot onto another.</summary>
    public event Action<World.SlotAddress, World.SlotAddress> SlotDropped;

    /// <summary>Raised when the buy button is pressed at a vendor.</summary>
    public event Action BuyPressed;

    private VitalBar _health;
    private VitalBar _mana;
    private Label _name;
    private VitalBar _level;
    private Label _levelLabel;
    private Label _guild;
    private HBoxContainer _stars;
    private Label _fame;
    private Label _gold;
    private HBoxContainer _panelButtons;
    private Control _healthPotions;
    private Control _manaPotions;
    private Control _interactions;

    /// <summary>Raised by the buttons in the identity panel.</summary>
    public event Action OptionsPressed;

    public event Action GuildPressed;

    public event Action NexusPressed;

    /// <summary>The potion slots, which the wire numbers just past the backpack.</summary>
    private const int HealthPotionSlot = 254;

    private const int MagicPotionSlot = 255;

    /// <summary>The minimap's size, which the corner groups arrange themselves around.</summary>
    private const int MinimapSize = 192;
    private Label _prompt;
    private VBoxContainer _containerPanel;
    private VBoxContainer _merchantPanel;
    private SlotView _merchandise;
    private Label _price;
    private Button _buy;
    private VBoxContainer _party;
    private VBoxContainer _partyPanel;
    private Label _containerName;

    private AssetLibrary _assets;
    private GameData _data;
    private TextureResolver _textures;

    public void Configure(AssetLibrary assets, GameData data)
    {
        _assets = assets;
        _data = data;
        _textures = new TextureResolver(assets);
    }

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;
        this.FillScreen();

        BuildPlayerPanel();
        BuildCurrencies();
        BuildNearby();
        BuildVitals();
        BuildCarried();
        BuildInteractions();
        BuildPrompt();

        ShowTab(Page.Inventory);
    }

    /// <summary>
    /// Top left: who you are, and the buttons that open the panels about you.
    /// </summary>
    /// <remarks>
    /// The identity block. Name, star rating, guild and progress to the next level, in the corner
    /// furthest from the action, because it is the part you read between fights rather than during
    /// one.
    /// </remarks>
    private void BuildPlayerPanel()
    {
        var panel = Corner(LayoutPreset.TopLeft, new Vector2(12, 12));
        panel.CustomMinimumSize = new Vector2(228, 0);

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 4);
        panel.AddChild(column);

        var heading = new HBoxContainer();
        heading.AddThemeConstantOverride("separation", 8);
        column.AddChild(heading);

        var names = new VBoxContainer { SizeFlagsHorizontal = SizeFlags.ExpandFill };
        names.AddThemeConstantOverride("separation", 0);
        heading.AddChild(names);

        _name = new Label { Text = "—" };
        _name.AddThemeFontSizeOverride("font_size", 18);
        names.AddChild(_name);

        _guild = new Label { Text = string.Empty };
        _guild.AddThemeFontSizeOverride("font_size", 12);
        _guild.AddThemeColorOverride("font_color", Style.Good);
        names.AddChild(_guild);

        _stars = new HBoxContainer { SizeFlagsVertical = SizeFlags.ShrinkCenter };
        _stars.AddThemeConstantOverride("separation", 1);
        heading.AddChild(_stars);

        // Level and progress on one line: the number matters, the bar behind it is the detail.
        var levelRow = new HBoxContainer();
        levelRow.AddThemeConstantOverride("separation", 6);
        column.AddChild(levelRow);

        _levelLabel = new Label { Text = "Lvl —" };
        _levelLabel.AddThemeFontSizeOverride("font_size", 13);
        _levelLabel.AddThemeColorOverride("font_color", Style.Muted);
        levelRow.AddChild(_levelLabel);

        _level = new VitalBar(new Color("5a8025")) { SizeFlagsHorizontal = SizeFlags.ExpandFill };
        levelRow.AddChild(_level);

        _panelButtons = new HBoxContainer();
        _panelButtons.AddThemeConstantOverride("separation", 4);
        column.AddChild(_panelButtons);

        _panelButtons.AddChild(IconButton("Stats", "Character stats", () => ShowTab(Page.Stats)));
        _panelButtons.AddChild(IconButton("Guild", "Guild", () => GuildPressed?.Invoke()));

        _panelButtons.AddChild(new Control { SizeFlagsHorizontal = SizeFlags.ExpandFill });
        _panelButtons.AddChild(IconButton("Options", "Options", () => OptionsPressed?.Invoke()));
    }

    /// <summary>Top right: fame and gold, over the minimap.</summary>
    private void BuildCurrencies()
    {
        var row = new HBoxContainer();
        row.AddThemeConstantOverride("separation", 14);
        row.SetAnchorsPreset(LayoutPreset.TopRight);
        row.MouseFilter = MouseFilterEnum.Ignore;

        // Left of the minimap rather than under it, so a wide number grows away from the screen
        // edge instead of into it.
        row.OffsetLeft = -MinimapSize - 210;
        row.OffsetRight = -MinimapSize - 14;
        row.OffsetTop = 14;
        row.OffsetBottom = 40;
        row.Alignment = BoxContainer.AlignmentMode.End;
        AddChild(row);

        _fame = Currency(row, Style.Gold.Lerp(Style.Danger, 0.4f));
        _gold = Currency(row, Style.Gold);
    }

    private Label Currency(Control parent, Color colour)
    {
        var group = new HBoxContainer { MouseFilter = MouseFilterEnum.Ignore };
        group.AddThemeConstantOverride("separation", 5);
        parent.AddChild(group);

        var amount = new Label
        {
            Text = "0",
            HorizontalAlignment = HorizontalAlignment.Right,
            MouseFilter = MouseFilterEnum.Ignore,
        };
        amount.AddThemeFontSizeOverride("font_size", 17);
        group.AddChild(amount);

        group.AddChild(new CurrencyPip(colour) { MouseFilter = MouseFilterEnum.Ignore });
        return amount;
    }

    /// <summary>Under the minimap: who else is nearby.</summary>
    private void BuildNearby()
    {
        _partyPanel = new VBoxContainer { Visible = false, MouseFilter = MouseFilterEnum.Ignore };
        _partyPanel.AddThemeConstantOverride("separation", 1);
        _partyPanel.SetAnchorsPreset(LayoutPreset.TopRight);
        _partyPanel.OffsetLeft = -MinimapSize - 14;
        _partyPanel.OffsetRight = -14;
        _partyPanel.OffsetTop = MinimapSize + 24;
        AddChild(_partyPanel);

        _party = new VBoxContainer { MouseFilter = MouseFilterEnum.Ignore };
        _party.AddThemeConstantOverride("separation", 1);
        _partyPanel.AddChild(_party);
    }

    /// <summary>
    /// Bottom centre: health, mana, the potions that refill them, and the way home.
    /// </summary>
    /// <remarks>
    /// The things you look at while something is hitting you, put where the eye already is -- under
    /// the character, not off in a corner. The potion counts sit against their own bars because
    /// that is the pair you check together.
    /// </remarks>
    private void BuildVitals()
    {
        var panel = Corner(LayoutPreset.CenterBottom, new Vector2(0, -14));
        panel.CustomMinimumSize = new Vector2(520, 0);

        var row = new HBoxContainer();
        row.AddThemeConstantOverride("separation", 8);
        panel.AddChild(row);

        var bars = new VBoxContainer { SizeFlagsHorizontal = SizeFlags.ExpandFill };
        bars.AddThemeConstantOverride("separation", 4);
        row.AddChild(bars);

        _health = new VitalBar(new Color("c83c3c"));
        _health.CustomMinimumSize = new Vector2(0, 22);
        bars.AddChild(_health);

        _mana = new VitalBar(new Color("5a7fd0"));
        _mana.CustomMinimumSize = new Vector2(0, 22);
        bars.AddChild(_mana);

        var potions = new VBoxContainer { SizeFlagsVertical = SizeFlags.ShrinkCenter };
        potions.AddThemeConstantOverride("separation", 4);
        row.AddChild(potions);

        // Buttons rather than slots. This fork sends no potion count -- the server tracks how many
        // you hold and the client only ever asks it to drink one -- so a square showing a number
        // would be showing a number nobody sent.
        _healthPotions = PotionButton("F", "Drink a health potion", new Color("c83c3c"), health: true);
        potions.AddChild(_healthPotions);

        _manaPotions = PotionButton("V", "Drink a magic potion", new Color("5a7fd0"), health: false);
        potions.AddChild(_manaPotions);

        // The way out, next to the bars, because reaching for it is the same reflex as watching
        // them. Labelled with the key that does the same thing.
        var nexus = new GameButton($"Nexus  [{NexusKey()}]", compact: true)
        {
            SizeFlagsVertical = SizeFlags.ShrinkCenter,
            CustomMinimumSize = new Vector2(96, 48),
        };
        nexus.Pressed += () => NexusPressed?.Invoke();
        row.AddChild(nexus);
    }

    /// <summary>Bottom right: what you are carrying and what you are wearing.</summary>
    private void BuildCarried()
    {
        var panel = Corner(LayoutPreset.BottomRight, new Vector2(-12, -14));

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 5);
        panel.AddChild(column);

        _tabs = new HBoxContainer();
        _tabs.AddThemeConstantOverride("separation", 4);
        column.AddChild(_tabs);

        _inventoryTab = AddTab("Inventory", Page.Inventory);
        _statsTab = AddTab("Stats", Page.Stats);
        _backpackTab = AddTab("Backpack", Page.Backpack);

        _inventoryPanel = new VBoxContainer();
        column.AddChild(_inventoryPanel);
        AddSlots(_inventoryPanel, _inventory, InventorySlots, firstIndex: 8);

        _statsPanel = NewSectionPanel();
        _statsPanel.Visible = false;
        column.AddChild(_statsPanel);
        BuildStatsPage(SectionBody(_statsPanel));

        _backpackPanel = new VBoxContainer { Visible = false };
        column.AddChild(_backpackPanel);
        AddSlots(_backpackPanel, _backpack, BackpackSlots, firstIndex: 16);

        column.AddChild(new HSeparator());

        // Worn, under carried: the four you are using, beneath the eight you are holding.
        var equipment = new HBoxContainer();
        equipment.AddThemeConstantOverride("separation", 4);
        column.AddChild(equipment);

        for (int i = 0; i < EquipmentSlots; i++)
        {
            int index = i;
            var slot = NewSlot(new World.SlotAddress(World.SlotOwner.Player, index), SlotSize);
            slot.Activated += () => SlotActivated?.Invoke(index);
            equipment.AddChild(slot);
            _equipment.Add(slot);
        }
    }

    /// <summary>What is at the player's feet: a container's contents, or a vendor's wares.</summary>
    private void BuildInteractions()
    {
        var panel = Corner(LayoutPreset.BottomRight, new Vector2(-12, -270));

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 5);
        panel.AddChild(column);

        _containerPanel = new VBoxContainer { Visible = false };
        column.AddChild(_containerPanel);

        _containerName = new Label { Text = "Contents" };
        _containerPanel.AddChild(_containerName);
        AddContainerSlots(_containerPanel, ContainerSlots);

        _merchantPanel = new VBoxContainer { Visible = false };
        column.AddChild(_merchantPanel);
        _merchantPanel.AddChild(new Label { Text = "For sale" });

        var row = new HBoxContainer();
        row.AddThemeConstantOverride("separation", 8);
        _merchantPanel.AddChild(row);

        _merchandise = NewSlot(default, SlotSize);
        row.AddChild(_merchandise);

        _price = new Label { VerticalAlignment = VerticalAlignment.Center };
        row.AddChild(_price);

        _buy = new GameButton("Buy", compact: true);
        _buy.Pressed += () => BuyPressed?.Invoke();
        _merchantPanel.AddChild(_buy);

        _interactions = panel;
    }

    private void BuildPrompt()
    {
        // Over the world rather than in a panel, because it refers to something in front of the
        // player rather than to their own state.
        _prompt = new Label
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            Visible = false,
        };
        _prompt.SetAnchorsPreset(LayoutPreset.CenterBottom);
        _prompt.OffsetTop = -122;
        _prompt.OffsetLeft = -200;
        _prompt.OffsetRight = 200;
        _prompt.OffsetBottom = -100;
        AddChild(_prompt);
    }

    /// <summary>A panel pinned to one corner of the screen, in the game's shape.</summary>
    private CornerPanel Corner(LayoutPreset preset, Vector2 inset)
    {
        var panel = new CornerPanel(preset, inset)
        {
            MouseFilter = MouseFilterEnum.Stop,
            Background = CutEdgePanel.PanelBackground with { A = 0.88f },
        };
        panel.Cuts(true, true, true, true).Padded(8);
        panel.Border = Style.Edge with { A = 0.5f };

        AddChild(panel);
        return panel;
    }

    /// <summary>
    /// A panel that sizes itself to what it holds and stays pinned to its corner.
    /// </summary>
    /// <remarks>
    /// Anchors alone do not size a control. A preset pins the edges but leaves the rectangle
    /// wherever it was, so a panel built this way starts at zero by zero and its contents pile up
    /// on top of each other outside it. KeepMinsize is the mode that means "as big as its contents
    /// and no bigger", and it has to be re-applied whenever those contents change size -- which is
    /// every time a tab is switched or a chest is opened.
    /// </remarks>
    private sealed partial class CornerPanel : CutEdgePanel
    {
        private readonly LayoutPreset _preset;
        private readonly Vector2 _inset;

        public CornerPanel(LayoutPreset preset, Vector2 inset)
        {
            _preset = preset;
            _inset = inset;
        }

        public override void _Ready() => Pin();

        public override void _Notification(int what)
        {
            base._Notification(what);

            if (what == NotificationSortChildren || what == NotificationResized)
                CallDeferred(nameof(Pin));
        }

        private void Pin()
        {
            var wanted = GetCombinedMinimumSize();
            if (wanted.X <= 0f || wanted.Y <= 0f)
                return;

            SetAnchorsAndOffsetsPreset(_preset, LayoutPresetMode.KeepSize);

            Size = wanted;
            Position = Corner(wanted) + _inset;
        }

        /// <summary>Where the panel's top-left goes for the corner it is pinned to.</summary>
        private Vector2 Corner(Vector2 size)
        {
            var screen = GetParentAreaSize();

            float x = _preset switch
            {
                LayoutPreset.TopRight or LayoutPreset.BottomRight => screen.X - size.X,
                LayoutPreset.CenterBottom or LayoutPreset.CenterTop => (screen.X - size.X) / 2f,
                _ => 0f,
            };

            float y = _preset switch
            {
                LayoutPreset.BottomLeft or LayoutPreset.BottomRight or LayoutPreset.CenterBottom =>
                    screen.Y - size.Y,
                _ => 0f,
            };

            return new Vector2(x, y);
        }
    }

    private SlotView NewSlot(World.SlotAddress address, int size)
    {
        var slot = new SlotView
        {
            CustomMinimumSize = new Vector2(size, size),
            Address = address,
            Draggable = true,
        };

        slot.Dropped += (from, to) => SlotDropped?.Invoke(from, to);
        return slot;
    }

    /// <summary>
    /// A potion button, sitting against the bar it refills.
    /// </summary>
    /// <remarks>
    /// Labelled with the key that does the same thing, because that is how it is actually used --
    /// the button is for the first hour, the key is for every hour after.
    /// </remarks>
    private Control PotionButton(string key, string tooltip, Color colour, bool health)
    {
        var button = new GameButton(key, compact: true)
        {
            TooltipText = tooltip,
            CustomMinimumSize = new Vector2(34, 22),
        };

        button.Pressed += () => PotionRequested?.Invoke(health);
        return button;
    }

    /// <summary>Raised when a potion button is pressed, with true for health.</summary>
    public event Action<bool> PotionRequested;

    /// <summary>A small square button that opens a panel.</summary>
    private static GameButton IconButton(string label, string tooltip, Action pressed)
    {
        // An explicit width. GameButton draws its own label, so the base class measures an empty
        // string and reports a button no wider than its padding -- and a row of those lands every
        // one of them on the same spot.
        var button = new GameButton(label, compact: true)
        {
            TooltipText = tooltip,
            CustomMinimumSize = new Vector2(78, 26),
        };
        button.Pressed += pressed;
        return button;
    }

    /// <summary>The key that returns to the Nexus, as it is currently bound.</summary>
    private static string NexusKey()
    {
        foreach (var bound in InputMap.ActionGetEvents("nexus"))
        {
            if (bound is InputEventKey key)
                return OS.GetKeycodeString(key.PhysicalKeycode != Key.None ? key.PhysicalKeycode : key.Keycode);
        }

        return "R";
    }

    /// <summary>The lozenge beside a currency, standing in for its icon.</summary>
    private sealed partial class CurrencyPip : Control
    {
        private readonly Color _colour;

        public CurrencyPip(Color colour)
        {
            _colour = colour;
            CustomMinimumSize = new Vector2(14, 14);
        }

        public override void _Draw()
        {
            var centre = Size / 2f;
            float radius = Mathf.Min(Size.X, Size.Y) / 2f;

            DrawCircle(centre, radius, _colour);
            DrawCircle(centre - new Vector2(0f, radius * 0.3f), radius * 0.45f,
                Colors.White with { A = 0.35f });
        }
    }

    /// <summary>Shows what pressing the interact key would do, or hides the prompt when null.</summary>
    public void ShowPrompt(string label)
    {
        if (_prompt == null)
            return;

        _prompt.Visible = !string.IsNullOrEmpty(label);
        if (_prompt.Visible)
            _prompt.Text = $"[{InteractKey()}] {label}";
    }

    private void AddContainerSlots(Control parent, int count)
    {
        var grid = new GridContainer { Columns = SlotsPerRow };
        grid.AddThemeConstantOverride("h_separation", 4);
        grid.AddThemeConstantOverride("v_separation", 4);
        parent.AddChild(grid);

        for (int i = 0; i < count; i++)
        {
            int slotIndex = i;
            var slot = new SlotView
            {
                CustomMinimumSize = new Vector2(SlotSize, SlotSize),
                Address = new World.SlotAddress(World.SlotOwner.Container, slotIndex),
                Draggable = true,
            };
            slot.Activated += () => ContainerSlotActivated?.Invoke(slotIndex);
            slot.Dropped += (from, to) => SlotDropped?.Invoke(from, to);
            grid.AddChild(slot);
            _container.Add(slot);
        }
    }

    /// <summary>Shows a container's contents, or hides the panel when given null.</summary>
    public void ShowContainer(Entity container)
    {
        if (_containerPanel == null)
            return;

        _containerPanel.Visible = container != null;
        if (container == null)
            return;

        // What the data calls it, not the Name stat -- a vault chest's Name is how full it is, and
        // "0/8" is a fine thing to write over the chest but not a title for the panel.
        var chest = _data?.GetObject(container.ObjectType);
        string name = chest?.DisplayId ?? chest?.Id;

        _containerName.Text = string.IsNullOrEmpty(name) ? "Contents" : name;

        for (int i = 0; i < _container.Count; i++)
        {
            int type = container.Equipment != null && i < container.Equipment.Length
                ? container.Equipment[i]
                : -1;

            if (type < 0)
            {
                _container[i].SetItem(default, null, _data);
                continue;
            }

            var desc = _data?.GetObject((ushort)type);
            var resolved = _textures?.Resolve(desc?.Texture) ?? default;
            _container[i].SetItem(resolved.Still, desc, _data);
        }
    }

    /// <summary>Lists the nearby players.</summary>
    public void ShowParty(IReadOnlyList<PartyMember> members)
    {
        if (_party == null)
            return;

        _partyPanel.Visible = members.Count > 0;
        if (members.Count == 0)
            return;

        // Few enough entries, changing seldom enough, that rebuilding the rows is simpler than
        // pooling them.
        foreach (var child in _party.GetChildren())
            child.QueueFree();

        foreach (var member in members)
        {
            float fraction = member.MaxHp > 0 ? member.Hp / (float)member.MaxHp : 0f;

            var row = new HBoxContainer();
            row.AddThemeConstantOverride("separation", 6);

            var name = new Label
            {
                Text = $"{(member.Starred ? "★ " : string.Empty)}{member.Name}",
                SizeFlagsHorizontal = SizeFlags.ExpandFill,
                ClipText = true,
            };
            name.AddThemeFontSizeOverride("font_size", 13);
            row.AddChild(name);

            // Red as it falls, so a name worth reacting to stands out without being read.
            var health = new Label
            {
                Text = $"{(int)(fraction * 100)}%",
                HorizontalAlignment = HorizontalAlignment.Right,
            };
            health.AddThemeFontSizeOverride("font_size", 13);
            health.AddThemeColorOverride("font_color", fraction < 0.35f
                ? new Color(0.95f, 0.35f, 0.35f)
                : new Color(0.72f, 0.72f, 0.72f));
            row.AddChild(health);

            _party.AddChild(row);
        }
    }

    /// <summary>Shows what a vendor is selling, or hides the panel when given null.</summary>
    public void ShowMerchant(Entity merchant, LocalPlayer player)
    {
        if (_merchantPanel == null)
            return;

        _merchantPanel.Visible = merchant is { MerchandiseType: >= 0 };
        if (!_merchantPanel.Visible)
            return;

        var desc = _data?.GetObject((ushort)merchant.MerchandiseType);
        var resolved = _textures?.Resolve(desc?.Texture) ?? default;
        _merchandise.SetItem(resolved.Still, desc, _data);

        // Currency zero is gold; anything else is fame on this server build.
        bool fame = merchant.MerchandiseCurrency != 0;
        string stock = merchant.MerchandiseCount >= 0 ? $"  ({merchant.MerchandiseCount} left)" : string.Empty;
        _price.Text = $"{desc?.DisplayId ?? desc?.Id}\n{merchant.MerchandisePrice} {(fame ? "fame" : "gold")}{stock}";

        int purse = player == null ? 0 : fame ? player.Fame : player.Credits;
        _buy.Disabled = purse < merchant.MerchandisePrice;
    }

    /// <summary>
    /// A sub-panel in the column: the original's cut-corner background with a heading over it.
    /// </summary>
    private Control AddSection(Control parent, string heading)
    {
        parent.AddChild(new Label { Text = heading });

        var panel = NewSectionPanel();
        parent.AddChild(panel);
        return SectionBody(panel);
    }

    /// <summary>
    /// An empty sub-panel background.
    /// </summary>
    /// <remarks>
    /// All four corners cut, unlike the column itself: these sit inside it rather than against the
    /// edge of the screen, so every corner is visible and should be shaped.
    /// </remarks>
    private static CutEdgePanel NewSectionPanel()
    {
        var panel = new CutEdgePanel { Background = SectionBackground };
        panel.Padded(4);

        var body = new VBoxContainer();
        body.AddThemeConstantOverride("separation", 4);
        panel.AddChild(body);
        return panel;
    }

    /// <summary>The container inside a sub-panel that its contents go into.</summary>
    private static Control SectionBody(CutEdgePanel panel) => panel.GetChild<Control>(0);

    /// <summary>
    /// A shade lighter than the column, so a grid reads as sitting on the column rather than in it.
    /// </summary>
    /// <summary>
    /// The key that works the thing in front of the player, as it is currently bound.
    /// </summary>
    /// <remarks>
    /// Read from the input map rather than written into the sentence. The prompt used to say R,
    /// which is Nexus -- pressing it did leave, so the prompt was not only wrong but actively
    /// misleading. Asking the map means it cannot drift from the binding again.
    /// </remarks>
    private static string InteractKey()
    {
        foreach (var bound in InputMap.ActionGetEvents("interact"))
        {
            if (bound is InputEventKey key)
                return OS.GetKeycodeString(key.PhysicalKeycode != Key.None ? key.PhysicalKeycode : key.Keycode);
        }

        return "?";
    }

    private static readonly Color SectionBackground = new(0.10f, 0.095f, 0.095f);

    /// <summary>The level cap, past which there is no experience left to earn.</summary>
    private const int MaxLevel = 20;

    /// <summary>The original's bar colours, taken from StatMetersView.</summary>
    private static readonly Color ExperienceBar = new("5a8025");

    private static readonly Color FameBar = new("e25f00");

    /// <summary>One tab in the strip over the carried grids.</summary>
    private Button AddTab(string text, Page page)
    {
        var tab = new Button
        {
            Text = text,
            ToggleMode = true,
            SizeFlagsHorizontal = SizeFlags.ExpandFill,
            ButtonPressed = page == Page.Inventory,

            // Click only. A focused toggle would swallow Enter, which belongs to the chat box.
            FocusMode = FocusModeEnum.None,
        };

        tab.Pressed += () => ShowTab(page);
        _tabs.AddChild(tab);
        return tab;
    }

    /// <summary>
    /// Steps to the next page. Bound to B, as the original bound it.
    /// </summary>
    /// <remarks>
    /// Skips the backpack for a character without one, so the key never lands on a page that is
    /// not there.
    /// </remarks>
    public void SwitchTab()
    {
        var next = _page switch
        {
            Page.Inventory => Page.Stats,
            Page.Stats => _backpackTab.Visible ? Page.Backpack : Page.Inventory,
            _ => Page.Inventory,
        };

        ShowTab(next);
    }

    private void ShowTab(Page page)
    {
        _page = page;

        _inventoryPanel.Visible = page == Page.Inventory;
        _statsPanel.Visible = page == Page.Stats;
        _backpackPanel.Visible = page == Page.Backpack;

        _inventoryTab.ButtonPressed = page == Page.Inventory;
        _statsTab.ButtonPressed = page == Page.Stats;
        _backpackTab.ButtonPressed = page == Page.Backpack;
    }

    /// <summary>
    /// The six stats, two to a row, the way the original's StatsView lays them out.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Attack, Defense, Speed, Dexterity, Vitality, Wisdom -- in that order, which is the original's
    /// order and not alphabetical or the order they arrive in. MaxHP and MaxMP are left out because
    /// their bars are already above; the original does the same.
    /// </para>
    /// <para>
    /// Labelled with the abbreviation rather than the name. That is what the original shows -- its
    /// StatView takes a short form for the label and keeps the full name and the description for
    /// the tooltip -- and it is also the only thing that fits: two columns inside a column this
    /// narrow leaves under a hundred pixels for a name, a value and a boost.
    /// </para>
    /// </remarks>
    private void BuildStatsPage(Control parent)
    {
        var grid = new GridContainer { Columns = 2 };
        grid.AddThemeConstantOverride("h_separation", 8);
        grid.AddThemeConstantOverride("v_separation", 3);
        parent.AddChild(grid);

        _statRows = new StatRow[StatKeys.Length];
        for (int i = 0; i < StatKeys.Length; i++)
        {
            _statRows[i] = new StatRow(StatKeys[i].Abbreviation, StatKeys[i].Name);
            grid.AddChild(_statRows[i]);
        }
    }

    /// <summary>
    /// The six stats, with the keys their words come from and the words to use until they arrive.
    /// </summary>
    /// <remarks>
    /// The language table is fetched over HTTP once the world starts, so it is usually absent when
    /// the page is first built and always absent offline. The English it would supply is the
    /// fallback, so the page reads correctly either way and simply gets more precise once the table
    /// lands.
    /// </remarks>
    private static readonly (string Key, string Abbreviation, string Name)[] StatKeys =
    {
        ("attack", "ATT", "Attack"),
        ("defense", "DEF", "Defense"),
        ("speed", "SPD", "Speed"),
        ("dexterity", "DEX", "Dexterity"),
        ("vitality", "VIT", "Vitality"),
        ("wisdom", "WIS", "Wisdom"),
    };

    private bool _statsLocalised;

    /// <summary>
    /// Replaces the stat labels with the language table's words, once it has one.
    /// </summary>
    /// <remarks>
    /// Done once rather than every frame. StringMap answers an unknown key with the key itself, so
    /// every lookup is guarded -- an unguarded one would put "StatModel.attack.long" in a tooltip.
    /// </remarks>
    private void LocaliseStats()
    {
        var strings = App.ServiceLocator.Strings;
        if (_statsLocalised || _statRows == null || strings.Count == 0)
            return;

        _statsLocalised = true;

        for (int i = 0; i < _statRows.Length && i < StatKeys.Length; i++)
        {
            string key = StatKeys[i].Key;
            string name = Localised(strings, $"StatModel.{key}.long") ?? StatKeys[i].Name;
            string description = Localised(strings, $"StatModel.{key}.description");

            _statRows[i].Relabel(
                Localised(strings, $"StatModel.{key}.short") ?? StatKeys[i].Abbreviation,
                description == null ? name : $"{name}\n{description}");
        }
    }

    private static string Localised(Text.StringMap strings, string key) =>
        strings.Has(key) ? strings.Get(key) : null;

    private void AddSlots(Control parent, List<SlotView> into, int count, int firstIndex)
    {
        var grid = new GridContainer { Columns = SlotsPerRow };
        grid.AddThemeConstantOverride("h_separation", 4);
        grid.AddThemeConstantOverride("v_separation", 4);
        parent.AddChild(grid);

        for (int i = 0; i < count; i++)
        {
            int slotIndex = firstIndex + i;
            var slot = new SlotView
            {
                CustomMinimumSize = new Vector2(SlotSize, SlotSize),
                Address = new World.SlotAddress(World.SlotOwner.Player, slotIndex),
                Draggable = true,
            };
            slot.Activated += () => SlotActivated?.Invoke(slotIndex);
            slot.Dropped += (from, to) => SlotDropped?.Invoke(from, to);
            grid.AddChild(slot);
            into.Add(slot);
        }
    }

    /// <summary>Refreshes from the player. Safe to call every frame; does nothing without one.</summary>
    public void Refresh(LocalPlayer player)
    {
        if (player == null)
            return;

        _name.Text = string.IsNullOrEmpty(player.Name) ? "—" : player.Name;

        RefreshIdentity(player);
        RefreshLevel(player);
        _health.Set(player.Hp, player.MaxHp, Vital(player.Hp, player.MaxHp, player.Boosts[0]));
        _mana.Set(player.Mp, player.MaxMp, Vital(player.Mp, player.MaxMp, player.Boosts[1]));

        RefreshStats(player);

        // Slots 0-3 are worn, 4-7 continue the worn row in the data model but the original shows
        // only the first four as equipment; 8 onward is the carried inventory.
        UpdateSlots(_equipment, player, 0);
        UpdateSlots(_inventory, player, 8);

        // The tab disappears with the bag, and takes the view back to the inventory with it.
        _backpackTab.Visible = player.HasBackpack;
        if (!player.HasBackpack && _page == Page.Backpack)
            ShowTab(Page.Inventory);

        if (player.HasBackpack)
            UpdateSlots(_backpack, player, 16);
    }

    /// <summary>
    /// The level bar, or the fame bar once there is no level left to earn.
    /// </summary>
    /// <remarks>
    /// Twenty is the cap. The original swaps the bar there rather than leaving a full one sitting
    /// at the top of the column, because fame is what the character is working toward from then on.
    /// </remarks>
    /// <summary>Guild, star rating, and the two currencies.</summary>
    private void RefreshIdentity(LocalPlayer player)
    {
        _guild.Text = player.Guild ?? string.Empty;
        _guild.Visible = !string.IsNullOrEmpty(player.Guild);

        _fame.Text = player.Fame.ToString("N0", CultureInfo.InvariantCulture);
        _gold.Text = player.Credits.ToString("N0", CultureInfo.InvariantCulture);

        // The character's own stars, from its fame, on the original's thresholds. Rebuilt only when
        // the count changes: it changes a handful of times in a character's life.
        int stars = Fame.Stars(player.Fame);
        if (stars == _shownStars)
            return;

        _shownStars = stars;
        foreach (var child in _stars.GetChildren())
            child.QueueFree();

        var colour = Fame.Colour(stars, 1);
        for (int i = 0; i < stars; i++)
            _stars.AddChild(new StarIcon(colour, 14));
    }

    private int _shownStars = -1;

    private void RefreshLevel(LocalPlayer player)
    {
        if (player.Level < 0)
        {
            _level.Set(0, 0, string.Empty);
            return;
        }

        if (player.Level >= MaxLevel)
        {
            _level.Fill = FameBar;
            _levelLabel.Text = "Fame";
            _level.Set(player.Fame, player.NextClassQuestFame,
                $"{player.Fame:N0} / {player.NextClassQuestFame:N0}");
            return;
        }

        _level.Fill = ExperienceBar;
        _levelLabel.Text = $"Lvl {player.Level}";
        _level.Set(player.Experience, player.NextLevelExperience,
            $"{player.Experience:N0} / {player.NextLevelExperience:N0}");
    }

    private void RefreshStats(LocalPlayer player)
    {
        if (_statRows == null)
            return;

        LocaliseStats();

        // The order the page is built in: Attack, Defense, Speed, Dexterity, Vitality, Wisdom.
        // The boosts and maxima are indexed with MaxHP and MaxMP first, so they run two ahead.
        int[] values = { player.Attack, player.Defense, player.Speed, player.Dexterity, player.Vitality, player.Wisdom };
        var maxima = _data?.GetObject(player.ObjectType)?.StatMaxima;

        for (int i = 0; i < _statRows.Length; i++)
        {
            int at = i + 2;
            _statRows[i].Set(values[i], player.Boosts[at], maxima != null && at < maxima.Length ? maxima[at] : 0);
        }
    }

    /// <summary>
    /// A vital's reading, with what equipment adds to its maximum called out.
    /// </summary>
    /// <remarks>
    /// The boosted part is shown separately rather than folded in, because a maximum that changed
    /// when you swapped a ring is otherwise indistinguishable from one that changed when you
    /// levelled.
    /// </remarks>
    private static string Vital(int current, int maximum, int boost) =>
        boost > 0 ? $"{current} / {maximum} (+{boost})" : $"{current} / {maximum}";

    private void UpdateSlots(List<SlotView> views, LocalPlayer player, int firstIndex)
    {
        for (int i = 0; i < views.Count; i++)
        {
            int index = firstIndex + i;
            int type = player.Equipment != null && index < player.Equipment.Length
                ? player.Equipment[index]
                : -1;

            if (type < 0)
            {
                views[i].SetItem(default, null, _data);
                continue;
            }

            var desc = _data?.GetObject((ushort)type);
            var resolved = _textures?.Resolve(desc?.Texture) ?? default;
            views[i].SetItem(resolved.Still, desc, _data);
        }
    }
}

/// <summary>A labelled bar for health or mana.</summary>
public sealed partial class VitalBar : Control
{
    private const int BarHeight = 18;

    private Color _fill;
    private Label _label;
    private float _fraction;

    /// <summary>Where the bar is drawn, chasing <see cref="_fraction"/>.</summary>
    private float _shown;

    /// <summary>Where it was, falling behind a drop so the loss is visible.</summary>
    private float _ghost;

    /// <summary>The bar's colour. Settable because the level bar becomes the fame bar at the cap.</summary>
    public Color Fill
    {
        get => _fill;
        set { _fill = value; QueueRedraw(); }
    }

    public VitalBar(Color fill)
    {
        _fill = fill;
        CustomMinimumSize = new Vector2(0, BarHeight);
        MouseFilter = MouseFilterEnum.Ignore;
    }

    public override void _Ready()
    {
        _label = new Label
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        };
        _label.SetAnchorsPreset(LayoutPreset.FullRect);
        AddChild(_label);
    }

    public void Set(int current, int maximum)
    {
        _fraction = maximum > 0 ? Mathf.Clamp(current / (float)maximum, 0f, 1f) : 0f;
        if (_label != null)
            _label.Text = $"{current} / {maximum}";
        QueueRedraw();
    }

    /// <summary>
    /// Fills the bar but labels it with something other than the numbers.
    /// </summary>
    /// <remarks>
    /// The original labels its experience bar "Level 7" rather than "340 / 800" -- the number that
    /// matters there is the one you are working toward, not the ratio, and the ratio is the bar.
    /// </remarks>
    public void Set(int current, int maximum, string label)
    {
        _fraction = maximum > 0 ? Mathf.Clamp(current / (float)maximum, 0f, 1f) : 0f;
        if (_label != null)
            _label.Text = label;
        QueueRedraw();
    }

    /// <summary>
    /// Eases the fill towards the value rather than snapping to it.
    /// </summary>
    /// <remarks>
    /// A bar that jumps tells you the number changed; a bar that slides tells you which way and by
    /// how much, which is the thing you actually need while something is hitting you. The ghost
    /// trails behind a drop so a hit leaves a mark you can see after it has landed.
    /// </remarks>
    public override void _Process(double delta)
    {
        float step = (float)delta * 6f;
        _shown = Mathf.MoveToward(_shown, _fraction, step);

        // The ghost catches up slowly on the way down and instantly on the way up.
        _ghost = _ghost < _shown ? _shown : Mathf.MoveToward(_ghost, _shown, step * 0.35f);

        if (!Mathf.IsEqualApprox(_shown, _fraction) || !Mathf.IsEqualApprox(_ghost, _shown))
            QueueRedraw();
    }

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);

        DrawRect(full, new Color(0.06f, 0.055f, 0.07f));

        // What the bar is losing, in a dimmed version of its own colour.
        if (_ghost > _shown)
        {
            DrawRect(new Rect2(0f, 0f, Size.X * _ghost, Size.Y),
                _fill.Lerp(Colors.White, 0.35f) with { A = 0.4f });
        }

        float width = Size.X * _shown;
        if (width > 0f)
        {
            // Lit along the top edge and shaded below, so the bar reads as a surface with a
            // highlight on it rather than as a coloured rectangle.
            DrawRect(new Rect2(0f, 0f, width, Size.Y), _fill.Darkened(0.25f));
            DrawRect(new Rect2(0f, 0f, width, Size.Y * 0.45f), _fill.Lightened(0.12f));
            DrawRect(new Rect2(0f, 0f, width, 1f), _fill.Lightened(0.45f) with { A = 0.8f });
        }

        // Quarter marks, so a glance gives a fraction rather than a length.
        for (int i = 1; i < 4; i++)
        {
            float x = Size.X * i / 4f;
            DrawLine(new Vector2(x, 2f), new Vector2(x, Size.Y - 2f), new Color(0f, 0f, 0f, 0.28f));
        }

        DrawRect(full, new Color(0f, 0f, 0f, 0.65f), filled: false, width: 1f);
    }
}

/// <summary>One inventory or equipment slot.</summary>
public sealed partial class SlotView : Control
{
    private static readonly Color Background = new("15141a");
    private static readonly Color Border = new("46424f");

    /// <summary>Eased towards one while the pointer is over the slot.</summary>
    private float _glow;

    private Assets.Sprite _sprite;
    private Resources.ObjectDesc _desc;
    private Resources.GameData _data;

    /// <summary>Which slot this is, so a drag can name where it came from and where it went.</summary>
    public World.SlotAddress Address { get; set; }

    /// <summary>
    /// Whether this slot takes part in dragging.
    /// </summary>
    /// <remarks>
    /// Off unless a slot has been given a real address. The trade screen uses the same view for
    /// its offers, and a slot there dragging under the default address would move whatever happens
    /// to be in the player's first equipment slot.
    /// </remarks>
    public bool Draggable { get; set; }

    /// <summary>Raised on a left click, whether or not the slot holds anything.</summary>
    public event Action Activated;

    /// <summary>Raised when something is dropped on this slot, with where it came from.</summary>
    public event Action<World.SlotAddress, World.SlotAddress> Dropped;

    /// <summary>
    /// Starts a drag, if there is anything here to drag.
    /// </summary>
    /// <remarks>
    /// Godot only asks for this once the pointer has moved a little way with the button held, which
    /// is the same threshold the original applies before it decides a press was a drag rather than
    /// a click. So the two gestures do not fight: a press that goes nowhere is still a click.
    /// </remarks>
    public override Variant _GetDragData(Vector2 atPosition)
    {
        if (!Draggable || !_sprite.IsValid || _desc == null)
            return default;

        SetDragPreview(new DragPreview(_sprite));

        return new Godot.Collections.Dictionary
        {
            ["hendra_slot"] = true,
            ["owner"] = (int)Address.Owner,
            ["index"] = Address.Index,
        };
    }

    public override bool _CanDropData(Vector2 atPosition, Variant data) =>
        Draggable && IsSlotPayload(data, out _);

    public override void _DropData(Vector2 atPosition, Variant data)
    {
        if (IsSlotPayload(data, out var from))
            Dropped?.Invoke(from, Address);
    }

    private static bool IsSlotPayload(Variant data, out World.SlotAddress address)
    {
        address = default;

        if (data.VariantType != Variant.Type.Dictionary)
            return false;

        var payload = data.AsGodotDictionary();
        if (!payload.ContainsKey("hendra_slot"))
            return false;

        address = new World.SlotAddress(
            (World.SlotOwner)(int)payload["owner"], (int)payload["index"]);
        return true;
    }

    /// <summary>The item riding the cursor while it is being dragged.</summary>
    private sealed partial class DragPreview : Control
    {
        private const int Size = 40;

        private readonly Assets.Sprite _sprite;

        public DragPreview(Assets.Sprite sprite)
        {
            _sprite = sprite;
            CustomMinimumSize = new Vector2(Size, Size);

            // Centred on the pointer, so the item sits under the finger that picked it up.
            Position = new Vector2(-Size / 2f, -Size / 2f);
            MouseFilter = MouseFilterEnum.Ignore;
        }

        public override void _Draw()
        {
            if (!_sprite.IsValid)
                return;

            DrawTextureRectRegion(_sprite.Sheet, new Rect2(0f, 0f, Size, Size), _sprite.Region);
        }
    }

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Stop;

        MouseEntered += QueueRedraw;
        MouseExited += QueueRedraw;
    }

    public override void _Process(double delta)
    {
        float target = _sprite.IsValid && GetGlobalRect().HasPoint(GetGlobalMousePosition()) ? 1f : 0f;
        float eased = Mathf.MoveToward(_glow, target, (float)delta * 8f);

        if (Mathf.IsEqualApprox(eased, _glow))
            return;

        _glow = eased;
        QueueRedraw();
    }

    public override void _GuiInput(InputEvent @event)
    {
        // On release, and only if the press did not turn into a drag. Acting on the press would
        // mean every drag also used the item it picked up. The original draws the same line: it
        // dispatches its click on mouse-up and skips it while a drag is running.
        if (@event is not InputEventMouseButton { Pressed: false, ButtonIndex: MouseButton.Left })
            return;

        if (GetViewport().GuiIsDragging())
            return;

        Activated?.Invoke();
    }

    public void SetItem(Assets.Sprite sprite, Resources.ObjectDesc desc, Resources.GameData data)
    {
        _sprite = sprite;
        _desc = desc;
        _data = data;

        // Godot only asks for a tooltip when this is non-empty, so it stands in for "there is
        // something here to describe". The text itself is never shown -- _MakeCustomTooltip
        // replaces it with the panel.
        TooltipText = desc == null ? string.Empty : " ";
        QueueRedraw();
    }

    /// <summary>
    /// Builds the panel the original shows, in place of Godot's own text tooltip.
    /// </summary>
    /// <remarks>
    /// Built fresh each time rather than kept: the engine frees the control when the tooltip
    /// closes, and an item's description does not change while the pointer is over it.
    /// </remarks>
    public override Control _MakeCustomTooltip(string forText)
    {
        if (_desc == null)
            return null;

        return new ItemTooltipPanel(_desc, _sprite, _data, App.ServiceLocator.Strings);
    }

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);

        // An empty slot is a recess; a full one is a plate with something sitting on it, and it
        // lifts under the pointer. The difference is what makes a grid of them scannable.
        DrawRect(full, _sprite.IsValid
            ? Background.Lightened(0.06f + _glow * 0.10f)
            : Background);

        if (_sprite.IsValid)
            DrawRect(new Rect2(1f, 1f, Size.X - 2f, Size.Y * 0.4f), new Color(1f, 1f, 1f, 0.035f));

        var border = _sprite.IsValid
            ? Border.Lerp(Style.Gold, _glow * 0.8f)
            : Border with { A = 0.55f };

        DrawRect(full, border, filled: false, width: _glow > 0.5f ? 2f : 1f);

        if (!_sprite.IsValid)
            return;

        // Item sprites are tiny and must not be smoothed when blown up to slot size.
        var inset = full.Grow(-4f);
        DrawTextureRectRegion(_sprite.Sheet, inset, _sprite.Region);

        DrawTierTag();
    }

    /// <summary>
    /// The tier, in the corner of the tile.
    /// </summary>
    /// <remarks>
    /// The original's ItemTile puts it at the bottom right with a text outline, and it is how you
    /// read a bag at a glance instead of hovering over every square in it.
    /// </remarks>
    private void DrawTierTag()
    {
        string tag = ItemTooltip.TierTag(_desc);
        if (tag == null)
            return;

        var font = GetThemeDefaultFont();
        const int FontSize = 11;

        var measured = font.GetStringSize(tag, HorizontalAlignment.Left, -1, FontSize);
        var at = new Vector2(Size.X - measured.X - 2f, Size.Y - 3f);

        // Outlined rather than shadowed: it sits over artwork, which can be any colour at all.
        for (int dx = -1; dx <= 1; dx++)
        {
            for (int dy = -1; dy <= 1; dy++)
            {
                if (dx == 0 && dy == 0)
                    continue;

                DrawString(font, at + new Vector2(dx, dy), tag, HorizontalAlignment.Left, -1, FontSize,
                    new Color(0f, 0f, 0f, 0.85f));
            }
        }

        DrawString(font, at, tag, HorizontalAlignment.Left, -1, FontSize,
            tag == "UT" ? new Color("b689f0") : Colors.White);
    }
}

/// <summary>
/// One stat on the stats page: its name, its value, and what equipment is adding to it.
/// </summary>
/// <remarks>
/// The original's StatView colours a stat that has reached its class ceiling, which is the whole
/// point of the page -- it is how you tell at a glance which potions are still worth drinking.
/// </remarks>
public sealed partial class StatRow : HBoxContainer
{
    private static readonly Color Maxed = new("ffd76e");
    private static readonly Color Boosted = new("6fdc6f");

    private string _name;
    private Label _label;
    private Label _value;
    private Label _boost;

    public StatRow(string name, string explains)
    {
        _name = name;
        TooltipText = explains;
        MouseFilter = MouseFilterEnum.Stop;
        AddThemeConstantOverride("separation", 4);

        // Half the column each. Without this the row is as wide as its contents, two of them side
        // by side are wider than the panel, and the boost falls off the edge of the screen.
        SizeFlagsHorizontal = SizeFlags.ExpandFill;
    }

    public override void _Ready()
    {
        _label = new Label { Text = _name };
        _label.AddThemeColorOverride("font_color", new Color(0.66f, 0.66f, 0.66f));
        AddChild(_label);

        // The value takes the slack, so it sits hard against the boost rather than against the name.
        _value = new Label
        {
            HorizontalAlignment = HorizontalAlignment.Right,
            SizeFlagsHorizontal = SizeFlags.ExpandFill,
        };
        AddChild(_value);

        _boost = new Label();
        _boost.AddThemeColorOverride("font_color", Boosted);
        AddChild(_boost);
    }

    /// <summary>Swaps in the language table's words once they have arrived.</summary>
    public void Relabel(string name, string explains)
    {
        _name = name;
        TooltipText = explains;

        if (_label != null)
            _label.Text = name;
    }

    /// <param name="value">The total, which already includes the boost.</param>
    /// <param name="boost">How much of the total comes from equipment.</param>
    /// <param name="maximum">The class ceiling, or zero if it is not known.</param>
    public void Set(int value, int boost, int maximum)
    {
        if (_value == null)
            return;

        _value.Text = value.ToString(System.Globalization.CultureInfo.InvariantCulture);

        // Against the ceiling it is the *unboosted* part that counts: equipment does not stop a
        // potion from working, so a stat that only reaches its maximum while something is equipped
        // has not actually been maxed.
        bool atMaximum = maximum > 0 && value - boost >= maximum;
        _value.AddThemeColorOverride("font_color", atMaximum ? Maxed : Colors.White);

        _boost.Text = boost > 0 ? $"+{boost}" : string.Empty;
    }
}
