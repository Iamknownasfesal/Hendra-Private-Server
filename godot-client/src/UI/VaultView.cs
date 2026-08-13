using System;
using System.Collections.Generic;
using System.Globalization;
using Godot;
using Hendra.Assets;
using Hendra.Resources;
using Hendra.World;

namespace Hendra.UI;

/// <summary>
/// The vault: every chest the account owns, as one grid.
/// </summary>
/// <remarks>
/// <para>
/// One purchased chest is one row of eight. That is the only place the chest survives as an idea --
/// it is still the unit you buy and the unit the server stores, but it has stopped being a place you
/// walk to, and nothing in here navigates between them.
/// </para>
/// <para>
/// The grid is virtualized: slot views are made for the rows on screen and moved as it scrolls,
/// never one per stored slot. A player with fifty chests has four hundred slots and forty of them
/// are visible, and mounting the other three hundred and sixty is how a panel that opens instantly
/// on a new account takes a second to open on an old one.
/// </para>
/// <para>
/// Nothing here writes to storage. The sort modes and the filters build a list of indices to draw;
/// see <see cref="VaultStore"/>, where that rule is enforced rather than merely intended.
/// </para>
/// </remarks>
public sealed partial class VaultView : ModalPanel
{
    /// <summary>Where the top edge sits, per the brief.</summary>
    public const float TopEdge = 60f;

    private const float Padding = 16f;
    private const float HeaderTall = 64f;
    private const float SortBarHeight = 44f;
    private const float RailButtonSize = 56f;
    private const float RailGap = 6f;

    private const int Columns = 8;
    private const float SlotSize = 96f;
    private const float SlotGap = 4f;
    private const float RowPitch = SlotSize + SlotGap;

    /// <summary>
    /// The grid's width: eight slots and the seven hairlines between them, and nothing else.
    /// </summary>
    /// <remarks>
    /// Seven hundred and ninety-six pixels, and the panel is built out to hold it rather than the
    /// grid being stretched to fill a panel. Letting the cells take up the slack is what turned
    /// squares into rectangles and four-pixel gutters into thirteen-pixel ones, and a grid that
    /// reads as a block of plates with hairlines between them is most of what makes it read as an
    /// inventory rather than as a settings screen.
    /// </remarks>
    private const float GridWidth = Columns * SlotSize + (Columns - 1) * SlotGap;

    /// <summary>The scrollbar and its gap, which sit beside the grid rather than inside it.</summary>
    private const float GutterWidth = RailGap + HudScrollbar.Width;

    public const float PanelWidth =
        Padding + RailButtonSize + RailGap * 2f + GridWidth + GutterWidth + Padding;

    /// <summary>What the panel falls back to before it knows how much screen it has.</summary>
    public const float PanelHeight = 700f;

    /// <summary>How far above the bottom of the screen the panel stops.</summary>
    private const float BottomMargin = 40f;

    /// <summary>Rows kept mounted beyond the visible ones, above and below.</summary>
    private const int Overscan = 2;

    /// <summary>How deep the locked section goes. Three, never the theoretical maximum.</summary>
    private const int LockedRows = 3;

    /// <summary>The full-width label that introduces a section. Gifts above, locked below.</summary>
    private const float BandHeight = 32f;

    /// <summary>How long the search waits after a keystroke before it filters.</summary>
    private const double SearchDebounceSeconds = 0.150;

    private VaultStore _store;
    private readonly GameData _data;
    private readonly TextureResolver _textures;

    private readonly List<SortTab> _tabs = new();
    private readonly List<RailButton> _rail = new();
    private readonly List<SlotView> _pool = new();

    private Control _grid;
    private Control _trough;
    private LineEdit _search;
    private HudIconButton _magnifier;
    private Label _note;
    private HudIconButton _info;
    private Label _explainer;

    private int[] _slotTypes = Array.Empty<int>();

    private float _offset;
    private bool _draggingThumb;

    private double _searchDueAt = -1.0;
    private string _pendingQuery = string.Empty;

    /// <summary>The first row currently mounted, so scrolling can tell when it must remount.</summary>
    private int _mountedFrom = -1;

    public VaultView(VaultStore store, GameData data, TextureResolver textures)
        : base("Vault")
    {
        _store = store;
        _data = data;
        _textures = textures;

        Size = new Vector2(PanelWidth, PanelHeight);
    }

    /// <summary>Raised when a slot is dragged onto another, either here or in the HUD.</summary>
    public event Action<SlotAddress, SlotAddress> Dropped;

    /// <summary>Raised when a slot is clicked: the quick move out to the inventory.</summary>
    public event Action<SlotAddress> Activated;

    /// <summary>Raised when a locked slot is clicked and the purchase should begin.</summary>
    public event Action PurchaseRequested;

    /// <summary>Whether the search field has the keyboard, so gameplay keys must be held back.</summary>
    public bool IsTyping => _search is { Visible: true } && _search.HasFocus();

    protected override float Header => HeaderTall;

    /// <summary>No cross: Escape and walking away are the exits. See the brief's header note.</summary>
    protected override bool ShowClose => false;

    protected override bool ShowOrnaments => true;

    public override void _Ready()
    {
        base._Ready();

        BuildHeader();
        BuildSortBar();
        BuildRail();
        BuildGrid();

        _store.Changed += OnStoreChanged;
        Closed += OnClosed;
    }

    /// <summary>
    /// Points the panel at another vault.
    /// </summary>
    /// <remarks>
    /// Changing world tears down the world controller and everything it owns, the vault's store
    /// among them, while the interface survives -- it is the same HUD from the login screen to the
    /// last dungeon. So the panel outlives the thing it is showing, and the second time a player
    /// walks into their vault the store it was built against is a dead object that will never be
    /// told anything again. It showed the first visit's contents for the rest of the session.
    /// </remarks>
    public void Use(VaultStore store)
    {
        if (store == null || ReferenceEquals(store, _store))
            return;

        if (_store != null && IsNodeReady())
            _store.Changed -= OnStoreChanged;

        _store = store;

        if (!IsNodeReady())
            return;

        _store.Changed += OnStoreChanged;

        _offset = 0f;
        _mountedFrom = -1;
        Refresh();
    }

    /// <summary>What the character can wear, so a slot can say whether the item is any use to it.</summary>
    public void SetSlotTypes(int[] slotTypes)
    {
        _slotTypes = slotTypes ?? Array.Empty<int>();
        Refresh();
    }

    /// <summary>Puts the panel where the brief says: centred across, sixty down.</summary>
    public void PlaceIn(Vector2 screen)
    {
        _screen = screen;
        Position = new Vector2(Mathf.Round((screen.X - PanelWidth) / 2f), TopEdge);
        Layout();
    }

    /// <summary>The space the panel has to fit in, so it can stop short of filling it.</summary>
    private Vector2 _screen = new(HudLayout.ReferenceWidth, HudLayout.ReferenceHeight);

    private void OnClosed()
    {
        // A search left running would filter the grid the next time it opened, which is a state the
        // player did not ask for and has no way to remember setting.
        CloseSearch();
    }

    private void OnStoreChanged()
    {
        _mountedFrom = -1;
        Refresh();
    }

    /// <summary>
    /// The one control in the header: what this panel is and how to get more of it.
    /// </summary>
    /// <remarks>
    /// There is no close cross beside it. Escape and walking away are the exits, and the corner it
    /// would have occupied is better spent on the only question the panel raises that it does not
    /// otherwise answer -- what a chest costs and how many you may have.
    /// </remarks>
    private void BuildHeader()
    {
        _info = new HudIconButton(Info, "About storage", inset: 8f) { Tint = Style.ModalFrame };
        _info.Pressed += () => _explainer.Visible = !_explainer.Visible;
        AddChild(_info);

        _explainer = new Label
        {
            Visible = false,
            AutowrapMode = TextServer.AutowrapMode.WordSmart,
            MouseFilter = MouseFilterEnum.Ignore,
        }.Typeset(Style.FontSmall, Style.Text);
        AddChild(_explainer);
    }

    private static void Info(CanvasItem into, Rect2 box, Color colour)
    {
        into.DrawRect(box, colour, filled: false, width: 2f);

        float x = box.Position.X + box.Size.X / 2f - 1f;
        into.DrawRect(new Rect2(x, box.Position.Y + box.Size.Y * 0.22f, 2f, 2f), colour);
        into.DrawRect(new Rect2(x, box.Position.Y + box.Size.Y * 0.40f, 2f, box.Size.Y * 0.36f), colour);
    }

    // ─── the sort bar ─────────────────────────────────────────────────────────────────────────

    private static readonly (VaultSort Mode, string Label)[] Modes =
    {
        (VaultSort.Custom, "Custom"),
        (VaultSort.Name, "A–Z"),
        (VaultSort.FeedPower, "Feed Power"),
        (VaultSort.Type, "Type"),
    };

    private void BuildSortBar()
    {
        // Behind the four, so they read as one control with four settings.
        _trough = new Control { MouseFilter = MouseFilterEnum.Ignore };
        _trough.Draw += () => _trough.DrawStyleBox(Rounded(Style.ModalTrough), new Rect2(Vector2.Zero, _trough.Size));
        Body.AddChild(_trough);

        foreach (var (mode, label) in Modes)
        {
            var tab = new SortTab(label) { Mode = mode };
            tab.Pressed += () => Choose(tab.Mode);

            Body.AddChild(tab);
            _tabs.Add(tab);
        }

        _magnifier = new HudIconButton(Magnifier, "Search", inset: 12f) { Tint = Style.Text };
        _magnifier.Pressed += OpenSearch;
        Body.AddChild(_magnifier);

        _search = new LineEdit
        {
            Visible = false,
            PlaceholderText = "Search",
            MouseFilter = MouseFilterEnum.Stop,
        };
        _search.TextChanged += OnSearchTyped;
        Body.AddChild(_search);

        _note = new Label { Visible = false }.Typeset(Style.FontSmall, Style.TextDim);
        Body.AddChild(_note);
    }

    private void Choose(VaultSort mode)
    {
        _store.Sort = mode;
        _offset = 0f;
        _mountedFrom = -1;
    }

    private static void Magnifier(CanvasItem into, Rect2 box, Color colour)
    {
        float radius = Mathf.Min(box.Size.X, box.Size.Y) * 0.34f;
        var centre = box.Position + box.Size / 2f - new Vector2(radius * 0.35f, radius * 0.35f);

        into.DrawArc(centre, radius, 0f, Mathf.Tau, 20, colour, 2f);
        into.DrawLine(
            centre + new Vector2(radius * 0.7f, radius * 0.7f),
            centre + new Vector2(radius * 1.6f, radius * 1.6f), colour, 2f);
    }

    private void OpenSearch()
    {
        _search.Visible = true;
        _magnifier.Visible = false;
        _search.GrabFocus();
        Layout();
    }

    private void CloseSearch()
    {
        if (_search == null || !_search.Visible)
            return;

        _search.Text = string.Empty;
        _search.Visible = false;
        _search.ReleaseFocus();
        _magnifier.Visible = true;

        _pendingQuery = string.Empty;
        _searchDueAt = -1.0;
        _store.Query = string.Empty;

        Layout();
    }

    /// <summary>
    /// Takes a keystroke, and waits before acting on it.
    /// </summary>
    /// <remarks>
    /// A hundred and fifty milliseconds. Filtering on every character would rebuild the view eight
    /// times while somebody types "seal", and the eighth is the only one they wanted.
    /// </remarks>
    private void OnSearchTyped(string text)
    {
        _pendingQuery = text ?? string.Empty;
        _searchDueAt = Time.GetUnixTimeFromSystem() + SearchDebounceSeconds;
    }

    // ─── the filter rail ──────────────────────────────────────────────────────────────────────

    /// <summary>
    /// The mark on each filter button.
    /// </summary>
    /// <remarks>
    /// A silhouette, in the same geometry the rest of the interface draws its icons in -- not a
    /// picture of an item. The rail is read out of the corner of the eye at fifty-six pixels, and a
    /// sword sprite with a wooden hilt and a steel highlight stops being a sword down there; a
    /// sword-shaped hole in the plate never does. Which category gets which mark is a question about
    /// the interface, so it is answered here and not in the item data.
    /// </remarks>
    private static Action<CanvasItem, Rect2, Color> GlyphFor(string category) => category switch
    {
        "Weapon" => HudIcons.Sword,
        "Armor" => HudIcons.Robe,
        "Heavy" => HudIcons.Shield,
        "Ring" => HudIcons.Ring,
        "Ability" => HudIcons.Orb,
        "Consumable" => HudIcons.Flask,
        "Pet" => HudIcons.Paw,
        _ => HudIcons.Spark,
    };

    private void BuildRail()
    {
        // Text for the one that filters nothing, and a glyph for each of the rest. Only the
        // categories the loaded data actually has items in: see ItemCategories.Present.
        Add(null, ItemCategories.All, null);

        foreach (string category in ItemCategories.Present(_data))
            Add(category, category, GlyphFor(category));

        void Add(string category, string label, Action<CanvasItem, Rect2, Color> glyph)
        {
            var button = new RailButton(label, glyph) { Category = category };
            button.Pressed += () =>
            {
                _store.Filter = button.Category;
                _offset = 0f;
                _mountedFrom = -1;
            };

            Body.AddChild(button);
            _rail.Add(button);
        }
    }

    // ─── the grid ─────────────────────────────────────────────────────────────────────────────

    private void BuildGrid()
    {
        _grid = new Control { ClipContents = true, MouseFilter = MouseFilterEnum.Stop };
        _grid.GuiInput += OnGridInput;
        _grid.Draw += DrawGridChrome;
        Body.AddChild(_grid);
    }

    /// <summary>A filled box with soft corners, for the trough and the pill inside it.</summary>
    private static StyleBoxFlat Rounded(Color fill, int radius = 4)
    {
        var box = new StyleBoxFlat { BgColor = fill };
        box.SetCornerRadiusAll(radius);
        return box;
    }

    /// <summary>
    /// A divider across the grid: a plate the width of the grid with its name centred on it.
    /// </summary>
    /// <remarks>
    /// A plate rather than floating text. Both of these separate one kind of row from another --
    /// gifts from storage, bought from unbought -- and text on its own over a gutter reads as a
    /// caption on whatever is under it rather than as a line drawn across the grid.
    /// </remarks>
    private void Band(float top, string text)
    {
        if (top + BandHeight < 0f || top > _grid.Size.Y)
            return;

        _grid.DrawRect(new Rect2(0f, top, GridWidth, BandHeight), Style.ModalBand);
        _grid.DrawText(
            new Vector2(Mathf.Round((GridWidth - Style.Measure(text, Style.FontSmall, bold: true)) / 2f),
                top + Style.BaselineIn(BandHeight, Style.FontSmall)),
            text, Style.FontSmall, Style.Text, bold: true);
    }

    /// <summary>How many rows the grid can show at once.</summary>
    private int VisibleRows => Mathf.Max(1, Mathf.CeilToInt(_grid.Size.Y / RowPitch) + 1);

    /// <summary>Rows of owned storage, which is what the view currently has to show.</summary>
    private int ViewRows => Mathf.CeilToInt(_store.View.Count / (float)Columns);

    /// <summary>
    /// Rows of unclaimed gifts, which come first and are never sorted or filtered.
    /// </summary>
    /// <remarks>
    /// Hidden entirely under any view but plain Custom. A gift is not storage and has no place in
    /// an ordering of storage, and showing it under "A-Z" would invite a drag it cannot accept.
    /// </remarks>
    private int GiftRows => _store.CanReorder ? _store.GiftRows : 0;

    /// <summary>Where the owned rows start, below the gifts if there are any.</summary>
    private float ChestTop => GiftRows > 0 ? BandHeight + GiftRows * RowPitch : 0f;

    /// <summary>Locked rows offered: three, or none at all once the account is at its cap.</summary>
    private int LockedShown =>
        _store.Known && !_store.AtCapacity && _store.CanReorder
            ? Mathf.Min(LockedRows, _store.MaxChests - _store.ChestCount)
            : 0;

    /// <summary>Everything the grid can scroll through, in pixels.</summary>
    private float ContentHeight
    {
        get
        {
            float height = ChestTop + ViewRows * RowPitch;
            if (LockedShown > 0 || _store.AtCapacity)
                height += BandHeight + LockedShown * RowPitch;

            return height;
        }
    }

    public override void _Process(double delta)
    {
        if (_searchDueAt > 0.0 && Time.GetUnixTimeFromSystem() >= _searchDueAt)
        {
            _searchDueAt = -1.0;
            _store.Query = _pendingQuery;
        }
    }

    /// <summary>
    /// Escape clears the search before it closes the panel.
    /// </summary>
    /// <remarks>
    /// Two things are open and Escape means the innermost one, which is the rule everywhere else
    /// this shape appears. Falling through to the base is what closes the panel.
    /// </remarks>
    public override void _UnhandledKeyInput(InputEvent @event)
    {
        if (Visible && @event is InputEventKey { Pressed: true, Keycode: Key.Escape } &&
            _search is { Visible: true })
        {
            CloseSearch();
            GetViewport().SetInputAsHandled();
            return;
        }

        base._UnhandledKeyInput(@event);
    }

    private void OnGridInput(InputEvent @event)
    {
        float max = HudScrollbar.MaxOffset(_grid.Size, ContentHeight);

        switch (@event)
        {
            case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelUp }:
                Scroll(-RowPitch, max);
                return;

            case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelDown }:
                Scroll(RowPitch, max);
                return;

            case InputEventMouseButton { ButtonIndex: MouseButton.Left } click:
                if (!click.Pressed)
                {
                    _draggingThumb = false;
                    return;
                }

                switch (HudScrollbar.Test(_grid.Size, _offset, ContentHeight, click.Position))
                {
                    case HudScrollbar.Part.Thumb: _draggingThumb = true; break;
                    case HudScrollbar.Part.Up: Scroll(-RowPitch, max); break;
                    case HudScrollbar.Part.Down: Scroll(RowPitch, max); break;
                    case HudScrollbar.Part.TrackAbove: Scroll(-_grid.Size.Y, max); break;
                    case HudScrollbar.Part.TrackBelow: Scroll(_grid.Size.Y, max); break;
                    default: ClickedLocked(click.Position); break;
                }

                return;

            case InputEventMouseMotion motion when _draggingThumb:
                _offset = Mathf.Clamp(
                    HudScrollbar.OffsetForThumbTop(_grid.Size, ContentHeight, motion.Position.Y), 0f, max);
                Reflow();
                return;
        }
    }

    private void Scroll(float by, float max)
    {
        float was = _offset;
        _offset = Mathf.Clamp(_offset + by, 0f, max);

        if (!Mathf.IsEqualApprox(was, _offset))
            Reflow();
    }

    /// <summary>A click below the owned rows, which is the purchase affordance.</summary>
    private void ClickedLocked(Vector2 at)
    {
        if (LockedShown <= 0)
            return;

        float y = at.Y + _offset;
        float lockedTop = ChestTop + ViewRows * RowPitch + BandHeight;

        if (y >= lockedTop && y < lockedTop + LockedShown * RowPitch)
            PurchaseRequested?.Invoke();
    }

    // ─── layout ───────────────────────────────────────────────────────────────────────────────

    private void Layout()
    {
        if (_grid == null)
            return;

        SizeToContent();

        // The header's one control, inset from the frame by the brief's sixteen.
        _info.Size = new Vector2(40f, 40f);
        _info.Position = new Vector2(Size.X - Padding - 40f, (HeaderTall - 40f) / 2f + ModalPanel.FrameWidth);

        _explainer.Text = _store.Known
            ? $"One row is one chest. You own {_store.ChestCount} of a possible {_store.MaxChests}; " +
              $"the next costs {_store.NextChestPrice} fame, and locked rows can be bought by " +
              "clicking them. Sorting only changes what you see -- your own arrangement is kept."
            : "One row is one chest.";

        _explainer.Position = new Vector2(Padding, HeaderTall + ModalPanel.FrameWidth);
        _explainer.Size = new Vector2(Size.X - Padding * 2f, 72f);

        float inner = PanelWidth - (ModalPanel.FrameWidth + 1f) * 2f - Padding * 2f;
        float y = Padding;

        // The sort bar. The four modes live inside one trough with the search affordance outside it,
        // right-aligned: spread edge to edge they stopped reading as a set of alternatives and
        // started reading as four unrelated controls.
        float searchWidth = _search.Visible ? 220f : SortBarHeight;
        float troughWidth = inner - searchWidth - RailGap * 2f;

        _trough.Position = new Vector2(Padding, y);
        _trough.Size = new Vector2(troughWidth, SortBarHeight);

        float tabWidth = Mathf.Floor((troughWidth - Pill * 2f) / _tabs.Count);

        for (int i = 0; i < _tabs.Count; i++)
        {
            _tabs[i].Position = new Vector2(Padding + Pill + i * tabWidth, y + Pill);
            _tabs[i].Size = new Vector2(tabWidth, SortBarHeight - Pill * 2f);
        }

        var searchBox = new Rect2(Padding + inner - searchWidth, y, searchWidth, SortBarHeight);

        _magnifier.Position = searchBox.Position;
        _magnifier.Size = searchBox.Size;

        _search.Position = searchBox.Position;
        _search.Size = searchBox.Size;

        y += SortBarHeight + Padding;

        // The rail down the left, the grid beside it at exactly its own width.
        for (int i = 0; i < _rail.Count; i++)
        {
            _rail[i].Position = new Vector2(Padding, y + i * (RailButtonSize + RailGap));
            _rail[i].Size = new Vector2(RailButtonSize, RailButtonSize);
        }

        float gridLeft = Padding + RailButtonSize + RailGap * 2f;

        _note.Position = new Vector2(gridLeft, y + GridHeight + 4f);
        _note.Size = new Vector2(GridWidth, 16f);

        _grid.Position = new Vector2(gridLeft, y);
        _grid.Size = new Vector2(GridWidth + GutterWidth, GridHeight);

        Reflow();
    }

    /// <summary>How far the pill is inset inside its trough.</summary>
    private const float Pill = 4f;

    /// <summary>
    /// How tall the grid is: what there is to show, within what there is room for.
    /// </summary>
    /// <remarks>
    /// Never the maximum a fully-bought vault would need. Reserving that meant a player with one
    /// chest opened the panel onto three locked rows and then a screen-tall hole, which reads as
    /// something failing to load rather than as an empty vault.
    ///
    /// The floor is the filter rail: it is a fixed stack of buttons down the left and the panel
    /// cannot be shorter than the thing beside the grid.
    /// </remarks>
    private float GridHeight
    {
        get
        {
            float rail = _rail.Count * RailButtonSize + Mathf.Max(0, _rail.Count - 1) * RailGap;
            float room = _screen.Y - TopEdge - BottomMargin - Chrome;

            return Mathf.Max(Mathf.Min(ContentHeight, room), Mathf.Min(rail, room));
        }
    }

    /// <summary>Everything the panel spends on itself, above and below the grid.</summary>
    private float Chrome =>
        (ModalPanel.FrameWidth + 1f) * 2f + HeaderTall + Padding + SortBarHeight + Padding + Padding
        + (_note is { Visible: true } ? 20f : 0f);

    /// <summary>Grows or shrinks the panel to hold what is in it, and no more.</summary>
    private void SizeToContent()
    {
        float height = Mathf.Round(Chrome + GridHeight);

        if (!Mathf.IsEqualApprox(Size.Y, height))
            Size = new Vector2(PanelWidth, height);
    }

    /// <summary>
    /// Puts the mounted slot views over the rows now on screen.
    /// </summary>
    /// <remarks>
    /// The pool is sized to what fits plus the overscan and then never grows, however deep the
    /// vault is. Rows are addressed by where they are scrolled to rather than by index, so a scroll
    /// moves views and refills them instead of creating any.
    /// </remarks>
    private void Reflow()
    {
        if (_grid == null)
            return;

        // Rows are counted across both sections at once -- gifts first, then storage -- so one walk
        // over the pool fills whatever is on screen wherever the scroll happens to be sitting.
        int gifts = GiftRows;
        int total = gifts + ViewRows;

        int first = Mathf.Clamp(Mathf.FloorToInt(_offset / RowPitch) - Overscan, 0, Mathf.Max(0, total));
        int rows = VisibleRows + Overscan * 2;

        Mount(rows * Columns);

        var view = _store.View;

        for (int cell = 0; cell < _pool.Count; cell++)
        {
            int row = first + cell / Columns;
            int column = cell % Columns;

            var slot = _pool[cell];

            if (row >= total)
            {
                slot.Visible = false;
                continue;
            }

            bool gift = row < gifts;
            int at = gift ? row * Columns + column : (row - gifts) * Columns + column;

            if (gift ? at >= _store.Gifts.Count : at >= view.Count)
            {
                slot.Visible = false;
                continue;
            }

            slot.Visible = true;
            slot.Size = new Vector2(SlotSize, SlotSize);
            slot.Position = new Vector2(
                column * (SlotSize + SlotGap),
                (gift ? BandHeight + row * RowPitch : ChestTop + (row - gifts) * RowPitch) - _offset);

            // The address is the storage index, never the position on screen: a drag under a filter
            // would otherwise name whichever slot happens to be drawn fourth. Gifts are addressed
            // in their own space and cannot be dragged at all -- they only come out.
            slot.Address = new SlotAddress(gift ? SlotOwner.VaultGift : SlotOwner.Vault,
                gift ? at : view[at]);
            slot.Draggable = !gift && _store.CanReorder;

            Fill(slot, gift, slot.Address.Index);
        }

        _grid.QueueRedraw();
        UpdateNote();
    }

    /// <summary>Grows the pool to what is on screen, and no further.</summary>
    private void Mount(int cells)
    {
        while (_pool.Count < cells)
        {
            var slot = new SlotView { Draggable = true };
            slot.Dropped += (from, to) => Dropped?.Invoke(from, to);
            slot.Activated += () => Activated?.Invoke(slot.Address);

            _grid.AddChild(slot);
            _pool.Add(slot);
        }
    }

    private void Fill(SlotView slot, bool gift, int index)
    {
        var desc = gift ? _store.DescOfGift(index) : _store.DescAt(index);
        if (desc == null)
        {
            slot.Usable = true;
            slot.SetItem(default, null, _data);
            return;
        }

        var resolved = _textures?.Resolve(desc.Texture) ?? default;

        slot.Usable = HudView.CanEquip(desc, _slotTypes);
        slot.SetItem(resolved.Still, desc, _data);
    }

    /// <summary>
    /// The line under the grid saying why the player cannot rearrange it.
    /// </summary>
    /// <remarks>
    /// Quiet, and only present when it has something to say. A grid that has silently stopped
    /// accepting drags is indistinguishable from one that is broken.
    /// </remarks>
    private void UpdateNote()
    {
        bool needed = _store.Known && !_store.CanReorder;

        if (_note.Visible != needed)
        {
            _note.Visible = needed;
            Layout();
            return;
        }

        if (needed)
            _note.Text = "Sorted view — return to Custom to rearrange. Items can still be taken out.";
    }

    private void Refresh()
    {
        foreach (var tab in _tabs)
            tab.Active = tab.Mode == _store.Sort;

        foreach (var button in _rail)
            button.Active = button.Category == _store.Filter;

        Layout();
    }

    /// <summary>The locked rows and the scrollbar, which are drawn rather than built.</summary>
    private void DrawGridChrome()
    {
        if (GiftRows > 0)
            Band(-_offset, "Gifts");

        float bandTop = ChestTop + ViewRows * RowPitch - _offset;
        float lockedTop = bandTop + BandHeight;

        if (LockedShown > 0 || _store.AtCapacity)
            Band(bandTop, _store.AtCapacity ? "Maximum capacity" : "Locked");

        for (int row = 0; row < LockedShown; row++)
            for (int column = 0; column < Columns; column++)
            {
                var box = new Rect2(
                    column * (SlotSize + SlotGap), lockedTop + row * RowPitch, SlotSize, SlotSize);

                if (box.End.Y < 0f || box.Position.Y > _grid.Size.Y)
                    continue;

                _grid.DrawRect(box, Style.SlotLocked);
                _grid.DrawRect(box.Grow(-SlotView.Border / 2f), Style.SlotEmptyEdge.Darkened(0.35f),
                    filled: false, width: SlotView.Border);
                Padlock(box);
            }

        HudScrollbar.Draw(_grid, _grid.Size, _offset, ContentHeight, _draggingThumb);
    }

    /// <summary>A padlock, small and centred, on a row that has not been bought.</summary>
    private void Padlock(in Rect2 box)
    {
        float side = Mathf.Round(box.Size.X * 0.22f);
        var centre = box.Position + box.Size / 2f;
        var colour = Style.SlotLockedIcon;

        var body = new Rect2(centre.X - side / 2f, centre.Y - side * 0.1f, side, side * 0.7f);
        _grid.DrawRect(body, colour);

        _grid.DrawArc(
            new Vector2(centre.X, body.Position.Y), side * 0.3f, Mathf.Pi, Mathf.Tau, 12, colour, 2f);
    }

    // ─── the two kinds of button this panel has ───────────────────────────────────────────────

    /// <summary>One of the four sort modes: bare text until it is the one in use.</summary>
    private sealed partial class SortTab : Control
    {
        private readonly string _label;
        private bool _hover;

        public SortTab(string label)
        {
            _label = label;
            MouseFilter = MouseFilterEnum.Stop;

            MouseEntered += () => { _hover = true; QueueRedraw(); };
            MouseExited += () => { _hover = false; QueueRedraw(); };
        }

        public event Action Pressed;

        public VaultSort Mode { get; init; }

        private bool _active;

        public bool Active
        {
            get => _active;
            set
            {
                if (_active == value)
                    return;

                _active = value;
                QueueRedraw();
            }
        }

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is InputEventMouseButton { Pressed: false, ButtonIndex: MouseButton.Left })
                Pressed?.Invoke();
        }

        public override void _Draw()
        {
            var full = new Rect2(Vector2.Zero, Size);

            // A filled pill for the mode in use, which is the only mark the bar carries: the trough
            // behind all four is what says these are alternatives.
            if (_active)
                DrawStyleBox(Rounded(Style.ButtonFace, 3), full);
            else if (_hover)
                DrawStyleBox(Rounded(Style.ButtonFace.Darkened(0.45f), 3), full);

            float width = Style.Measure(_label, Style.FontBody, bold: _active);
            float baseline = Style.BaselineIn(Size.Y, Style.FontBody);

            this.DrawText(
                new Vector2(Mathf.Round((Size.X - width) / 2f), baseline), _label, Style.FontBody,
                _active ? Style.Text : Style.TextDim, bold: _active);
        }
    }

    /// <summary>One category down the left. Square, and labelled with as much as fits.</summary>
    private sealed partial class RailButton : Control
    {
        /// <summary>The mark's colour at rest: light enough to clear the plate under it.</summary>
        private static readonly Color RailMark = new("c8c8c8");

        private static readonly Color RailActive = new("5a5a5a");
        private static readonly Color RailHover = new("484848");

        private readonly string _label;
        private readonly Action<CanvasItem, Rect2, Color> _glyph;
        private bool _hover;

        public RailButton(string label, Action<CanvasItem, Rect2, Color> glyph)
        {
            _label = label;
            _glyph = glyph;
            MouseFilter = MouseFilterEnum.Stop;
            TooltipText = label;

            MouseEntered += () => { _hover = true; QueueRedraw(); };
            MouseExited += () => { _hover = false; QueueRedraw(); };
        }

        public event Action Pressed;

        /// <summary>Null for the button that filters nothing.</summary>
        public string Category { get; init; }

        private bool _active;

        public bool Active
        {
            get => _active;
            set
            {
                if (_active == value)
                    return;

                _active = value;
                QueueRedraw();
            }
        }

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is InputEventMouseButton { Pressed: false, ButtonIndex: MouseButton.Left })
                Pressed?.Invoke();
        }

        public override void _Draw()
        {
            var full = new Rect2(Vector2.Zero, Size);

            // The plate lightens when the filter is on and the mark goes white with it. Revision
            // four drew a dim grey mark on a dim grey plate, which at fifty-six pixels is a button
            // you can see and a mark you cannot.
            DrawRect(full, _active ? RailActive : _hover ? RailHover : Style.Slot);
            DrawRect(full.Grow(-SlotView.Border / 2f), _active ? Style.SlotBorderHi : Style.SlotEmptyEdge,
                filled: false, width: SlotView.Border);

            var mark = _active ? Style.Text : RailMark;

            if (_glyph != null)
            {
                // Square and centred, inset so the plate still reads as a button around it.
                float side = Mathf.Round(Mathf.Min(Size.X, Size.Y) - 20f);
                var box = new Rect2(
                    Mathf.Round((Size.X - side) / 2f), Mathf.Round((Size.Y - side) / 2f), side, side);

                _glyph(this, box, mark);
                return;
            }

            // No artwork: the all-items button. Three letters is what fits across this face.
            string text = _label.Length <= 3 ? _label : _label.Substring(0, 3).ToUpperInvariant();

            float width = Style.Measure(text, Style.FontSmall, bold: true);
            float baseline = Style.BaselineIn(Size.Y, Style.FontSmall);

            this.DrawText(
                new Vector2(Mathf.Round((Size.X - width) / 2f), baseline), text, Style.FontSmall,
                mark, bold: true);
        }
    }
}
