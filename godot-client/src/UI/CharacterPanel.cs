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
/// The character sheet: attributes, tallies and dungeon counts, docked to the right of the world.
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
/// The rows are built once and mutated afterwards. A statistics list rebuilt on every refresh would
/// allocate a few hundred nodes a second and lose the scroll position twice a second with it.
/// </para>
/// </remarks>
public partial class CharacterPanel : Control
{
    private const float Inset = 12f;
    private const float IdentityHeight = 96f;
    private const float PortraitSize = 64f;
    private const float AttributeRowHeight = 48f;
    private const float SectionHeight = 28f;
    private const float RowHeight = 26f;
    private const float FooterHeight = 40f;

    /// <summary>The right-hand column every value is right-aligned into.</summary>
    private const float ValueColumn = 120f;

    /// <summary>Twice a second, which is as often as any of these numbers is worth reading.</summary>
    private const double RefreshSeconds = 0.5;

    private readonly List<AttributeCell> _attributes = new();

    private ModalPanel _shell;
    private Portrait _portrait;
    private Label _name;
    private Label _classLine;
    private Label _created;
    private Label _fame;
    private Control _grid;
    private RowList _rows;
    private Label _footerValue;

    private GameData _data;
    private TextureResolver _textures;
    private AppEngineClient _accounts;
    private string _guid;
    private string _password;
    private int _characterId;

    private CharacterStats _stats;
    private DateTime? _createdAt;
    private double _sinceRefresh;
    private bool _fetching;

    /// <summary>Whether the panel is open, so the caller can keep its own button in step.</summary>
    public bool IsOpen => _shell is { Visible: true };

    /// <summary>Raised when the panel closes by any route, including Escape and its own button.</summary>
    public event Action Closed;

    public void Configure(GameData data, AssetLibrary assets)
    {
        _data = data;
        _textures = new TextureResolver(assets);
    }

    /// <summary>Points the panel at the app server, which is the only source of the tallies.</summary>
    public void Connect(string appServerUrl, string guid, string password, int characterId)
    {
        _accounts = new AppEngineClient(appServerUrl);
        _guid = guid;
        _password = password;
        _characterId = characterId;

        // A different character has different everything, including where its list was scrolled.
        _stats = null;
        _createdAt = null;
        _rows?.Reset();
    }

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;

        _shell = new ModalPanel("Attributes");
        _shell.Closed += () => Closed?.Invoke();
        AddChild(_shell);

        BuildIdentity();
        BuildGrid();
        BuildTabs();
        BuildRows();
        BuildFooter();

        Resized += Reflow;
        if (GetParent() is HudLayer layer)
            layer.Reflowed += Reflow;

        Reflow();
    }

    private void BuildIdentity()
    {
        _portrait = new Portrait();
        _shell.Body.AddChild(_portrait);

        _name = new Label
        {
            ClipText = true,
            TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis,
        }.Typeset(Style.FontName, Style.Text);
        _shell.Body.AddChild(_name);

        _classLine = new Label
        {
            ClipText = true,
            TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis,
        }.Typeset(Style.FontSmall, Style.TextDim);
        _shell.Body.AddChild(_classLine);

        _created = new Label().Typeset(Style.FontTag, Style.TextDim);
        _shell.Body.AddChild(_created);

        _fame = new Label { HorizontalAlignment = HorizontalAlignment.Right }.Typeset(Style.FontName, Style.Text);
        _shell.Body.AddChild(_fame);

        _fameIcon = new HudGlyph(HudIcons.Fame, Style.IconFame);
        _shell.Body.AddChild(_fameIcon);
    }

    private HudGlyph _fameIcon;

    private void BuildGrid()
    {
        _grid = new Control { MouseFilter = MouseFilterEnum.Ignore };
        _shell.Body.AddChild(_grid);
    }

    private void BuildTabs()
    {
        // No tabs. There were two pages and the second listed a dungeon completion count per
        // dungeon, which is a table nobody opened this panel to read -- the panel is for the
        // attributes at the top of it, and the tallies below are what you glance at afterwards.
    }

    private void BuildRows()
    {
        _rows = new RowList();
        _shell.Body.AddChild(_rows);
    }

    private void BuildFooter()
    {
        _band = new FooterBand();
        _shell.Body.AddChild(_band);

        _footerLabel = new Label
        {
            Text = "Fame on Death",
            VerticalAlignment = VerticalAlignment.Center,
        }.Typeset(Style.FontSmall, Style.Text);
        _shell.Body.AddChild(_footerLabel);

        _footerValue = new Label
        {
            HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Center,
        }.Typeset(Style.FontSmall, Style.StatValueMax);
        _shell.Body.AddChild(_footerValue);

        _footerIcon = new HudGlyph(HudIcons.Fame, Style.StatValueMax);
        _shell.Body.AddChild(_footerIcon);
    }

    private Label _footerLabel;
    private HudGlyph _footerIcon;
    private FooterBand _band;

    /// <summary>
    /// The rule and band behind the pinned footer.
    /// </summary>
    /// <remarks>
    /// A child of the shell rather than something the panel drew on its own canvas. Drawn outside
    /// it, it stayed on screen after the panel closed -- nothing was queueing the panel a redraw
    /// when its shell was hidden, so the last band it painted sat over the world until something
    /// else happened to invalidate it.
    /// </remarks>
    private sealed partial class FooterBand : Control
    {
        public FooterBand() => MouseFilter = MouseFilterEnum.Ignore;

        public override void _Draw()
        {
            DrawRect(new Rect2(0f, 0f, Size.X, 1f), Style.ModalFrameDark);
            DrawRect(new Rect2(0f, 1f, Size.X, Size.Y - 1f), Style.ModalBand);
        }
    }

    /// <summary>
    /// Places the panel and everything in it.
    /// </summary>
    /// <remarks>
    /// The attribute grid's column width is derived from the panel's interior rather than written
    /// down: the brief's 228 plus a 12 gutter plus 12 insets comes to 492 in a 480-wide panel, and
    /// the interior is what the cells actually have to fit inside.
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

        _portrait.Position = new Vector2(Inset, Inset);
        _portrait.Size = new Vector2(PortraitSize, PortraitSize);

        float textLeft = Inset + PortraitSize + Inset;
        float fameWidth = 130f;

        _name.Position = new Vector2(textLeft, 14f);
        _name.Size = new Vector2(width - textLeft - fameWidth - Inset, 22f);

        _classLine.Position = new Vector2(textLeft, 40f);
        _classLine.Size = new Vector2(width - textLeft - fameWidth - Inset, 18f);

        _created.Position = new Vector2(textLeft, 60f);
        _created.Size = new Vector2(width - textLeft - fameWidth - Inset, 16f);

        _fameIcon.Position = new Vector2(width - Inset - 18f, IdentityHeight / 2f - 9f);
        _fameIcon.Size = new Vector2(18f, 18f);
        _fame.Position = new Vector2(width - Inset - fameWidth, IdentityHeight / 2f - 12f);
        _fame.Size = new Vector2(fameWidth - 24f, 24f);

        _grid.Position = new Vector2(0f, IdentityHeight);
        _grid.Size = new Vector2(width, GridHeight());
        LayoutGrid();

        float rowsTop = _grid.Position.Y + _grid.Size.Y;
        float rowsHeight = Mathf.Max(0f, body.Size.Y - rowsTop - FooterHeight);

        _rows.Position = new Vector2(0f, rowsTop);
        _rows.Size = new Vector2(width, rowsHeight);

        float footerTop = body.Size.Y - FooterHeight;

        _band.Position = new Vector2(0f, footerTop);
        _band.Size = new Vector2(width, FooterHeight);

        _footerLabel.Position = new Vector2(Inset, footerTop);
        _footerLabel.Size = new Vector2(width / 2f, FooterHeight);

        _footerIcon.Position = new Vector2(width - Inset - 16f, footerTop + FooterHeight / 2f - 8f);
        _footerIcon.Size = new Vector2(16f, 16f);
        _footerValue.Position = new Vector2(width - Inset - ValueColumn - 20f, footerTop);
        _footerValue.Size = new Vector2(ValueColumn, FooterHeight);

    }

    /// <summary>Two cells to a row, however many attributes there turn out to be.</summary>
    private float GridHeight() =>
        Mathf.Ceil(Mathf.Max(_attributes.Count, 1) / 2f) * AttributeRowHeight + Inset;

    private void LayoutGrid()
    {
        float column = (_grid.Size.X - Inset * 2f - Inset) / 2f;

        for (int i = 0; i < _attributes.Count; i++)
        {
            _attributes[i].Position = new Vector2(
                Inset + i % 2 * (column + Inset), i / 2 * AttributeRowHeight);

            _attributes[i].Size = new Vector2(column, AttributeRowHeight);
        }
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
                FillRows();
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
        RefreshIdentity(player);
        RefreshAttributes(player);
    }

    private void RefreshIdentity(LocalPlayer player)
    {
        var desc = _data?.GetObject(player.ObjectType);

        _portrait.Set(ClassPortrait(player.ObjectType), Style.SlotBorder);
        Write(_name, string.IsNullOrEmpty(player.Name) ? "—" : player.Name);

        string className = desc?.DisplayId ?? desc?.Id ?? "Adventurer";
        Write(_classLine, $"Level {player.Level}, {className}");

        // Formatted in whatever the machine's locale is, from a round-trip timestamp. A server that
        // does not send one leaves the line out rather than showing a guess.
        Write(_created, _createdAt.HasValue
            ? $"Created on {_createdAt.Value.ToLocalTime():D}"
            : string.Empty);

        Write(_fame, player.Fame.ToString("N0", CultureInfo.InvariantCulture));

        // Fame on death is what is banked plus whatever the bonuses come to, and the bonuses are
        // computed server-side at the moment of death -- /char/fame refuses a living character. So
        // this is the floor, and it is labelled as the total the character currently carries.
        Write(_footerValue, player.Fame.ToString("N0", CultureInfo.InvariantCulture));
    }

    private static void Write(Label label, string text)
    {
        if (label.Text != text)
            label.Text = text;
    }

    /// <summary>
    /// The six attributes, or however many the class turns out to have.
    /// </summary>
    /// <remarks>
    /// Built from the array the first time and mutated after. The cells are what carry the rule
    /// that matters here: gold once the unboosted value has reached the class ceiling, and no
    /// parenthetical at all when equipment is adding nothing.
    /// </remarks>
    private void RefreshAttributes(LocalPlayer player)
    {
        var maxima = _data?.GetObject(player.ObjectType)?.StatMaxima;

        int[] values = { player.Attack, player.Defense, player.Speed, player.Dexterity, player.Vitality, player.Wisdom };

        if (_attributes.Count != values.Length)
        {
            foreach (var cell in _attributes)
                cell.QueueFree();

            _attributes.Clear();

            for (int i = 0; i < values.Length; i++)
            {
                var cell = new AttributeCell(Keys[i]);
                _grid.AddChild(cell);
                _attributes.Add(cell);
            }

            Reflow();
        }

        for (int i = 0; i < _attributes.Count; i++)
        {
            // The boosts and maxima are indexed with MaxHP and MaxMP first, so they run two ahead
            // of the six shown here.
            int at = i + 2;
            int bonus = player.Boosts[at];
            int max = maxima != null && at < maxima.Length ? maxima[at] : 0;

            _attributes[i].Set(values[i], bonus, max);
        }
    }

    private static readonly string[] Keys = { "ATT", "DEF", "SPD", "DEX", "VIT", "WIS" };

    /// <summary>One attribute: its name, its value, and what equipment is adding to it.</summary>
    private sealed partial class AttributeCell : Control
    {
        private readonly Label _label;
        private readonly Label _value;
        private readonly Label _bonus;

        public AttributeCell(string key)
        {
            MouseFilter = MouseFilterEnum.Ignore;

            _label = new Label { Text = key, Position = new Vector2(0f, 2f), Size = new Vector2(120f, 14f) }
                .Typeset(Style.FontTag, Style.StatLabel);
            AddChild(_label);

            _value = new Label { Position = new Vector2(0f, 18f), Size = new Vector2(80f, 26f) }
                .Typeset(Style.FontTitle, Style.StatValue);
            AddChild(_value);

            _bonus = new Label { VerticalAlignment = VerticalAlignment.Bottom }
                .Typeset(Style.FontSmall, Style.StatBonus);
            AddChild(_bonus);
        }

        /// <param name="value">The total, which already includes the bonus.</param>
        /// <param name="bonus">How much of the total comes from equipment.</param>
        /// <param name="maximum">The class ceiling, or zero if this build does not know one.</param>
        public void Set(int value, int bonus, int maximum)
        {
            string text = value.ToString(CultureInfo.InvariantCulture);
            if (_value.Text != text)
            {
                _value.Text = text;

                float width = Style.Measure(text, Style.FontTitle) + 6f;
                _bonus.Position = new Vector2(width, 22f);
                _bonus.Size = new Vector2(Mathf.Max(0f, Size.X - width), 20f);
            }

            // Against the ceiling it is the unboosted part that counts: equipment does not stop a
            // potion working, so a stat that only reaches its maximum while a ring is on has not
            // been maxed. An unknown ceiling is never gold -- a guess here would be a lie in the
            // one colour the panel exists to show.
            bool maxed = maximum > 0 && value - bonus >= maximum;
            _value.AddThemeColorOverride("font_color", maxed ? Style.StatValueMax : Style.StatValue);

            // Nothing at all at zero. "(+0)" is noise in six cells at once.
            string suffix = bonus == 0 ? string.Empty
                : bonus > 0 ? $"(+{bonus})"
                : $"(−{-bonus})";

            if (_bonus.Text == suffix)
                return;

            _bonus.Text = suffix;
            _bonus.AddThemeColorOverride("font_color", bonus < 0 ? Style.StatPenalty : Style.StatBonus);
        }
    }

    private Assets.Sprite ClassPortrait(ushort objectType)
    {
        var resolved = _textures?.Resolve(_data?.GetObject(objectType)?.Texture) ?? default;

        if (resolved.Animated != null)
            return resolved.Animated.Frame(0f, 0f, CharAction.Stand, 0f).Sprite;

        return resolved.Still;
    }

    /// <summary>
    /// Fills the list for whichever tab is showing.
    /// </summary>
    /// <remarks>
    /// Both tabs are the same row component over the same blob: the server numbers the dungeon
    /// tallies in two contiguous runs inside the statistics, which is what lets one fetch answer
    /// both. Rows come out in the order the server wrote them and are never listed here.
    /// </remarks>
    private void FillRows()
    {
        if (_rows == null)
            return;

        _rows.Begin(0);

        if (_stats == null)
            return;

        foreach (var entry in _stats.Statistics)
            _rows.Add(CharacterStats.Label(entry.Id), entry.Value, dim: false, group: null);

        _rows.End();
    }

    /// <summary>
    /// The scrolling list of rows, drawn rather than built out of nodes.
    /// </summary>
    /// <remarks>
    /// Two hundred rows of two labels each would be four hundred nodes to lay out and free every
    /// time a tab is switched. Drawing them costs one pass over the visible dozen and lets the
    /// group headers stick to the top of the list while the rows run under them.
    /// </remarks>
    private sealed partial class RowList : Control
    {
        private readonly List<Row> _all = new();
        private readonly float[] _scrollPerTab = new float[2];

        private int _tab;
        private float _scroll;
        private bool _dragging;
        private float _grabbedAt;

        private readonly struct Row
        {
            public readonly string Label;
            public readonly string Value;
            public readonly bool Dim;
            public readonly bool IsGroup;

            public Row(string label, string value, bool dim, bool isGroup)
            {
                Label = label;
                Value = value;
                Dim = dim;
                IsGroup = isGroup;
            }
        }

        public RowList()
        {
            MouseFilter = MouseFilterEnum.Stop;
            FocusMode = FocusModeEnum.None;
            ClipContents = true;
        }

        /// <summary>Starts filling for a tab, remembering where the last one was scrolled to.</summary>
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

        public void Group(string title) => _all.Add(new Row(title, null, false, true));

        public void Add(string label, int value, bool dim, string group) =>
            _all.Add(new Row(label, value.ToString("N0", CultureInfo.InvariantCulture), dim, false));

        public void End()
        {
            _scroll = Mathf.Clamp(_scroll, 0f, HudScrollbar.MaxOffset(Size, Content));
            QueueRedraw();
        }

        /// <summary>Forgets everything, including where each tab was. Used on a character change.</summary>
        public void Reset()
        {
            _all.Clear();
            _scroll = 0f;
            _scrollPerTab[0] = 0f;
            _scrollPerTab[1] = 0f;
            QueueRedraw();
        }

        private float Content => _all.Count * RowHeight;

        public override void _GuiInput(InputEvent @event)
        {
            switch (@event)
            {
                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelUp }:
                    Scroll(-3f);
                    return;

                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelDown }:
                    Scroll(3f);
                    return;

                case InputEventMouseButton { ButtonIndex: MouseButton.Left } button:
                    OnClick(button);
                    AcceptEvent();
                    return;

                case InputEventMouseMotion motion when _dragging:
                    _scroll = HudScrollbar.OffsetForThumbTop(Size, Content, motion.Position.Y - _grabbedAt);
                    QueueRedraw();
                    AcceptEvent();
                    return;
            }
        }

        private void OnClick(InputEventMouseButton button)
        {
            if (!button.Pressed)
            {
                _dragging = false;
                return;
            }

            switch (HudScrollbar.Test(Size, _scroll, Content, button.Position))
            {
                case HudScrollbar.Part.Up:
                    Scroll(-1f);
                    return;

                case HudScrollbar.Part.Down:
                    Scroll(1f);
                    return;

                case HudScrollbar.Part.Thumb:
                    _dragging = true;
                    _grabbedAt = button.Position.Y - HudScrollbar.Thumb(Size, _scroll, Content).Position.Y;
                    return;

                case HudScrollbar.Part.TrackAbove:
                    Scroll(-5f);
                    return;

                case HudScrollbar.Part.TrackBelow:
                    Scroll(5f);
                    return;
            }
        }

        private void Scroll(float rows)
        {
            _scroll = Mathf.Clamp(_scroll + rows * RowHeight, 0f, HudScrollbar.MaxOffset(Size, Content));
            QueueRedraw();
        }

        public override void _Draw()
        {
            float width = Size.X - HudScrollbar.Width - 2f;
            int first = Mathf.Max(0, (int)(_scroll / RowHeight));

            string sticky = null;
            for (int i = 0; i <= first && i < _all.Count; i++)
            {
                if (_all[i].IsGroup)
                    sticky = _all[i].Label;
            }

            for (int i = first; i < _all.Count; i++)
            {
                float y = i * RowHeight - _scroll;
                if (y > Size.Y)
                    break;

                DrawRow(_all[i], y, width, i);
            }

            // The group a scrolled-past header belongs to, pinned at the top so a count is never
            // read under the wrong tier.
            if (sticky != null && first < _all.Count && !_all[first].IsGroup)
                DrawGroup(sticky, 0f, width);

            HudScrollbar.Draw(this, Size, _scroll, Content, _dragging);
        }

        private void DrawRow(in Row row, float y, float width, int index)
        {
            if (row.IsGroup)
            {
                DrawGroup(row.Label, y, width);
                return;
            }

            if (index % 2 == 1)
                DrawRect(new Rect2(0f, y, width, RowHeight), Style.ModalStripe);

            float baseline = Mathf.Round(y + (RowHeight + Style.Sans.GetAscent(Style.FontSmall) - Style.Sans.GetDescent(Style.FontSmall)) / 2f);

            // A fixed column for the value and an ellipsis on the label: a seven-digit number must
            // never be pushed off the row by a long name.
            float labelWidth = width - Inset * 2f - ValueColumn;
            this.DrawText(new Vector2(Inset, baseline), Truncate(row.Label, labelWidth), Style.FontSmall,
                row.Dim ? Style.TextDim : Style.Text);

            this.DrawText(
                new Vector2(width - Inset - Style.Measure(row.Value, Style.FontSmall), baseline), row.Value, Style.FontSmall,
                row.Dim ? Style.TextDim : Style.StatNumber);
        }

        private void DrawGroup(string title, float y, float width)
        {
            DrawRect(new Rect2(0f, y, width, SectionHeight), Style.ModalHeader);
            DrawRect(new Rect2(0f, y + SectionHeight - 1f, width, 1f), Style.ModalFrameDark);

            float baseline = Mathf.Round(y + (SectionHeight + Style.Sans.GetAscent(Style.FontHeader) - Style.Sans.GetDescent(Style.FontHeader)) / 2f);
            this.DrawText(new Vector2(Inset, baseline), title, Style.FontHeader, Style.TextDim);
        }

        /// <summary>Cuts a label to fit its column, with an ellipsis. Never wraps.</summary>
        private static string Truncate(string text, float width)
        {
            if (Style.Measure(text, Style.FontSmall) <= width)
                return text;

            for (int length = text.Length - 1; length > 1; length--)
            {
                string cut = text[..length] + "…";
                if (Style.Measure(cut, Style.FontSmall) <= width)
                    return cut;
            }

            return "…";
        }
    }
}
