using System;
using System.Collections.Generic;
using System.Globalization;
using Godot;
using Hendra.Assets;
using Hendra.Resources;
using Hendra.World;

namespace Hendra.UI;

/// <summary>
/// The in-game overlay: the player card, the vitals, the hotbar and everything else drawn over the
/// world.
/// </summary>
/// <remarks>
/// <para>
/// Nine clusters, each pinned to its own corner or edge and each sized from <see cref="HudLayout"/>
/// rather than from a container. That is deliberate. Containers are the right tool when the
/// contents decide the size; here the design decides the size and the contents have to fit it, and
/// a chain of nested boxes each negotiating with its neighbours is how a layout ends up a few
/// pixels away from its reference everywhere.
/// </para>
/// <para>
/// The root ignores the pointer and only the panels take it back. Clicking the dark space between
/// two clusters has to reach the world -- it is where most of the screen is, and a HUD that eats
/// those clicks makes the character stop responding wherever the eye says there is nothing. The
/// arithmetic behind it is checked in <c>HudLayoutTests</c>.
/// </para>
/// <para>
/// Nothing here queries the world. Everything is pushed in once a frame from
/// <see cref="Refresh(LocalPlayer)"/> and the handful of Show methods beside it, and each of those
/// only writes text and widths -- no node is built or freed for a number changing.
/// </para>
/// </remarks>
public partial class HudView : Control
{
    /// <summary>The eight carried slots, which the number keys address.</summary>
    private const int HotbarSlots = 8;

    /// <summary>The first carried slot in the player's 24-entry equipment array.</summary>
    private const int CarriedFirstSlot = 8;

    /// <summary>Worn equipment: weapon, ability, armour and ring.</summary>
    private const int EquipmentSlots = 4;

    /// <summary>The extra carried slots a backpack grants, at indices 16 to 23.</summary>
    private const int BackpackSlots = 8;

    /// <summary>The first of them.</summary>
    private const int BackpackFirstSlot = 16;

    /// <summary>Loot bags carry eight; vaults carry more, but eight is what fits the panel.</summary>
    private const int ContainerSlots = 8;

    /// <summary>The level cap, past which there is no experience left to earn.</summary>
    private const int MaxLevel = 20;

    /// <summary>How many of each potion the server lets a character stack. Its own init.xml value.</summary>
    private const int PotionStackMax = 6;

    /// <summary>Below this share of health the heart pulses.</summary>
    private const float LowHealth = 0.25f;

    private readonly List<SlotView> _equipment = new();
    private readonly List<SlotView> _hotbar = new();
    private readonly List<PartyRow> _partyEntries = new();

    private GameData _data;
    private TextureResolver _textures;

    // --- Clusters ------------------------------------------------------------------------------
    private Control _card;
    private CardPortrait _avatar;
    private Label _name;
    private Label _rating;
    private HudGlyph _ratingStar;
    private Control _guildRow;
    private HudGlyph _guildDot;
    private Label _guild;
    private ClockLine _clock;
    private XpBar _xpBar;

    private HudIconButton _news;

    private CurrencyRow _currency;
    private Control _party;
    private Label _worldLabel;
    private QuestTracker _quest;

    private ColumnPlate _column;
    private Control _iconRow;
    private HudBar _fame;
    private HudBar _health;
    private HudBar _mana;

    private ContainerPanel _containerPanel;
    private VaultView _vaultView;
    private MerchantPanel _merchantPanel;
    private InventoryPage _inventoryPage;
    private Control _hotbarPanel;
    private Control _hotbarTabs;
    private readonly ColumnTab[] _tabs = new ColumnTab[2];
    private EquipmentStrip _equipmentPanel;
    private Control _potionRow;
    private readonly PotionCell[] _potions = new PotionCell[HudLayout.PotionSlots];



    private InteractBlock _prompt;

    // --- Events --------------------------------------------------------------------------------

    /// <summary>Raised with the slot's index in the player's 24-entry equipment array.</summary>
    public event Action<int> SlotActivated;

    /// <summary>Raised with the slot's index in the open container.</summary>
    public event Action<int> ContainerSlotActivated;

    /// <summary>A vault or gift slot was clicked: the quick move out to the inventory.</summary>
    public event Action<World.SlotAddress> VaultSlotActivated;

    /// <summary>A locked vault row was clicked and the purchase should begin.</summary>
    public event Action VaultPurchaseRequested;

    /// <summary>Raised when an item is dragged from one slot onto another.</summary>
    public event Action<SlotAddress, SlotAddress> SlotDropped;

    /// <summary>Raised when an item is dragged out of a slot and let go over the world.</summary>
    public event Action<SlotAddress> SlotDroppedOutside;

    /// <summary>Raised when the buy button is pressed at a vendor.</summary>
    public event Action BuyPressed;

    /// <summary>Raised by a potion counter, with true for health.</summary>
    public event Action<bool> PotionRequested;

    /// <summary>Asked whether a dragged slot may be dropped on a potion counter.</summary>
    /// <remarks>
    /// A question rather than an event because the answer is needed while the drag is still in the
    /// air -- Godot refuses the drop itself if this is false, so the wrong potion never lands.
    /// </remarks>
    public event Func<SlotAddress, bool, bool> PotionAccepted;

    /// <summary>Raised when a potion is dropped onto a counter, with true for health.</summary>
    public event Action<SlotAddress, bool> PotionStacked;

    public event Action OptionsPressed;

    /// <summary>The buttons on the card, wired as named events rather than to panels.</summary>
    public event Action AccountPressed;

    public event Action StatsPressed;

    public event Action ShopPressed;

    public event Action NewsPressed;

    /// <summary>Raised with the name of the party member whose row was clicked.</summary>
    public event Action<string> PartyMemberActivated;

    public void Configure(AssetLibrary assets, GameData data)
    {
        _data = data;
        _textures = new TextureResolver(assets);

        // The potion row may already be built, depending on which happens first.
        _potions[0]?.UseSprite(SpriteOf("Health Potion"));
        _potions[1]?.UseSprite(SpriteOf("Magic Potion"));
    }

    public override void _Ready()
    {
        // The whole point of the overlay: it is a hole the world can be clicked through, and only
        // the panels below patch it.
        MouseFilter = MouseFilterEnum.Ignore;

        // The column's plate first, so every band in it is drawn over its own background rather
        // than over the world.
        BuildColumn();

        BuildPlayerCard();
        BuildCurrency();
        BuildIconRow();
        BuildVitals();
        BuildEquipment();
        BuildHotbar();
        BuildPotions();
        BuildParty();
        BuildInteractions();
        BuildPrompt();

        Resized += Reflow;
        if (GetParent() is HudLayer layer)
            layer.Reflowed += Reflow;

        Reflow();
    }

    /// <summary>
    /// The space the layout is solved in.
    /// </summary>
    /// <remarks>
    /// The control's own rectangle, which the layer sizes in reference pixels. Falls back to the
    /// reference resolution for the one frame before the layer has measured the window, so nothing
    /// is ever laid out against a zero-sized screen.
    /// </remarks>
    private HudLayout Layout => new(
        Size.X > 0f && Size.Y > 0f ? Size : new Vector2(HudLayout.ReferenceWidth, HudLayout.ReferenceHeight),
        _guildRow is { Visible: true });

    /// <summary>Re-pins every cluster. Cheap, and only run when the window or the card changes.</summary>
    private void Reflow()
    {
        // The Resized signal can arrive before the clusters exist, since sizing a control is what
        // the layer does the moment it is added to one.
        if (_card == null)
            return;

        var layout = Layout;

        Place(_card, layout.PlayerCard);
        LayoutCard(layout.PlayerCard.Size);

        Place(_clock, layout.Clock);
        Place(_xpBar, layout.XpBar);
        Place(_currency, layout.Currency);
        Place(_quest, layout.Quest);

        LayoutColumn(layout);

        // The panels that are not part of the reference are hung off the ones that are, so they
        // move with them rather than needing their own corner.
        StackInteractions(layout);

        if (_prompt != null)
        {
            var block = layout.Interact;
            Place(_prompt, block);

            var plate = layout.InteractButton;
            _prompt.ButtonRect = new Rect2(plate.Position - block.Position, plate.Size);
        }
    }

    /// <summary>
    /// Pins every band of the right-hand column.
    /// </summary>
    /// <remarks>
    /// Each band is placed from the layout directly rather than stacked off the one above it. The
    /// bands butt together with no gaps in the reference and every one of their heights is a
    /// measured constant, so stacking would turn a single mis-measured band into an error that
    /// grows all the way down the column.
    /// </remarks>
    private void LayoutColumn(in HudLayout layout)
    {
        Place(_column, layout.Column);
        Place(_iconRow, layout.IconRow);

        for (int i = 0; i < _iconRow.GetChildCount(); i++)
        {
            var icon = layout.IconAt(i);
            var button = _iconRow.GetChild<Control>(i);

            button.Position = icon.Position - layout.IconRow.Position;
            button.Size = icon.Size;
        }

        Place(_fame, layout.FameBar);
        Place(_health, layout.HealthBar);
        Place(_mana, layout.ManaBar);

        var strip = layout.EquipmentRow;
        Place(_equipmentPanel, strip);

        var cells = new Rect2[_equipment.Count];
        for (int i = 0; i < _equipment.Count; i++)
        {
            // The bevel wraps the plate on three sides and stops at its foot, which is what leaves
            // the light frame an even band along the bottom of the strip.
            var slot = layout.EquipmentSlot(i);
            cells[i] = new Rect2(
                slot.Position - strip.Position,
                slot.Size - new Vector2(0f, HudLayout.EquipmentBevel));

            // The slot itself is the plate inside the strip's bevel; the strip draws the bevel.
            _equipment[i].Position = slot.Position + Vector2.One * HudLayout.EquipmentBevel;
            _equipment[i].Size = slot.Size - Vector2.One * (2f * HudLayout.EquipmentBevel);
        }

        _equipmentPanel.Cells = cells;
        _equipmentPanel.QueueRedraw();

        Place(_inventoryPage, layout.InventoryPanel);
        Place(_hotbarTabs, layout.HotbarTabs);
        LayoutTabs(layout.HotbarTabs.Size);

        Place(_hotbarPanel, layout.Hotbar);
        for (int i = 0; i < _hotbar.Count; i++)
        {
            var slot = layout.HotbarSlot(i);
            _hotbar[i].Position = slot.Position - layout.Hotbar.Position;
            _hotbar[i].Size = slot.Size;
        }

        Place(_potionRow, layout.PotionRow);
        for (int i = 0; i < _potions.Length; i++)
        {
            var cell = layout.PotionSlot(i);
            _potions[i].Position = cell.Position - layout.PotionRow.Position;
            _potions[i].Size = cell.Size;
        }

        Place(_worldLabel, layout.PartyHeader);
        Place(_party, layout.Party);

        int fits = layout.PartyRowsThatFit * HudLayout.PartyColumns;
        for (int i = 0; i < _partyEntries.Count; i++)
        {
            var row = layout.PartyEntry(i);
            _partyEntries[i].Position = row.Position;
            _partyEntries[i].Size = row.Size;

            if (i >= fits)
                _partyEntries[i].Visible = false;
        }
    }

    /// <summary>Two tabs, half the strip each, with the page showing between them.</summary>
    private void LayoutTabs(Vector2 size)
    {
        float half = Mathf.Round((size.X - HudLayout.TabGap) / 2f);

        _tabs[0].Position = Vector2.Zero;
        _tabs[0].Size = new Vector2(half, size.Y);
        _tabs[1].Position = new Vector2(half + HudLayout.TabGap, 0f);
        _tabs[1].Size = new Vector2(size.X - half - HudLayout.TabGap, size.Y);
    }

    /// <summary>
    /// Stacks whatever is at the player's feet against the column, growing upward.
    /// </summary>
    /// <remarks>
    /// Beside the column rather than under it: the column runs the full height of the screen now
    /// and there is no space below the inventory for a panel to open into. They stack rather than
    /// share a rectangle because standing on a bag next to a vendor is an ordinary thing to do, and
    /// the two used to overwrite each other's title.
    /// </remarks>
    private void StackInteractions(in HudLayout layout)
    {
        if (_containerPanel == null)
            return;

        float right = layout.ColumnLeft - HudLayout.ModalGutter;
        float bottom = layout.Size.Y - HudLayout.Margin;

        foreach (Control panel in new Control[] { _merchantPanel, _containerPanel })
        {
            if (!panel.Visible)
                continue;

            panel.Position = new Vector2(right - panel.Size.X, bottom - panel.Size.Y);
            bottom -= panel.Size.Y + 8f;
        }
    }

    private static void Place(Control control, Rect2 rect)
    {
        if (control == null)
            return;

        control.Position = rect.Position;
        control.Size = rect.Size;
    }

    // ---------------------------------------------------------------------------------------------
    // The top-left corner: who you are, the clock, and what you are working on
    // ---------------------------------------------------------------------------------------------

    /// <summary>
    /// Top left: who you are.
    /// </summary>
    /// <remarks>
    /// Flush into the corner and almost entirely unbacked -- outlined text straight onto the world,
    /// with one translucent plate under the portrait. The plate the card used to be, and the row of
    /// buttons on it, are both gone: the buttons moved to the column's icon row, where the
    /// reference keeps them, and a plate around four lines of outlined text is a box drawn around
    /// nothing.
    /// </remarks>
    private void BuildPlayerCard()
    {
        _card = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_card);

        _avatar = new CardPortrait();
        _card.AddChild(_avatar);

        _name = new Label { ClipText = true, TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis }
            .TypesetOverWorld(NameSize, Style.Text);
        _card.AddChild(_name);

        _rating = new Label { HorizontalAlignment = HorizontalAlignment.Right }
            .TypesetOverWorld(RatingSize, Style.Text);
        _card.AddChild(_rating);

        // The star beside the rating is the reference's own blue, not a colour that climbs with the
        // number: the number already says how many, and a star that changes hue as well says it
        // twice in a way that has to be learnt first.
        _ratingStar = new HudGlyph(HudIcons.Star, CardInk.RatingStar);
        _card.AddChild(_ratingStar);

        // Hidden until the player turns out to have a guild, so a guildless character never sees a
        // shield with nothing beside it.
        _guildRow = new Control { MouseFilter = MouseFilterEnum.Ignore, Visible = false };
        _card.AddChild(_guildRow);

        _guildDot = new HudGlyph(HudIcons.Shield, CardInk.Guild);
        _guildRow.AddChild(_guildDot);

        _guild = new Label().TypesetOverWorld(GuildSize, CardInk.Guild);
        _guildRow.AddChild(_guild);

        _clock = new ClockLine();
        AddChild(_clock);

        _xpBar = new XpBar();
        AddChild(_xpBar);
    }

    /// <summary>The player's own name, which is the largest string outside the column.</summary>
    private const int NameSize = 34;

    private const int RatingSize = 28;
    private const int GuildSize = 28;

    /// <summary>
    /// Places the portrait, the two lines beside it and the rating against the far edge.
    /// </summary>
    /// <remarks>
    /// The guild line is hidden for a player without one and leaves no gap: it sits under the name
    /// in the column beside the portrait, and the corner is as tall as the portrait either way.
    /// </remarks>
    private void LayoutCard(Vector2 size)
    {
        _avatar.Position = new Vector2(HudLayout.AvatarLeft, HudLayout.AvatarTop);
        _avatar.Size = new Vector2(HudLayout.AvatarSize, HudLayout.AvatarSize);

        const float textLeft = HudLayout.CardTextLeft;

        // The rating and its star are right-aligned against the far edge of the corner, and the
        // name truncates rather than running under them.
        var star = new Rect2(size.X - 12f - 30f, 22f, 30f, 30f);
        _ratingStar.Position = star.Position;
        _ratingStar.Size = star.Size;

        _rating.Position = new Vector2(star.Position.X - 90f, 20f);
        _rating.Size = new Vector2(84f, 34f);

        _name.Position = new Vector2(textLeft, 6f);
        _name.Size = new Vector2(_rating.Position.X - 10f - textLeft, 36f);

        _guildRow.Position = new Vector2(textLeft, 42f);
        _guildRow.Size = new Vector2(size.X - textLeft, 32f);
        _guildDot.Position = new Vector2(4f, 3f);
        _guildDot.Size = new Vector2(24f, 26f);
        _guild.Position = new Vector2(34f, 0f);
        _guild.Size = new Vector2(_guildRow.Size.X - 34f, 32f);
    }

    /// <summary>
    /// Shows or hides the unread mark on the news icon.
    /// </summary>
    /// <remarks>
    /// The icon is on the column now. Kept as a method because the world controller calls it and
    /// the mark is still worth carrying.
    /// </remarks>
    public void SetUnreadNews(bool unread)
    {
        if (_news != null)
            _news.Badge = unread ? 1 : 0;
    }

    // ---------------------------------------------------------------------------------------------
    // 2.3 Currency
    // ---------------------------------------------------------------------------------------------

    private void BuildCurrency()
    {
        _currency = new CurrencyRow();
        AddChild(_currency);
    }

    /// <summary>
    /// Gems and coins, right-aligned so a number that gains a digit grows away from the map.
    /// </summary>
    /// <remarks>
    /// The displayed number chases the real one over about three hundred milliseconds and the icon
    /// brightens for two hundred when it changes, which is what makes a pickup read as a pickup
    /// rather than as a number that was already different the next time you looked.
    /// </remarks>
    private sealed partial class CurrencyRow : Control
    {
        private const float IconSize = 20f;
        private const float LabelGap = 8f;
        private const float PairGap = 18f;
        private const float FlashSeconds = 0.2f;

        private int _gems;
        private int _coins;
        private float _shownGems;
        private float _shownCoins;
        private float _gemFlash;
        private float _coinFlash;

        public CurrencyRow()
        {
            // Two numbers drawn over the world. There is nothing to click, and swallowing a click
            // here would stop the character for no reason.
            MouseFilter = MouseFilterEnum.Ignore;
        }

        public void Set(int gems, int coins)
        {
            if (gems != _gems)
                _gemFlash = FlashSeconds;

            if (coins != _coins)
                _coinFlash = FlashSeconds;

            _gems = gems;
            _coins = coins;
        }

        public override void _Process(double delta)
        {
            float before = _shownGems + _shownCoins + _gemFlash + _coinFlash;

            // A little over three hundred milliseconds to close the gap, whatever its size.
            float t = 1f - Mathf.Exp(-10f * (float)delta);
            _shownGems = Mathf.Lerp(_shownGems, _gems, t);
            _shownCoins = Mathf.Lerp(_shownCoins, _coins, t);

            if (Mathf.Abs(_shownGems - _gems) < 0.75f)
                _shownGems = _gems;
            if (Mathf.Abs(_shownCoins - _coins) < 0.75f)
                _shownCoins = _coins;

            _gemFlash = Mathf.Max(0f, _gemFlash - (float)delta);
            _coinFlash = Mathf.Max(0f, _coinFlash - (float)delta);

            if (!Mathf.IsEqualApprox(before, _shownGems + _shownCoins + _gemFlash + _coinFlash))
                QueueRedraw();
        }

        public override void _Draw()
        {
            float right = Size.X;
            right = Pair(right, _shownCoins, HudIcons.Coin, Style.IconGold, _coinFlash);
            Pair(right - PairGap, _shownGems, HudIcons.Fame, Style.IconFame, _gemFlash);
        }

        /// <summary>Draws one number and its icon, right to left, and answers where it ended.</summary>
        private float Pair(
            float right, float value, Action<CanvasItem, Rect2, Color> icon, Color colour, float flash)
        {
            float top = (Size.Y - IconSize) / 2f;
            var box = new Rect2(right - IconSize, top, IconSize, IconSize);

            icon(this, box, colour.Lightened(flash / FlashSeconds * 0.5f));

            string text = ((int)MathF.Round(value)).ToString(CultureInfo.InvariantCulture);
            float width = Style.Measure(text, Style.FontName);

            var at = new Vector2(
                Mathf.Round(box.Position.X - LabelGap - width),
                Style.BaselineIn(Size.Y, Style.FontName));

            this.DrawOverWorld(at, text, Style.FontName, Style.Text);

            return at.X;
        }
    }

    // ---------------------------------------------------------------------------------------------
    // 2.5 Party
    // ---------------------------------------------------------------------------------------------

    /// <summary>
    /// The plate the whole right-hand column stands on.
    /// </summary>
    /// <remarks>
    /// Built first and added first, so it is behind every band. Nothing else in the column draws a
    /// background of its own except the inventory page, which is a genuinely different surface.
    /// </remarks>
    private void BuildColumn()
    {
        _column = new ColumnPlate();
        AddChild(_column);
    }

    /// <summary>
    /// The row of small buttons between the map and the bars.
    /// </summary>
    /// <remarks>
    /// Six marks, in the reference's own order and at its own stops: stats, pet, alignment, quests,
    /// party, and settings pushed against the right edge. Two of them -- the pet and the alignment
    /// -- open panels this server has no protocol for, and one -- the party -- has no party system
    /// behind it, so all three are drawn greyed rather than left out. A row that closes up around a
    /// missing icon moves every icon after it, and the row's shape is half of what makes it
    /// recognisable.
    /// </remarks>
    private void BuildIconRow()
    {
        _iconRow = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_iconRow);

        _iconRow.AddChild(ColumnIcon(HudIcons.BarChart, "Character", () => StatsPressed?.Invoke()));
        _iconRow.AddChild(ColumnIcon(HudIcons.Cat, "Account", () => AccountPressed?.Invoke()));
        _iconRow.AddChild(ColumnIcon(HudIcons.Alignment, "Alignment", null));

        _news = ColumnIcon(HudIcons.Flag, "News", () => NewsPressed?.Invoke());
        _iconRow.AddChild(_news);

        _iconRow.AddChild(ColumnIcon(HudIcons.Sword, "Party", null));
        _iconRow.AddChild(ColumnIcon(HudIcons.Gear, "Options", () => OptionsPressed?.Invoke()));
    }

    /// <summary>One button in that row. A null action greys it out.</summary>
    private static HudIconButton ColumnIcon(
        Action<CanvasItem, Rect2, Color> icon, string tooltip, Action pressed)
    {
        var button = new HudIconButton(icon, tooltip, inset: 0f)
        {
            Hover = Style.Panel.Lightened(0.18f),
            Disabled = pressed == null,
        };

        if (pressed != null)
            button.Pressed += pressed;

        return button;
    }

    private void BuildParty()
    {
        _worldLabel = new Label
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        }.Typeset(WorldNameSize, ColumnInk.WorldName);

        AddChild(_worldLabel);

        _party = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_party);

        _quest = new QuestTracker();
        AddChild(_quest);

        // Two columns, filled left to right and collapsing upward: an empty row draws nothing
        // rather than leaving a hole in the list.
        for (int i = 0; i < HudLayout.PartyColumns * HudLayout.PartyRows; i++)
        {
            var entry = new PartyRow { Visible = false };
            entry.Activated += name => PartyMemberActivated?.Invoke(name);

            _party.AddChild(entry);
            _partyEntries.Add(entry);
        }
    }

    /// <summary>The world's name, which is the quietest heading in the column.</summary>
    private const int WorldNameSize = 30;

    /// <summary>
    /// Says which world this is and how full it is.
    /// </summary>
    /// <remarks>
    /// The count is the players this client can see, which in a Nexus is very nearly everyone in it
    /// and in a realm is the ones near you. Nothing on the wire carries a world's true population
    /// or its ceiling -- the server list's usage fraction is per server, not per world -- so the
    /// capacity is only shown when the caller knows one.
    /// </remarks>
    public void ShowWorld(string name, int population, int capacity)
    {
        if (_worldLabel == null)
            return;

        string text = string.IsNullOrEmpty(name) ? string.Empty
            : capacity > 0 ? $"{name} ({population}/{capacity})"
            : $"{name} ({population})";

        if (_worldLabel.Text != text)
            _worldLabel.Text = text;
    }

    /// <summary>
    /// Names what the realm has asked for, over the objective panel.
    /// </summary>
    /// <remarks>
    /// The heading tier of the tracker: the longer-running thing you are inside, which for this
    /// server is whatever the realm is currently pointing at. The panel under it carries the
    /// objective with a bar on it; see <see cref="RefreshQuest"/>.
    /// </remarks>
    public void ShowQuest(string name, ushort objectType, bool isNew)
    {
        _questName = name ?? string.Empty;
        _questIsNew = isNew;
    }

    private string _questName = string.Empty;

    private bool _questIsNew;

    /// <summary>Lists the nearby players. Writes into the existing rows rather than rebuilding them.</summary>
    public void ShowParty(IReadOnlyList<PartyMember> members)
    {
        if (_party == null)
            return;

        for (int i = 0; i < _partyEntries.Count; i++)
        {
            if (i >= members.Count)
            {
                _partyEntries[i].Visible = false;
                continue;
            }

            // Starred players take the second colour. The reference lists two -- a gold for
            // everyone and a teal for the ones you already know -- and starred is the only such
            // distinction this server puts on the wire.
            var member = members[i];
            _partyEntries[i].Set(member.Name, ClassPortrait(member.ObjectType), member.Starred);
        }
    }

    /// <summary>
    /// A class's standing frame, for the party list and the card.
    /// </summary>
    /// <remarks>
    /// Kept by object type rather than resolved per frame. There are fourteen classes and the
    /// answer never changes, while the party list is written to twice a second.
    /// </remarks>
    private Assets.Sprite ClassPortrait(ushort objectType)
    {
        if (_portraits.TryGetValue(objectType, out var cached))
            return cached;

        var sprite = default(Assets.Sprite);
        var resolved = _textures?.Resolve(_data?.GetObject(objectType)?.Texture) ?? default;

        if (resolved.Animated != null)
            sprite = resolved.Animated.Frame(0f, 0f, CharAction.Stand, 0f).Sprite;
        else if (resolved.Still.IsValid)
            sprite = resolved.Still;

        _portraits[objectType] = sprite;
        return sprite;
    }

    private readonly Dictionary<ushort, Assets.Sprite> _portraits = new();

    // ---------------------------------------------------------------------------------------------
    // The bars, and the potions that refill two of them
    // ---------------------------------------------------------------------------------------------

    /// <summary>
    /// Fame, health and magic: three full-width bars down the column, under the icon row.
    /// </summary>
    /// <remarks>
    /// Each is the whole width of the column rather than one of several things sharing a row. That
    /// is what makes the three of them read as one block you check with a glance rather than as
    /// three widgets you have to find first -- and it is why the value is centred on the bar and
    /// not pushed into a corner of it.
    /// </remarks>
    private void BuildVitals()
    {
        _fame = Bar(Style.FameFill, Style.FameFillHigh, Style.Text);
        _health = Bar(Style.HpFill, Style.HpFillHigh, Style.StatNumber);
        _mana = Bar(Style.MpFill, Style.MpFillHigh, Style.StatNumber);
    }

    /// <summary>An item's still, by the name the data files give it, or nothing if it has none.</summary>
    private Assets.Sprite SpriteOf(string id)
    {
        var desc = _data?.GetObject(id ?? string.Empty);
        return desc == null ? default : (_textures?.Resolve(desc.Texture) ?? default).Still;
    }

    private HudBar Bar(Color fill, Color high, Color value)
    {
        var bar = new HudBar(fill, high) { ValueColour = value };

        AddChild(bar);
        return bar;
    }

    /// <summary>
    /// The three potion cells along the bottom of the inventory.
    /// </summary>
    /// <remarks>
    /// Three because the reference has three, and the third is drawn as an empty cell: this
    /// server's protocol carries a health stack and a magic stack and nothing else, and a cell
    /// showing a permanent nought out of nought would be a lie where an empty plate is only a slot
    /// with nothing in it.
    /// </remarks>
    private void BuildPotions()
    {
        _potionRow = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_potionRow);

        _potions[0] = Potions(true);
        _potions[1] = Potions(false);

        _potions[2] = new PotionCell(false, 0);
        _potionRow.AddChild(_potions[2]);
    }

    private PotionCell Potions(bool health)
    {
        var cell = new PotionCell(health, PotionStackMax);

        cell.Pressed += () => PotionRequested?.Invoke(health);
        cell.Accepts = from => PotionAccepted?.Invoke(from, health) ?? false;
        cell.Filled += from => PotionStacked?.Invoke(from, health);
        cell.UseSprite(SpriteOf(health ? "Health Potion" : "Magic Potion"));

        _potionRow.AddChild(cell);
        return cell;
    }

    private void BuildHotbar()
    {
        // The page the slots and the potions sit on, added before either, so both are drawn over it.
        _inventoryPage = new InventoryPage();
        AddChild(_inventoryPage);

        _hotbarTabs = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_hotbarTabs);

        // Two pages of eight. The first is what the character carries, the second is the backpack,
        // which is a separate eight slots on the wire rather than a continuation of the first.
        _tabs[0] = NewTab(HudIcons.Pouch, "Carried", 0);
        _tabs[1] = NewTab(HudIcons.Chest, "Backpack", 1);

        _hotbarPanel = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_hotbarPanel);

        for (int i = 0; i < HotbarSlots; i++)
        {
            int at = i;
            var slot = NewSlot(default);

            // One through eight, whichever page is showing: the keys address the visible slots, so
            // the number on a square is always the key that uses it.
            slot.Hotkey = (i + 1).ToString(CultureInfo.InvariantCulture);
            slot.EmptyNumberColour = ColumnInk.EmptySlotNumber;
            slot.EmptyNumberSize = EmptySlotNumberSize;
            slot.Activated += () => SlotActivated?.Invoke(HotbarFirstSlot + at);

            _hotbarPanel.AddChild(slot);
            _hotbar.Add(slot);
        }

        SetHotbarPage(App.ServiceLocator.Settings?.HotbarPage ?? 0, save: false);
    }

    /// <summary>The figure an empty carried slot carries, which is half the height of the cell.</summary>
    private const int EmptySlotNumberSize = 56;

    private ColumnTab NewTab(Action<CanvasItem, Rect2, Color> icon, string tooltip, int page)
    {
        var tab = new ColumnTab(icon) { TooltipText = tooltip };
        tab.Pressed += () => SetHotbarPage(page, save: true);

        _hotbarTabs.AddChild(tab);
        return tab;
    }

    /// <summary>Steps to the next carried page. Bound to B, as the original bound its tab key.</summary>
    public void SwitchTab() => SetHotbarPage(_hotbarPage == 0 ? 1 : 0, save: true);

    /// <summary>The first slot of the page the hotbar is showing. The number keys address from here.</summary>
    public int HotbarFirstSlot => _hotbarPage == 0 ? CarriedFirstSlot : BackpackFirstSlot;

    private int _hotbarPage;

    /// <summary>Whether the character owns a backpack, which is what makes the second page real.</summary>
    private bool _backpackVisible;

    /// <summary>
    /// Shows one of the two carried pages.
    /// </summary>
    /// <remarks>
    /// The keys follow the page rather than the page following the keys, which is the only
    /// arrangement that does not need explaining: 3 always means the third square you can see.
    /// </remarks>
    private void SetHotbarPage(int page, bool save)
    {
        page = Mathf.Clamp(page, 0, 1);

        // A backpack page is only real once the character owns one.
        if (page == 1 && !_backpackVisible)
            page = 0;

        _hotbarPage = page;
        _tabs[0].Active = page == 0;
        _tabs[1].Active = page == 1;

        for (int i = 0; i < _hotbar.Count; i++)
            _hotbar[i].Address = new SlotAddress(SlotOwner.Player, HotbarFirstSlot + i);

        if (!save)
            return;

        var settings = App.ServiceLocator.Settings;
        if (settings == null || settings.HotbarPage == page)
            return;

        settings.HotbarPage = page;
        settings.Save();
    }

    private void BuildEquipment()
    {
        _equipmentPanel = new EquipmentStrip();
        AddChild(_equipmentPanel);

        for (int i = 0; i < EquipmentSlots; i++)
        {
            int index = i;
            var slot = NewSlot(new SlotAddress(SlotOwner.Player, index));
            slot.Activated += () => SlotActivated?.Invoke(index);

            // The strip draws the bevel around each of these, so the slot itself is only a plate.
            slot.BorderWidth = 0f;

            // The weapon and the ability are the one piece of the control scheme written nowhere
            // else, so each slot names the action it fires and draws whatever that is bound to.
            if (i == 0)
                slot.BoundAction = "shoot";
            else if (i == 1)
                slot.BoundAction = "use_ability";

            AddChild(slot);
            _equipment.Add(slot);
        }

        // No loadout cycle. Its two arrows were drawn for it and nothing else, and behind them was
        // a message saying the feature does not exist on this server.
    }

    private SlotView NewSlot(SlotAddress address)
    {
        var slot = new SlotView { Address = address, Draggable = true };
        slot.Dropped += (from, to) => SlotDropped?.Invoke(from, to);
        slot.DroppedOutside += from =>
        {
            // Only when it was let go over the world. Godot calls a drag unsuccessful whenever
            // nothing accepted it, which includes letting go over the chat panel or the minimap --
            // and dropping a good item on the floor because you released over the wrong bit of
            // interface is not a mistake anyone should be able to make.
            if (!Layout.HitsCluster(GetLocalMousePosition()))
                SlotDroppedOutside?.Invoke(from);
        };
        return slot;
    }

    /// <summary>Flashes a carried slot's border, so a key press is visibly acknowledged.</summary>
    public void FlashSlot(int slotIndex)
    {
        int at = slotIndex - CarriedFirstSlot;
        if (at >= 0 && at < _hotbar.Count)
            _hotbar[at].Flash();
    }

    /// <summary>
    /// Sets the wipe over the ability slot.
    /// </summary>
    /// <remarks>
    /// Handed in from the clock the cooldown was started against rather than counted down here, so
    /// thirty seconds of being tabbed out does not become thirty seconds of cooldown.
    /// </remarks>
    public void SetAbilityCooldown(float remainingMs, float totalMs)
    {
        if (_equipment.Count > 1)
            _equipment[1].SetCooldown(remainingMs, totalMs);
    }

    // ---------------------------------------------------------------------------------------------
    // The panels the reference does not show, because in it they are closed
    // ---------------------------------------------------------------------------------------------

    /// <summary>What is at the player's feet: a container's contents, and a vendor's wares.</summary>
    private void BuildInteractions()
    {
        _containerPanel = new ContainerPanel();
        _containerPanel.Activated += index => ContainerSlotActivated?.Invoke(index);
        _containerPanel.Dropped += (from, to) => SlotDropped?.Invoke(from, to);
        AddChild(_containerPanel);

        _merchantPanel = new MerchantPanel();
        _merchantPanel.Pressed += () => BuyPressed?.Invoke();
        AddChild(_merchantPanel);
    }

    /// <summary>
    /// Builds the vault panel, which needs the store and so cannot be made with the rest.
    /// </summary>
    /// <remarks>
    /// Once, on the first world that has a vault in it. It is a large panel and most sessions never
    /// open it, so it is not built for the sake of the Nexus.
    /// </remarks>
    public void UseVault(World.VaultStore store)
    {
        if (_vaultView != null)
        {
            // The panel is kept; the store behind it is not. See VaultView.Use.
            _vaultView.Use(store);
            return;
        }

        _vaultView = new VaultView(store, _data, _textures);
        _vaultView.Dropped += (from, to) => SlotDropped?.Invoke(from, to);
        _vaultView.Activated += address => VaultSlotActivated?.Invoke(address);
        _vaultView.PurchaseRequested += () => VaultPurchaseRequested?.Invoke();
        AddChild(_vaultView);

        _vaultView.SetSlotTypes(_slotTypes);
    }

    /// <summary>Whether the vault panel is on screen.</summary>
    public bool VaultOpen => _vaultView is { Visible: true };

    /// <summary>Whether the vault's search field has the keyboard.</summary>
    public bool VaultTyping => _vaultView is { Visible: true } && _vaultView.IsTyping;

    /// <summary>Opens or closes the vault panel.</summary>
    public void ShowVault(bool show)
    {
        if (_vaultView == null || _vaultView.Visible == show)
            return;

        if (show)
        {
            _vaultView.SetSlotTypes(_slotTypes);
            _vaultView.PlaceIn(Size);
            _vaultView.Open();
        }
        else
        {
            _vaultView.Close();
        }
    }

    private void BuildPrompt()
    {
        // At the foot of the column rather than over the world: it names a place you can go, which
        // is the same kind of thing as the name of the place you are already in, and the reference
        // puts the two in the same rectangle.
        _prompt = new InteractBlock();
        _prompt.Pressed += PressInteract;

        AddChild(_prompt);
    }

    // ---------------------------------------------------------------------------------------------
    // The once-a-frame update
    // ---------------------------------------------------------------------------------------------

    /// <summary>Refreshes from the player. Safe to call every frame; does nothing without one.</summary>
    public void Refresh(LocalPlayer player)
    {
        if (player == null)
            return;

        if (_backpackVisible != player.HasBackpack)
        {
            _backpackVisible = player.HasBackpack;
            _tabs[1].Enabled = _backpackVisible;

            // A bag that has just been lost cannot go on being the page in front of you.
            if (!_backpackVisible && _hotbarPage == 1)
                SetHotbarPage(0, save: false);
        }

        RefreshIdentity(player);
        RefreshVitals(player);

        // Slots 0-3 are worn; the hotbar shows eight of the sixteen carried, from whichever page
        // its tabs have selected.
        UpdateSlots(_equipment, player, 0);
        UpdateSlots(_hotbar, player, HotbarFirstSlot);
    }

    /// <summary>Name, rating, guild and the two currencies.</summary>
    private void RefreshIdentity(LocalPlayer player)
    {
        string name = string.IsNullOrEmpty(player.Name) ? "—" : player.Name;
        if (_name.Text != name)
            _name.Text = name;

        bool guild = !string.IsNullOrEmpty(player.Guild);
        if (_guildRow.Visible != guild)
        {
            _guildRow.Visible = guild;
            Reflow();
        }

        if (guild && _guild.Text != player.Guild)
            _guild.Text = player.Guild;

        // The rating is the character's own stars, on the original's fame thresholds.
        int stars = Fame.Stars(player.Fame);
        string rating = stars.ToString(CultureInfo.InvariantCulture);

        if (_rating.Text != rating)
            _rating.Text = rating;

        _currency.Set(player.Fame, player.Credits);
        _avatar.Set(ClassPortrait(player.ObjectType));

        _clock.Set(player.Level);
        _xpBar.Set(player.Level >= 0 && player.Level < MaxLevel && player.NextLevelExperience > 0
            ? player.Experience / (float)player.NextLevelExperience
            : 1f);

        RefreshQuest(player);
    }

    /// <summary>
    /// The objective tracker: what the realm is pointing at, and the class quest under it.
    /// </summary>
    /// <remarks>
    /// Two tiers because the reference has two, and both are things this server actually sends.
    /// The heading is the realm's current target; the panel is the class quest, which is the one
    /// long-running objective a character has -- fame towards the next star, with the star count
    /// in the tally box at the end of the bar.
    /// </remarks>
    private void RefreshQuest(LocalPlayer player)
    {
        int stars = Fame.Stars(player.Fame);
        int goal = Fame.NextThreshold(player.Fame);

        string progress = goal > 0
            ? $"{player.Fame}/{goal}"
            : player.Fame.ToString(CultureInfo.InvariantCulture);

        _quest.Set(
            _questName,
            _questIsNew ? "NEW" : string.Empty,
            "Class Quest",
            goal > 0 ? $"{goal - player.Fame} to go" : "complete",
            progress,
            goal > 0 ? player.Fame / (float)goal : 1f,
            stars.ToString(CultureInfo.InvariantCulture));
    }

    /// <summary>
    /// Which of the bars write their numbers on themselves. See <see cref="App.Settings.BarText"/>.
    /// </summary>
    /// <remarks>
    /// A bar with no number is still a bar: the fill says roughly where you are, which is what the
    /// setting is for -- someone who finds the figures noisy still wants the length.
    /// </remarks>
    private static bool VitalNumbers => (App.ServiceLocator.Settings?.BarText ?? 3) is 2 or 3;

    private static bool ProgressNumbers => (App.ServiceLocator.Settings?.BarText ?? 3) is 1 or 3;

    private void RefreshVitals(LocalPlayer player)
    {
        RefreshProgress(player);

        _health.Set(player.Hp, player.MaxHp, "HP", VitalNumbers
            ? Vital(player.Hp, player.MaxHp, HealthRegen(player), player.Boosts[0])
            : string.Empty);

        _mana.Set(player.Mp, player.MaxMp, "MP", VitalNumbers
            ? Vital(player.Mp, player.MaxMp, ManaRegen(player), player.Boosts[1])
            : string.Empty);

        _potions[0].Set(player.HealthPotions, PotionStackMax);
        _potions[1].Set(player.MagicPotions, PotionStackMax);
    }

    /// <summary>
    /// One vital's whole line: what you have of it, how fast it comes back, and what kit is adding.
    /// </summary>
    /// <remarks>
    /// Three facts in one string, in the order the reference writes them -- <c>760/760|6 (+90)</c>.
    /// The bar between the total and the regeneration is what tells the two numbers apart at a
    /// glance: without it the line reads as one figure with a stray digit on the end.
    /// </remarks>
    private static string Vital(int current, int maximum, int regen, int boost)
    {
        // Clamped, because the server reports the overkill on a killing blow and the bar was
        // reading "-292/100" for the moment between the hit landing and the death screen.
        string line = $"{Mathf.Clamp(current, 0, maximum)}/{maximum}|{regen}";
        return boost == 0 ? line : $"{line} {Bonus(boost)}";
    }

    /// <summary>
    /// How much health comes back a second, on the original's own formula.
    /// </summary>
    /// <remarks>
    /// One a second plus a little over a tenth per point of vitality. Magic is half that base and
    /// half that rate against wisdom. Both are rounded rather than truncated: the number is a rate
    /// read against what is hitting you, and its fractional part is noise at this size.
    /// </remarks>
    private static int HealthRegen(LocalPlayer player) =>
        Mathf.RoundToInt(1f + 0.12f * player.Vitality);

    private static int ManaRegen(LocalPlayer player) =>
        Mathf.RoundToInt(0.5f + 0.06f * player.Wisdom);

    /// <summary>
    /// The top bar of the three: levelling, and then fame once there is no levelling left.
    /// </summary>
    /// <remarks>
    /// One bar for both because they never overlap. A character earns no fame at all before the
    /// cap, so below twenty this is the experience bar -- green, counting to the next level -- and
    /// at twenty it becomes the fame bar in amber. Fame is written as a plain total, as the
    /// reference writes it: there is no ceiling on fame, and inventing one to make a fraction out
    /// of would be a bar that is always nearly empty for a number that only ever grows.
    /// </remarks>
    private void RefreshProgress(LocalPlayer player)
    {
        if (player.Level >= 0 && player.Level < MaxLevel)
        {
            _fame.Fill = Style.XpFill;
            _fame.High = Style.XpFillHigh;
            _fame.ValueColour = Style.StatNumber;
            _fame.Set(player.Experience, player.NextLevelExperience, $"Lvl {player.Level}",
                ProgressNumbers ? $"{player.Experience}/{player.NextLevelExperience}" : string.Empty);
            return;
        }

        _fame.Fill = Style.FameFill;
        _fame.High = Style.FameFillHigh;
        _fame.ValueColour = Style.Text;
        _fame.Set(1, 1, "Fame",
            ProgressNumbers ? player.Fame.ToString(CultureInfo.InvariantCulture) : string.Empty);
    }

    /// <summary>What equipment adds to a maximum, or nothing at all when it adds nothing.</summary>
    private static string Bonus(int boost) => boost == 0
        ? string.Empty
        : boost > 0
            ? $"(+{boost})"
            : $"(−{-boost})";

    private void UpdateSlots(List<SlotView> views, LocalPlayer player, int firstIndex)
    {
        _slotTypes = _data?.GetObject(player.ObjectType)?.SlotTypes;
        var slotTypes = _slotTypes;

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

            views[i].Usable = CanEquip(desc, slotTypes);
            views[i].SetItem(resolved.Still, desc, _data);
        }
    }

    /// <summary>
    /// Whether this class can equip an item, on the original's rule.
    /// </summary>
    /// <remarks>
    /// An item with no slot type at all counts as unusable -- that is what
    /// <c>ObjectLibrary.isUsableByPlayer</c> does, and it is how a piece of treasure reads as
    /// something to sell rather than something to wear. Potions and eggs are usable by everyone
    /// whatever their slot says.
    /// </remarks>
    public static bool CanEquip(ObjectDesc desc, int[] slotTypes)
    {
        if (desc == null || slotTypes == null)
            return true;

        if (desc.SlotType is PotionSlotType or EggSlotType)
            return true;

        foreach (int slot in slotTypes)
        {
            if (slot == desc.SlotType)
                return true;
        }

        return false;
    }

    /// <summary>The class's equippable slots, kept so the loose panels can ask the same question.</summary>
    private int[] _slotTypes;

    /// <summary>The two slot types every class can use, whatever it is wearing.</summary>
    private const int PotionSlotType = 10;

    private const int EggSlotType = 26;

    /// <summary>Shows what pressing the interact key would do, or hides the prompt when null.</summary>
    public void ShowPrompt(string label)
    {
        if (_prompt == null)
            return;

        var (verb, name) = Split(label);
        _prompt.Set(name, verb, string.Empty);

        // The world's name and the list of who is in it live in the same rectangle, so they stand
        // aside while there is something under the player's feet.
        bool free = !_prompt.Visible;
        if (_worldLabel != null && _worldLabel.Visible != free)
        {
            _worldLabel.Visible = free;
            _party.Visible = free;
        }
    }

    /// <summary>
    /// Splits an interaction's label into the verb the plate carries and the name over it.
    /// </summary>
    /// <remarks>
    /// The tracker writes one sentence -- "Enter Nexus Portal" -- because that is what a one-line
    /// prompt needed. The block wants the two halves apart, and the verbs are a closed set, so they
    /// are matched rather than the first word being taken on faith.
    /// </remarks>
    private static (string Verb, string Name) Split(string label)
    {
        if (string.IsNullOrEmpty(label))
            return (string.Empty, string.Empty);

        foreach (var (prefix, verb) in Verbs)
        {
            if (label.StartsWith(prefix, StringComparison.Ordinal))
                return (verb, label[prefix.Length..]);
        }

        return ("Use", label);
    }

    private static readonly (string Prefix, string Verb)[] Verbs =
    {
        ("Enter ", "Enter"),
        ("Open ", "Open"),
        ("Buy from ", "Buy"),
    };

    /// <summary>
    /// Fires the interact action, as though its key had been pressed.
    /// </summary>
    /// <remarks>
    /// The plate does not know what interacting means and should not: the world already listens for
    /// one action, and pressing the plate is the same gesture as pressing the key bound to it. The
    /// release comes a frame later, because an action pressed and released inside one frame is
    /// never seen as just-pressed by anything reading it.
    /// </remarks>
    private void PressInteract()
    {
        Input.ActionPress("interact");
        _releaseInteractIn = 2;
    }

    private int _releaseInteractIn;

    public override void _Process(double delta)
    {
        if (_releaseInteractIn <= 0)
            return;

        if (--_releaseInteractIn == 0)
            Input.ActionRelease("interact");
    }

    /// <summary>Shows a container's contents, or hides the panel when given null.</summary>
    public void ShowContainer(Entity container)
    {
        bool was = _containerPanel.Visible;
        _containerPanel.Show(container, _data, _textures, _slotTypes);

        if (was != _containerPanel.Visible)
            Reflow();
    }

    /// <summary>Shows what a vendor is selling, or hides the panel when given null.</summary>
    public void ShowMerchant(Entity merchant, LocalPlayer player)
    {
        bool was = _merchantPanel.Visible;
        _merchantPanel.Show(merchant, player, _data, _textures, _slotTypes);

        if (was != _merchantPanel.Visible)
            Reflow();
    }

    // ---------------------------------------------------------------------------------------------

    /// <summary>The key that returns to the Nexus, as it is currently bound.</summary>
    /// <summary>
    /// The key that works the thing in front of the player, as it is currently bound.
    /// </summary>
    /// <remarks>
    /// Read from the input map rather than written into the sentence. The prompt used to say R,
    /// which is Nexus -- pressing it did leave, so the prompt was not only wrong but actively
    /// misleading.
    /// </remarks>
    private static string InteractKey() => BoundKey("interact", "?");

    private static string BoundKey(string action, string fallback)
    {
        foreach (var bound in InputMap.ActionGetEvents(action))
        {
            if (bound is InputEventKey key)
                return OS.GetKeycodeString(key.PhysicalKeycode != Key.None ? key.PhysicalKeycode : key.Keycode);
        }

        return fallback;
    }

}
