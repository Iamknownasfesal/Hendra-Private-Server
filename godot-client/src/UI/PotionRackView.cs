using System;
using System.Collections.Generic;
using System.Globalization;
using Godot;
using Hendra.Assets;
using Hendra.Resources;
using Hendra.World;

namespace Hendra.UI;

/// <summary>
/// The potion rack: the stat potions in storage, counted by what they raise.
/// </summary>
/// <remarks>
/// <para>
/// Eight attributes, two sizes each. A vault full of potions is a grid of near-identical bottles
/// that you have to count by eye and cannot compare across; the rack answers the only question
/// anybody asks of them -- how many of each do I have, and how far is that from maxing the stat --
/// by holding one square per kind and putting the number on it.
/// </para>
/// <para>
/// It stores nothing of its own. The counts are read out of <see cref="VaultStore"/> every time
/// the vault changes, so a potion deposited from the grid is a potion the rack has already
/// counted, and the two views can never disagree about the same bottle.
/// </para>
/// </remarks>
public sealed partial class PotionRackView : ModalPanel
{
    /// <summary>Where the top edge sits, matching the vault it opens beside.</summary>
    public const float TopEdge = 68f;

    private const float BottomEdge = 8f;

    /// <summary>Air around the two columns of groups.</summary>
    private const float Pad = 11f;

    /// <summary>A group: a caption, a chip and two slots, all the same width as the pair of slots.</summary>
    private const float SlotSize = 97f;

    private const float SlotGap = 4f;
    private const float GroupWidth = SlotSize * 2f + SlotGap;

    /// <summary>The lighter frame around a square. Four, which is what draws this grid.</summary>
    private const float BottleBorder = 4f;

    /// <summary>
    /// The square a bottle stands on.
    /// </summary>
    /// <remarks>
    /// The rack's own pair, not the vault's: the reference sets these squares a shade darker than
    /// the vault's plates and gives them a distinctly lighter frame, so the frame is what draws the
    /// grid. The vault's pair is only ten points apart and the frame disappears at this size.
    /// </remarks>
    private static readonly Color BottlePlate = new("4e4e4e");

    private static readonly Color BottleEdge = new("6d6d6d");

    /// <summary>The board between the two columns of groups.</summary>
    private const float ColumnGap = 31f;

    /// <summary>How much of a group is spent above its slots, on the caption and the chip.</summary>
    private const float CaptionBlock = 52f;

    private const float GroupPitch = 176f;

    private const int Rows = 4;

    /// <summary>The chip that says the attribute is already at its ceiling.</summary>
    private const float ChipWidth = 63f;

    private const float ChipHeight = 36f;
    private const float ChipTop = 13f;

    /// <summary>The three-part relief every olive control in this interface has.</summary>
    private const float ReliefTop = 5f;

    private const float ReliefLow = 3f;
    private const float ReliefShadow = 5f;

    private const float ButtonWidth = 188f;
    private const float ButtonHeight = 47f;

    /// <summary>Where the capacity line and the button sit, measured down the body.</summary>
    private const float CapacityBaseline = 718f;

    private const float ButtonTop = 742f;

    private const float BodyWidth = Pad * 2f + GroupWidth * 2f + ColumnGap;

    private const float BodyHeight = 794f;

    public const float PanelWidth = BodyInset * 2f + BodyWidth;

    public const float PanelHeight = FrameWidth + HeaderHeight + HeaderGap + BodyHeight + BodyInset;

    /// <summary>
    /// The sizes this panel sets text at, measured off the reference.
    /// </summary>
    /// <remarks>
    /// Larger than the type scale in <c>Style</c>, which sits roughly two thirds under what the
    /// reference actually uses. Kept here so the panel matches what it is measured against.
    /// </remarks>
    private const int CaptionSize = 34;

    private const int ChipSize = 26;
    private const int CountSize = 28;
    private const int FooterSize = 32;
    private const int ButtonSize = 30;

    /// <summary>The attributes the rack has a shelf for, in the order it stacks them.</summary>
    private static readonly string[] Attributes =
    {
        "Attack", "Defense", "Speed", "Dexterity", "Vitality", "Wisdom", "Life", "Mana",
    };

    private readonly GameData _data;
    private readonly TextureResolver _textures;

    /// <summary>The two bottles of each attribute, resolved once from the item data.</summary>
    private readonly List<Shelf> _shelves = new();

    private VaultStore _store;

    private Control _sheet;
    private RackButton _deposit;

    /// <summary>The space the panel has to fit in.</summary>
    private Vector2 _screen = new(HudLayout.ReferenceWidth, HudLayout.ReferenceHeight);

    public PotionRackView(GameData data, TextureResolver textures)
        : base("Potion Rack")
    {
        _data = data;
        _textures = textures;

        Size = new Vector2(PanelWidth, PanelHeight);
    }

    /// <summary>Raised when a bottle is clicked: one of that kind comes out of storage.</summary>
    public event Action<SlotAddress> Withdraw;

    /// <summary>Raised when every stat potion the player is carrying should go into storage.</summary>
    public event Action DepositAll;

    protected override bool ShowClose => false;

    protected override bool ShowInfo => true;

    /// <summary>Narrow enough that the mark belongs on the end of the title rather than the corner.</summary>
    protected override bool InfoBesideTitle => true;

    protected override bool ShowOrnaments => true;

    public override void _Ready()
    {
        base._Ready();

        BuildShelves();

        _sheet = new Control { MouseFilter = MouseFilterEnum.Stop };
        _sheet.Draw += DrawSheet;
        _sheet.GuiInput += OnSheetInput;
        Body.AddChild(_sheet);

        _deposit = new RackButton("Deposit All");
        _deposit.Pressed += () => DepositAll?.Invoke();
        Body.AddChild(_deposit);

        Resized += Layout;
        Layout();
    }

    /// <summary>Points the rack at a vault, and follows it from then on.</summary>
    public void Use(VaultStore store)
    {
        if (store == null || ReferenceEquals(store, _store))
            return;

        if (_store != null)
            _store.Changed -= OnStoreChanged;

        _store = store;
        _store.Changed += OnStoreChanged;

        OnStoreChanged();
    }

    /// <summary>Tells the panel how much screen it has, and docks it in it.</summary>
    public void PlaceIn(Vector2 screen)
    {
        _screen = screen;
        Layout();
    }

    private void OnStoreChanged()
    {
        Recount();
        _sheet?.QueueRedraw();
    }

    /// <summary>
    /// One attribute's shelf: the two bottles that raise it, and how many of each are in store.
    /// </summary>
    /// <remarks>
    /// The item types are looked up by name once, because the names are the only stable handle the
    /// data gives -- the numbers are file offsets and move whenever the data does.
    /// </remarks>
    private sealed class Shelf
    {
        public string Attribute;
        public ObjectDesc Small;
        public ObjectDesc Large;
        public Sprite SmallArt;
        public Sprite LargeArt;
        public int SmallCount;
        public int LargeCount;

        /// <summary>Where in storage to find one, so a click can take it out. -1 when there is none.</summary>
        public int SmallAt = -1;

        public int LargeAt = -1;
    }

    private void BuildShelves()
    {
        foreach (string attribute in Attributes)
        {
            var small = _data?.GetObject($"Potion of {attribute}");
            var large = _data?.GetObject($"Greater Potion of {attribute}");

            _shelves.Add(new Shelf
            {
                Attribute = attribute,
                Small = small,
                Large = large,
                SmallArt = small != null ? (_textures?.Resolve(small.Texture) ?? default).Still : default,
                LargeArt = large != null ? (_textures?.Resolve(large.Texture) ?? default).Still : default,
            });
        }
    }

    /// <summary>How many potions are in store altogether, which is what the capacity line counts.</summary>
    private int Stored;

    /// <summary>How many the vault could hold: every slot the account has bought.</summary>
    private int Capacity;

    /// <summary>
    /// Walks storage once and tallies every bottle it recognises.
    /// </summary>
    /// <remarks>
    /// One pass over the item types rather than sixteen searches, and it records the first slot
    /// each kind was found in so that taking one out names a real storage index instead of a
    /// position on this panel.
    /// </remarks>
    private void Recount()
    {
        foreach (var shelf in _shelves)
        {
            shelf.SmallCount = 0;
            shelf.LargeCount = 0;
            shelf.SmallAt = -1;
            shelf.LargeAt = -1;
        }

        Stored = 0;
        Capacity = _store == null ? 0 : _store.ChestCount * VaultStore.SlotsPerChest;

        if (_store == null)
            return;

        var slots = _store.Slots;

        for (int index = 0; index < slots.Count; index++)
        {
            var desc = _store.DescAt(index);
            if (desc == null)
                continue;

            foreach (var shelf in _shelves)
            {
                if (shelf.Small != null && ReferenceEquals(desc, shelf.Small))
                {
                    shelf.SmallCount++;
                    Stored++;

                    if (shelf.SmallAt < 0)
                        shelf.SmallAt = index;

                    break;
                }

                if (shelf.Large != null && ReferenceEquals(desc, shelf.Large))
                {
                    shelf.LargeCount++;
                    Stored++;

                    if (shelf.LargeAt < 0)
                        shelf.LargeAt = index;

                    break;
                }
            }
        }
    }

    private void Layout()
    {
        if (_sheet == null)
            return;

        _sheet.Position = Vector2.Zero;
        _sheet.Size = Body.Size;

        _deposit.Position = new Vector2(Mathf.Round((Body.Size.X - ButtonWidth) / 2f), ButtonTop);
        _deposit.Size = new Vector2(ButtonWidth, ButtonHeight);

        Position = new Vector2(
            Mathf.Round(Mathf.Max(0f,
                _screen.X - HudLayout.Margin * 2f - HudLayout.ColumnWidth - PanelWidth)),
            Mathf.Round(Mathf.Min(TopEdge, Mathf.Max(0f, _screen.Y - Size.Y - BottomEdge))));

        _sheet.QueueRedraw();
    }

    /// <summary>Where a group's box sits in the body, by its place in the two columns.</summary>
    private static Rect2 GroupBox(int index)
    {
        float x = Pad + (index % 2) * (GroupWidth + ColumnGap);
        float y = (index / 2) * GroupPitch;

        return new Rect2(x, y, GroupWidth, CaptionBlock + SlotSize);
    }

    /// <summary>Which bottle is under the pointer, or null for none.</summary>
    private (Shelf Shelf, bool Large)? BottleAt(Vector2 at)
    {
        for (int i = 0; i < _shelves.Count && i < Rows * 2; i++)
        {
            var box = GroupBox(i);
            float top = box.Position.Y + CaptionBlock;

            if (at.Y < top || at.Y >= top + SlotSize)
                continue;

            if (at.X >= box.Position.X && at.X < box.Position.X + SlotSize)
                return (_shelves[i], false);

            if (at.X >= box.Position.X + SlotSize + SlotGap && at.X < box.End.X)
                return (_shelves[i], true);
        }

        return null;
    }

    private void OnSheetInput(InputEvent @event)
    {
        if (@event is not InputEventMouseButton { Pressed: false, ButtonIndex: MouseButton.Left } click)
            return;

        var hit = BottleAt(click.Position);
        if (hit == null)
            return;

        int at = hit.Value.Large ? hit.Value.Shelf.LargeAt : hit.Value.Shelf.SmallAt;
        if (at >= 0)
            Withdraw?.Invoke(new SlotAddress(SlotOwner.Vault, at));
    }

    private void DrawSheet()
    {
        for (int i = 0; i < _shelves.Count && i < Rows * 2; i++)
            DrawGroup(_shelves[i], GroupBox(i));

        DrawCapacity();
    }

    private void DrawGroup(Shelf shelf, in Rect2 box)
    {
        // The caption sits on the board rather than on a plate: it labels the pair of squares
        // under it, and a plate behind it would read as a third square.
        _sheet.DrawText(
            new Vector2(box.Position.X, box.Position.Y + 42f), shelf.Attribute, CaptionSize,
            Style.Text, bold: true);

        DrawChip(new Rect2(box.End.X - ChipWidth, box.Position.Y + ChipTop, ChipWidth, ChipHeight));

        float top = box.Position.Y + CaptionBlock;

        Bottle(new Rect2(box.Position.X, top, SlotSize, SlotSize), shelf.SmallArt, shelf.SmallCount);
        Bottle(new Rect2(box.Position.X + SlotSize + SlotGap, top, SlotSize, SlotSize),
            shelf.LargeArt, shelf.LargeCount);
    }

    /// <summary>One square: the plate, the bottle on it, and how many there are in the corner.</summary>
    private void Bottle(in Rect2 box, in Sprite art, int count)
    {
        _sheet.DrawRect(box, BottlePlate);
        _sheet.DrawRect(box.Grow(-BottleBorder / 2f), BottleEdge,
            filled: false, width: BottleBorder);

        if (art.IsValid)
        {
            // A whole multiple of the source sprite, so every pixel of the bottle is square.
            float source = Mathf.Max(1f, Mathf.Min(art.Region.Size.X, art.Region.Size.Y));
            float room = Mathf.Floor(box.Size.X * 0.62f);
            float side = Mathf.Max(source, Mathf.Floor(room / source) * source);

            _sheet.DrawSprite(art, new Rect2(
                box.Position + (box.Size - new Vector2(side, side)) / 2f, new Vector2(side, side)));
        }

        string text = count.ToString(CultureInfo.InvariantCulture);
        float width = Style.Measure(text, CountSize);

        _sheet.DrawToken(
            new Vector2(box.End.X - width - 6f, box.End.Y - 8f), text, CountSize, Style.Text);
    }

    /// <summary>The olive chip that says the attribute is already at its ceiling.</summary>
    private void DrawChip(in Rect2 box)
    {
        Relief(_sheet, box);

        const string text = "Max";
        float width = Style.Measure(text, ChipSize, bold: true);

        _sheet.DrawText(
            new Vector2(Mathf.Round(box.Position.X + (box.Size.X - width) / 2f),
                box.Position.Y + Style.BaselineIn(box.Size.Y - ReliefShadow, ChipSize)),
            text, ChipSize, Style.Text, bold: true);
    }

    /// <summary>
    /// The three tones every olive control in this interface is built from.
    /// </summary>
    /// <remarks>
    /// A bright band along the top, the face, a darker band under it and then a near-black shadow:
    /// four flat strips rather than a gradient, which is what makes it read as the same pixel art
    /// as everything else on the panel instead of as a web button that wandered in.
    /// </remarks>
    private static void Relief(CanvasItem into, in Rect2 box, bool lit = false)
    {
        var face = lit ? Style.ButtonCommitHigh : Style.ButtonCommit;

        into.DrawRect(box, face);
        into.DrawRect(new Rect2(box.Position, new Vector2(box.Size.X, ReliefTop)),
            Style.ButtonCommitHigh);
        into.DrawRect(new Rect2(box.Position.X, box.End.Y - ReliefShadow - ReliefLow,
            box.Size.X, ReliefLow), Style.ButtonCommitLow);
        into.DrawRect(new Rect2(box.Position.X, box.End.Y - ReliefShadow, box.Size.X, ReliefShadow),
            Style.BarEdge);
    }

    /// <summary>How much is in store against how much there is room for.</summary>
    private void DrawCapacity()
    {
        string text = $"{Stored} / {Capacity}";
        float width = Style.Measure(text, FooterSize, bold: true);

        _sheet.DrawText(
            new Vector2(Mathf.Round((_sheet.Size.X - width) / 2f), CapacityBaseline), text,
            FooterSize, Style.TextDim, bold: true);
    }

    /// <summary>The one control on the panel, in the same olive as the chips above it.</summary>
    private sealed partial class RackButton : Control
    {
        private readonly string _label;
        private bool _hover;

        public RackButton(string label)
        {
            _label = label;
            MouseFilter = MouseFilterEnum.Stop;

            MouseEntered += () => { _hover = true; QueueRedraw(); };
            MouseExited += () => { _hover = false; QueueRedraw(); };
        }

        public event Action Pressed;

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is InputEventMouseButton { Pressed: false, ButtonIndex: MouseButton.Left })
                Pressed?.Invoke();
        }

        public override void _Draw()
        {
            Relief(this, new Rect2(Vector2.Zero, Size), _hover);

            float width = Style.Measure(_label, ButtonSize, bold: true);

            this.DrawText(
                new Vector2(Mathf.Round((Size.X - width) / 2f),
                    Style.BaselineIn(Size.Y - ReliefShadow, ButtonSize)),
                _label, ButtonSize, Style.Text, bold: true);
        }
    }
}
