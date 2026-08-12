using System.Collections.Generic;
using System.Globalization;
using Godot;
using Hendra.Data;
using Hendra.Resources;

namespace Hendra.UI;

/// <summary>
/// The panel shown when the pointer rests on an item.
/// </summary>
/// <remarks>
/// <para>
/// A reconstruction of the original's <c>EquipmentToolTip</c>, which is a drawn panel rather than a
/// string: the item's picture and name across the top with a tier tag pinned to the right, the
/// flavour line under them, a rule, the list of what it does, a rule, and what stops you using it —
/// then the feed power at the foot. Two hundred and thirty pixels wide, as it is there.
/// </para>
/// <para>
/// The wording is the original's too, taken from the same language keys: "Damage: {damage}",
/// "Range: {range}", "Shots: {numShots}", "MP Cost: {cost}", "On Equip:", "Consumed with use". The
/// English is the fallback for each, since the table is fetched over HTTP and is not there at once.
/// </para>
/// <para>
/// Not reproduced: the comparison against what you already have equipped, which colours each line
/// green or red against the item in that slot. That needs the player's equipment threaded down here
/// and a per-slot comparison table -- worth doing, but it is a second piece of work.
/// </para>
/// </remarks>
public partial class ItemTooltipPanel : MarginContainer
{
    /// <summary>The original's MAX_WIDTH.</summary>
    private const int Width = 230;

    private const int IconSize = 40;

    /// <summary>Space for text once the margins are taken out.</summary>
    private const int TextWidth = Width - 12;

    /// <summary>The original's panel fill and border, 0x363636 over 0x9B9B9B.</summary>
    private static readonly Color Background = new("363636");

    private static readonly Color Border = new("9b9b9b");

    /// <summary>Its TooltipHelper colours, by their own names.</summary>
    private static readonly Color NoDiff = new("ffff8f");

    private static readonly Color Legendary = new("ffff00");
    private static readonly Color Special = new("8b0000");
    private static readonly Color Untiered = new("8a2be2");
    private static readonly Color SetItem = new("ff8c00");

    /// <summary>Its description grey, 0xB3B3B3.</summary>
    private static readonly Color Muted = new("b3b3b3");

    private readonly ObjectDesc _desc;
    private readonly Assets.Sprite _sprite;
    private readonly GameData _data;
    private readonly Text.StringMap _strings;

    public ItemTooltipPanel(ObjectDesc desc, Assets.Sprite sprite, GameData data, Text.StringMap strings)
    {
        _desc = desc;
        _sprite = sprite;
        _data = data;
        _strings = strings;
    }

    public override void _Ready()
    {
        foreach (string side in new[] { "margin_left", "margin_top", "margin_right", "margin_bottom" })
            AddThemeConstantOverride(side, 6);

        CustomMinimumSize = new Vector2(Width, 0);

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 3);
        AddChild(column);

        BuildHeading(column);
        BuildDescription(column);
        BuildEffects(column);
        BuildRestrictions(column);
        BuildFeedPower(column);
    }

    /// <summary>The picture, the name, and the tier tag pinned to the right.</summary>
    private void BuildHeading(Control column)
    {
        var row = new HBoxContainer();
        row.AddThemeConstantOverride("separation", 4);
        column.AddChild(row);

        if (_sprite.IsValid)
            row.AddChild(new ItemIcon(_sprite) { CustomMinimumSize = new Vector2(IconSize, IconSize) });

        var title = new Label
        {
            Text = _desc.DisplayId ?? _desc.Id ?? "Unknown item",
            AutowrapMode = TextServer.AutowrapMode.WordSmart,
            SizeFlagsHorizontal = SizeFlags.ExpandFill,
            VerticalAlignment = VerticalAlignment.Center,

            // The icon and the tier tag take the rest of the row.
            CustomMinimumSize = new Vector2(TextWidth - IconSize - 34, 0),
        };
        title.AddThemeFontSizeOverride("font_size", 16);
        title.AddThemeColorOverride("font_color", TitleColour());
        row.AddChild(title);

        var (tag, colour) = TierTag();
        if (tag == null)
            return;

        var tier = new Label
        {
            Text = tag,
            VerticalAlignment = VerticalAlignment.Center,
            HorizontalAlignment = HorizontalAlignment.Right,
            CustomMinimumSize = new Vector2(26, 0),
        };
        tier.AddThemeFontSizeOverride("font_size", 16);
        tier.AddThemeColorOverride("font_color", colour);
        row.AddChild(tier);
    }

    private Color TitleColour() => _desc.Class == "Equipment" && _desc.Tier < 0 ? Colors.White : Colors.White;

    /// <summary>
    /// The tag at the top right, and its colour.
    /// </summary>
    /// <remarks>
    /// The original's TierUtil: a tier if the item has one, otherwise UT for untiered — and nothing
    /// at all for consumables, treasure and pet food, which have no notion of quality.
    /// </remarks>
    private (string Tag, Color Colour) TierTag()
    {
        string tag = ItemTooltip.TierTag(_desc);
        return (tag, tag == "UT" ? Untiered : Colors.White);
    }

    private void BuildDescription(Control column)
    {
        if (string.IsNullOrWhiteSpace(_desc.Description))
            return;

        var text = Wrapping(_desc.Description);
        text.AddThemeFontSizeOverride("font_size", 14);
        text.AddThemeColorOverride("font_color", Muted);
        column.AddChild(text);
    }

    /// <summary>Everything the item does, under a rule, as the original arranges it.</summary>
    private void BuildEffects(Control column)
    {
        var lines = ItemTooltip.Effects(_desc, _strings);
        if (lines.Count == 0)
            return;

        column.AddChild(Rule());

        foreach (var line in lines)
        {
            var label = Wrapping(line.Text);
            label.AddThemeFontSizeOverride("font_size", 14);
            label.AddThemeColorOverride("font_color", line.IsHeading ? Muted : NoDiff);
            column.AddChild(label);
        }
    }

    /// <summary>What stops you using it: the classes it suits, whether it binds, whether it is spent.</summary>
    private void BuildRestrictions(Control column)
    {
        var lines = new List<string>();

        string classes = UsableBy();
        if (classes != null)
        {
            lines.Add(Get("EquipmentToolTip.usableBy", "Usable by: {usableClasses}")
                .Replace("{usableClasses}", classes));
        }

        lines.AddRange(ItemTooltip.Restrictions(_desc, _strings));

        if (lines.Count == 0)
            return;

        column.AddChild(Rule());

        foreach (string text in lines)
        {
            var label = Wrapping(text);
            label.AddThemeFontSizeOverride("font_size", 13);
            label.AddThemeColorOverride("font_color", Muted);
            column.AddChild(label);
        }
    }

    /// <summary>
    /// The classes whose slots accept this item.
    /// </summary>
    /// <remarks>
    /// Worked out from the class descriptors rather than stored on the item: each class lists the
    /// slot types it can fill, and an item names the one it goes in. Null when every class can use
    /// it, since a line naming all fourteen tells you nothing.
    /// </remarks>
    private string UsableBy()
    {
        if (_desc.SlotType < 0 || _data == null)
            return null;

        var names = new List<string>();
        int classes = 0;

        foreach (var playerClass in _data.PlayerClasses)
        {
            classes++;
            if (playerClass.SlotTypes == null)
                continue;

            foreach (int slot in playerClass.SlotTypes)
            {
                if (slot != _desc.SlotType)
                    continue;

                names.Add(playerClass.DisplayId ?? playerClass.Id);
                break;
            }
        }

        if (names.Count == 0 || names.Count == classes)
            return null;

        return string.Join(", ", names);
    }

    private void BuildFeedPower(Control column)
    {
        if (_desc.FeedPower <= 0)
            return;

        var label = new Label { Text = $"Feed Power: {_desc.FeedPower}" };
        label.AddThemeFontSizeOverride("font_size", 12);
        label.AddThemeColorOverride("font_color", Colors.White);
        column.AddChild(label);
    }

    /// <summary>
    /// A label that wraps inside the panel rather than widening it.
    /// </summary>
    /// <remarks>
    /// The width has to be stated. An autowrapping label with none reports the height it would need
    /// if it were one character wide, which is how a six-line tooltip asked for most of the screen.
    /// </remarks>
    private static Label Wrapping(string text) => new()
    {
        Text = text,
        AutowrapMode = TextServer.AutowrapMode.WordSmart,
        CustomMinimumSize = new Vector2(TextWidth, 0),
    };

    private static HSeparator Rule() => new();

    /// <summary>A line from the language table, with its one token filled in.</summary>
    private string Line(string key, string fallback, string token, string value) =>
        Get(key, fallback).Replace("{" + token + "}", value);

    private string Get(string key, string fallback) =>
        _strings != null && _strings.Has(key) ? _strings.Get(key) : fallback;

    private static string Round(float value) => value.ToString("0.#", CultureInfo.InvariantCulture);


    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);
        DrawRect(full, Background);
        DrawRect(full, Border, filled: false, width: 1f);
    }

    /// <summary>The item's picture, drawn at whatever size the heading gives it.</summary>
    private sealed partial class ItemIcon : Control
    {
        private readonly Assets.Sprite _sprite;

        public ItemIcon(Assets.Sprite sprite) => _sprite = sprite;

        public override void _Draw()
        {
            if (!_sprite.IsValid)
                return;

            // Fitted rather than stretched: item art is square, but the box it lands in is only
            // square while the name beside it is one line long.
            float side = Mathf.Min(Size.X, Size.Y);
            var box = new Rect2((Size.X - side) / 2f, (Size.Y - side) / 2f, side, side);
            DrawTextureRectRegion(_sprite.Sheet, box, _sprite.Region);
        }
    }
}
