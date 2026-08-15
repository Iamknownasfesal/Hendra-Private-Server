using System;
using System.Collections.Generic;
using Godot;
using Hendra.Net.Packets;

namespace Hendra.UI;

/// <summary>
/// The chat log and its input line, in the bottom left corner.
/// </summary>
/// <remarks>
/// <para>
/// The log is drawn rather than assembled out of a RichTextLabel. Three things in the brief are
/// awkward or impossible through one: a rank icon at the head of every line, a hanging indent that
/// puts wrapped text under the message rather than under the name, and a scrollbar with arrow
/// buttons at its ends. Drawing it also settles the escaping question by construction -- there is no
/// markup parser between a player's words and the screen, so there is nothing for a player to type
/// that the log will interpret.
/// </para>
/// <para>
/// Wrapping is done once, when a line arrives, and kept. A busy world delivers dozens of lines a
/// second and re-measuring two hundred of them per frame would cost more than everything else the
/// interface does put together; laying out only the new line costs nothing and the draw is then a
/// walk over the visible rows with no allocation in it.
/// </para>
/// <para>
/// The input is hidden until summoned and gives the keyboard back on submit or Escape, because the
/// movement keys are letters -- leaving a focused text field around would silently eat them.
/// </para>
/// </remarks>
public partial class ChatView : Control
{
    /// <summary>The brief's buffer. The original kept 150; the oldest is dropped past this.</summary>
    private const int MaxLines = 200;

    private const float Pad = 8f;

    /// <summary>The row along the bottom of the panel that carries the bubble and the input.</summary>
    private const float InputRow = 34f;

    private const float ScrollbarWidth = 14f;

    /// <summary>
    /// The names the original reserves for messages it speaks in its own voice.
    /// </summary>
    /// <remarks>
    /// From <c>Parameters.as:24-28</c>. The server picks one of these as the Text packet's name and
    /// the client reads the message's kind back off it: there is no separate field saying whether a
    /// line is an error, a hint or somebody talking.
    /// </remarks>
    private const string ServerName = "";

    private const string ClientName = "*Client*";
    private const string ErrorName = "*Error*";
    private const string HelpName = "*Help*";
    private const string GuildName = "*Guild*";

    private HudPanel _panel;
    private ChatLog _log;
    private LineEdit _input;
    private HudIconButton _bubble;

    private Rect2 _open;

    /// <summary>Raised when the player submits a line. Empty lines never reach here.</summary>
    public event Action<string> Submitted;

    /// <summary>Whether the input has focus, and movement keys should be ignored.</summary>
    public bool IsTyping => _input is { Visible: true } && _input.HasFocus();

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;

        _panel = new HudPanel(Style.Panel);
        AddChild(_panel);

        _log = new ChatLog();
        _panel.AddChild(_log);

        _bubble = new HudIconButton(HudIcons.SpeechBubble, "Say something [Enter]", inset: 3f);
        _bubble.Tint = Style.TextDim;
        _bubble.Pressed += () => BeginTyping();
        _panel.AddChild(_bubble);

        _input = new LineEdit { Visible = false, PlaceholderText = $"[{ChatKey()}] to chat" };
        _input.TextSubmitted += OnSubmitted;
        _panel.AddChild(_input);

        Resized += Reflow;
        if (GetParent() is HudLayer layer)
            layer.Reflowed += Reflow;

        Reflow();
    }

    private void Reflow()
    {
        if (_panel == null)
            return;

        var layout = new HudLayout(Size.X > 0f && Size.Y > 0f
            ? Size
            : new Vector2(HudLayout.ReferenceWidth, HudLayout.ReferenceHeight));

        _open = layout.Chat;
        Apply();
    }

    /// <summary>
    /// Whether the log is worth a background: it has something in it, or it is being written to.
    /// </summary>
    private bool ShowsPlate => _log.LineCount > 0 || IsTyping;

    /// <summary>What <see cref="ShowsPlate"/> last answered, so the plate is only re-applied on a change.</summary>
    private bool _plated;

    /// <summary>Lays the panel out.</summary>
    private void Apply()
    {
        _plated = ShowsPlate;

        _panel.Position = _open.Position;
        _panel.Size = _open.Size;

        // The panel is the biggest opaque thing on the screen and it sits over the corner the
        // player walks into. Letting a little of the world through it is the difference between a
        // log and a wall -- and with nothing to say it shows no plate at all, which is what the
        // original does: its chat is bare glyph bitmaps with a black glow baked in
        // (ChatListItemFactory.as:305-312) over no background whatsoever, so an empty log is an
        // empty corner rather than a grey slab.
        _panel.Background = ShowsPlate ? Style.Panel with { A = 0.78f } : Colors.Transparent;
        _panel.Edged = ShowsPlate;

        _log.Position = new Vector2(Pad, 6f);
        _log.Size = new Vector2(_open.Size.X - Pad * 2f, _open.Size.Y - 6f - InputRow);

        _bubble.Position = new Vector2(Pad, _open.Size.Y - InputRow + 2f);
        _bubble.Size = new Vector2(26f, 26f);

        _input.Position = new Vector2(Pad + 32f, _open.Size.Y - InputRow);
        _input.Size = new Vector2(_open.Size.X - Pad * 2f - 32f, 28f);
    }

    /// <summary>
    /// Retires lines that have been on screen their allotted time.
    /// </summary>
    /// <remarks>
    /// The original sweeps the log on a one-second timer and drops whatever has been up for twenty
    /// seconds (<c>ChatList.as:9,34-56</c> against <c>ChatListItem.as:15,69-70</c>). It is the only
    /// thing that ever takes a line off the screen: the panel itself stays where it is, saying
    /// nothing, rather than folding away and taking the unread lines with it.
    /// </remarks>
    public override void _Process(double delta)
    {
        // The plate follows the log's contents, so it appears with the first line and goes again
        // when the last one is retired.
        if (_plated != ShowsPlate)
            Apply();

        _sinceSweep += delta;
        if (_sinceSweep < 1.0)
            return;

        _sinceSweep = 0.0;
        _log.Retire(IsTyping);
    }

    /// <summary>How long since the log was last swept, so it is swept on the original's cadence.</summary>
    private double _sinceSweep;

    /// <summary>
    /// Shows the input and takes the keyboard.
    /// </summary>
    /// <param name="prefix">
    /// Text to start with, and to leave the caret after. The original opens the box already
    /// carrying a slash, a whisper or a guild prefix depending on which key was pressed.
    /// </param>
    public void BeginTyping(string prefix = null)
    {
        _input.Visible = true;
        _input.Text = prefix ?? string.Empty;
        _input.GrabFocus();
        _input.CaretColumn = _input.Text.Length;
    }

    /// <summary>Hides the input and gives the keyboard back to the game.</summary>
    public void EndTyping()
    {
        _input.Text = string.Empty;
        _input.Visible = false;
        _input.ReleaseFocus();
    }

    /// <summary>The key that opens the input, as it is currently bound.</summary>
    private static string ChatKey()
    {
        foreach (var bound in InputMap.ActionGetEvents("toggle_chat"))
        {
            if (bound is InputEventKey key)
                return OS.GetKeycodeString(key.PhysicalKeycode != Key.None ? key.PhysicalKeycode : key.Keycode);
        }

        return "Enter";
    }

    /// <summary>Scrolls the log by whole rows. Negative goes back through the history.</summary>
    public void Scroll(int rows) => _log?.Scroll(rows);

    private void OnSubmitted(string text)
    {
        string trimmed = text?.Trim();
        EndTyping();

        // An empty line would be worse than useless: the server tests the first character for a
        // leading slash before checking the length, so it throws on an empty string and drops the
        // connection.
        if (string.IsNullOrEmpty(trimmed))
            return;

        Submitted?.Invoke(trimmed);
    }

    /// <summary>Appends a line from the server.</summary>
    public void Add(TextPacket text, string displayText)
    {
        string name = text.Name ?? string.Empty;
        string recipient = text.Recipient ?? string.Empty;

        var rank = Fame.Colour(text.NumStars, 14, text.Admin > 0);

        _log.Add(
            IsSpokenByTheServer(name) ? null : Displayed(name),
            displayText,
            ColourOf(text.NameColor, NameColour(name, recipient)),
            ColourOf(text.TextColor, BodyColour(name, recipient)),
            rank);
    }

    /// <summary>
    /// A name as it is written at the head of a line.
    /// </summary>
    /// <remarks>
    /// The marker that chose the colour is not part of the name and is dropped before it is shown,
    /// as <c>processName()</c> drops it (<c>ChatListItemFactory.as:170-176</c>): a spawn announced
    /// by the server arrives as <c>#Adventurer</c> and belongs on screen as <c>&lt;Adventurer&gt;</c>
    /// in the orange that the hash asked for.
    /// </remarks>
    private static string Displayed(string name) =>
        name.StartsWith('#') || name.StartsWith('@') ? name[1..] : name;

    /// <summary>
    /// Appends a line the client wrote itself.
    /// </summary>
    /// <param name="error">
    /// Whether this is a refusal rather than a notice, which the original sends under a different
    /// name and therefore shows in a different colour (<c>Player.Chat.cs:110</c> against <c>:132</c>).
    /// </param>
    public void AddSystem(string message, bool error = false) =>
        _log.Add(null, message, Style.ChatName,
            BodyColour(error ? ErrorName : ServerName, string.Empty), Style.TextDim);

    /// <summary>
    /// Whether the name is one the server speaks under rather than a player's, in which case no
    /// <c>&lt;name&gt;</c> is drawn at the head of the line.
    /// </summary>
    /// <remarks><c>ChatListItemFactory.as:159-162</c>.</remarks>
    private static bool IsSpokenByTheServer(string name) =>
        name is ServerName or ClientName or HelpName or ErrorName or GuildName;

    /// <summary>
    /// What colour the message body is, from who sent it and to whom.
    /// </summary>
    /// <remarks>
    /// <c>ChatListItemFactory.as:277-304</c>, in the same order: the kinds the server speaks under
    /// win over the kinds a recipient implies, so a guild announcement is an announcement rather
    /// than guild chat.
    /// </remarks>
    private static Color BodyColour(string name, string recipient) => name switch
    {
        ServerName => Yellow,
        ClientName => Blue,
        HelpName => Orange,
        ErrorName => Red,
        _ when name.StartsWith('@') => Yellow,
        _ when recipient == GuildName => GuildGreen,
        _ when recipient.Length > 0 => Cyan,
        _ => Colors.White,
    };

    /// <summary>
    /// What colour the <c>&lt;name&gt;</c> at the head of the line is.
    /// </summary>
    /// <remarks>
    /// <c>ChatListItemFactory.as:258-275</c>. Chosen separately from the body, so the usual line of
    /// public chat is a green name in front of white words.
    /// </remarks>
    private static Color NameColour(string name, string recipient) => name switch
    {
        _ when name.StartsWith('#') => NpcOrange,
        _ when name.StartsWith('@') => Yellow,
        _ when recipient == GuildName => GuildGreen,
        _ when recipient.Length > 0 => Cyan,
        _ => PlayerGreen,
    };

    // The original's chat palette, as literals rather than as anything derived: these are the
    // numbers in ChatListItemFactory and they are not related to the rest of this interface's.
    private static readonly Color Yellow = new("ffff00");
    private static readonly Color Blue = new("0000ff");
    private static readonly Color Orange = new("ff5b05");
    private static readonly Color Red = new("ff0000");
    private static readonly Color GuildGreen = new("a6ff5d");
    private static readonly Color Cyan = new("00f0ff");
    private static readonly Color NpcOrange = new("ffa800");
    private static readonly Color PlayerGreen = new("00ff00");

    /// <summary>
    /// Unpacks a 24-bit colour, honouring the sentinel the server uses for "not specified" -- a
    /// value no one would pick deliberately, which is presumably why it was chosen.
    /// </summary>
    /// <remarks>
    /// Zero counts as unspecified too, which the original does not do. This fork's wire format
    /// carries no colours at all, so every line arrives with both fields at their default and a
    /// literal reading of the sentinel paints the whole log black on a black panel.
    /// </remarks>
    private static Color ColourOf(int packed, Color fallback)
    {
        const int Unset = 0x123456;
        if (packed == Unset || packed == 0)
            return fallback;

        return new Color(
            (packed >> 16 & 0xFF) / 255f,
            (packed >> 8 & 0xFF) / 255f,
            (packed & 0xFF) / 255f);
    }

    /// <summary>
    /// The log itself: the lines, their wrapping, the scrollbar and everything that scrolls them.
    /// </summary>
    private sealed partial class ChatLog : Control
    {
        /// <summary>The rank icon at the head of each line.</summary>
        private const float RankSize = 14f;

        private const float RankGap = 6f;

        /// <summary>The scrollbar's arrow buttons, at the ends of the track.</summary>
        private const float ArrowHeight = 12f;

        private readonly List<Line> _lines = new(MaxLines);
        private readonly List<Row> _rows = new(MaxLines * 2);

        /// <summary>Reused by the wrapper, which runs when a line arrives rather than per frame.</summary>
        private readonly System.Text.StringBuilder _builder = new(160);

        private float _scroll;
        private bool _atBottom = true;
        private bool _unread;
        private bool _draggingThumb;
        private float _dragOffset;

        /// <summary>The width the rows were wrapped at, so a resize can rebuild them.</summary>
        private float _wrappedAt = -1f;

        public ChatLog()
        {
            MouseFilter = MouseFilterEnum.Stop;
            FocusMode = FocusModeEnum.None;

            // A long line is cut off at the frame rather than drawn over the world beside it.
            ClipContents = true;
        }

        private struct Line
        {
            public string Author;
            public string Body;
            public Color NameColour;
            public Color BodyColour;
            public Color RankColour;

            /// <summary>When this line arrived, in milliseconds, so it can be retired on time.</summary>
            public ulong Born;

            /// <summary>How many rows this line wrapped to, so trimming knows what to remove.</summary>
            public int Rows;
        }

        /// <summary>One drawn row: a whole line, or one wrap of one.</summary>
        private readonly struct Row
        {
            public readonly int Line;
            public readonly string Text;
            public readonly float X;

            /// <summary>Whether this row carries the rank icon and the name.</summary>
            public readonly bool First;

            public Row(int line, string text, float x, bool first)
            {
                Line = line;
                Text = text;
                X = x;
                First = first;
            }
        }

        /// <summary>
        /// The size the log is drawn at, which the player can change.
        /// </summary>
        /// <remarks>
        /// Everything about a line -- its height, where it wraps, where the hanging indent falls --
        /// is measured from this, so it is read rather than baked in at construction: changing it
        /// with the panel open re-flows the log on the next redraw.
        /// </remarks>
        private static int FontSize => App.ServiceLocator.Settings?.ChatFontSize ?? Style.FontSmall;

        private float RowHeight => Mathf.Round(Mathf.Max(Style.Sans.GetHeight(FontSize), 14f) + 4f);

        private float TextWidth => Size.X - ScrollbarWidth - 6f;

        /// <summary>How far down the content the log can be scrolled.</summary>
        private float MaxScroll => Mathf.Max(0f, _rows.Count * RowHeight - Size.Y);

        /// <summary>
        /// Where the log is actually showing from.
        /// </summary>
        /// <remarks>
        /// Derived rather than stored, because everything that moves the bottom moves it behind the
        /// log's back: a line arriving, a line ageing out of the buffer, the panel being re-wrapped
        /// at a new width. Sticking to the bottom has to mean "wherever the bottom is now", not
        /// "where the bottom was when the last line arrived" -- storing it left the scrollbar's
        /// thumb sitting a page behind the text it was supposed to be describing.
        /// </remarks>
        private float Offset => _atBottom ? MaxScroll : Mathf.Clamp(_scroll, 0f, MaxScroll);

        public void Add(string author, string body, Color name, Color text, Color rank)
        {
            var line = new Line
            {
                Author = author,
                Body = body ?? string.Empty,
                NameColour = name,
                BodyColour = text,
                RankColour = rank,
                Born = Time.GetTicksMsec(),
            };

            _lines.Add(line);
            Wrap(_lines.Count - 1);

            // The buffer, and the rows the dropped line owned with it.
            while (_lines.Count > MaxLines)
                DropOldest();

            // Someone who has scrolled back to read something is reading it. Following
            // unconditionally is what pulls them off it the moment anyone speaks, and in a busy
            // world that is every second.
            if (!_atBottom)
                _unread = true;

            QueueRedraw();
        }

        /// <summary>How long a line stays on screen. <c>ChatListItem.as:15</c>.</summary>
        private const ulong LineLifetimeMs = 20000;

        /// <summary>
        /// Drops the lines that have been up their allotted time.
        /// </summary>
        /// <param name="held">
        /// Whether the log is being read, in which case nothing is dropped. The original suspends
        /// the same sweep whenever the player has scrolled back (<c>ChatList.as:101</c>) and
        /// resumes it when they return to the foot of the log (<c>ChatList.as:120-121</c>).
        /// </param>
        /// <summary>How many lines are on screen. Zero means the log is drawing nothing.</summary>
        public int LineCount => _lines.Count;

        public void Retire(bool held)
        {
            if (held || !_atBottom)
                return;

            ulong now = Time.GetTicksMsec();
            bool dropped = false;

            while (_lines.Count > 0 && now - _lines[0].Born >= LineLifetimeMs)
            {
                DropOldest();
                dropped = true;
            }

            if (dropped)
                QueueRedraw();
        }

        /// <summary>Removes the line at the head of the buffer and the rows it owned.</summary>
        private void DropOldest()
        {
            int rows = _lines[0].Rows;
            _lines.RemoveAt(0);
            _rows.RemoveRange(0, rows);

            for (int i = 0; i < _rows.Count; i++)
                _rows[i] = new Row(_rows[i].Line - 1, _rows[i].Text, _rows[i].X, _rows[i].First);

            // Held position is held against the text, not against the top of the buffer: the rows
            // that just aged out were above what is being read.
            _scroll = Mathf.Max(0f, _scroll - rows * RowHeight);
        }

        public void Scroll(int rows)
        {
            _scroll = Mathf.Clamp(Offset + rows * RowHeight, 0f, MaxScroll);
            _atBottom = _scroll >= MaxScroll - 1f;

            if (_atBottom)
                _unread = false;

            QueueRedraw();
        }

        /// <summary>
        /// Lays one line out into rows.
        /// </summary>
        /// <remarks>
        /// The hanging indent is the width of the name, so a wrapped line continues under the
        /// message rather than under whoever said it -- which is what lets the eye run down the
        /// left edge of the text and find where each message starts.
        /// </remarks>
        private void Wrap(int index)
        {
            var line = _lines[index];
            float limit = TextWidth;

            float left = RankSize + RankGap;
            float hang = left;

            if (!string.IsNullOrEmpty(line.Author))
                hang += Style.Measure($"<{line.Author}> ", FontSize);

            int before = _rows.Count;
            bool first = true;

            // Greedy, a word at a time. A single word too long for the row is emitted anyway and
            // clipped at the frame, which beats breaking mid-word for the one message in ten
            // thousand that is a wall of characters with no spaces in it.
            var current = _builder.Clear();

            foreach (string word in line.Body.Split(' '))
            {
                if (current.Length == 0)
                {
                    current.Append(word);
                    continue;
                }

                int mark = current.Length;
                current.Append(' ').Append(word);

                if (hang + Style.Measure(current.ToString(), FontSize) <= limit - 4f)
                    continue;

                current.Length = mark;
                _rows.Add(new Row(index, current.ToString(), hang, first));

                first = false;
                current.Clear().Append(word);
            }

            _rows.Add(new Row(index, current.ToString(), hang, first));

            line.Rows = _rows.Count - before;
            _lines[index] = line;
        }

        /// <summary>Re-wraps everything. Only on a resize, which is not something that repeats.</summary>
        private void Rewrap()
        {
            _wrappedAt = TextWidth;
            _rows.Clear();

            for (int i = 0; i < _lines.Count; i++)
                Wrap(i);
        }

        public override void _GuiInput(InputEvent @event)
        {
            switch (@event)
            {
                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelUp }:
                    Scroll(-3);
                    AcceptEvent();
                    return;

                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelDown }:
                    Scroll(3);
                    AcceptEvent();
                    return;

                case InputEventMouseButton { ButtonIndex: MouseButton.Left } button:
                    OnClick(button);
                    AcceptEvent();
                    return;

                case InputEventMouseMotion motion when _draggingThumb:
                    DragThumb(motion.Position.Y);
                    AcceptEvent();
                    return;
            }
        }

        private void OnClick(InputEventMouseButton button)
        {
            if (!button.Pressed)
            {
                _draggingThumb = false;
                return;
            }

            var at = button.Position;

            // The affordance across the bottom, which is only there when there is something below
            // the fold to go to.
            if (_unread && at.Y > Size.Y - RowHeight)
            {
                _scroll = MaxScroll;
                _atBottom = true;
                _unread = false;
                QueueRedraw();
                return;
            }

            if (at.X < Size.X - ScrollbarWidth)
                return;

            if (at.Y <= ArrowHeight)
            {
                Scroll(-1);
                return;
            }

            if (at.Y >= Size.Y - ArrowHeight)
            {
                Scroll(1);
                return;
            }

            var thumb = Thumb();
            if (thumb.HasPoint(at))
            {
                _draggingThumb = true;
                _dragOffset = at.Y - thumb.Position.Y;
                return;
            }

            // A click in the empty track jumps a page towards it.
            Scroll(at.Y < thumb.Position.Y ? -5 : 5);
        }

        private void DragThumb(float y)
        {
            var track = Track();
            var thumb = Thumb();

            float travel = track.Size.Y - thumb.Size.Y;
            if (travel <= 0f)
                return;

            float fraction = Mathf.Clamp((y - _dragOffset - track.Position.Y) / travel, 0f, 1f);

            _scroll = fraction * MaxScroll;
            _atBottom = fraction >= 0.999f;

            if (_atBottom)
                _unread = false;

            QueueRedraw();
        }

        private Rect2 Track() => new(
            Size.X - ScrollbarWidth, ArrowHeight, ScrollbarWidth, Mathf.Max(0f, Size.Y - ArrowHeight * 2f));

        private Rect2 Thumb()
        {
            var track = Track();
            float content = Mathf.Max(_rows.Count * RowHeight, Size.Y);
            float height = Mathf.Max(20f, track.Size.Y * Size.Y / content);
            float travel = track.Size.Y - height;
            float fraction = MaxScroll <= 0f ? 0f : Mathf.Clamp(Offset / MaxScroll, 0f, 1f);

            return new Rect2(track.Position.X + 2f, track.Position.Y + travel * fraction,
                ScrollbarWidth - 4f, height);
        }

        public override void _Draw()
        {
            if (!Mathf.IsEqualApprox(_wrappedAt, TextWidth))
                Rewrap();

            float rowHeight = RowHeight;
            float ascent = Style.Sans.GetAscent(FontSize);

            float offset = Offset;
            int firstRow = Mathf.Max(0, (int)(offset / rowHeight));
            float top = firstRow * rowHeight - offset;

            for (int i = firstRow; i < _rows.Count; i++)
            {
                float y = top + (i - firstRow) * rowHeight;
                if (y > Size.Y)
                    break;

                DrawRow(_rows[i], y + ascent);
            }

            // Nothing to scroll through means nothing to scroll with. Without this the track and its
            // arrows are the only thing left standing in the corner once the last line is retired
            // and the plate behind them has gone.
            if (_rows.Count > 0)
                DrawScrollbar();

            if (_unread)
                DrawUnread();
        }

        private void DrawRow(in Row row, float baseline)
        {
            var line = _lines[row.Line];

            if (row.First)
            {
                HudIcons.Star(this,
                    new Rect2(0f, baseline - RankSize + 2f, RankSize, RankSize), line.RankColour);

                if (!string.IsNullOrEmpty(line.Author))
                    Text(new Vector2(RankSize + RankGap, baseline), $"<{line.Author}>", line.NameColour);
            }

            Text(new Vector2(row.X, baseline), row.Text, line.BodyColour);
        }

        /// <summary>
        /// One run of text, outlined.
        /// </summary>
        /// <remarks>
        /// The panel lets the world through, so chat is read against whatever the player is
        /// standing on. The original bakes a black glow into every chat glyph for exactly that
        /// reason -- <c>BitmapTextFactory.as:15,42</c>, switched on for chat at
        /// <c>ChatListItemFactory.as:310</c> -- and without it the darker colours in its palette are
        /// unreadable over a dark floor.
        /// </remarks>
        private void Text(Vector2 at, string text, Color colour) =>
            this.DrawOverWorld(at, text, FontSize, colour);

        private void DrawScrollbar()
        {
            var track = Track();

            DrawRect(new Rect2(Size.X - ScrollbarWidth, 0f, ScrollbarWidth, Size.Y), Style.PanelInset);

            HudIcons.Chevron(this,
                new Rect2(Size.X - ScrollbarWidth + 3f, 2f, ScrollbarWidth - 6f, ArrowHeight - 4f),
                Style.TextDim, up: true);

            HudIcons.Chevron(this,
                new Rect2(Size.X - ScrollbarWidth + 3f, Size.Y - ArrowHeight + 2f, ScrollbarWidth - 6f, ArrowHeight - 4f),
                Style.TextDim, up: false);

            if (track.Size.Y <= 0f)
                return;

            var thumb = Thumb();
            DrawRect(thumb, _draggingThumb ? Style.SlotBorderHi : Style.ButtonFace);
            DrawRect(thumb, Style.PanelEdge, filled: false, width: 1f);
        }

        /// <summary>
        /// The mark that says the log has moved on without you.
        /// </summary>
        /// <remarks>
        /// The other half of the autoscroll rule. Holding position without saying anything looks
        /// like a log that has stopped, which is worse than being yanked to the bottom.
        /// </remarks>
        private void DrawUnread()
        {
            const string Label = "New messages";

            float width = Style.Measure(Label, Style.FontSmall);
            var pill = new Rect2(
                Mathf.Round((Size.X - ScrollbarWidth - width) / 2f) - 8f,
                Size.Y - RowHeight,
                width + 16f,
                RowHeight - 2f);

            DrawRect(pill, Style.ButtonFace);
            DrawRect(pill, Style.PanelEdge, filled: false, width: 1f);

            this.DrawText(
                new Vector2(pill.Position.X + 8f, pill.Position.Y + pill.Size.Y - 6f),
                Label, Style.FontSmall, Style.Text);
        }
    }
}
