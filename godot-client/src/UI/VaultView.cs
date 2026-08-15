using System;
using System.Collections.Generic;
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
    /// <summary>Where the top edge sits on a full-height screen.</summary>
    public const float TopEdge = 68f;

    /// <summary>How little is left under it. The panel runs nearly the whole height of the screen.</summary>
    private const float BottomEdge = 8f;

    private const int Columns = 8;

    /// <summary>A slot cell, border included. Eighty, which is what the reference measures.</summary>
    private const float SlotSize = 80f;

    /// <summary>The board showing between two cells.</summary>
    private const float SlotGap = 5f;

    private const float RowPitch = SlotSize + SlotGap;

    private const float GridWidth = Columns * SlotSize + (Columns - 1) * SlotGap;

    /// <summary>Air between the grid and the plate it sits on: left and top, then right and bottom.</summary>
    private const float GridPad = 15f;

    private const float GridTopPad = 9f;
    private const float GridEdgePad = 5f;

    /// <summary>The scrollbar and the gap that separates it from the last column.</summary>
    private const float GutterWidth = GridEdgePad + HudScrollbar.Width;

    /// <summary>The plate the grid sits on, which is wider than the grid by its padding.</summary>
    private const float GridBoxWidth = GridPad + GridWidth + GutterWidth + GridEdgePad;

    /// <summary>The plate the category rail sits on, and where the buttons sit inside it.</summary>
    private const float RailBoxWidth = 63f;

    private const float RailButtonSize = 50f;

    /// <summary>The one that filters nothing is a wide, short plate rather than a square.</summary>
    private const float RailAllHeight = 36f;

    private const float RailGap = 8f;
    private const float RailPad = 11f;
    private const float RailLeftPad = 9f;

    public const float PanelWidth = BodyInset * 2f + RailBoxWidth + GridBoxWidth;

    /// <summary>What the panel falls back to before it knows how much screen it has.</summary>
    public const float PanelHeight = 900f;

    /// <summary>The sort bar, which sits on the shell above the two plates rather than on either.</summary>
    private const float SortBarHeight = 50f;

    private const float SortBarGap = 5f;

    /// <summary>How far the active tab's pill is inset inside the trough.</summary>
    private const float Pill = 4f;

    /// <summary>Where the two plates start, measured down the body.</summary>
    private const float PlatesTop = SortBarHeight + SortBarGap;

    /// <summary>Rows kept mounted beyond the visible ones, above and below.</summary>
    private const int Overscan = 2;

    /// <summary>How deep the locked section goes. Three, never the theoretical maximum.</summary>
    private const int LockedRows = 3;

    /// <summary>The full-width label that introduces a section. Gifts above, locked below.</summary>
    private const float BandHeight = 29f;

    /// <summary>
    /// The gap between a band and the first row under it.
    /// </summary>
    /// <remarks>
    /// A band with rows tight against it reads as the top edge of the first row rather than as a
    /// heading over all of them. This is the one gap in the grid that is not the ordinary gutter,
    /// which is the point: it says the thing below is a different kind of row.
    /// </remarks>
    private const float BandGap = 10f;

    /// <summary>A band and the air under it, which is what a section costs before its first row.</summary>
    private const float BandBlock = BandHeight + BandGap;

    /// <summary>
    /// The three sizes this panel sets text at, measured off the reference.
    /// </summary>
    /// <remarks>
    /// Larger than the type scale in <c>Style</c>, which is roughly two thirds of what the reference
    /// actually uses at every step. Written here so the panel matches what it is measured against;
    /// the scale itself is one change in one file and belongs to whoever owns it.
    /// </remarks>
    private const int TabSize = 28;

    private const int BandSize = 22;
    private const int RailSize = 30;

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

    /// <summary>No cross: Escape and walking away are the exits.</summary>
    protected override bool ShowClose => false;

    protected override bool ShowInfo => true;

    protected override bool ShowOrnaments => true;

    /// <summary>The rail and the grid are two plates, not one, so the shell paints neither.</summary>
    protected override bool FillBody => false;

    public override void _Ready()
    {
        base._Ready();

        Body.Draw += DrawPlates;

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

    /// <summary>Tells the panel how much screen it has, and lays it out in it.</summary>
    public void PlaceIn(Vector2 screen)
    {
        _screen = screen;
        Layout();
    }

    /// <summary>
    /// Docks the panel against the right-hand column of the HUD.
    /// </summary>
    /// <remarks>
    /// Not centred. The reference hangs it from the top of the screen with its right edge against
    /// the column that carries the minimap and the vault's own counter, which is what makes the two
    /// read as one thing you are doing rather than as a panel that happens to be open. Centring it
    /// put the grid over the middle of the world and the counter a third of a screen away from the
    /// grid it counts.
    /// </remarks>
    private void Reposition()
    {
        float right = _screen.X - HudLayout.Margin * 2f - HudLayout.MinimapWidth;

        Position = new Vector2(
            Mathf.Round(Mathf.Max(0f, right - PanelWidth)),
            Mathf.Round(Mathf.Min(TopEdge, Mathf.Max(0f, _screen.Y - Size.Y - BottomEdge))));
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
        // One dark trough behind the four, so they read as one control with four settings.
        _trough = new Control { MouseFilter = MouseFilterEnum.Ignore };
        _trough.Draw += DrawTrough;
        Body.AddChild(_trough);

        foreach (var (mode, label) in Modes)
        {
            var tab = new SortTab(label) { Mode = mode };
            tab.Pressed += () => Choose(tab.Mode);

            Body.AddChild(tab);
            _tabs.Add(tab);
        }

        _magnifier = new HudIconButton(Magnifier, "Search", inset: 14f) { Tint = Style.Text };
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
    }

    /// <summary>
    /// The trough and the plate the magnifier sits on, which are one bar with two tones.
    /// </summary>
    /// <remarks>
    /// The search affordance is not in the trough with the sort modes: it is a different kind of
    /// thing -- the modes are four alternatives and one of them is always on, while the search is a
    /// control you reach for -- so it gets its own lighter square at the end of the bar.
    /// </remarks>
    private void DrawTrough()
    {
        var full = new Rect2(Vector2.Zero, _trough.Size);

        var edge = new StyleBoxFlat { BgColor = Style.ModalFrameDark };
        edge.SetCornerRadiusAll(6);
        _trough.DrawStyleBox(edge, full);

        float inset = 3f;
        float plate = SortBarHeight;

        var well = new StyleBoxFlat { BgColor = Style.ModalBody };
        well.SetCornerRadiusAll(4);
        _trough.DrawStyleBox(well, new Rect2(
            inset, inset, Mathf.Max(0f, full.Size.X - plate - inset), full.Size.Y - inset * 2f));
    }

    private void Choose(VaultSort mode)
    {
        _store.Sort = mode;
        _offset = 0f;
        _mountedFrom = -1;
    }

    private static void Magnifier(CanvasItem into, Rect2 box, Color colour)
    {
        float radius = Mathf.Min(box.Size.X, box.Size.Y) * 0.38f;
        var centre = box.Position + box.Size / 2f - new Vector2(radius * 0.3f, radius * 0.3f);

        into.DrawArc(centre, radius, 0f, Mathf.Tau, 24, colour, 3f);
        into.DrawLine(
            centre + new Vector2(radius * 0.72f, radius * 0.72f),
            centre + new Vector2(radius * 1.7f, radius * 1.7f), colour, 3f);
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
    /// The item whose shape stands for each category.
    /// </summary>
    /// <remarks>
    /// <para>
    /// A real sprite out of the game's own sheets, flattened to one tone -- see
    /// <see cref="SpriteSilhouette"/>. Two earlier passes drew these by hand and neither survived
    /// contact: a shape invented for the rail is a shape that belongs to nothing else on the
    /// screen, and next to seven of its neighbours it reads as a symbol you have to learn rather
    /// than as the thing itself. A sword the game already draws is a sword.
    /// </para>
    /// <para>
    /// Which item stands for which category is a question about the interface, not about the data,
    /// so it is answered here. They are picked for their outline and nothing else: a robe because
    /// its flared skirt is the one worn shape that is not the same vest as heavy armour, an egg for
    /// pets because a pet has no item of its own, a plain shield for abilities because the
    /// twenty-odd things that go in that slot have no common shape at all.
    /// </para>
    /// </remarks>
    private static string MarkItem(string category) => category switch
    {
        "Weapon" => "Long Sword",
        "Armor" => "Robe of the Neophyte",
        "Heavy" => "Plate Mail",
        "Ring" => "Ring of Attack",
        "Ability" => "Wooden Shield",
        "Consumable" => "Health Potion",
        "Pet" => "Common Feline Egg",
        _ => null,
    };

    /// <summary>
    /// Turns that into something that can draw itself on a button.
    /// </summary>
    /// <remarks>
    /// Sized to a whole multiple of the source sprite, for the reason every sprite in this
    /// interface is: an eight-pixel shape drawn at thirty-six puts four and a half screen pixels on
    /// each of its own, and half the mark comes out a pixel fatter than the other half.
    /// </remarks>
    private Action<CanvasItem, Rect2, Color> MarkFor(string category)
    {
        var desc = _data?.GetObject(MarkItem(category) ?? string.Empty);
        var sprite = desc != null ? (_textures?.Resolve(desc.Texture) ?? default).Still : default;
        var shape = SpriteSilhouette.Of(sprite);

        if (shape == null)
            return HudIcons.Spark;

        return (into, box, colour) =>
        {
            float source = Mathf.Max(1f, Mathf.Min(shape.GetWidth(), shape.GetHeight()));
            float room = Mathf.Min(box.Size.X, box.Size.Y);
            float side = Mathf.Max(source, Mathf.Floor(room / source) * source);

            into.DrawTextureRect(shape, new Rect2(
                box.Position + (box.Size - new Vector2(side, side)) / 2f,
                new Vector2(side, side)), false, colour);
        };
    }

    private void BuildRail()
    {
        // Text for the one that filters nothing, and a glyph for each of the rest. Only the
        // categories the loaded data actually has items in: see ItemCategories.Present.
        Add(null, ItemCategories.All, null);

        foreach (string category in ItemCategories.Present(_data))
            Add(category, category, MarkFor(category));

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

    /// <summary>How tall the stack of rail buttons is, which is what its plate is sized to.</summary>
    private float RailHeight
    {
        get
        {
            if (_rail.Count == 0)
                return RailPad * 2f;

            float stack = RailAllHeight + (_rail.Count - 1) * (RailButtonSize + RailGap);
            return RailPad * 2f + stack;
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

    /// <summary>The two near-black plates the body is made of: the rail's, and the grid's.</summary>
    private void DrawPlates()
    {
        DrawBodyPlate(new Rect2(0f, PlatesTop, RailBoxWidth, RailHeight));
        DrawBodyPlate(new Rect2(
            RailBoxWidth, PlatesTop, GridBoxWidth, Mathf.Max(0f, Body.Size.Y - PlatesTop)));
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

        var plate = new StyleBoxFlat { BgColor = Style.ModalFrameDark };
        plate.SetCornerRadiusAll(3);
        _grid.DrawStyleBox(plate, new Rect2(0f, top, GridWidth, BandHeight));

        _grid.DrawText(
            new Vector2(Mathf.Round((GridWidth - Style.Measure(text, BandSize, bold: true)) / 2f),
                top + Style.BaselineIn(BandHeight, BandSize)),
            text, BandSize, Style.Text, bold: true);
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
    private float ChestTop => GiftRows > 0 ? BandBlock + GiftRows * RowPitch : 0f;

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
                height += BandBlock + LockedShown * RowPitch;

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
        float lockedTop = ChestTop + ViewRows * RowPitch + BandBlock;

        if (y >= lockedTop && y < lockedTop + LockedShown * RowPitch)
            PurchaseRequested?.Invoke();
    }

    // ─── layout ───────────────────────────────────────────────────────────────────────────────

    private void Layout()
    {
        if (_grid == null)
            return;

        SizeToContent();

        float inner = Body.Size.X;

        // The sort bar runs the whole width of the body, on the shell above both plates.
        float searchWidth = _search.Visible ? 260f : SortBarHeight;

        _trough.Position = Vector2.Zero;
        _trough.Size = new Vector2(inner, SortBarHeight);

        float modes = inner - SortBarHeight;
        float tabWidth = Mathf.Floor((modes - Pill * 2f) / _tabs.Count);

        for (int i = 0; i < _tabs.Count; i++)
        {
            _tabs[i].Position = new Vector2(Pill + i * tabWidth, Pill);
            _tabs[i].Size = new Vector2(tabWidth, SortBarHeight - Pill * 2f);
        }

        var searchBox = new Rect2(inner - searchWidth, 0f, searchWidth, SortBarHeight);

        _magnifier.Position = searchBox.Position;
        _magnifier.Size = searchBox.Size;

        _search.Position = searchBox.Position;
        _search.Size = searchBox.Size;

        // The rail down the left, the grid beside it at exactly its own width.
        float railTop = PlatesTop + RailPad;

        for (int i = 0; i < _rail.Count; i++)
        {
            float height = i == 0 ? RailAllHeight : RailButtonSize;

            _rail[i].Position = new Vector2(RailLeftPad, railTop);
            _rail[i].Size = new Vector2(RailButtonSize, height);

            railTop += height + RailGap;
        }

        _grid.Position = new Vector2(RailBoxWidth + GridPad, PlatesTop + GridTopPad);
        _grid.Size = new Vector2(GridWidth + GutterWidth, GridHeight);

        Body.QueueRedraw();
        Reposition();
        Reflow();
    }

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
            float rail = RailHeight - GridTopPad - GridEdgePad;
            float room = _screen.Y - TopEdge - BottomEdge - Chrome;

            return Mathf.Max(Mathf.Min(ContentHeight, room), Mathf.Min(rail, room));
        }
    }

    /// <summary>Everything the panel spends on itself, above and below the grid.</summary>
    private float Chrome =>
        FrameWidth + HeaderHeight + HeaderGap + PlatesTop + GridTopPad + GridEdgePad + BodyInset;

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
                column * RowPitch,
                (gift ? BandBlock + row * RowPitch : ChestTop + (row - gifts) * RowPitch) - _offset);

            // The address is the storage index, never the position on screen: a drag under a filter
            // would otherwise name whichever slot happens to be drawn fourth. Gifts are addressed
            // in their own space and cannot be dragged at all -- they only come out.
            slot.Address = new SlotAddress(gift ? SlotOwner.VaultGift : SlotOwner.Vault,
                gift ? at : view[at]);
            slot.Draggable = !gift && _store.CanReorder;

            Fill(slot, gift, slot.Address.Index);
        }

        _grid.QueueRedraw();
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
        float lockedTop = bandTop + BandBlock;

        if (LockedShown > 0 || _store.AtCapacity)
            Band(bandTop, _store.AtCapacity ? "Maximum capacity" : "Locked");

        for (int row = 0; row < LockedShown; row++)
            for (int column = 0; column < Columns; column++)
            {
                var box = new Rect2(column * RowPitch, lockedTop + row * RowPitch, SlotSize, SlotSize);

                if (box.End.Y < 0f || box.Position.Y > _grid.Size.Y)
                    continue;

                _grid.DrawRect(box, Style.SlotLocked);
                _grid.DrawRect(box.Grow(-SlotView.Border / 2f), Style.SlotEmptyEdge.Darkened(0.35f),
                    filled: false, width: SlotView.Border);
                Padlock(box);
            }

        if (Nothing != null)
        {
            float width = Style.Measure(Nothing, Style.FontBody);
            _grid.DrawText(
                new Vector2(Mathf.Round((GridWidth - width) / 2f),
                    Mathf.Round(ChestTop + RowPitch / 2f) + Style.BaselineIn(0f, Style.FontBody)),
                Nothing, Style.FontBody, Style.TextDim);
        }

        HudScrollbar.Draw(_grid, _grid.Size, _offset, ContentHeight, _draggingThumb);
    }

    /// <summary>
    /// What the grid says when it has nothing to show, or null when it has.
    /// </summary>
    /// <remarks>
    /// Two different nothings, and telling them apart is the whole value of saying anything: an
    /// empty vault is a state you fix by putting something in it, and a filter that matches nothing
    /// is a state you fix by changing the filter. A grid that just sits there blank leaves the
    /// player to work out which one they are looking at.
    /// </remarks>
    private string Nothing
    {
        get
        {
            if (!_store.Known || _store.View.Count > 0 || GiftRows > 0)
                return null;

            return _store.CanReorder ? "Nothing stored yet" : "No items match";
        }
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
            if (_active || _hover)
            {
                var pill = new StyleBoxFlat
                {
                    BgColor = _active ? Style.TabActive : Style.TabActive.Darkened(0.35f),
                };
                pill.SetCornerRadiusAll(4);
                DrawStyleBox(pill, full);
            }

            float width = Style.Measure(_label, TabSize, bold: _active);
            float baseline = Style.BaselineIn(Size.Y, TabSize);

            this.DrawText(
                new Vector2(Mathf.Round((Size.X - width) / 2f), baseline), _label, TabSize,
                _active ? Style.Text : Style.TextDim, bold: _active);
        }
    }

    /// <summary>One category down the left. Square, and labelled with as much as fits.</summary>
    private sealed partial class RailButton : Control
    {
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

            // The plate lightens when the filter is on and the mark goes white with it. An earlier
            // revision drew a dim grey mark on a dim grey plate, which at fifty pixels is a button
            // you can see and a mark you cannot.
            var plate = new StyleBoxFlat
            {
                BgColor = _active ? Style.TabActive : _hover ? Style.TabActive.Darkened(0.3f) : Style.ModalTrough,
            };
            plate.SetCornerRadiusAll(4);
            DrawStyleBox(plate, full);

            var mark = _active ? Style.Text : Style.StatLabel;

            if (_glyph != null)
            {
                // Square and centred, inset so the plate still reads as a button around it.
                float side = Mathf.Round(Mathf.Min(Size.X, Size.Y) - 16f);
                var box = new Rect2(
                    Mathf.Round((Size.X - side) / 2f), Mathf.Round((Size.Y - side) / 2f), side, side);

                _glyph(this, box, mark);
                return;
            }

            // No artwork: the all-items button. Three letters is what fits across this face.
            string text = _label.Length <= 3 ? _label : _label.Substring(0, 3).ToUpperInvariant();

            float width = Style.Measure(text, RailSize, bold: true);
            float baseline = Style.BaselineIn(Size.Y, RailSize);

            this.DrawText(
                new Vector2(Mathf.Round((Size.X - width) / 2f), baseline), text, RailSize,
                mark, bold: true);
        }
    }
}
