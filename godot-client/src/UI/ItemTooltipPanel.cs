using System.Collections.Generic;
using Godot;
using Hendra.Resources;

namespace Hendra.UI;

/// <summary>
/// The panel shown when the pointer rests on an item.
/// </summary>
/// <remarks>
/// <para>
/// The original's <c>EquipmentToolTip</c> arranged in the original's order — picture and name across
/// the top with a tier tag to the right, what the item does, the flavour line, what stops you using
/// it — and worded from the same language keys, but dressed rather better than a flat grey box with
/// a hairline around it.
/// </para>
/// <para>
/// What the dressing is doing. The frame takes the game's own cut-corner shape, so a tooltip looks
/// like it belongs to the same interface as the panels behind it. Its edge, its rules and its
/// figures are the item's rarity colour, so quality registers before a word is read — untiered keeps
/// the original's violet, and the tiers run cool to warm as they climb, which the original does not
/// do at all: every tier there is the same white, and tells you nothing at a glance. The body is
/// graded rather than flat and sits on a shadow, so it reads as lying over the world instead of
/// being cut into it.
/// </para>
/// <para>
/// Not reproduced: the comparison against what you already have equipped, which colours each line
/// green or red against the item in that slot. That needs the player's equipment threaded down here
/// and a per-slot comparison table.
/// </para>
/// </remarks>
public partial class ItemTooltipPanel : MarginContainer
{
    /// <summary>The original's MAX_WIDTH.</summary>
    private const int Width = 230;

    private const int Padding = 9;

    /// <summary>Space for text once the padding is taken out.</summary>
    private const int TextWidth = Width - Padding * 2;

    private const int IconSize = 44;

    /// <summary>How far the frame's corners are cut, as the game's panels cut theirs.</summary>
    private const int CornerCut = 7;

    /// <summary>How many faint copies make up the shadow's falloff.</summary>
    private const int Steps = 4;

    /// <summary>The body, lighter at the top than at the bottom.</summary>
    private static readonly Color BodyTop = new("2e2c31");

    private static readonly Color BodyBottom = new("1a191c");

    /// <summary>The original's TooltipHelper violet, which is its one rarity colour.</summary>
    private static readonly Color Untiered = new("8a2be2");

    /// <summary>Its description grey, 0xB3B3B3.</summary>
    private static readonly Color Muted = new("b3b3b3");

    private static readonly Color Faint = new("7d7a80");

    private readonly ObjectDesc _desc;
    private readonly Assets.Sprite _sprite;
    private readonly GameData _data;
    private readonly Text.StringMap _strings;
    private readonly Color _accent;

    public ItemTooltipPanel(ObjectDesc desc, Assets.Sprite sprite, GameData data, Text.StringMap strings)
    {
        _desc = desc;
        _sprite = sprite;
        _data = data;
        _strings = strings;
        _accent = Accent(desc);
    }

    /// <summary>
    /// The colour that stands for this item's quality.
    /// </summary>
    /// <remarks>
    /// Untiered keeps the original's violet, which is the one rarity colour players already read
    /// without thinking. The tiers are the addition: they run from a cool steel at the bottom to a
    /// warm gold at the top, so a good item announces itself before the number is read.
    /// </remarks>
    private static Color Accent(ObjectDesc desc)
    {
        if (desc == null)
            return Muted;

        if (desc.Tier < 0)
            return desc.Consumable ? new Color("6f8fa8") : Untiered;

        // Twelve tiers is the practical range; past that it stays gold.
        float climb = Mathf.Clamp(desc.Tier / 11f, 0f, 1f);
        return new Color("7f9bb5").Lerp(new Color("ffc94a"), climb);
    }

    public override void _Ready()
    {
        foreach (string side in new[] { "margin_left", "margin_top", "margin_right", "margin_bottom" })
            AddThemeConstantOverride(side, Padding);

        CustomMinimumSize = new Vector2(Width, 0);

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 5);
        AddChild(column);

        BuildHeading(column);
        BuildEffects(column);
        BuildDescription(column);
        BuildRestrictions(column);
    }

    /// <summary>The picture on its plate, the name, the slot it fills, and the tier badge.</summary>
    private void BuildHeading(Control column)
    {
        var row = new HBoxContainer();
        row.AddThemeConstantOverride("separation", 7);
        column.AddChild(row);

        row.AddChild(new ItemIcon(_sprite, _accent)
        {
            CustomMinimumSize = new Vector2(IconSize, IconSize),
        });

        var names = new VBoxContainer
        {
            SizeFlagsHorizontal = SizeFlags.ExpandFill,
            SizeFlagsVertical = SizeFlags.ShrinkCenter,
        };
        names.AddThemeConstantOverride("separation", 1);
        row.AddChild(names);

        string tag = ItemTooltip.TierTag(_desc);

        var title = new Label
        {
            Text = _desc.DisplayId ?? _desc.Id ?? "Unknown item",
            AutowrapMode = TextServer.AutowrapMode.WordSmart,

            // The icon and the badge take the rest of the row.
            CustomMinimumSize = new Vector2(TextWidth - IconSize - (tag == null ? 7 : 46), 0),
        };
        title.AddThemeFontSizeOverride("font_size", 15);
        title.AddThemeColorOverride("font_color", Colors.White);
        names.AddChild(title);

        // The slot an item fills is among the first things you check about it, and the original
        // never says at all -- so it sits under the name, where a subtitle goes.
        string slot = SlotName();
        if (slot != null)
        {
            var kind = new Label { Text = slot };
            kind.AddThemeFontSizeOverride("font_size", 11);
            kind.AddThemeColorOverride("font_color", Faint);
            names.AddChild(kind);
        }

        if (tag != null)
            row.AddChild(new TierBadge(tag, _accent) { SizeFlagsVertical = SizeFlags.ShrinkCenter });
    }

    /// <summary>What the item does, under the rule that separates it from the heading.</summary>
    private void BuildEffects(Control column)
    {
        var lines = ItemTooltip.Effects(_desc, _strings);
        if (lines.Count == 0)
            return;

        column.AddChild(new Rule(_accent, strong: true));

        foreach (var line in lines)
        {
            var label = Wrapping(line.Text);
            label.AddThemeFontSizeOverride("font_size", line.IsHeading ? 11 : 14);
            label.AddThemeColorOverride("font_color", line.IsHeading ? Faint : _accent);
            column.AddChild(label);
        }
    }

    private void BuildDescription(Control column)
    {
        if (string.IsNullOrWhiteSpace(_desc.Description))
            return;

        column.AddChild(new Rule(_accent, strong: false));

        var text = Wrapping(_desc.Description);
        text.AddThemeFontSizeOverride("font_size", 13);
        text.AddThemeColorOverride("font_color", Muted);
        column.AddChild(text);
    }

    /// <summary>Who may use it, whether it binds, whether it is spent, what it is worth to a pet.</summary>
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

        if (_desc.FeedPower > 0)
            lines.Add($"Feed Power: {_desc.FeedPower}");

        if (lines.Count == 0)
            return;

        column.AddChild(new Rule(_accent, strong: false));

        foreach (string text in lines)
        {
            var label = Wrapping(text);
            label.AddThemeFontSizeOverride("font_size", 11);
            label.AddThemeColorOverride("font_color", Faint);
            column.AddChild(label);
        }
    }

    /// <summary>The slot numbers, as the XML's SlotType uses them.</summary>
    private static readonly Dictionary<int, string> SlotNames = new()
    {
        [1] = "Sword", [2] = "Dagger", [3] = "Bow", [4] = "Tome", [5] = "Shield",
        [6] = "Leather Armor", [7] = "Heavy Armor", [8] = "Wand", [9] = "Ring",
        [10] = "Potion", [11] = "Spell", [12] = "Seal", [13] = "Cloak", [14] = "Robe",
        [15] = "Quiver", [16] = "Helm", [17] = "Staff", [18] = "Poison", [19] = "Skull",
        [20] = "Trap", [21] = "Orb", [22] = "Prism", [23] = "Scepter", [24] = "Katana",
        [25] = "Shuriken",
    };

    private string SlotName() =>
        _desc != null && SlotNames.TryGetValue(_desc.SlotType, out string name) ? name : null;

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

    /// <summary>
    /// A label that wraps inside the panel rather than widening it.
    /// </summary>
    /// <remarks>
    /// The width has to be stated. An autowrapping label with none reports the height it would need
    /// if it were one character wide, which is how a six-line tooltip once asked for most of the
    /// screen.
    /// </remarks>
    private static Label Wrapping(string text) => new()
    {
        Text = text,
        AutowrapMode = TextServer.AutowrapMode.WordSmart,
        CustomMinimumSize = new Vector2(TextWidth, 0),
    };

    private string Get(string key, string fallback) =>
        _strings != null && _strings.Has(key) ? _strings.Get(key) : fallback;

    /// <summary>The frame: a shadow, a graded body, and an edge in the item's own colour.</summary>
    public override void _Draw()
    {
        var outline = CutEdgePanel.Outline(Size, CornerCut, new[] { true, true, true, true });

        // Cast down and to the right, so the panel reads as lying over the world rather than as a
        // hole cut into it. Laid down in thin steps rather than as one offset copy: a single hard
        // silhouette at that offset reads as a second panel behind the first, which is what it
        // looked like, where a few faint ones stacked read as a shadow falling off.
        var shadow = new Vector2[outline.Length];
        for (int step = Steps; step >= 1; step--)
        {
            var drop = new Vector2(step * 0.9f, step * 1.2f);
            for (int i = 0; i < outline.Length; i++)
                shadow[i] = outline[i] + drop;

            DrawColoredPolygon(shadow, new Color(0f, 0f, 0f, 0.13f));
        }

        // Lighter at the top, which is where the light comes from everywhere else in the interface.
        var shades = new Color[outline.Length];
        for (int i = 0; i < outline.Length; i++)
            shades[i] = BodyTop.Lerp(BodyBottom, Size.Y <= 0f ? 0f : outline[i].Y / Size.Y);

        DrawPolygon(outline, shades);

        var closed = new Vector2[outline.Length + 1];
        outline.CopyTo(closed, 0);
        closed[^1] = outline[0];

        // The edge carries the rarity, at a strength that frames the panel without competing with
        // the words inside it.
        DrawPolyline(closed, _accent with { A = 0.75f }, 1.5f, antialiased: true);
    }

    /// <summary>A divider. Strong under the heading, faint between the sections below it.</summary>
    private sealed partial class Rule : Control
    {
        private readonly Color _colour;
        private readonly bool _strong;

        public Rule(Color colour, bool strong)
        {
            _colour = colour;
            _strong = strong;
            CustomMinimumSize = new Vector2(0, strong ? 6 : 5);
        }

        public override void _Draw()
        {
            float y = Size.Y / 2f;

            if (!_strong)
            {
                DrawLine(new Vector2(0f, y), new Vector2(Size.X, y), _colour with { A = 0.22f });
                return;
            }

            // Solid where the eye lands and fading out to the right, so it reads as a rule rather
            // than as another box edge.
            const int Steps = 24;
            for (int i = 0; i < Steps; i++)
            {
                float from = Size.X * i / Steps;
                float to = Size.X * (i + 1) / Steps;
                float strength = Mathf.Lerp(0.9f, 0.1f, i / (float)(Steps - 1));
                DrawLine(new Vector2(from, y), new Vector2(to, y), _colour with { A = strength }, 2f);
            }
        }
    }

    /// <summary>The tier, in a pill of its own colour.</summary>
    private sealed partial class TierBadge : Control
    {
        private readonly string _text;
        private readonly Color _colour;

        public TierBadge(string text, Color colour)
        {
            _text = text;
            _colour = colour;
            CustomMinimumSize = new Vector2(34, 21);
        }

        public override void _Ready()
        {
            var label = new Label
            {
                Text = _text,
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
            };
            label.SetAnchorsPreset(LayoutPreset.FullRect);
            label.AddThemeFontSizeOverride("font_size", 12);
            label.AddThemeColorOverride("font_color", _colour);
            AddChild(label);
        }

        public override void _Draw()
        {
            var box = new Rect2(Vector2.Zero, Size);
            DrawRect(box, _colour with { A = 0.14f });
            DrawRect(box, _colour with { A = 0.55f }, filled: false, width: 1f);
        }
    }

    /// <summary>The item's picture, on a plate of its own.</summary>
    private sealed partial class ItemIcon : Control
    {
        private readonly Assets.Sprite _sprite;
        private readonly Color _accent;

        public ItemIcon(Assets.Sprite sprite, Color accent)
        {
            _sprite = sprite;
            _accent = accent;
        }

        public override void _Draw()
        {
            var plate = new Rect2(Vector2.Zero, Size);
            DrawRect(plate, new Color(0f, 0f, 0f, 0.35f));
            DrawRect(plate, _accent with { A = 0.35f }, filled: false, width: 1f);

            if (!_sprite.IsValid)
                return;

            // Fitted rather than stretched, and inset so the plate frames it.
            const float Inset = 6f;
            float side = Mathf.Min(Size.X, Size.Y) - Inset * 2f;
            var box = new Rect2((Size.X - side) / 2f, (Size.Y - side) / 2f, side, side);
            DrawTextureRectRegion(_sprite.Sheet, box, _sprite.Region);
        }
    }
}
