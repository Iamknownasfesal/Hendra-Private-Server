using System;
using System.Collections.Generic;
using System.Globalization;
using Godot;
using Hendra.Assets;
using Hendra.Resources;
using Hendra.World;

namespace Hendra.UI;

/// <summary>
/// A small panel with a title band: the shape every transient interface in the HUD is built in.
/// </summary>
/// <remarks>
/// Lighter than <see cref="ModalPanel"/>, which is for the things you open and read. These appear
/// because you walked next to something and vanish when you walk away, so they carry the HUD's own
/// grey chrome rather than the gold frame -- they are part of the furniture, not a page over it.
/// </remarks>
public partial class HudSection : Control
{
    public const float HeaderHeight = 24f;
    public const float Padding = 8f;

    private readonly Label _title;

    public HudSection(string title)
    {
        MouseFilter = MouseFilterEnum.Stop;
        Visible = false;

        _title = new Label { Text = title, VerticalAlignment = VerticalAlignment.Center }
            .Typeset(Style.FontBody, Style.Text);
        AddChild(_title);

        Body = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(Body);
    }

    /// <summary>What the panel holds. Positioned under the title band, inside the padding.</summary>
    public Control Body { get; }

    public void SetTitle(string title)
    {
        if (_title.Text != title)
            _title.Text = title;
    }

    /// <summary>Sizes the panel around a body of the given size.</summary>
    public void Fit(Vector2 body)
    {
        Size = new Vector2(body.X + Padding * 2f, HeaderHeight + body.Y + Padding * 2f);

        _title.Position = new Vector2(Padding, 0f);
        _title.Size = new Vector2(Size.X - Padding * 2f, HeaderHeight);

        Body.Position = new Vector2(Padding, HeaderHeight + Padding);
        Body.Size = body;

        QueueRedraw();
    }

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);

        DrawRect(full, Style.Panel);
        DrawRect(new Rect2(0f, 0f, Size.X, HeaderHeight), Style.ModalHeader);
        DrawRect(new Rect2(0f, HeaderHeight - 1f, Size.X, 1f), Style.PanelEdge);
        DrawRect(full, Style.PanelEdge, filled: false, width: 1f);
    }
}

/// <summary>
/// What is inside the thing at the player's feet: a loot bag, a chest, a vault.
/// </summary>
/// <remarks>
/// Eight slots, because that is what every container on this server carries -- a vault chest and a
/// loot bag are the same eight on the wire, and the panel is the same panel. Clicking a slot takes
/// the item; dragging works both ways, which is the only way to put something back.
/// </remarks>
public partial class ContainerPanel : HudSection
{
    private const int Slots = 8;
    private const int Columns = 4;

    private readonly List<SlotView> _slots = new(Slots);
    private readonly Label _hint;

    public ContainerPanel()
        : base("Contents")
    {
        float pitch = HudLayout.HotbarSlotWidth + HudLayout.SlotGap;
        float rows = HudLayout.HotbarSlotHeight + HudLayout.SlotGap;

        for (int i = 0; i < Slots; i++)
        {
            int index = i;
            var slot = new SlotView
            {
                Address = new SlotAddress(SlotOwner.Container, index),
                Draggable = true,
                Position = new Vector2(i % Columns * pitch, i / Columns * rows),
                Size = new Vector2(HudLayout.HotbarSlotWidth, HudLayout.HotbarSlotHeight),
            };

            slot.Activated += () => Activated?.Invoke(index);
            slot.Dropped += (from, to) => Dropped?.Invoke(from, to);

            Body.AddChild(slot);
            _slots.Add(slot);
        }

        float width = Columns * pitch - HudLayout.SlotGap;
        float height = 2f * rows - HudLayout.SlotGap;

        // A line of instruction, once, at the bottom. Taking things out of a chest by clicking them
        // is not guessable -- the alternative is a player who drags every item one at a time.
        _hint = new Label { Text = "Click to take", VerticalAlignment = VerticalAlignment.Center }
            .Typeset(Style.FontTag, Style.TextDim);

        _hint.Position = new Vector2(0f, height + 4f);
        _hint.Size = new Vector2(width, 14f);
        Body.AddChild(_hint);

        Fit(new Vector2(width, height + 18f));
    }

    /// <summary>Raised with the slot's index in the open container.</summary>
    public event Action<int> Activated;

    /// <summary>Raised when something is dropped on one of these slots.</summary>
    public event Action<SlotAddress, SlotAddress> Dropped;

    /// <summary>Shows a container's contents, or hides the panel when given null.</summary>
    public void Show(Entity container, GameData data, TextureResolver textures, int[] slotTypes)
    {
        Visible = container != null;
        if (container == null)
            return;

        // What the data calls it, not the Name stat -- a vault chest's Name is how full it is, and
        // "0/8" is a fine thing to write over the chest but not a title for the panel.
        var chest = data?.GetObject(container.ObjectType);
        string name = chest?.DisplayId ?? chest?.Id;
        SetTitle(string.IsNullOrEmpty(name) ? "Contents" : name);

        for (int i = 0; i < _slots.Count; i++)
        {
            int type = container.Equipment != null && i < container.Equipment.Length
                ? container.Equipment[i]
                : -1;

            if (type < 0)
            {
                _slots[i].SetItem(default, null, data);
                continue;
            }

            var desc = data?.GetObject((ushort)type);
            var resolved = textures?.Resolve(desc?.Texture) ?? default;

            _slots[i].Usable = HudView.CanEquip(desc, slotTypes);
            _slots[i].SetItem(resolved.Still, desc, data);
        }
    }
}

/// <summary>
/// What the vendor at the player's feet is selling.
/// </summary>
/// <remarks>
/// One item, because that is all a merchant entity carries: its type, its price, the currency and
/// how many are left. The price is the part that has to be readable at a glance -- it turns red
/// when the purse is short, which is the answer to the only question anyone asks this panel.
/// </remarks>
public partial class MerchantPanel : HudSection
{
    private readonly SlotView _item;
    private readonly Label _name;
    private readonly Label _price;
    private readonly Label _stock;
    private readonly HudGlyph _currency;
    private readonly HudMenuButton _buy;

    public MerchantPanel()
        : base("For sale")
    {
        const float Width = 232f;
        float slot = HudLayout.HotbarSlotHeight;

        _item = new SlotView
        {
            Position = Vector2.Zero,
            Size = new Vector2(HudLayout.HotbarSlotWidth, slot),
        };
        Body.AddChild(_item);

        float textLeft = HudLayout.HotbarSlotWidth + 8f;

        _name = new Label
        {
            ClipText = true,
            TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis,
            Position = new Vector2(textLeft, 0f),
            Size = new Vector2(Width - textLeft, 16f),
        }.Typeset(Style.FontSmall, Style.Text);
        Body.AddChild(_name);

        _price = new Label { Position = new Vector2(textLeft, 18f), Size = new Vector2(100f, 16f) }
            .Typeset(Style.FontSmall, Style.Text);
        Body.AddChild(_price);

        _currency = new HudGlyph(HudIcons.Coin, Style.IconGold)
        {
            Position = new Vector2(textLeft, 20f),
            Size = new Vector2(12f, 12f),
        };
        Body.AddChild(_currency);

        _stock = new Label { Position = new Vector2(textLeft, 34f), Size = new Vector2(Width - textLeft, 14f) }
            .Typeset(Style.FontTag, Style.TextDim);
        Body.AddChild(_stock);

        _buy = new HudMenuButton("Buy")
        {
            Position = new Vector2(0f, slot + 8f),
            Size = new Vector2(Width, 28f),
        };
        _buy.Pressed += () => Pressed?.Invoke();
        Body.AddChild(_buy);

        Fit(new Vector2(Width, slot + 8f + 28f));
    }

    /// <summary>Raised when the buy button is pressed.</summary>
    public event Action Pressed;

    /// <summary>Shows what a vendor is selling, or hides the panel when given null.</summary>
    public void Show(Entity merchant, LocalPlayer player, GameData data, TextureResolver textures, int[] slotTypes)
    {
        Visible = merchant is { MerchandiseType: >= 0 };
        if (!Visible)
            return;

        var desc = data?.GetObject((ushort)merchant.MerchandiseType);
        var resolved = textures?.Resolve(desc?.Texture) ?? default;

        _item.Usable = HudView.CanEquip(desc, slotTypes);
        _item.SetItem(resolved.Still, desc, data);

        _name.Text = desc?.DisplayId ?? desc?.Id ?? "Unknown item";

        // Currency zero is gold; anything else is fame on this server build.
        bool fame = merchant.MerchandiseCurrency != 0;
        int purse = player == null ? 0 : fame ? player.Fame : player.Credits;
        bool afford = purse >= merchant.MerchandisePrice;

        _currency.Tint = fame ? Style.IconFame : Style.IconGold;
        _currency.Icon = fame ? HudIcons.Fame : HudIcons.Coin;

        string price = merchant.MerchandisePrice.ToString("N0", CultureInfo.InvariantCulture);
        _price.Text = price;
        _price.AddThemeColorOverride("font_color", afford ? Style.Text : Style.StatPenalty);

        // The icon sits after the number rather than before it, as the currencies do at the top of
        // the screen, so the two read the same way round.
        _currency.Position = new Vector2(
            _price.Position.X + Style.Measure(price, Style.FontSmall) + 4f, _currency.Position.Y);

        _stock.Text = merchant.MerchandiseCount >= 0
            ? $"{merchant.MerchandiseCount} left"
            : string.Empty;

        _buy.Disabled = !afford;
    }
}
