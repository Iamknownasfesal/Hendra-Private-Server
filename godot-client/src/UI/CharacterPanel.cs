using System;
using System.Collections.Generic;
using System.Globalization;
using Godot;
using Hendra.Account;
using Hendra.Assets;
using Hendra.Resources;
using Hendra.World;

namespace Hendra.UI;

/// <summary>
/// The character sheet: attributes, tallies and dungeon counts, docked beside the world.
/// </summary>
/// <remarks>
/// <para>
/// Two halves with different clocks. The identity block and the attribute grid come off the player
/// entity and are refreshed twice a second while the panel is open, because they change while you
/// play. The tallies do not: the only place the server publishes a living character's statistics is
/// the <c>PCStats</c> blob in the character list, over HTTP, so they are fetched once when the
/// panel opens and left alone until it is opened again.
/// </para>
/// <para>
/// Everything below the shell is drawn rather than built out of nodes. The sheet is a fixed
/// arrangement of a dozen strings and six plates; as controls that is fifty nodes to lay out twice
/// a second, and every one of them a place the arithmetic could disagree with the reference.
/// </para>
/// </remarks>
public partial class CharacterPanel : Control
{
    // Metrics, in the body's own pixels, measured off references/Menu/Stats UI.png against its
    // position in references/Fullscreen/Stats fs.png. Vertical measurements are absolute because
    // the sheet above the list is a fixed block; horizontal ones are derived from the body's width
    // so the panel keeps its proportions whatever rectangle the layout hands it.

    private const float Margin = 10f;

    private const float PortraitLeft = 14f;
    private const float PortraitTop = 7f;
    private const float PortraitSize = 86f;
    private const float TextLeft = 111f;
    private const float NameBaseline = 36f;
    private const float ClassBaseline = 65f;
    private const float CreatedBaseline = 84f;
    private const float FameIcon = 30f;
    private const float FameIconTop = 15f;

    private const float GridTop = 109f;
    private const float CellHeight = 54f;
    private const float CellPitch = 72f;
    private const float CellGap = 9f;
    private const float CellEdge = 4f;

    /// <summary>How far the cell's edges stop short of its corners, which is what rounds them.</summary>
    private const float CellCorner = 5f;

    private const float TabsTop = 331f;
    private const float TabHeight = 47f;

    /// <summary>The darkened step under a tab, where the list's shadow falls across it.</summary>
    private const float TabFoot = 11f;

    private const float TabGap = 4f;
    private const float TabEdge = 5f;
    private const float TabBaseline = 31f;

    private const float ListTop = TabsTop + TabHeight + TabFoot;

    /// <summary>The gap between the top of the list and its first plate.</summary>
    private const float ListPad = 9f;

    private const float RowPlate = 36f;

    /// <summary>Plate plus gutter. Fractional because the reference's rows alternate 41 and 42.</summary>
    private const float RowPitch = 41.5f;

    private const float RowBaseline = 23f;
    private const float GroupHeight = 46f;
    private const float GroupBaseline = 31f;
    private const float RowInset = 15f;
    private const float PlateInset = 9f;
    private const float ScrollWidth = 9f;

    /// <summary>How far the last rows fade out before the footer band, as the reference does.</summary>
    private const float ListFade = 28f;

    private const float FooterHeight = 61f;
    private const float FooterBaseline = 39f;
    private const float FooterIcon = 36f;

    // The type scale this panel is set in. The interface's shared scale tops out at 28, which is a
    // display size on a 300-wide cluster and a body size on a sheet this large: every size below
    // is Jersey 10 at whatever matches the cap height the reference sets that line in.
    private const int FontSheetName = 32;
    private const int FontSheetClass = 26;
    private const int FontSheetCreated = 20;
    private const int FontSheetFame = 26;
    private const int FontCellLabel = 22;
    private const int FontCellValue = 30;
    private const int FontTabLabel = 26;
    private const int FontRowLabel = 26;
    private const int FontRowValue = 30;
    private const int FontGroup = 32;
    private const int FontFooterLabel = 28;
    private const int FontFooterValue = 36;

    // Colours the shared palette has no name for yet. Every one is a measurement off the reference
    // rather than a choice; see the report that came with this panel for the tokens it wants.

    /// <summary>The plate the class portrait sits on.</summary>
    private static readonly Color PortraitPlate = new("474747");

    /// <summary>A character's own name, which is duller than the gold on an attribute.</summary>
    private static readonly Color NameGold = new("c9b200");

    /// <summary>A statistic's label: not quite white, so the green beside it reads as the answer.</summary>
    private static readonly Color RowLabel = new("e8e8e8");

    private static readonly Color TabActiveEdge = new("696969");
    private static readonly Color TabIdleFill = new("373737");
    private static readonly Color TabIdleEdge = new("444444");
    private static readonly Color TabIdleText = new("7b7c7d");
    private static readonly Color TabActiveFoot = new("3e3e3e");
    private static readonly Color TabActiveFootEdge = new("201f1f");
    private static readonly Color TabIdleFoot = new("2e2e2e");
    private static readonly Color TabIdleFootEdge = new("343434");
    private static readonly Color ScrollThumb = new("666666");

    /// <summary>Twice a second, which is as often as any of these numbers is worth reading.</summary>
    private const double RefreshSeconds = 0.5;

    private SheetShell _shell;
    private Sheet _sheet;
    private TabStrip _tabs;
    private RowList _rows;
    private Footer _footer;

    private GameData _data;
    private TextureResolver _textures;
    private AppEngineClient _accounts;
    private string _guid;
    private string _password;
    private int _characterId = -1;

    private CharacterStats _stats;
    private DateTime? _createdAt;
    private double _sinceRefresh;
    private bool _fetching;
    private int _tab;

    /// <summary>Whether the panel is open, so the caller can keep its own button in step.</summary>
    public bool IsOpen => _shell is { Visible: true };

    /// <summary>Raised when the panel closes by any route, including Escape and its own button.</summary>
    public event Action Closed;

    public void Configure(GameData data, AssetLibrary assets)
    {
        _data = data;
        _textures = new TextureResolver(assets);
    }

    /// <summary>
    /// Points the panel at the app server, which is the only source of the tallies.
    /// </summary>
    /// <remarks>
    /// Every reconnection comes back through here -- a portal, the nexus button, a world change --
    /// and almost always with the character that is already on show. Throwing the tallies away on
    /// each of those emptied the list and dropped the creation date the moment the player took a
    /// portal, and nothing asked for them again. Only a different character is a different sheet.
    /// </remarks>
    public void Connect(string appServerUrl, string guid, string password, int characterId)
    {
        _accounts = new AppEngineClient(appServerUrl);
        _guid = guid;
        _password = password;

        if (characterId != _characterId)
        {
            _characterId = characterId;
            _stats = null;
            _createdAt = null;
            _rows?.Reset();
        }

        if (IsOpen)
            Fetch();
    }

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;

        _shell = new SheetShell("Attributes");
        _shell.Closed += () => Closed?.Invoke();
        AddChild(_shell);

        _sheet = new Sheet();
        _shell.Body.AddChild(_sheet);

        _tabs = new TabStrip(Pages);
        _tabs.Selected += Show;
        _shell.Body.AddChild(_tabs);

        _rows = new RowList();
        _shell.Body.AddChild(_rows);

        _footer = new Footer();
        _shell.Body.AddChild(_footer);

        Resized += Reflow;
        if (GetParent() is HudLayer layer)
            layer.Reflowed += Reflow;

        Reflow();
    }

    private static readonly string[] Pages = { "Stats", "Dungeons" };

    /// <summary>
    /// Places the panel and the four blocks in it.
    /// </summary>
    /// <remarks>
    /// The rectangle comes from <see cref="HudLayout.Modal"/> rather than from anything written
    /// down here, so that moving the panel is one edit in the layout and none in the panel.
    /// </remarks>
    private void Reflow()
    {
        if (_shell == null)
            return;

        var layout = new HudLayout(Size.X > 0f && Size.Y > 0f
            ? Size
            : new Vector2(HudLayout.ReferenceWidth, HudLayout.ReferenceHeight));

        var rect = layout.Modal;
        _shell.Position = rect.Position;
        _shell.Size = rect.Size;

        var body = _shell.Body;
        float width = body.Size.X;

        _sheet.Position = Vector2.Zero;
        _sheet.Size = new Vector2(width, TabsTop);

        _tabs.Position = new Vector2(Margin, TabsTop);
        _tabs.Size = new Vector2(width - Margin * 2f, TabHeight + TabFoot);

        float footerTop = body.Size.Y - FooterHeight;

        _rows.Position = new Vector2(Margin, ListTop);
        _rows.Size = new Vector2(
            Mathf.Max(0f, width - Margin * 2f - 1f), Mathf.Max(0f, footerTop - ListTop));

        _footer.Position = new Vector2(0f, footerTop);
        _footer.Size = new Vector2(width, FooterHeight);
    }

    /// <summary>Closes the sheet, for whoever needs the slot it is in.</summary>
    public void Close() => _shell?.Close();

    public void Toggle()
    {
        if (_shell.Visible)
        {
            _shell.Close();
            return;
        }

        _shell.Open();
        _sinceRefresh = RefreshSeconds;
        Fetch();
    }

    /// <summary>
    /// Asks the app server for the character list, which is where the tallies live.
    /// </summary>
    /// <remarks>
    /// Once per opening rather than on a timer. It is an HTTP round trip, and the numbers in it --
    /// shots fired, dungeons completed -- do not move fast enough to be worth polling.
    /// </remarks>
    private async void Fetch()
    {
        if (_accounts == null || _fetching)
            return;

        _fetching = true;

        try
        {
            string xml = await _accounts.PostAsync("/char/list", new Dictionary<string, string>
            {
                ["guid"] = _guid,
                ["password"] = _password,
            });

            foreach (var character in CharListResult.Parse(xml).Characters)
            {
                if (character.CharacterId != _characterId)
                    continue;

                _stats = character.Stats;
                _createdAt = character.CreatedAt;
                break;
            }
        }
        catch (Exception ex)
        {
            // The panel is still worth showing without them: the attributes come off the player.
            GD.PushWarning($"[character] could not read the tallies: {ex.Message}");
        }
        finally
        {
            _fetching = false;
            Show(_tab);
        }
    }

    /// <summary>Called every frame by whoever owns the panel; does its own throttling.</summary>
    public void Refresh(LocalPlayer player, double delta)
    {
        if (!IsOpen || player == null)
            return;

        _sinceRefresh += delta;
        if (_sinceRefresh < RefreshSeconds)
            return;

        _sinceRefresh = 0.0;
        RefreshSheet(player);
    }

    private void RefreshSheet(LocalPlayer player)
    {
        var desc = _data?.GetObject(player.ObjectType);
        var maxima = desc?.StatMaxima;

        _sheet.Portrait = ClassPortrait(player.ObjectType);
        _sheet.Name = string.IsNullOrEmpty(player.Name) ? "—" : player.Name;
        _sheet.ClassLine = $"Level {player.Level}, {desc?.DisplayId ?? desc?.Id ?? "Adventurer"}";

        // One fixed pattern rather than the machine's long date, which puts the weekday in and the
        // month wherever the locale keeps it. A server that sends no timestamp leaves the line out
        // rather than showing a guess.
        _sheet.Created = _createdAt.HasValue
            ? "Created on " + _createdAt.Value.ToLocalTime().ToString("MMMM d, yyyy", CultureInfo.InvariantCulture)
            : string.Empty;

        _sheet.Fame = player.Fame.ToString("N0", CultureInfo.InvariantCulture);

        int[] values =
        {
            player.Attack, player.Defense, player.Speed,
            player.Dexterity, player.Vitality, player.Wisdom,
        };

        for (int i = 0; i < Keys.Length; i++)
        {
            // The boosts and maxima are indexed with MaxHP and MaxMP first, so they run two ahead
            // of the six shown here.
            int at = i + 2;
            int bonus = player.Boosts[at];
            int max = maxima != null && at < maxima.Length ? maxima[at] : 0;

            _sheet.Cells[i] = new Sheet.Cell
            {
                Key = Keys[i],
                Value = values[i].ToString(CultureInfo.InvariantCulture),

                // Nothing at all at zero. "(+0)" is noise in six cells at once.
                Bonus = bonus == 0 ? null : bonus > 0 ? $"(+{bonus})" : $"(−{-bonus})",

                // The one colour change the grid draws: an attribute equipment has carried past
                // what the class can reach on its own. An unknown ceiling is never marked, because
                // a guess here would be a lie in the one place the grid exists to be read.
                Above = max > 0 && values[i] > max,
            };
        }

        // Fame on death is what is banked plus whatever the bonuses come to, and the bonuses are
        // computed server-side at the moment of death -- /char/fame refuses a living character. So
        // this is the floor, and it is the fame the character currently carries.
        _footer.Value = _sheet.Fame;
        _footer.QueueRedraw();
        _sheet.QueueRedraw();
    }

    private static readonly string[] Keys = { "ATT", "DEF", "SPD", "DEX", "VIT", "WIS" };

    private Assets.Sprite ClassPortrait(ushort objectType)
    {
        var resolved = _textures?.Resolve(_data?.GetObject(objectType)?.Texture) ?? default;

        if (resolved.Animated != null)
            return resolved.Animated.Frame(0f, 0f, CharAction.Stand, 0f).Sprite;

        return resolved.Still;
    }

    /// <summary>
    /// Fills the list for one of the two pages.
    /// </summary>
    /// <remarks>
    /// Both pages come out of one blob: the server writes the dungeon tallies in two contiguous
    /// runs inside the statistics, which is what lets a single fetch answer both.
    /// </remarks>
    private void Show(int tab)
    {
        _tab = tab;
        _tabs.Current = tab;

        if (_rows == null)
            return;

        _rows.Begin(tab);

        if (_stats == null)
        {
            _rows.Empty = "Statistics are not available for this character.";
            _rows.End();
            return;
        }

        _rows.Empty = null;

        if (tab == 0)
        {
            // The reference carries a Power Level above the group heading. This server publishes no
            // such figure -- it is not in FameStats and not in the character list -- so the row
            // keeps its place and says so rather than showing a number nothing computed.
            _rows.Add("Power Level", null);
            _rows.Group("Statistics");

            foreach (int id in Statistics)
                _rows.Add(CharacterStats.Label(id), _stats.Value(id));

            _rows.End();
            return;
        }

        _rows.Group("Dungeons");

        foreach (int id in CharacterStats.DungeonsByName)
            _rows.Add(CharacterStats.Label(id), _stats.Value(id));

        _rows.End();
    }

    /// <summary>
    /// The statistics, in the order the reference lists them.
    /// </summary>
    /// <remarks>
    /// Not the order the server writes them in: the reference puts party level-ups between the
    /// assists and the god kills, which is where a player looks for them. Every id here is a field
    /// the server actually sends; nothing in the reference's list is faked to fill a gap.
    /// </remarks>
    private static readonly int[] Statistics = { 0, 1, 2, 3, 4, 5, 6, 7, 19, 8, 10, 9, 11, 12, 20 };

    /// <summary>The identity block and the attribute grid, which are one drawn block.</summary>
    private sealed partial class Sheet : Control
    {
        /// <summary>One attribute: its caption, its number, and what equipment is adding.</summary>
        public struct Cell
        {
            public string Key;
            public string Value;
            public string Bonus;

            /// <summary>Whether the total stands above what the class can reach unaided.</summary>
            public bool Above;
        }

        public readonly Cell[] Cells = new Cell[6];

        public Assets.Sprite Portrait;
        public string Name = "—";
        public string ClassLine = string.Empty;
        public string Created = string.Empty;
        public string Fame = "0";

        public Sheet() => MouseFilter = MouseFilterEnum.Ignore;

        public override void _Draw()
        {
            DrawIdentity();

            float column = (Size.X - Margin * 2f - CellGap) / 2f;

            for (int i = 0; i < Cells.Length; i++)
            {
                DrawCell(Cells[i], new Rect2(
                    Margin + i % 2 * (column + CellGap),
                    GridTop + i / 2 * CellPitch,
                    column,
                    CellHeight));
            }
        }

        private void DrawIdentity()
        {
            var plate = new Rect2(PortraitLeft, PortraitTop, PortraitSize, PortraitSize);
            DrawRect(plate, PortraitPlate);
            this.DrawSprite(Portrait, plate.Grow(-7f));

            this.DrawText(new Vector2(TextLeft, NameBaseline), Name, FontSheetName, NameGold);
            this.DrawText(new Vector2(TextLeft, ClassBaseline), ClassLine, FontSheetClass, Style.TextDim);
            this.DrawText(new Vector2(TextLeft, CreatedBaseline), Created, FontSheetCreated, Style.TextDim);

            float iconLeft = Size.X - 6f - FameIcon;
            HudIcons.Fame(this, new Rect2(iconLeft, FameIconTop, FameIcon, FameIcon), Style.FameFillHigh);

            this.DrawText(
                new Vector2(iconLeft - 7f - Style.Measure(Fame, FontSheetFame), NameBaseline),
                Fame, FontSheetFame, Style.FameFillHigh);
        }

        /// <summary>
        /// One cell: a plate outlined rather than filled, with its caption sitting in the outline.
        /// </summary>
        /// <remarks>
        /// The reference breaks the top edge for the caption instead of putting it above the plate,
        /// and stops every edge short of the corners, which is what makes a hard-edged rectangle
        /// read as a rounded one without a single curve being drawn.
        /// </remarks>
        private void DrawCell(in Cell cell, in Rect2 box)
        {
            if (string.IsNullOrEmpty(cell.Value))
                return;

            var edge = Style.StatCellEdge;

            DrawRect(new Rect2(box.Position.X, box.Position.Y + CellCorner,
                CellEdge, box.Size.Y - CellCorner * 2f), edge);

            DrawRect(new Rect2(box.End.X - CellEdge, box.Position.Y + CellCorner,
                CellEdge, box.Size.Y - CellCorner * 2f), edge);

            DrawRect(new Rect2(box.Position.X + CellCorner, box.End.Y - CellEdge,
                box.Size.X - CellCorner * 2f, CellEdge), edge);

            float gap = box.Size.X * 0.54f;
            float arm = (box.Size.X - CellCorner * 2f - gap) / 2f;

            DrawRect(new Rect2(box.Position.X + CellCorner, box.Position.Y, arm, CellEdge), edge);
            DrawRect(new Rect2(box.End.X - CellCorner - arm, box.Position.Y, arm, CellEdge), edge);

            float middle = box.Position.X + box.Size.X / 2f;

            this.DrawText(
                new Vector2(middle - Style.Measure(cell.Key, FontCellLabel) / 2f, box.Position.Y + 8f),
                cell.Key, FontCellLabel, Style.StatLabel);

            float value = Style.Measure(cell.Value, FontCellValue);
            float bonus = cell.Bonus == null ? 0f : Style.Measure(" " + cell.Bonus, FontCellValue);
            float at = middle - (value + bonus) / 2f;
            float baseline = box.Position.Y + 37f;

            this.DrawText(new Vector2(at, baseline), cell.Value, FontCellValue, Style.StatValue);

            if (cell.Bonus == null)
                return;

            this.DrawText(new Vector2(at + value, baseline), " " + cell.Bonus, FontCellValue,
                cell.Above ? Style.StatValueMax : Style.StatValue);
        }
    }

    /// <summary>The two pages, as the plates that choose between them.</summary>
    private sealed partial class TabStrip : Control
    {
        private readonly string[] _titles;

        private int _current;

        public TabStrip(string[] titles)
        {
            _titles = titles;
            MouseFilter = MouseFilterEnum.Stop;
            FocusMode = FocusModeEnum.None;
        }

        public event Action<int> Selected;

        public int Current
        {
            get => _current;
            set
            {
                if (_current == value)
                    return;

                _current = value;
                QueueRedraw();
            }
        }

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is not InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left } click)
                return;

            for (int i = 0; i < _titles.Length; i++)
            {
                if (!Tab(i).HasPoint(click.Position))
                    continue;

                AcceptEvent();
                if (i != _current)
                    Selected?.Invoke(i);

                return;
            }
        }

        private Rect2 Tab(int index)
        {
            float width = (Size.X - TabGap * (_titles.Length - 1)) / _titles.Length;
            return new Rect2(index * (width + TabGap), 0f, width, Size.Y);
        }

        public override void _Draw()
        {
            for (int i = 0; i < _titles.Length; i++)
            {
                var box = Tab(i);
                bool on = i == _current;

                var body = new Rect2(box.Position, new Vector2(box.Size.X, TabHeight));
                var foot = new Rect2(box.Position.X, TabHeight, box.Size.X, TabFoot);

                DrawRect(body, on ? TabActiveEdge : TabIdleEdge);
                DrawRect(new Rect2(body.Position.X + TabEdge, body.Position.Y + TabEdge,
                    body.Size.X - TabEdge * 2f, body.Size.Y - TabEdge), on ? Style.TabActive : TabIdleFill);

                DrawRect(foot, on ? TabActiveFootEdge : TabIdleFootEdge);
                DrawRect(new Rect2(foot.Position.X + TabEdge, foot.Position.Y,
                    foot.Size.X - TabEdge * 2f, foot.Size.Y), on ? TabActiveFoot : TabIdleFoot);

                this.DrawText(
                    new Vector2(
                        box.Position.X + box.Size.X / 2f - Style.Measure(_titles[i], FontTabLabel) / 2f,
                        TabBaseline),
                    _titles[i], FontTabLabel, on ? Style.TabActiveText : TabIdleText);
            }
        }
    }

    /// <summary>
    /// The scrolling list of tallies, drawn rather than built out of nodes.
    /// </summary>
    /// <remarks>
    /// Thirty rows of two labels each would be sixty nodes to lay out and free every time a page is
    /// switched. Drawing them costs one pass over the visible dozen and lets the last rows fade
    /// into the footer band the way the reference does.
    /// </remarks>
    private sealed partial class RowList : Control
    {
        private readonly List<Row> _all = new();
        private readonly float[] _scrollPerTab = new float[2];

        private int _tab;
        private float _scroll;
        private bool _dragging;
        private float _grabbedAt;

        /// <summary>What to say instead of rows, when there is nothing to say.</summary>
        public string Empty;

        private readonly struct Row
        {
            public readonly string Label;
            public readonly string Value;
            public readonly bool IsGroup;

            public Row(string label, string value, bool isGroup)
            {
                Label = label;
                Value = value;
                IsGroup = isGroup;
            }

            public float Height => IsGroup ? GroupHeight : RowPitch;
        }

        public RowList()
        {
            MouseFilter = MouseFilterEnum.Stop;
            FocusMode = FocusModeEnum.None;
            ClipContents = true;
        }

        /// <summary>Starts filling for a page, remembering where the last one was scrolled to.</summary>
        public void Begin(int tab)
        {
            if (tab != _tab)
            {
                _scrollPerTab[_tab] = _scroll;
                _tab = tab;
                _scroll = _scrollPerTab[tab];
            }

            _all.Clear();
        }

        public void Group(string title) => _all.Add(new Row(title, null, true));

        /// <summary>A tally. A null value is one the server does not publish.</summary>
        public void Add(string label, int? value) => _all.Add(new Row(
            label, value?.ToString("N0", CultureInfo.InvariantCulture) ?? "—", false));

        public void End()
        {
            _scroll = Mathf.Clamp(_scroll, 0f, MaxOffset);
            QueueRedraw();
        }

        /// <summary>Forgets everything, including where each page was. Used on a character change.</summary>
        public void Reset()
        {
            _all.Clear();
            _scroll = 0f;
            _scrollPerTab[0] = 0f;
            _scrollPerTab[1] = 0f;
            QueueRedraw();
        }

        private float Content
        {
            get
            {
                float total = ListPad;
                foreach (var row in _all)
                    total += row.Height;

                return total;
            }
        }

        private float MaxOffset => Mathf.Max(0f, Content - Size.Y);

        private Rect2 Thumb
        {
            get
            {
                float track = Size.Y;
                float height = Mathf.Max(24f, track * Size.Y / Mathf.Max(Content, Size.Y));
                float travel = track - height;
                float fraction = MaxOffset <= 0f ? 0f : Mathf.Clamp(_scroll / MaxOffset, 0f, 1f);

                return new Rect2(
                    Size.X - ScrollWidth - PlateInset, Mathf.Round(travel * fraction),
                    ScrollWidth, Mathf.Round(height));
            }
        }

        public override void _GuiInput(InputEvent @event)
        {
            switch (@event)
            {
                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelUp }:
                    Scroll(-3f);
                    return;

                case InputEventMouseButton { ButtonIndex: MouseButton.Left, Pressed: true } down:
                    if (Thumb.HasPoint(down.Position))
                    {
                        _dragging = true;
                        _grabbedAt = down.Position.Y - Thumb.Position.Y;
                    }

                    AcceptEvent();
                    return;

                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelDown }:
                    Scroll(3f);
                    return;

                case InputEventMouseButton { ButtonIndex: MouseButton.Left }:
                    _dragging = false;
                    return;

                case InputEventMouseMotion motion when _dragging:
                    float travel = Size.Y - Thumb.Size.Y;
                    _scroll = travel <= 0f
                        ? 0f
                        : Mathf.Clamp((motion.Position.Y - _grabbedAt) / travel, 0f, 1f) * MaxOffset;

                    QueueRedraw();
                    AcceptEvent();
                    return;
            }
        }

        private void Scroll(float rows)
        {
            _scroll = Mathf.Clamp(_scroll + rows * RowPitch, 0f, MaxOffset);
            QueueRedraw();
        }

        public override void _Draw()
        {
            DrawRect(new Rect2(Vector2.Zero, Size), Style.ModalBody);

            if (Empty != null)
            {
                this.DrawText(
                    new Vector2(Size.X / 2f - Style.Measure(Empty, FontRowLabel) / 2f, ListPad + 40f),
                    Empty, FontRowLabel, Style.TextDim);

                return;
            }

            float plate = Size.X - PlateInset * 2f - ScrollWidth - PlateInset;
            float y = ListPad - _scroll;

            foreach (var row in _all)
            {
                if (y > Size.Y)
                    break;

                if (y + row.Height >= 0f)
                    DrawRow(row, y, plate);

                y += row.Height;
            }

            Fade();
            DrawThumb();
        }

        private void DrawRow(in Row row, float y, float plate)
        {
            if (row.IsGroup)
            {
                this.DrawText(new Vector2(PlateInset + 4f, y + GroupBaseline), row.Label,
                    FontGroup, Style.Text);

                return;
            }

            DrawRect(new Rect2(PlateInset, y, plate, RowPlate), Style.ModalTrough);

            float baseline = y + RowBaseline;
            this.DrawText(new Vector2(PlateInset + RowInset, baseline), row.Label, FontRowLabel, RowLabel);

            this.DrawText(
                new Vector2(PlateInset + plate - RowInset - Style.Measure(row.Value, FontRowValue), baseline),
                row.Value, FontRowValue, row.Value == "—" ? Style.TextDim : Style.StatNumber);
        }

        /// <summary>The last rows dissolving into the footer, which is how the reference ends.</summary>
        private void Fade()
        {
            for (float i = 0f; i < ListFade; i++)
            {
                DrawRect(new Rect2(0f, Size.Y - ListFade + i, Size.X, 1f),
                    Style.ModalBody with { A = i / ListFade });
            }
        }

        private void DrawThumb()
        {
            if (MaxOffset <= 0f)
                return;

            DrawRect(Thumb, ScrollThumb);
        }
    }

    /// <summary>The band pinned to the bottom: what this character is worth if it dies now.</summary>
    private sealed partial class Footer : Control
    {
        public string Value = "0";

        public Footer() => MouseFilter = MouseFilterEnum.Ignore;

        public override void _Draw()
        {
            DrawRect(new Rect2(Vector2.Zero, Size), Style.ModalBand);

            this.DrawText(new Vector2(Margin + 12f, FooterBaseline), "Fame on Death",
                FontFooterLabel, Style.Text);

            float iconLeft = Size.X - 8f - FooterIcon;
            HudIcons.Fame(this,
                new Rect2(iconLeft, (Size.Y - FooterIcon) / 2f, FooterIcon, FooterIcon),
                Style.FameFillHigh);

            this.DrawText(
                new Vector2(iconLeft - 7f - Style.Measure(Value, FontFooterValue), FooterBaseline),
                Value, FontFooterValue, Style.FameFillHigh);
        }
    }
}
