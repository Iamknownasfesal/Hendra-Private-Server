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
    private readonly List<PartyEntry> _partyEntries = new();

    private GameData _data;
    private TextureResolver _textures;

    // --- Clusters ------------------------------------------------------------------------------
    private HudPanel _card;
    private Portrait _avatar;
    private Label _name;
    private Label _rating;
    private HudGlyph _ratingStar;
    private Control _guildRow;
    private Control _guildDot;
    private Label _guild;
    private Control _cardIcons;

    private HudIconButton _news;

    private QuickTray _quickTray;
    private SeasonPass _seasonPass;
    private CurrencyRow _currency;
    private Control _party;
    private Label _worldLabel;
    private QuestMarker _quest;

    private Control _vitals;
    private HudBar _fame;
    private HudBar _health;
    private PotionCounter _healthPotions;
    private HudBar _mana;
    private PotionCounter _manaPotions;

    /// <summary>
    /// Air, shown only where there is any to lose.
    ///
    /// One dungeon takes it away and every other world leaves it full. A gauge that is always
    /// there and always full stops being read, and this one has to be read the moment it starts
    /// moving, so it appears when the first breath is spent and goes when it is back to full.
    /// </summary>
    private HudBar _breath;

    private ContainerPanel _containerPanel;
    private VaultView _vaultView;
    private MerchantPanel _merchantPanel;
    private Control _hotbarPanel;
    private Control _hotbarTabs;
    private readonly HotbarTab[] _tabs = new HotbarTab[2];
    private Control _equipmentPanel;



    private Label _prompt;

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

        // The vitals may already be built, depending on which happens first.
        _healthPotions?.UseSprite(SpriteOf("Health Potion"));
        _manaPotions?.UseSprite(SpriteOf("Magic Potion"));
    }

    public override void _Ready()
    {
        // The whole point of the overlay: it is a hole the world can be clicked through, and only
        // the panels below patch it.
        MouseFilter = MouseFilterEnum.Ignore;

        BuildPlayerCard();
        BuildQuickTray();
        BuildCurrency();
        BuildParty();
        BuildVitals();
        BuildHotbar();
        BuildEquipment();
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

        // The tray and the pass hang off the bottom of the card and collapse when empty, so neither
        // reserves space it is not using.
        float under = layout.PlayerCard.End.Y + 10f;

        if (_quickTray != null)
        {
            _quickTray.Position = new Vector2(layout.PlayerCard.Position.X, under);
            _quickTray.Size = new Vector2(HudLayout.CardWidth, QuickTray.Height);
            under += _quickTray.Visible ? QuickTray.Height + 10f : 0f;
        }

        if (_seasonPass != null)
        {
            _seasonPass.Position = new Vector2(layout.PlayerCard.Position.X, under);
            _seasonPass.Size = new Vector2(HudLayout.CardWidth, _seasonPass.Size.Y);
        }

        Place(_currency, layout.Currency);
        Place(_worldLabel, layout.PartyHeader);
        Place(_party, layout.Party);
        Place(_quest, layout.Quest);
        Place(_vitals, layout.Vitals);
        Place(_hotbarTabs, layout.HotbarTabs);
        LayoutTabs(layout.HotbarTabs.Size);
        Place(_hotbarPanel, layout.Hotbar);
        Place(_equipmentPanel, layout.EquipmentRow);

        // The panels that are not part of the reference are hung off the ones that are, so they
        // move with them rather than needing their own corner.
        StackInteractions(layout);

        if (_prompt != null)
        {
            var vitals = layout.Vitals;
            _prompt.Position = new Vector2(Size.X / 2f - 220f, vitals.Position.Y - 46f);
            _prompt.Size = new Vector2(440f, 24f);
        }
    }

    /// <summary>Two tabs, half the strip each.</summary>
    private void LayoutTabs(Vector2 size)
    {
        float half = Mathf.Round(size.X / 2f);

        _tabs[0].Position = Vector2.Zero;
        _tabs[0].Size = new Vector2(half, size.Y);
        _tabs[1].Position = new Vector2(half, 0f);
        _tabs[1].Size = new Vector2(size.X - half, size.Y);
    }

    /// <summary>
    /// Stacks whatever is at the player's feet above the hotbar, growing upward.
    /// </summary>
    /// <remarks>
    /// Above the tab strip rather than above the grid: the strip is part of the hotbar and a panel
    /// that started at the grid's top edge sat on it. They stack rather than share a rectangle
    /// because standing on a bag next to a vendor is an ordinary thing to do, and the two used to
    /// overwrite each other's title.
    /// </remarks>
    private void StackInteractions(in HudLayout layout)
    {
        if (_containerPanel == null)
            return;

        float right = layout.Hotbar.End.X;
        float bottom = layout.HotbarTabs.Position.Y - 12f;

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
    // 2.1 Player card
    // ---------------------------------------------------------------------------------------------

    /// <summary>
    /// Top left: who you are, and the buttons that open the panels about you.
    /// </summary>
    /// <remarks>
    /// The corner furthest from the action, because it is the part you read between fights rather
    /// than during one.
    /// </remarks>
    private void BuildPlayerCard()
    {
        _card = new HudPanel(Style.Panel);
        AddChild(_card);

        _avatar = new Portrait();
        _card.AddChild(_avatar);

        _name = new Label { ClipText = true, TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis }
            .Typeset(Style.FontName, Style.Text);
        _card.AddChild(_name);

        _rating = new Label { HorizontalAlignment = HorizontalAlignment.Right }
            .Typeset(Style.FontBody, Style.Text);
        _card.AddChild(_rating);

        // The colour is the rating, not decoration: it climbs the original's ladder from a pale
        // blue at nothing through blue, red and orange to gold, so a star that has just appeared
        // does not look like one that took a thousand fame to earn.
        _ratingStar = new HudGlyph(HudIcons.Star, Fame.Colour(0, 1));
        _card.AddChild(_ratingStar);

        // Hidden until the player turns out to have a guild, so a guildless character never sees
        // the card flash a row taller on the first frame.
        _guildRow = new Control { MouseFilter = MouseFilterEnum.Ignore, Visible = false };
        _card.AddChild(_guildRow);

        _guildDot = new HudGlyph(HudIcons.Dot, Style.Guild);
        _guildRow.AddChild(_guildDot);

        _guild = new Label().Typeset(Style.FontSmall, Style.Guild);
        _guildRow.AddChild(_guild);

        _cardIcons = new Control { MouseFilter = MouseFilterEnum.Ignore };
        _card.AddChild(_cardIcons);

        // The row is the whole of the card's navigation now. Pets went with the system that never
        // existed here, and the button stack under the card went with it -- Shop and News are
        // places to visit, which is what an icon is for, and a plate the width of the card was
        // shouting at a player who has already read it once.
        _cardIcons.AddChild(CardIcon(HudIcons.Bust, "Account", () => AccountPressed?.Invoke()));
        _cardIcons.AddChild(CardIcon(HudIcons.BarChart, "Stats", () => StatsPressed?.Invoke()));
        _cardIcons.AddChild(CardIcon(HudIcons.Shop, "Shop", () => ShopPressed?.Invoke()));

        _news = CardIcon(HudIcons.News, "News", () => NewsPressed?.Invoke());
        _cardIcons.AddChild(_news);

        _cardIcons.AddChild(CardIcon(HudIcons.Gear, "Settings", () => OptionsPressed?.Invoke()));
    }

    private static HudIconButton CardIcon(
        Action<CanvasItem, Rect2, Color> icon, string tooltip, Action pressed)
    {
        var button = new HudIconButton(icon, tooltip, inset: 4f);
        button.Pressed += pressed;
        return button;
    }

    /// <summary>
    /// Places the card's contents: a portrait, two lines beside it, and the icon row under both.
    /// </summary>
    /// <remarks>
    /// The guild line is hidden for a player without one and leaves no gap, because it sits in the
    /// column beside the portrait rather than in a row of its own -- the card is as tall as the
    /// portrait either way.
    /// </remarks>
    private void LayoutCard(Vector2 size)
    {
        const float Pad = 8f;

        _avatar.Position = new Vector2(Pad, 5f);
        _avatar.Size = new Vector2(HudLayout.AvatarSize, HudLayout.AvatarSize);

        float textLeft = Pad + HudLayout.AvatarSize + 10f;

        // The rating and its star are right-aligned against the panel's inner edge, and the name
        // truncates rather than running under them.
        var star = new Rect2(size.X - Pad - 16f, 6f, 16f, 16f);
        _ratingStar.Position = star.Position;
        _ratingStar.Size = star.Size;

        _rating.Position = new Vector2(textLeft, 5f);
        _rating.Size = new Vector2(star.Position.X - 6f - textLeft, 18f);

        _name.Position = new Vector2(textLeft, 3f);
        _name.Size = new Vector2(180f, 22f);

        _guildRow.Position = new Vector2(textLeft + 2f, 30f);
        _guildRow.Size = new Vector2(size.X - textLeft - Pad, 16f);
        _guildDot.Position = new Vector2(0f, 4f);
        _guildDot.Size = new Vector2(8f, 8f);
        _guild.Position = new Vector2(12f, 0f);
        _guild.Size = new Vector2(_guildRow.Size.X - 12f, 16f);

        // The icons sit on the bottom edge either way, which keeps the card's own padding even
        // whichever of the two heights it is.
        _cardIcons.Position = new Vector2(0f, size.Y - 32f);
        _cardIcons.Size = new Vector2(size.X, 28f);

        // Three together on the left; settings pushed to the far right, as the reference has it.
        float[] x = { 10f, 52f, 94f, 136f, size.X - Pad - 28f };
        for (int i = 0; i < _cardIcons.GetChildCount() && i < x.Length; i++)
        {
            var button = _cardIcons.GetChild<Control>(i);
            button.Position = new Vector2(x[i], 0f);
            button.Size = new Vector2(28f, 28f);
        }
    }

    /// <summary>Shows or hides the unread mark on the news icon.</summary>
    public void SetUnreadNews(bool unread) => _news.Badge = unread ? 1 : 0;

    /// <summary>
    /// The row of quick actions under the card, and the seasonal pass under that.
    /// </summary>
    /// <remarks>
    /// Both are built and both are empty: nothing in this fork's protocol carries event entries,
    /// gifts or a pass, so each stays collapsed until something feeds it. That is the specified
    /// behaviour for an empty tray anyway -- it collapses rather than reserving space -- and it
    /// means the day the server does send one, there is somewhere for it to go.
    /// </remarks>
    private void BuildQuickTray()
    {
        _quickTray = new QuickTray();
        _quickTray.Activated += id => QuickActionPressed?.Invoke(id);
        AddChild(_quickTray);

        _seasonPass = new SeasonPass();
        AddChild(_seasonPass);
    }

    /// <summary>Raised with the id of whichever quick action was pressed.</summary>
    public event Action<string> QuickActionPressed;

    /// <summary>Replaces the quick actions. An empty list collapses the row.</summary>
    public void ShowQuickActions(IReadOnlyList<QuickAction> actions)
    {
        _quickTray.Set(actions);
        Reflow();
    }

    /// <summary>Shows or hides the seasonal pass. A null title hides it.</summary>
    public void ShowSeasonPass(string title, string countdown, int tier, float progress, string body)
    {
        _seasonPass.Set(title, countdown, tier, progress, body);
        Reflow();
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

    private void BuildParty()
    {
        _worldLabel = new Label { HorizontalAlignment = HorizontalAlignment.Center }
            .Typeset(Style.FontBody, Style.TextDim);
        AddChild(_worldLabel);

        _party = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_party);

        _quest = new QuestMarker();
        AddChild(_quest);

        // Six rows, two columns, filled left to right and collapsing upward: an empty slot draws
        // nothing rather than leaving a hole in the grid.
        for (int i = 0; i < HudLayout.PartyColumns * HudLayout.PartyRows; i++)
        {
            var entry = new PartyEntry();
            entry.Activated += name => PartyMemberActivated?.Invoke(name);
            entry.Position = new Vector2(
                i % HudLayout.PartyColumns * HudLayout.PartyColumnWidth,
                i / HudLayout.PartyColumns * HudLayout.PartyRowHeight);
            entry.Size = new Vector2(HudLayout.PartyColumnWidth, HudLayout.PartyRowHeight);
            entry.Visible = false;

            _party.AddChild(entry);
            _partyEntries.Add(entry);
        }
    }

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
    /// One nearby player: their class portrait, bordered in their state, and their name.
    /// </summary>
    /// <remarks>
    /// The border carries what a separate dot used to. Three states rather than a gradient -- fine,
    /// hurt, dead -- because a gradient reads as decoration and needs comparing against itself,
    /// while three colours are a glance.
    /// </remarks>
    private sealed partial class PartyEntry : Control
    {
        private const float PortraitSize = 16f;

        private readonly Label _name;
        private readonly Portrait _portrait;

        private string _member = string.Empty;

        public PartyEntry()
        {
            MouseFilter = MouseFilterEnum.Stop;
            FocusMode = FocusModeEnum.None;

            _portrait = new Portrait
            {
                Position = new Vector2(0f, Mathf.Round((HudLayout.PartyRowHeight - PortraitSize) / 2f)),
                Size = new Vector2(PortraitSize, PortraitSize),
            };
            AddChild(_portrait);

            _name = new Label
            {
                ClipText = true,
                TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis,
                VerticalAlignment = VerticalAlignment.Center,
                Position = new Vector2(PortraitSize + 6f, 0f),
                Size = new Vector2(HudLayout.PartyColumnWidth - PortraitSize - 10f, HudLayout.PartyRowHeight),
            }.Typeset(Style.FontBody, Style.Text);
            AddChild(_name);
        }

        public event Action<string> Activated;

        public void Set(string name, int hp, int maxHp, bool starred, Assets.Sprite portrait)
        {
            float fraction = maxHp > 0 ? hp / (float)maxHp : 0f;

            _portrait.Set(portrait, fraction <= 0f ? Style.StatusDead
                : fraction < 0.35f ? Style.StatusLow
                : Style.StatusOk);

            _member = name;

            string text = starred ? $"* {name}" : name;
            if (_name.Text != text)
                _name.Text = text;

            TooltipText = maxHp > 0 ? $"{name}\n{hp} / {maxHp} HP" : name;
            Visible = true;
        }

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
            {
                Activated?.Invoke(_member);
                AcceptEvent();
            }
        }
    }

    /// <summary>
    /// Shows what the realm has asked for, or hides the marker when there is nothing.
    /// </summary>
    /// <param name="isNew">Whether it has only just been given, which is worth noticing.</param>
    public void ShowQuest(string name, ushort objectType, bool isNew) =>
        _quest?.Set(name, name == null ? default : ClassPortrait(objectType), isNew);

    /// <summary>
    /// The current quest: a portrait, a name, and a mark while it is new.
    /// </summary>
    /// <remarks>
    /// The original puts the same thing on the arrow at the edge of the screen as a tooltip, which
    /// means it is only readable when the target is off screen and the pointer is on it. Here it is
    /// in the column with the map and the party, where the other things you check between fights
    /// are.
    /// </remarks>
    private sealed partial class QuestMarker : Control
    {
        private const float PortraitSize = 32f;

        private readonly Portrait _portrait;
        private readonly Label _label;
        private readonly Label _heading;

        private bool _isNew;

        public QuestMarker()
        {
            MouseFilter = MouseFilterEnum.Ignore;
            Visible = false;

            _portrait = new Portrait
            {
                Position = new Vector2(0f, 6f),
                Size = new Vector2(PortraitSize, PortraitSize),
            };
            AddChild(_portrait);

            _heading = new Label { Text = "QUEST", Position = new Vector2(PortraitSize + 8f, 2f) }
                .Typeset(Style.FontSmall, Style.StatLabel);
            _heading.Size = new Vector2(200f, 14f);
            AddChild(_heading);

            _label = new Label
            {
                ClipText = true,
                TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis,
                Position = new Vector2(PortraitSize + 8f, 16f),
                Size = new Vector2(200f, 20f),
            }.Typeset(Style.FontBody, Style.Text);
            AddChild(_label);
        }

        public void Set(string name, Assets.Sprite portrait, bool isNew)
        {
            Visible = !string.IsNullOrEmpty(name);
            if (!Visible)
                return;

            if (_label.Text != name)
                _label.Text = name;

            // The frame carries the state: amber while the quest is new, then the ordinary border.
            _portrait.Set(portrait, isNew ? Style.FameFill : Style.SlotBorder);

            if (_isNew == isNew)
                return;

            _isNew = isNew;
            _heading.AddThemeColorOverride("font_color", isNew ? Style.FameFill : Style.StatLabel);
            QueueRedraw();
        }
    }

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

            var member = members[i];
            _partyEntries[i].Set(
                member.Name, member.Hp, member.MaxHp, member.Starred, ClassPortrait(member.ObjectType));
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
    // 2.7 Vitals
    // ---------------------------------------------------------------------------------------------

    /// <summary>
    /// Bottom centre: health, magic, the potions that refill them, and the way home.
    /// </summary>
    /// <remarks>
    /// The things you look at while something is hitting you, put where the eye already is -- under
    /// the character, not off in a corner. It is the only cluster measured from the middle of the
    /// screen rather than from an edge.
    /// </remarks>
    private void BuildVitals()
    {
        _vitals = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_vitals);

        // No icons down the left any more: a heart beside a bar that says "HP" is the same fact
        // twice, and the second telling costs twenty-eight pixels of bar.
        const float barLeft = 0f;
        float potionLeft = barLeft + HudLayout.VitalBarWidth + 6f;
        float abilityLeft = potionLeft + HudLayout.PotionBoxWidth + 8f;

        const float Row = HudLayout.VitalBarHeight + HudLayout.VitalRowGap;

        // Fame on top, then health, then magic. Fame is the one you read between fights and the
        // other two are the ones you read during them, so the pair that matter sit closest to the
        // character.
        _fame = Bar(Style.FameFill, barLeft, 0f);

        _health = Bar(Style.HpFill, barLeft, Row);
        _healthPotions = Potions(true, potionLeft, Row);

        _mana = Bar(Style.MpFill, barLeft, Row * 2f);
        _manaPotions = Potions(false, potionLeft, Row * 2f);

        // Above the other three rather than below them. The cluster is anchored to the bottom of
        // the viewport and sized from VitalRows, so a fourth row would hang off the edge; and a
        // bar that comes and goes must not shove the three that are always there up and down the
        // screen when it does.
        _breath = Bar(Style.BreathFill, barLeft, -Row);
        _breath.Visible = false;

        // No button back to the Nexus. It was a plate with a temple drawn on it, and that temple
        // was a mark invented for one button. Escaping to the Nexus is a key, it has always been a
        // key, and the button never did anything the key did not.
    }

    /// <summary>An item's still, by the name the data files give it, or nothing if it has none.</summary>
    private Assets.Sprite SpriteOf(string id)
    {
        var desc = _data?.GetObject(id ?? string.Empty);
        return desc == null ? default : (_textures?.Resolve(desc.Texture) ?? default).Still;
    }

    /// <summary>A full breath, matching the server's own ceiling.</summary>
    private const int FullBreath = 100;

    private HudBar Bar(Color fill, float x, float y)
    {
        var bar = new HudBar(fill)
        {
            Position = new Vector2(x, y),
            Size = new Vector2(HudLayout.VitalBarWidth, HudLayout.VitalBarHeight),
        };

        _vitals.AddChild(bar);
        return bar;
    }

    private PotionCounter Potions(bool health, float x, float y)
    {
        var counter = new PotionCounter(health)
        {
            Position = new Vector2(x, y),
            Size = new Vector2(HudLayout.PotionBoxWidth, HudLayout.VitalBarHeight),
        };

        counter.Pressed += () => PotionRequested?.Invoke(health);
        counter.Accepts = from => PotionAccepted?.Invoke(from, health) ?? false;
        counter.Filled += from => PotionStacked?.Invoke(from, health);
        counter.UseSprite(SpriteOf(health ? "Health Potion" : "Magic Potion"));
        _vitals.AddChild(counter);
        return counter;
    }

    /// <summary>
    /// How many stacked potions of one kind are held, beside the bar they refill.
    /// </summary>
    /// <remarks>
    /// A slot rather than a chip: the same dark plate and light border as the hotbar, holding the
    /// potion and its count. It is a button as well as a counter -- the two potions live outside the
    /// inventory array, addressed on the wire by slot id rather than by index, so this is the only
    /// place they can be clicked.
    /// </remarks>
    private sealed partial class PotionCounter : Control
    {
        private readonly bool _health;

        private int _count;
        private bool _hovered;

        /// <summary>The item this counts, so the plate shows the thing rather than a shape.</summary>
        private Assets.Sprite _bottle;

        /// <summary>Raised when a potion is dropped onto this counter, with where it came from.</summary>
        public event Action<SlotAddress> Filled;

        /// <summary>
        /// Only takes what it is a counter for.
        /// </summary>
        /// <remarks>
        /// Godot asks this while the drag is over the control and refuses the drop itself when it
        /// answers false, so a health potion dragged onto the magic stack never leaves the cursor.
        /// The check is the item's type against the one this counter holds -- the owner answers
        /// that, since a counter knows nothing about what is in any slot.
        /// </remarks>
        public Func<SlotAddress, bool> Accepts { get; set; }

        public override bool _CanDropData(Vector2 atPosition, Variant data) =>
            SlotView.PayloadAddress(data, out var from) && (Accepts?.Invoke(from) ?? false);

        public override void _DropData(Vector2 atPosition, Variant data)
        {
            if (SlotView.PayloadAddress(data, out var from))
                Filled?.Invoke(from);
        }

        public void UseSprite(Assets.Sprite sprite)
        {
            _bottle = sprite;
            QueueRedraw();
        }

        public PotionCounter(bool health)
        {
            _health = health;
            MouseFilter = MouseFilterEnum.Stop;
            FocusMode = FocusModeEnum.None;
            TooltipText = health ? "Drink a health potion" : "Drink a magic potion";
        }

        public event Action Pressed;

        public void Set(int count)
        {
            if (_count == count)
                return;

            _count = count;
            QueueRedraw();
        }

        public override void _Ready()
        {
            MouseEntered += () => { _hovered = true; QueueRedraw(); };
            MouseExited += () => { _hovered = false; QueueRedraw(); };
        }

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
            {
                Pressed?.Invoke();
                AcceptEvent();
            }
        }

        /// <summary>
        /// What colour a count of something you can run out of is written in.
        /// </summary>
        /// <remarks>
        /// Green used to mean "this is a potion count" and was worn at nought out of six, which
        /// reads as stocked at exactly the moment you are not. It means full now: amber under half,
        /// red at empty. The same three steps the health bar already uses, for the same reason.
        /// </remarks>
        private static Color Supply(int held, int of) =>
            held <= 0 ? Style.StatPenalty
            : held * 2 < of ? Style.FameFill
            : Style.PotionCount;

        public override void _Draw()
        {
            var full = new Rect2(Vector2.Zero, Size);

            DrawRect(full, _hovered ? Style.Slot.Lightened(0.12f) : Style.Slot);
            DrawRect(full, _hovered ? Style.SlotBorderHi : Style.SlotBorder,
                filled: false, width: SlotView.Border);

            // The game's own potion, not a drawing of one. The bottle here used to be geometry --
            // a circle, a neck and a cork -- which is a picture of the idea of a potion sitting
            // next to a bag full of the actual things.
            if (_bottle.IsValid)
            {
                float side = Mathf.Min(Size.Y - 6f, Size.X * 0.42f);
                this.DrawSprite(_bottle,
                    new Rect2(3f, Mathf.Round((Size.Y - side) / 2f), side, side));
            }

            string text = $"{_count}/{PotionStackMax}";
            float baseline = Style.BaselineIn(Size.Y, Style.FontSmall);

            this.DrawOverWorld(
                new Vector2(Size.X - Style.Measure(text, Style.FontSmall) - 4f, baseline),
                text, Style.FontSmall, Supply(_count, PotionStackMax));
        }
    }


    private void BuildHotbar()
    {
        _hotbarTabs = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_hotbarTabs);

        // Two pages of eight. The first is what the character carries, the second is the backpack,
        // which is a separate eight slots on the wire rather than a continuation of the first.
        _tabs[0] = NewTab(HudIcons.Grid, "Carried", 0);
        _tabs[1] = NewTab(HudIcons.Backpack, "Backpack", 1);

        _hotbarPanel = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_hotbarPanel);

        for (int i = 0; i < HotbarSlots; i++)
        {
            int at = i;
            var slot = NewSlot(default);

            // One through eight, whichever page is showing: the keys address the visible slots, so
            // the number on a square is always the key that uses it.
            slot.Hotkey = (i + 1).ToString(CultureInfo.InvariantCulture);
            slot.Activated += () => SlotActivated?.Invoke(HotbarFirstSlot + at);
            slot.Position = new Vector2(
                i % HudLayout.HotbarColumns * (HudLayout.HotbarSlotWidth + HudLayout.SlotGap),
                i / HudLayout.HotbarColumns * (HudLayout.HotbarSlotHeight + HudLayout.SlotGap));
            slot.Size = new Vector2(HudLayout.HotbarSlotWidth, HudLayout.HotbarSlotHeight);

            _hotbarPanel.AddChild(slot);
            _hotbar.Add(slot);
        }

        SetHotbarPage(App.ServiceLocator.Settings?.HotbarPage ?? 0, save: false);
    }

    private HotbarTab NewTab(Action<CanvasItem, Rect2, Color> icon, string tooltip, int page)
    {
        var tab = new HotbarTab(icon) { TooltipText = tooltip };
        tab.Pressed += () => SetHotbarPage(page, save: true);

        _hotbarTabs.AddChild(tab);
        return tab;
    }

    /// <summary>
    /// One of the two tabs over the hotbar.
    /// </summary>
    /// <remarks>
    /// The active tab is light and the body below it is the panel colour, so the pair read as one
    /// shape with a page attached. The inactive one is dark and reads as behind it.
    /// </remarks>
    private sealed partial class HotbarTab : Control
    {
        private readonly Action<CanvasItem, Rect2, Color> _icon;

        private bool _hovered;
        private bool _active;
        private bool _enabled = true;

        public HotbarTab(Action<CanvasItem, Rect2, Color> icon)
        {
            _icon = icon;
            MouseFilter = MouseFilterEnum.Stop;
            FocusMode = FocusModeEnum.None;
        }

        public event Action Pressed;

        public bool Active
        {
            get => _active;
            set { _active = value; QueueRedraw(); }
        }

        /// <summary>A page the character does not have -- a backpack they have not bought.</summary>
        public bool Enabled
        {
            get => _enabled;
            set { _enabled = value; QueueRedraw(); }
        }

        public override void _Ready()
        {
            MouseEntered += () => { _hovered = true; QueueRedraw(); };
            MouseExited += () => { _hovered = false; QueueRedraw(); };
        }

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is not InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
                return;

            AcceptEvent();

            if (_enabled)
                Pressed?.Invoke();
        }

        public override void _Draw()
        {
            var full = new Rect2(Vector2.Zero, Size);

            var face = !_enabled ? Style.TabIdle.Darkened(0.3f)
                : _active ? Style.TabActive
                : _hovered ? Style.TabIdle.Lightened(0.15f)
                : Style.TabIdle;

            DrawRect(full, face);
            DrawRect(full, Style.PanelEdge, filled: false, width: 1f);

            float side = Mathf.Round(Mathf.Min(Size.X, Size.Y) * 0.62f);
            var box = new Rect2(
                Mathf.Round((Size.X - side) / 2f), Mathf.Round((Size.Y - side) / 2f), side, side);

            _icon(this, box, !_enabled ? Style.TabIdle.Lightened(0.3f)
                : _active ? Style.PanelEdge
                : Style.TextDim);
        }
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
        _equipmentPanel = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_equipmentPanel);

        for (int i = 0; i < EquipmentSlots; i++)
        {
            int index = i;
            var slot = NewSlot(new SlotAddress(SlotOwner.Player, index));
            slot.Activated += () => SlotActivated?.Invoke(index);

            // The weapon and the ability are the one piece of the control scheme written nowhere
            // else, so each slot names the action it fires and draws whatever that is bound to.
            if (i == 0)
                slot.BoundAction = "shoot";
            else if (i == 1)
                slot.BoundAction = "use_ability";

            slot.Position = new Vector2(i * (HudLayout.EquipmentSlotWidth + HudLayout.SlotGap), 0f);
            slot.Size = new Vector2(HudLayout.EquipmentSlotWidth, HudLayout.EquipmentSlotHeight);

            _equipmentPanel.AddChild(slot);
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
        // Over the world rather than in a panel, because it refers to something in front of the
        // player rather than to their own state.
        _prompt = new Label
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            Visible = false,
        }.Typeset(Style.FontBody, Style.Text);

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

        // The rating is the character's own stars, on the original's fame thresholds, and its
        // colour is the rung of the ladder it is on.
        int stars = Fame.Stars(player.Fame);
        string rating = stars.ToString(CultureInfo.InvariantCulture);

        if (_rating.Text != rating)
        {
            _rating.Text = rating;
            _ratingStar.Tint = Fame.Colour(stars, 1);
        }

        _currency.Set(player.Fame, player.Credits);
        _avatar.Set(ClassPortrait(player.ObjectType), Style.SlotBorder);
    }

    private void RefreshVitals(LocalPlayer player)
    {
        RefreshProgress(player);

        _health.Set(player.Hp, player.MaxHp, "HP", $"{player.Hp}/{player.MaxHp}", Bonus(player.Boosts[0]));
        _mana.Set(player.Mp, player.MaxMp, "MP", $"{player.Mp}/{player.MaxMp}", Bonus(player.Boosts[1]));

        // Hidden at full, because everywhere but the trench it is always full.
        var drowning = player.Breath < FullBreath;
        _breath.Visible = drowning;
        if (drowning)
            _breath.Set(player.Breath, FullBreath, "AIR", $"{player.Breath}%");

        _healthPotions.Set(player.HealthPotions);
        _manaPotions.Set(player.MagicPotions);

    }

    /// <summary>
    /// The top row of the vitals: levelling, and then fame once there is no levelling left.
    /// </summary>
    /// <remarks>
    /// One bar for both because they never overlap. A character earns no fame at all before the
    /// cap, so below twenty the row is the experience bar -- green, counting to the next level --
    /// and at twenty it becomes the fame bar in amber, counting to the next star. The card used to
    /// carry the experience bar and now carries neither, which is what let it shrink to its
    /// portrait.
    /// </remarks>
    private void RefreshProgress(LocalPlayer player)
    {
        if (player.Level >= 0 && player.Level < MaxLevel)
        {
            _fame.Fill = Style.XpFill;
            _fame.Set(player.Experience, player.NextLevelExperience,
                $"Lvl {player.Level}", $"{player.Experience}/{player.NextLevelExperience}");
            return;
        }

        _fame.Fill = Style.FameFill;

        // Past the last star there is nothing left to be a fraction of, so the bar shows the total
        // and stays full rather than inventing a ceiling.
        int nextStar = Fame.NextThreshold(player.Fame);

        if (nextStar > 0)
            _fame.Set(player.Fame, nextStar, "Fame", $"{player.Fame}/{nextStar}");
        else
            _fame.Set(1, 1, "Fame", player.Fame.ToString(CultureInfo.InvariantCulture));
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

        bool show = !string.IsNullOrEmpty(label);
        if (_prompt.Visible != show)
            _prompt.Visible = show;

        if (show)
            _prompt.Text = $"[{InteractKey()}] {label}";
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
