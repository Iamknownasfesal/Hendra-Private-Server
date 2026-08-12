using System;
using System.Collections.Generic;
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
    private CutEdgePanel _backpackPanel;

    private CutEdgePanel _inventoryPanel;
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

    /// <summary>Raised when the buy button is pressed at a vendor.</summary>
    public event Action BuyPressed;

    private VitalBar _health;
    private VitalBar _mana;
    private Label _name;
    private VitalBar _level;
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

        // The column itself, in the original's cut-corner shape and its background colour. Only the
        // left corners are cut: the right two sit against the edge of the screen where a bevel
        // would show as a notch out of the frame.
        var panel = new CutEdgePanel
        {
            CustomMinimumSize = new Vector2(PanelWidth, 0),
            MouseFilter = MouseFilterEnum.Stop,
            Background = CutEdgePanel.PanelBackground,
        };
        panel.Cuts(topLeft: true, topRight: false, bottomRight: false, bottomLeft: true).Padded(7);
        panel.SetAnchorsPreset(LayoutPreset.RightWide);
        panel.OffsetLeft = -PanelWidth;
        AddChild(panel);

        var margin = panel;

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 6);
        margin.AddChild(column);

        // The minimap is drawn over the top of this column by its own node, so the column starts
        // below it. Reserved rather than parented, because the map has to clip its own rotated
        // drawing and a container would fight it for the size.
        column.AddChild(new Control { CustomMinimumSize = new Vector2(0, MinimapAllowance) });

        _name = new Label { Text = "—" };
        column.AddChild(_name);

        // Level is a bar, as it is in the original: progress toward the next level, labelled with
        // the level itself. At the cap it becomes the fame bar, measured against the next class
        // quest -- the same swap StatMetersView makes, and the colours are its colours.
        _level = new VitalBar(new Color(0.35f, 0.50f, 0.14f));
        column.AddChild(_level);

        _health = new VitalBar(new Color(0.88f, 0.20f, 0.20f));
        column.AddChild(_health);

        _mana = new VitalBar(new Color(0.38f, 0.52f, 0.88f));
        column.AddChild(_mana);

        // The original sits its grids on their own backgrounds rather than straight on the column:
        // an 186 by 92 cut-corner panel behind each. It is what separates the interface into parts
        // you can find by shape rather than by reading it.
        var equipmentPanel = AddSection(column, "Equipment");
        AddSlots(equipmentPanel, _equipment, EquipmentSlots, firstIndex: 0);

        // Inventory and backpack share the space, as they do in the original: a strip of two tabs
        // above one grid, rather than both grids stacked. The backpack tab only appears for a
        // character that has bought the bag -- the stat that grants it is HasBackpack, and until
        // then the server refuses a swap into those slots anyway.
        _tabs = new HBoxContainer();
        _tabs.AddThemeConstantOverride("separation", 4);
        column.AddChild(_tabs);

        _inventoryTab = AddTab("Inventory", Page.Inventory);
        _statsTab = AddTab("Stats", Page.Stats);
        _backpackTab = AddTab("Backpack", Page.Backpack);

        _inventoryPanel = NewSectionPanel();
        column.AddChild(_inventoryPanel);
        AddSlots(SectionBody(_inventoryPanel), _inventory, InventorySlots, firstIndex: 8);

        _statsPanel = NewSectionPanel();
        _statsPanel.Visible = false;
        column.AddChild(_statsPanel);
        BuildStatsPage(SectionBody(_statsPanel));

        _backpackPanel = NewSectionPanel();
        _backpackPanel.Visible = false;
        column.AddChild(_backpackPanel);
        AddSlots(SectionBody(_backpackPanel), _backpack, BackpackSlots, firstIndex: 16);

        // Everything below is pushed to the foot of the column, where the original puts its
        // interact panel -- it is the part that appears and disappears as the player walks around,
        // and it is less distracting from down there.
        column.AddChild(new Control { SizeFlagsVertical = SizeFlags.ExpandFill });

        // Only present while standing over something that holds items.
        _containerPanel = new VBoxContainer { Visible = false };
        column.AddChild(_containerPanel);

        // Named after whatever is being looked into, so it is obvious which chest the grid belongs
        // to when there are several on the floor.
        _containerName = new Label { Text = "Contents" };
        _containerPanel.AddChild(_containerName);
        AddContainerSlots(_containerPanel, ContainerSlots);

        // Only present while standing at a vendor.
        _merchantPanel = new VBoxContainer { Visible = false };
        column.AddChild(_merchantPanel);
        _merchantPanel.AddChild(new Label { Text = "For sale" });

        var row = new HBoxContainer();
        row.AddThemeConstantOverride("separation", 8);
        _merchantPanel.AddChild(row);

        _merchandise = new SlotView { CustomMinimumSize = new Vector2(SlotSize, SlotSize) };
        row.AddChild(_merchandise);

        _price = new Label { VerticalAlignment = VerticalAlignment.Center };
        row.AddChild(_price);

        _buy = new GameButton("Buy", compact: true);
        _buy.Pressed += () => BuyPressed?.Invoke();
        _merchantPanel.AddChild(_buy);

        // Nearby players. Hidden outright when there are none, which is most of the time -- a
        // heading with nothing under it is three lines of column spent saying nothing, and this
        // sits at the foot where the space runs out first.
        _partyPanel = new VBoxContainer { Visible = false };
        column.AddChild(_partyPanel);
        _partyPanel.AddChild(new HSeparator());

        var partyHeading = new Label { Text = "Nearby" };
        partyHeading.AddThemeColorOverride("font_color", new Color(0.66f, 0.66f, 0.66f));
        _partyPanel.AddChild(partyHeading);

        _party = new VBoxContainer();
        _party.AddThemeConstantOverride("separation", 0);
        _partyPanel.AddChild(_party);

        // Sits over the world rather than in the panel, because it refers to something in front of
        // the player rather than to their own state.
        _prompt = new Label
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            Visible = false,
        };
        _prompt.SetAnchorsPreset(LayoutPreset.CenterBottom);
        _prompt.OffsetTop = -96;
        _prompt.OffsetLeft = -200;
        _prompt.OffsetRight = 200;
        _prompt.OffsetBottom = -72;
        AddChild(_prompt);
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
            var slot = new SlotView { CustomMinimumSize = new Vector2(SlotSize, SlotSize) };
            slot.Activated += () => ContainerSlotActivated?.Invoke(slotIndex);
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
            var slot = new SlotView { CustomMinimumSize = new Vector2(SlotSize, SlotSize) };
            slot.Activated += () => SlotActivated?.Invoke(slotIndex);
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

        RefreshLevel(player);
        _health.Set(player.Hp, player.MaxHp);
        _mana.Set(player.Mp, player.MaxMp);
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
            _level.Set(player.Fame, player.NextClassQuestFame, "Fame");
            return;
        }

        _level.Fill = ExperienceBar;
        _level.Set(player.Experience, player.NextLevelExperience, $"Level {player.Level}");
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

    /// <summary>Raised on a left click, whether or not the slot holds anything.</summary>
    public event Action Activated;

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
        if (@event is InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
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
