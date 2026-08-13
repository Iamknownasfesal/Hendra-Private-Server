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
    public const float PanelWidth = 920f;
    public const float PanelHeight = 960f;

    /// <summary>Where the top edge sits, per the brief.</summary>
    public const float TopEdge = 60f;

    private const float Padding = 16f;
    private const float HeaderTall = 64f;
    private const float SortBarHeight = 56f;
    private const float RailButtonSize = 56f;
    private const float RailGap = 6f;

    private const int Columns = 8;
    private const float SlotSize = 96f;
    private const float SlotGap = 4f;
    private const float RowPitch = SlotSize + SlotGap;

    /// <summary>Rows kept mounted beyond the visible ones, above and below.</summary>
    private const int Overscan = 2;

    /// <summary>How deep the locked section goes. Three, never the theoretical maximum.</summary>
    private const int LockedRows = 3;

    private const float LockedBandHeight = 32f;

    /// <summary>How long the search waits after a keystroke before it filters.</summary>
    private const double SearchDebounceSeconds = 0.150;

    private readonly VaultStore _store;
    private readonly GameData _data;
    private readonly TextureResolver _textures;

    private readonly List<SortTab> _tabs = new();
    private readonly List<RailButton> _rail = new();
    private readonly List<SlotView> _pool = new();

    private Control _grid;
    private LineEdit _search;
    private HudIconButton _magnifier;
    private Label _note;
    private Label _lockedBand;

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

    /// <summary>Raised when a slot is clicked: the quick move between vault and inventory.</summary>
    public event Action<int> Activated;

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

        BuildSortBar();
        BuildRail();
        BuildGrid();

        _store.Changed += OnStoreChanged;
        Closed += OnClosed;
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
        Position = new Vector2(Mathf.Round((screen.X - PanelWidth) / 2f), TopEdge);
    }

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

        _note = new Label { Visible = false }.Typeset(Style.FontTag, Style.TextDim);
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

    private void BuildRail()
    {
        Add(null, ItemCategories.All);

        foreach (string category in ItemCategories.Order)
            Add(category, category);

        void Add(string category, string label)
        {
            var button = new RailButton(label) { Category = category };
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

        _lockedBand = new Label
        {
            Text = "Locked",
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        }.Typeset(Style.FontSmall, Style.TextDim);
        _grid.AddChild(_lockedBand);
    }

    /// <summary>How many rows the grid can show at once.</summary>
    private int VisibleRows => Mathf.Max(1, Mathf.CeilToInt(_grid.Size.Y / RowPitch) + 1);

    /// <summary>Rows of owned storage, which is what the view currently has to show.</summary>
    private int ViewRows => Mathf.CeilToInt(_store.View.Count / (float)Columns);

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
            float height = ViewRows * RowPitch;
            if (LockedShown > 0 || _store.AtCapacity)
                height += LockedBandHeight + LockedShown * RowPitch;

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
        float lockedTop = ViewRows * RowPitch + LockedBandHeight;

        if (y >= lockedTop && y < lockedTop + LockedShown * RowPitch)
            PurchaseRequested?.Invoke();
    }

    // ─── layout ───────────────────────────────────────────────────────────────────────────────

    private void Layout()
    {
        if (_grid == null)
            return;

        float inner = Body.Size.X - Padding * 2f;
        float y = Padding;

        // The sort bar: four tabs sharing the width left of the search affordance.
        float searchWidth = _search.Visible ? 220f : SortBarHeight;
        float tabsWidth = inner - searchWidth - RailGap;
        float tabWidth = Mathf.Floor(tabsWidth / _tabs.Count);

        for (int i = 0; i < _tabs.Count; i++)
        {
            _tabs[i].Position = new Vector2(Padding + i * tabWidth, y);
            _tabs[i].Size = new Vector2(tabWidth, SortBarHeight);
        }

        var searchBox = new Rect2(
            Padding + inner - searchWidth, y, searchWidth, SortBarHeight);

        _magnifier.Position = searchBox.Position;
        _magnifier.Size = searchBox.Size;

        _search.Position = searchBox.Position + new Vector2(0f, 8f);
        _search.Size = new Vector2(searchBox.Size.X, SortBarHeight - 16f);

        y += SortBarHeight + RailGap;

        // The rail down the left, the grid filling what is left beside it.
        for (int i = 0; i < _rail.Count; i++)
        {
            _rail[i].Position = new Vector2(Padding, y + i * (RailButtonSize + RailGap));
            _rail[i].Size = new Vector2(RailButtonSize, RailButtonSize);
        }

        float gridLeft = Padding + RailButtonSize + RailGap * 2f;
        float gridWidth = Columns * SlotSize + (Columns - 1) * SlotGap + HudScrollbar.Width + RailGap;

        _note.Position = new Vector2(gridLeft, Body.Size.Y - Padding - 16f);
        _note.Size = new Vector2(gridWidth, 16f);

        float noteRoom = _note.Visible ? 20f : 0f;

        _grid.Position = new Vector2(gridLeft, y);
        _grid.Size = new Vector2(gridWidth, Mathf.Max(0f, Body.Size.Y - y - Padding - noteRoom));

        Reflow();
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

        int first = Mathf.Max(0, Mathf.FloorToInt(_offset / RowPitch) - Overscan);
        int rows = VisibleRows + Overscan * 2;

        Mount(rows * Columns);

        var view = _store.View;

        for (int cell = 0; cell < _pool.Count; cell++)
        {
            int row = first + cell / Columns;
            int column = cell % Columns;
            int at = row * Columns + column;

            var slot = _pool[cell];

            if (at >= view.Count)
            {
                slot.Visible = false;
                continue;
            }

            int index = view[at];

            slot.Visible = true;
            slot.Position = new Vector2(
                column * (SlotSize + SlotGap), row * RowPitch - _offset);
            slot.Size = new Vector2(SlotSize, SlotSize);

            // The address is the storage index, never the position on screen: a drag under a filter
            // would otherwise name whichever slot happens to be drawn fourth.
            slot.Address = new SlotAddress(SlotOwner.Vault, index);
            slot.Draggable = _store.CanReorder;

            Fill(slot, index);
        }

        float lockedTop = ViewRows * RowPitch - _offset;

        _lockedBand.Visible = LockedShown > 0 || _store.AtCapacity;
        _lockedBand.Text = _store.AtCapacity ? "Maximum capacity" : "Locked";
        _lockedBand.Position = new Vector2(0f, lockedTop);
        _lockedBand.Size = new Vector2(
            Columns * SlotSize + (Columns - 1) * SlotGap, LockedBandHeight);

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
            slot.Activated += () => Activated?.Invoke(slot.Address.Index);

            _grid.AddChild(slot);
            _pool.Add(slot);
        }
    }

    private void Fill(SlotView slot, int index)
    {
        var desc = _store.DescAt(index);
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
        float lockedTop = ViewRows * RowPitch - _offset + LockedBandHeight;

        for (int row = 0; row < LockedShown; row++)
            for (int column = 0; column < Columns; column++)
            {
                var box = new Rect2(
                    column * (SlotSize + SlotGap), lockedTop + row * RowPitch, SlotSize, SlotSize);

                if (box.End.Y < 0f || box.Position.Y > _grid.Size.Y)
                    continue;

                _grid.DrawRect(box, Style.Slot.Darkened(0.4f));
                _grid.DrawRect(box, Style.SlotBorder.Darkened(0.5f), filled: false, width: 1f);
                Padlock(box);
            }

        HudScrollbar.Draw(_grid, _grid.Size, _offset, ContentHeight, _draggingThumb);
    }

    /// <summary>A padlock, small and centred, on a row that has not been bought.</summary>
    private void Padlock(in Rect2 box)
    {
        float side = Mathf.Round(box.Size.X * 0.22f);
        var centre = box.Position + box.Size / 2f;
        var colour = Style.TextDim.Darkened(0.2f);

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

            if (_active)
                DrawRect(full.Grow(-6f), Style.ButtonFace);
            else if (_hover)
                DrawRect(full.Grow(-6f), Style.ButtonFace.Darkened(0.35f));

            float width = Style.Measure(_label, Style.FontBody);
            float baseline = Mathf.Round(
                (Size.Y + Style.Pixel.GetAscent(Style.FontBody) - Style.Pixel.GetDescent(Style.FontBody)) / 2f);

            this.DrawOutlined(
                new Vector2(Mathf.Round((Size.X - width) / 2f), baseline), _label, Style.FontBody,
                _active ? Style.Text : Style.TextDim);
        }
    }

    /// <summary>One category down the left. Square, and labelled with as much as fits.</summary>
    private sealed partial class RailButton : Control
    {
        private readonly string _label;
        private bool _hover;

        public RailButton(string label)
        {
            _label = label;
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

            DrawRect(full, _active ? Style.ButtonFace
                : _hover ? Style.ButtonFace.Darkened(0.35f)
                : Style.Slot);
            DrawRect(full, Style.SlotBorder.Darkened(0.3f), filled: false, width: 1f);

            // Three letters is what fits in fifty-six pixels of this face at tag size.
            string text = _label.Length <= 3
                ? _label
                : _label.Substring(0, 3).ToUpperInvariant();

            float width = Style.Measure(text, Style.FontTag);
            float baseline = Mathf.Round(
                (Size.Y + Style.Pixel.GetAscent(Style.FontTag) - Style.Pixel.GetDescent(Style.FontTag)) / 2f);

            this.DrawOutlined(
                new Vector2(Mathf.Round((Size.X - width) / 2f), baseline), text, Style.FontTag,
                _active ? Style.Text : Style.TextDim);
        }
    }
}
