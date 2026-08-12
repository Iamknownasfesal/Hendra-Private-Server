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
        _containerPanel.AddChild(new Label { Text = "Contents" });
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

        _buy = new Button { Text = "Buy" };
        _buy.Pressed += () => BuyPressed?.Invoke();
        _merchantPanel.AddChild(_buy);

        column.AddChild(new HSeparator());
        column.AddChild(new Label { Text = "Nearby" });
        _party = new VBoxContainer();
        column.AddChild(_party);

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
            _prompt.Text = $"[R] {label}";
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

        for (int i = 0; i < _container.Count; i++)
        {
            int type = container.Equipment != null && i < container.Equipment.Length
                ? container.Equipment[i]
                : -1;

            if (type < 0)
            {
                _container[i].SetItem(default, null);
                continue;
            }

            var desc = _data?.GetObject((ushort)type);
            var resolved = _textures?.Resolve(desc?.Texture) ?? default;
            _container[i].SetItem(resolved.Still, desc?.DisplayId ?? desc?.Id);
        }
    }

    /// <summary>Lists the nearby players.</summary>
    public void ShowParty(IReadOnlyList<PartyMember> members)
    {
        if (_party == null)
            return;

        // Few enough entries, changing seldom enough, that rebuilding the rows is simpler than
        // pooling them.
        foreach (var child in _party.GetChildren())
            child.QueueFree();

        foreach (var member in members)
        {
            float fraction = member.MaxHp > 0 ? member.Hp / (float)member.MaxHp : 0f;
            _party.AddChild(new Label
            {
                Text = $"{(member.Starred ? "* " : string.Empty)}{member.Name}  {(int)(fraction * 100)}%",
            });
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
        _merchandise.SetItem(resolved.Still, desc?.DisplayId ?? desc?.Id);

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
    /// Attack, Defense, Speed, Dexterity, Vitality, Wisdom -- in that order, which is the original's
    /// order and not alphabetical or the order they arrive in. MaxHP and MaxMP are left out because
    /// their bars are already above; the original does the same.
    /// </remarks>
    private void BuildStatsPage(Control parent)
    {
        (string Name, string Explains)[] stats =
        {
            ("Attack", "How hard your shots hit."),
            ("Defense", "How much of each hit you shrug off."),
            ("Speed", "How fast you move."),
            ("Dexterity", "How fast you shoot."),
            ("Vitality", "How fast you recover health."),
            ("Wisdom", "How fast you recover mana, and how much your abilities do."),
        };

        var grid = new GridContainer { Columns = 2 };
        grid.AddThemeConstantOverride("h_separation", 10);
        grid.AddThemeConstantOverride("v_separation", 3);
        parent.AddChild(grid);

        _statRows = new StatRow[stats.Length];
        for (int i = 0; i < stats.Length; i++)
        {
            _statRows[i] = new StatRow(stats[i].Name, stats[i].Explains);
            grid.AddChild(_statRows[i]);
        }
    }

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
                views[i].SetItem(default, null);
                continue;
            }

            var desc = _data?.GetObject((ushort)type);
            var resolved = _textures?.Resolve(desc?.Texture) ?? default;
            views[i].SetItem(resolved.Still, desc?.DisplayId ?? desc?.Id);
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

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);
        DrawRect(full, new Color(0.12f, 0.12f, 0.12f));
        DrawRect(new Rect2(Vector2.Zero, new Vector2(Size.X * _fraction, Size.Y)), _fill);
        DrawRect(full, new Color(0f, 0f, 0f, 0.6f), filled: false, width: 1f);
    }
}

/// <summary>One inventory or equipment slot.</summary>
public sealed partial class SlotView : Control
{
    private static readonly Color Background = new(0.10f, 0.10f, 0.10f);
    private static readonly Color Border = new(0.35f, 0.35f, 0.35f);

    private Assets.Sprite _sprite;

    /// <summary>Raised on a left click, whether or not the slot holds anything.</summary>
    public event Action Activated;

    public override void _Ready() => MouseFilter = MouseFilterEnum.Stop;

    public override void _GuiInput(InputEvent @event)
    {
        if (@event is InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
            Activated?.Invoke();
    }

    public void SetItem(Assets.Sprite sprite, string tooltip)
    {
        _sprite = sprite;
        TooltipText = tooltip ?? string.Empty;
        QueueRedraw();
    }

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);
        DrawRect(full, Background);
        DrawRect(full, Border, filled: false, width: 1f);

        if (!_sprite.IsValid)
            return;

        // Item sprites are tiny and must not be smoothed when blown up to slot size.
        var inset = full.Grow(-4f);
        DrawTextureRectRegion(_sprite.Sheet, inset, _sprite.Region);
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

    private readonly string _name;
    private Label _label;
    private Label _value;
    private Label _boost;

    public StatRow(string name, string explains)
    {
        _name = name;
        TooltipText = explains;
        MouseFilter = MouseFilterEnum.Stop;
        AddThemeConstantOverride("separation", 4);
    }

    public override void _Ready()
    {
        _label = new Label { Text = _name, SizeFlagsHorizontal = SizeFlags.ExpandFill };
        _label.AddThemeColorOverride("font_color", new Color(0.66f, 0.66f, 0.66f));
        AddChild(_label);

        _value = new Label { HorizontalAlignment = HorizontalAlignment.Right };
        AddChild(_value);

        _boost = new Label();
        _boost.AddThemeColorOverride("font_color", Boosted);
        AddChild(_boost);
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
