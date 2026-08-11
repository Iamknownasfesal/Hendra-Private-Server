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
    private const int PanelWidth = 232;
    private const int SlotSize = 40;
    private const int SlotsPerRow = 4;

    /// <summary>Worn equipment, which the character screen shows on its own row.</summary>
    private const int EquipmentSlots = 4;

    /// <summary>Carried items, the backpack aside.</summary>
    private const int InventorySlots = 8;

    private readonly List<SlotView> _equipment = new();
    private readonly List<SlotView> _inventory = new();

    /// <summary>Raised with the slot's index in the player's 24-entry equipment array.</summary>
    public event Action<int> SlotActivated;

    private VitalBar _health;
    private VitalBar _mana;
    private Label _name;
    private Label _level;
    private Label _stats;
    private Label _prompt;

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

        var panel = new PanelContainer
        {
            CustomMinimumSize = new Vector2(PanelWidth, 0),
            MouseFilter = MouseFilterEnum.Stop,
        };

        // The default panel style is transparent, which leaves the world showing through the stats.
        panel.AddThemeStyleboxOverride("panel", new StyleBoxFlat
        {
            BgColor = new Color(0.07f, 0.07f, 0.08f, 0.92f),
            BorderColor = new Color(0.25f, 0.25f, 0.28f),
            BorderWidthLeft = 1,
        });
        panel.SetAnchorsPreset(LayoutPreset.RightWide);
        panel.OffsetLeft = -PanelWidth;
        AddChild(panel);

        var margin = new MarginContainer();
        foreach (string side in new[] { "margin_left", "margin_top", "margin_right", "margin_bottom" })
            margin.AddThemeConstantOverride(side, 8);
        panel.AddChild(margin);

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 6);
        margin.AddChild(column);

        _name = new Label { Text = "—" };
        column.AddChild(_name);

        _level = new Label { Text = string.Empty };
        column.AddChild(_level);

        _health = new VitalBar(new Color(0.13f, 0.75f, 0.13f));
        column.AddChild(_health);

        _mana = new VitalBar(new Color(0.20f, 0.40f, 0.90f));
        column.AddChild(_mana);

        _stats = new Label { AutowrapMode = TextServer.AutowrapMode.WordSmart };
        column.AddChild(_stats);

        column.AddChild(new HSeparator());
        column.AddChild(new Label { Text = "Equipment" });
        AddSlots(column, _equipment, EquipmentSlots, firstIndex: 0);

        column.AddChild(new Label { Text = "Inventory" });
        AddSlots(column, _inventory, InventorySlots, firstIndex: 8);

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
        _level.Text = player.Level >= 0 ? $"Level {player.Level}" : string.Empty;

        _health.Set(player.Hp, player.MaxHp);
        _mana.Set(player.Mp, player.MaxMp);

        _stats.Text =
            $"ATT {player.Attack}   DEF {player.Defense}\n" +
            $"SPD {player.Speed}   DEX {player.Dexterity}\n" +
            $"VIT {player.Vitality}   WIS {player.Wisdom}";

        // Slots 0-3 are worn, 4-7 continue the worn row in the data model but the original shows
        // only the first four as equipment; 8 onward is the carried inventory.
        UpdateSlots(_equipment, player, 0);
        UpdateSlots(_inventory, player, 8);
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
internal sealed partial class VitalBar : Control
{
    private const int BarHeight = 18;

    private readonly Color _fill;
    private Label _label;
    private float _fraction;

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

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);
        DrawRect(full, new Color(0.12f, 0.12f, 0.12f));
        DrawRect(new Rect2(Vector2.Zero, new Vector2(Size.X * _fraction, Size.Y)), _fill);
        DrawRect(full, new Color(0f, 0f, 0f, 0.6f), filled: false, width: 1f);
    }
}

/// <summary>One inventory or equipment slot.</summary>
internal sealed partial class SlotView : Control
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
