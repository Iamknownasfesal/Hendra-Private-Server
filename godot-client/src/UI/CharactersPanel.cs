using System;
using System.Collections.Generic;
using System.Linq;
using Godot;
using Hendra.Account;

namespace Hendra.UI;

/// <summary>
/// The account's characters, as a panel docked into the top left corner of the world.
/// </summary>
/// <remarks>
/// <para>
/// The original opens this over the running game rather than as a page you leave the world for:
/// it is anchored under the player card, the world keeps playing beside it, and everything it
/// offers — switching character, rolling a new one, deleting one, buying a slot — is reached
/// without disconnecting first.
/// </para>
/// <para>
/// It is drawn rather than assembled from containers. Every measurement here is taken off
/// <c>references/Menu/Characters UI.png</c>, which is a pixel-exact crop of the panel at 1080p,
/// and a stack of themed containers cannot be held to a pixel.
/// </para>
/// </remarks>
public partial class CharactersPanel : Control
{
    public const float PanelWidth = 559f;
    public const float PanelHeight = 785f;

    /// <summary>The light hairline around the whole panel.</summary>
    private const float Edge = 3f;

    private const float HeaderHeight = 72f;

    /// <summary>Left and right gutter between the panel edge and everything inside it.</summary>
    private const float Gutter = 7f;

    private const float TabTop = 82f;
    private const float TabHeight = 57f;

    /// <summary>The darker lip along the bottom of a tab, which is what makes it look pressed in.</summary>
    private const float TabFoot = 10f;

    private const float OrderTop = 144f;
    private const float OrderHeight = 40f;
    private const float OrderWidth = 260f;

    private const float ListTop = 188f;
    private const float RowHeight = 79f;
    private const float RowGap = 9f;
    private const float RowInset = 9f;
    private const float PlateWidth = 75f;
    private const float ScrollbarWidth = 9f;

    // Measured from the reference crop. None of these has a token in Style yet; they are the tab
    // bevels, the tab lips, the dropdown, and the three greys a row is built from.
    private static readonly Color TabActiveEdge = new("696969");
    private static readonly Color TabIdleEdge = new("444444");
    private static readonly Color TabActiveFoot = new("3e3e3e");
    private static readonly Color TabIdleFoot = new("2e2e2e");
    private static readonly Color TabIdleFill = new("373737");
    private static readonly Color DropdownFill = new("565656");
    private static readonly Color DropdownEdge = new("222222");
    private static readonly Color RowFill = new("3a3939");
    private static readonly Color RowPlate = new("4c4b4b");
    private static readonly Color RowFillCurrent = new("4c4b4b");
    private static readonly Color RowPlateCurrent = new("535252");
    private static readonly Color RowMuted = new("2d2d2d");
    private static readonly Color ScrollThumb = new("666666");

    private Control _plate;
    private Label _title;
    private TabHead _alive;
    private TabHead _grave;
    private Dropdown _order;
    private RowList _list;

    private CharacterRoster _roster;
    private bool _graveyard;
    private Order _sort = Order.Fame;

    private string _appServerUrl;
    private string _guid;
    private string _password;
    private bool _fetching;

    /// <summary>The character being played, drawn as the current row. Negative when there is none.</summary>
    public int CurrentCharacterId { get; set; } = -1;

    /// <summary>Where the panel's top left corner goes, in interface pixels.</summary>
    public Vector2 Dock { get; set; } = new(4f, 79f);

    public bool IsOpen => _plate is { Visible: true };

    /// <summary>Raised with the character the player wants to load.</summary>
    public event Action<int> PlayRequested;

    /// <summary>Raised when the player asks for the create-a-character page.</summary>
    public event Action NewCharacterRequested;

    /// <summary>Raised with the character the player asked to scrap.</summary>
    public event Action<int> DeleteRequested;

    /// <summary>Raised when the player asks to buy another slot.</summary>
    public event Action BuySlotRequested;

    public event Action Closed;

    /// <summary>Which way the list is ordered. The original's dropdown offers the same three.</summary>
    private enum Order
    {
        Fame,
        Level,
        Class,
    }

    public void Connect(string appServerUrl, string guid, string password)
    {
        _appServerUrl = appServerUrl;
        _guid = guid;
        _password = password;
    }

    /// <summary>What the panel is currently drawing, once it has anything.</summary>
    public CharacterRoster Roster => _roster;

    /// <summary>Raised whenever a fresh roster lands, so the create page can follow it.</summary>
    public event Action<CharacterRoster> Loaded;

    /// <summary>Hands the panel a roster somebody else has already fetched.</summary>
    public void Show(CharacterRoster roster)
    {
        _roster = roster;
        Rebuild();
        Loaded?.Invoke(roster);
    }

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;

        _plate = new Backdrop { Visible = false, MouseFilter = MouseFilterEnum.Stop };
        AddChild(_plate);

        _title = new Label
        {
            Text = "Characters",
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        }.Typeset(TitleSize, Style.Text);
        _plate.AddChild(_title);

        _alive = new TabHead("Alive", true);
        _alive.Pressed += () => SetTab(false);
        _plate.AddChild(_alive);

        _grave = new TabHead("Graveyard", false);
        _grave.Pressed += () => SetTab(true);
        _plate.AddChild(_grave);

        _order = new Dropdown();
        _order.Chose += order => { _sort = order; Rebuild(); };
        _plate.AddChild(_order);

        _list = new RowList();
        _plate.AddChild(_list);

        Resized += Reflow;
        Reflow();
    }

    private const int TitleSize = 42;

    /// <summary>Puts the panel where it docks and lays its parts out inside it.</summary>
    private void Reflow()
    {
        if (_plate == null)
            return;

        _plate.Position = Dock;
        _plate.Size = new Vector2(PanelWidth, PanelHeight);

        _title.Position = new Vector2(Edge, Edge);
        _title.Size = new Vector2(PanelWidth - Edge * 2f, HeaderHeight);

        float left = Edge + Gutter;
        float width = PanelWidth - (Edge + Gutter) * 2f;
        float tabWidth = (width - 3f) / 2f;

        _alive.Position = new Vector2(left, TabTop);
        _alive.Size = new Vector2(tabWidth, TabHeight);

        _grave.Position = new Vector2(left + tabWidth + 3f, TabTop);
        _grave.Size = new Vector2(tabWidth, TabHeight);

        _order.Position = new Vector2(left, OrderTop);
        _order.Size = new Vector2(OrderWidth, OrderHeight);

        _list.Position = new Vector2(left, ListTop);
        _list.Size = new Vector2(width, PanelHeight - ListTop - Edge - 1f);
    }

    private void SetTab(bool graveyard)
    {
        if (_graveyard == graveyard)
            return;

        _graveyard = graveyard;
        _alive.Selected = !graveyard;
        _grave.Selected = graveyard;
        Rebuild();
    }

    public void Open()
    {
        _plate.Visible = true;
        Refresh();
    }

    /// <summary>Opens on a named tab. "graveyard" is the only one that is not the default.</summary>
    public void ShowTab(string tab)
    {
        SetTab(string.Equals(tab, "graveyard", StringComparison.OrdinalIgnoreCase));
    }

    public void Close()
    {
        if (_plate == null || !_plate.Visible)
            return;

        _plate.Visible = false;
        Closed?.Invoke();
    }

    public void Toggle()
    {
        if (IsOpen)
            Close();
        else
            Open();
    }

    /// <summary>Fetches the roster, unless one is already on its way.</summary>
    public async void Refresh()
    {
        if (_fetching || string.IsNullOrEmpty(_appServerUrl))
            return;

        _fetching = true;

        try
        {
            var roster = await CharacterRoster.FetchAsync(_appServerUrl, _guid, _password);
            if (IsInstanceValid(this))
                Show(roster);
        }
        catch (Exception ex)
        {
            GD.PushWarning($"[characters] {ex.Message}");
        }
        finally
        {
            _fetching = false;
        }
    }

    /// <summary>
    /// Escape closes it, and nothing else here takes the keyboard.
    /// </summary>
    /// <remarks>
    /// Unhandled rather than a shortcut, so the world's keys keep working beside an open panel.
    /// </remarks>
    public override void _UnhandledKeyInput(InputEvent @event)
    {
        if (IsOpen && @event is InputEventKey { Pressed: true, Keycode: Key.Escape })
        {
            Close();
            GetViewport().SetInputAsHandled();
        }
    }

    /// <summary>Fills the list with whichever tab is showing.</summary>
    private void Rebuild()
    {
        if (_list == null)
            return;

        _list.Clear();

        if (_roster == null)
            return;

        var characters = Sorted(_graveyard ? _roster.Graveyard : _roster.Alive);

        foreach (var character in characters)
        {
            var row = new CharacterRow(character, _roster, _graveyard)
            {
                Current = character.CharacterId == CurrentCharacterId && !_graveyard,
            };

            int id = character.CharacterId;
            if (!_graveyard)
            {
                row.Pressed += () => PlayRequested?.Invoke(id);
                row.DeletePressed += () => DeleteRequested?.Invoke(id);
            }

            _list.Add(row);
        }

        if (_graveyard)
        {
            // The server keeps the account's ten most recent deaths and nothing else, so an account
            // that has never lost a character has an empty graveyard rather than a broken one.
            if (characters.Count == 0)
                _list.Add(new NoticeRow(_roster.GraveyardReadable
                    ? "No characters have died yet."
                    : "The server did not answer for this account's deaths."));

            _list.Reflow();
            return;
        }

        var classes = App.ServiceLocator.Data?.PlayerClasses;
        int outstanding = classes == null
            ? 0
            : _roster.ClassQuestsOutstanding(classes.Select(c => c.Type));

        var create = new NewCharacterRow(outstanding, _roster.Alive.Count < Math.Max(_roster.MaxCharacters, 1));
        create.Pressed += () => NewCharacterRequested?.Invoke();
        _list.Add(create);

        if (_roster.Account.NextSlotPrice > 0)
        {
            var buy = new BuySlotRow(_roster.Account.NextSlotPrice);
            buy.Pressed += () => BuySlotRequested?.Invoke();
            _list.Add(buy);
        }

        _list.Reflow();
    }

    private List<CharacterInfo> Sorted(IEnumerable<CharacterInfo> characters) => _sort switch
    {
        Order.Level => characters.OrderByDescending(c => c.Level).ThenByDescending(c => c.CurrentFame).ToList(),
        Order.Class => characters.OrderBy(ClassName).ToList(),
        _ => characters.OrderByDescending(c => c.CurrentFame).ToList(),
    };

    internal static string ClassName(CharacterInfo character)
    {
        var desc = App.ServiceLocator.Data?.GetObject(character.ObjectType);
        return desc?.DisplayId ?? desc?.Id ?? $"Type {character.ObjectType}";
    }

    /// <summary>
    /// The standing frame of a class's own sprite.
    /// </summary>
    /// <remarks>
    /// The same one the world draws when the character is not moving, pulled through the texture
    /// resolver rather than picked out of a sheet by hand.
    /// </remarks>
    internal static Assets.Sprite ClassSprite(ushort objectType)
    {
        var desc = App.ServiceLocator.Data?.GetObject(objectType);
        if (desc?.Texture == null || App.ServiceLocator.Assets == null)
            return default;

        var resolved = new Assets.TextureResolver(App.ServiceLocator.Assets).Resolve(desc.Texture);
        return resolved.Animated != null
            ? resolved.Animated.Frame(0f, 0f, Assets.CharAction.Stand, 0f).Sprite
            : resolved.Still;
    }

    /// <summary>An item's sprite, for the small mark beside a character's inventory count.</summary>
    internal static Assets.Sprite ItemSprite(int objectType)
    {
        if (objectType <= 0 || App.ServiceLocator.Assets == null)
            return default;

        var desc = App.ServiceLocator.Data?.GetObject((ushort)objectType);
        if (desc?.Texture == null)
            return default;

        return new Assets.TextureResolver(App.ServiceLocator.Assets).Resolve(desc.Texture).Still;
    }

    /// <summary>
    /// How full a character's eight-slot bag is.
    /// </summary>
    /// <remarks>
    /// The equipment array is the character's whole loadout: four worn slots first, then the eight
    /// the bag holds. The original's row counts the bag, not the loadout.
    /// </remarks>
    internal static (int Used, int First) Bag(CharacterInfo character)
    {
        int used = 0;
        int first = 0;

        for (int slot = 4; slot < 12 && slot < character.Equipment.Length; slot++)
        {
            if (character.Equipment[slot] <= 0)
                continue;

            used++;
            if (first == 0)
                first = character.Equipment[slot];
        }

        return (used, first);
    }

    // ---------------------------------------------------------------------------------------------
    // The plate, and the parts drawn on it
    // ---------------------------------------------------------------------------------------------

    /// <summary>The panel itself: a hairline, a header band, and a field on a grey board.</summary>
    private sealed partial class Backdrop : Control
    {
        public override void _Draw()
        {
            var full = new Rect2(Vector2.Zero, Size);

            DrawRect(full, Style.ModalFrame);
            DrawRect(full.Grow(-Edge), Style.ModalBand);
            DrawRect(new Rect2(Edge, Edge, Size.X - Edge * 2f, HeaderHeight), Style.ModalHeader);

            // The list's own field, a step darker than the board the tabs sit on.
            DrawRect(new Rect2(
                Edge + Gutter, ListTop, Size.X - (Edge + Gutter) * 2f,
                Size.Y - ListTop - Edge - 1f), Style.ModalBody);

            Ornaments(full);
        }

        /// <summary>The four bracket ticks, which are the only decoration the panel carries.</summary>
        private void Ornaments(in Rect2 full)
        {
            const float inset = 5f;
            const float arm = 16f;

            for (int corner = 0; corner < 4; corner++)
            {
                bool right = corner is 1 or 2;
                bool bottom = corner is 2 or 3;

                float x = right ? full.End.X - inset : full.Position.X + inset;
                float y = bottom ? full.End.Y - inset : full.Position.Y + inset;
                float dx = right ? -arm : arm;
                float dy = bottom ? -arm : arm;

                DrawLine(new Vector2(x, y), new Vector2(x + dx, y), Style.ModalFrame, 3f);
                DrawLine(new Vector2(x, y), new Vector2(x, y + dy), Style.ModalFrame, 3f);
            }
        }
    }

    /// <summary>One of the two tabs: a bevelled plate with a darker lip along the bottom.</summary>
    private sealed partial class TabHead : Control
    {
        private readonly string _text;
        private bool _selected;

        public TabHead(string text, bool selected)
        {
            _text = text;
            _selected = selected;
            MouseFilter = MouseFilterEnum.Stop;
        }

        public event Action Pressed;

        public bool Selected
        {
            get => _selected;
            set
            {
                _selected = value;
                QueueRedraw();
            }
        }

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
                Pressed?.Invoke();
        }

        public override void _Draw()
        {
            var body = new Rect2(0f, 0f, Size.X, Size.Y - TabFoot);

            DrawRect(body, _selected ? TabActiveEdge : TabIdleEdge);
            DrawRect(body.Grow(-5f), _selected ? Style.TabActive : TabIdleFill);
            DrawRect(new Rect2(0f, Size.Y - TabFoot, Size.X, TabFoot),
                _selected ? TabActiveFoot : TabIdleFoot);

            this.DrawText(
                new Vector2(0f, Style.BaselineIn(Size.Y - TabFoot, TabTextSize)), _text, TabTextSize,
                _selected ? Style.TabActiveText : Style.StatLabel, alignment: HorizontalAlignment.Center,
                width: Size.X);
        }

        private const int TabTextSize = 28;
    }

    /// <summary>The order control: a plate with a chevron, and a menu of the three orders.</summary>
    private sealed partial class Dropdown : Control
    {
        private PopupMenu _menu;

        public Dropdown() => MouseFilter = MouseFilterEnum.Stop;

        public event Action<Order> Chose;

        public override void _Ready()
        {
            _menu = new PopupMenu();
            _menu.AddItem("Fame", (int)Order.Fame);
            _menu.AddItem("Level", (int)Order.Level);
            _menu.AddItem("Class", (int)Order.Class);
            _menu.IdPressed += id => Chose?.Invoke((Order)id);
            AddChild(_menu);
        }

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is not InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
                return;

            _menu.Position = (Vector2I)GetScreenPosition() + new Vector2I(0, (int)Size.Y);
            _menu.Popup();
        }

        public override void _Draw()
        {
            var full = new Rect2(Vector2.Zero, Size);

            DrawRect(full, DropdownEdge);
            DrawRect(full.Grow(-4f), DropdownFill);

            this.DrawText(
                new Vector2(0f, Style.BaselineIn(Size.Y, TextSize)), "Order", TextSize, Style.Text,
                alignment: HorizontalAlignment.Center, width: Size.X - 22f);

            HudIcons.ChevronDown(
                this, new Rect2(Size.X - 26f, Size.Y / 2f - 5f, 14f, 10f), Style.Text);
        }

        private const int TextSize = 26;
    }

    /// <summary>The scrolling field the rows sit in, and the bar beside them.</summary>
    private sealed partial class RowList : Control
    {
        private readonly List<Control> _rows = new();
        private float _offset;

        public RowList()
        {
            MouseFilter = MouseFilterEnum.Stop;
            ClipContents = true;
        }

        public void Clear()
        {
            foreach (var row in _rows)
            {
                RemoveChild(row);
                row.QueueFree();
            }

            _rows.Clear();
            _offset = 0f;
        }

        public void Add(Control row)
        {
            _rows.Add(row);
            AddChild(row);
        }

        public void Reflow()
        {
            float width = Size.X - RowInset * 3f - ScrollbarWidth - 9f;
            for (int i = 0; i < _rows.Count; i++)
            {
                _rows[i].Position = new Vector2(RowInset, RowInset - _offset + i * (RowHeight + RowGap));
                _rows[i].Size = new Vector2(width, RowHeight);
            }

            QueueRedraw();
        }

        private float Content => _rows.Count * (RowHeight + RowGap) - RowGap + RowInset * 2f;

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is not InputEventMouseButton { Pressed: true } wheel)
                return;

            float step = wheel.ButtonIndex switch
            {
                MouseButton.WheelDown => RowHeight + RowGap,
                MouseButton.WheelUp => -(RowHeight + RowGap),
                _ => 0f,
            };

            if (step == 0f)
                return;

            _offset = Mathf.Clamp(_offset + step, 0f, Mathf.Max(0f, Content - Size.Y));
            Reflow();
        }

        public override void _Draw()
        {
            // The bar is always there. The original draws its track whether or not the list is long
            // enough to need one, and a bar that comes and goes moves every row under it when it does.
            float travel = Mathf.Max(Content, Size.Y);
            float height = Mathf.Max(30f, Size.Y * Size.Y / travel);
            float top = travel <= Size.Y ? 0f : _offset / (travel - Size.Y) * (Size.Y - height);

            DrawRect(new Rect2(Size.X - ScrollbarWidth - RowInset, top, ScrollbarWidth, height), ScrollThumb);
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Rows
    // ---------------------------------------------------------------------------------------------

    /// <summary>Everything a row shares: the plate, the sprite square, and the two lines of text.</summary>
    private abstract partial class Row : Control
    {
        protected Row() => MouseFilter = MouseFilterEnum.Stop;

        public event Action Pressed;

        /// <summary>The character currently being played, which the original lifts out of the list.</summary>
        public bool Current { get; set; }

        protected virtual bool Clickable => true;

        /// <summary>Where the second line sits, and what the right-hand cluster lines up with.</summary>
        protected const float SubLine = 55f;

        protected const int NameSize = 28;
        protected const int SubSize = 22;
        protected const float TextLeft = 82f;

        public override void _GuiInput(InputEvent @event)
        {
            if (Clickable && @event is InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
                Pressed?.Invoke();
        }

        public override void _Draw()
        {
            DrawRect(new Rect2(Vector2.Zero, Size), Current ? RowFillCurrent : RowFill);

            if (Current)
                DrawRect(new Rect2(Vector2.Zero, Size), RowPlateCurrent, filled: false, width: 5f);

            DrawRect(new Rect2(0f, 0f, PlateWidth, Size.Y), Current ? RowPlateCurrent : RowPlate);
            Contents();
        }

        protected abstract void Contents();

        /// <summary>A character's sprite, centred on the row's square and drawn big.</summary>
        protected void DrawPortrait(ushort objectType)
        {
            var sprite = ClassSprite(objectType);
            if (!sprite.IsValid)
                return;

            float size = 46f;
            this.DrawSprite(sprite, new Rect2(
                (PlateWidth - size) / 2f, (Size.Y - size) / 2f, size, size), outline: 0f);
        }

        protected void Lines(string name, string under, Color nameColour, Color underColour)
        {
            this.DrawText(new Vector2(TextLeft, 34f), name, NameSize, nameColour);
            this.DrawText(new Vector2(TextLeft, SubLine + 12f), under, SubSize, underColour);
        }
    }

    /// <summary>One of the account's characters, living or buried.</summary>
    private sealed partial class CharacterRow : Row
    {
        private readonly CharacterInfo _character;
        private readonly CharacterRoster _roster;
        private readonly bool _dead;

        private HudIconButton _delete;
        private StarMark _star;

        public CharacterRow(CharacterInfo character, CharacterRoster roster, bool dead)
        {
            _character = character;
            _roster = roster;
            _dead = dead;
        }

        public event Action DeletePressed;

        public override void _Ready()
        {
            if (_dead)
                return;

            _delete = new HudIconButton(Trash, "Delete this character", inset: 3f) { Tint = Style.Text };
            _delete.Pressed += () => DeletePressed?.Invoke();
            AddChild(_delete);

            // Nothing on this server records a favourite character, so the mark is drawn in its
            // unset shape for every row rather than picking one at random to fill in.
            _star = new StarMark();
            AddChild(_star);

            Resized += Place;
            Place();
        }

        private void Place()
        {
            if (_delete == null)
                return;

            float right = Size.X - 8f;
            _star.Position = new Vector2(right - 25f, SubLine - 10f);
            _star.Size = new Vector2(25f, 25f);

            _delete.Position = new Vector2(right - 70f, SubLine - 10f);
            _delete.Size = new Vector2(26f, 26f);
        }

        protected override void Contents()
        {
            DrawPortrait(_character.ObjectType);

            string name = $"{ClassName(_character)} {_character.Level}";
            string under = _dead
                ? $"{_character.CurrentFame:N0} Fame"
                : _roster.FameLine(_character);

            Lines(name, under, Style.Text, Style.StatLabel);

            var (used, first) = Bag(_character);
            var sprite = ItemSprite(first);
            float right = Size.X - 8f;

            // The bag count, with the first thing in it as the mark beside the number.
            if (sprite.IsValid)
                this.DrawSprite(sprite, new Rect2(right - 160f, SubLine - 12f, 28f, 28f), outline: 1f);

            this.DrawText(new Vector2(right - 126f, SubLine + 12f), $"{used}/8", 26, Style.Text);
        }

        /// <summary>The bin, drawn as a lid, a body and two staves.</summary>
        private static void Trash(CanvasItem into, Rect2 box, Color colour)
        {
            float lid = box.Position.Y + box.Size.Y * 0.18f;

            into.DrawRect(new Rect2(box.Position.X, lid, box.Size.X, 2f), colour);
            into.DrawRect(new Rect2(
                box.Position.X + box.Size.X * 0.35f, box.Position.Y, box.Size.X * 0.3f, 3f), colour);

            var body = new Rect2(
                box.Position.X + box.Size.X * 0.12f, lid + 3f,
                box.Size.X * 0.76f, box.Size.Y - (lid - box.Position.Y) - 3f);

            into.DrawRect(body, colour, filled: false, width: 2f);

            for (int i = 1; i <= 2; i++)
            {
                float x = body.Position.X + body.Size.X * i / 3f;
                into.DrawLine(new Vector2(x, body.Position.Y + 4f), new Vector2(x, body.End.Y - 4f), colour, 2f);
            }
        }
    }

    /// <summary>The five-pointed outline the original marks a favourite with.</summary>
    private sealed partial class StarMark : Control
    {
        public StarMark()
        {
            MouseFilter = MouseFilterEnum.Stop;
            TooltipText = "Favourite";
        }

        public override void _Draw() => DrawStar(this, new Rect2(Vector2.Zero, Size), Style.Text, false);
    }

    /// <summary>
    /// A star, filled or hollow.
    /// </summary>
    /// <remarks>
    /// Drawn from its ten points rather than taken from a sheet: the interface needs it at half a
    /// dozen sizes — a row's favourite mark, a class quest chip, a class card's rating — and a
    /// sprite scaled to each of those is a sprite blurred at five of them.
    /// </remarks>
    internal static void DrawStar(CanvasItem into, in Rect2 box, Color colour, bool filled)
    {
        var centre = box.Position + box.Size / 2f;
        float outer = Mathf.Min(box.Size.X, box.Size.Y) / 2f;
        float inner = outer * 0.44f;

        var points = new Vector2[10];
        for (int i = 0; i < 10; i++)
        {
            float angle = -Mathf.Pi / 2f + i * Mathf.Pi / 5f;
            float radius = i % 2 == 0 ? outer : inner;
            points[i] = centre + new Vector2(Mathf.Cos(angle), Mathf.Sin(angle)) * radius;
        }

        if (filled)
        {
            into.DrawColoredPolygon(points, colour);
            return;
        }

        var loop = new Vector2[11];
        points.CopyTo(loop, 0);
        loop[10] = points[0];
        into.DrawPolyline(loop, colour, 2f);
    }

    /// <summary>The row that opens the create page, greyed the way the original greys it.</summary>
    private sealed partial class NewCharacterRow : Row
    {
        private readonly int _outstanding;
        private readonly bool _room;

        public NewCharacterRow(int outstanding, bool room)
        {
            _outstanding = outstanding;
            _room = room;
        }

        protected override void Contents()
        {
            // A question mark on a dark square where a character's sprite would be.
            var square = new Rect2(16f, Size.Y / 2f - 27f, 50f, 54f);
            DrawRect(square, RowMuted);
            this.DrawText(
                new Vector2(square.Position.X, square.Position.Y + 40f), "?", 40, Style.StatLabel,
                alignment: HorizontalAlignment.Center, width: square.Size.X);

            string under = _outstanding > 0
                ? $"{_outstanding} Class quests not yet completed"
                : _room ? "A slot is free" : "Every slot is in use";

            Lines("New Character", under, Style.StatLabel, Style.StatLabel);
        }
    }

    /// <summary>The offer at the bottom of the list: a plus, a label, and the price on a chip.</summary>
    private sealed partial class BuySlotRow : Row
    {
        private readonly int _price;

        public BuySlotRow(int price) => _price = price;

        protected override void Contents()
        {
            HudIcons.Plus(this, new Rect2(23f, Size.Y / 2f - 15f, 30f, 30f), Style.StatLabel);
            this.DrawText(new Vector2(TextLeft, Size.Y / 2f + 8f), "Buy Character Slot", NameSize, Style.StatLabel);

            var chip = new Rect2(Size.X - 154f, Size.Y / 2f - 23f, 118f, 46f);
            DrawRect(chip, Style.ButtonPromo.Darkened(0.25f));
            DrawRect(chip.Grow(-4f), Style.ButtonPromo);

            this.DrawText(
                new Vector2(chip.Position.X + 10f, chip.Position.Y + 33f), $"{_price:N0}", 28,
                Style.Text, alignment: HorizontalAlignment.Left);

            HudIcons.Coin(this, new Rect2(chip.End.X - 40f, chip.Position.Y + 9f, 28f, 28f), Style.IconGold);
        }
    }

    /// <summary>A line of explanation where a row would be. The empty graveyard is the only user.</summary>
    private sealed partial class NoticeRow : Row
    {
        private readonly string _text;

        public NoticeRow(string text) => _text = text;

        protected override bool Clickable => false;

        public override void _Draw() =>
            this.DrawText(new Vector2(8f, 30f), _text, SubSize, Style.StatLabel);

        protected override void Contents()
        {
        }
    }
}
