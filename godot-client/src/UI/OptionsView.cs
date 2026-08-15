using System;
using System.Collections.Generic;
using System.Linq;
using Godot;
using Hendra.App;

namespace Hendra.UI;

/// <summary>
/// The options screen: a full-screen page of four tabs, drawn in the game's own chrome.
/// </summary>
/// <remarks>
/// <para>
/// Custom-drawn rather than assembled out of engine widgets, for the same reason the rest of the
/// interface is: a checkbox and a dropdown from the engine's default theme sit in a pixel-art game
/// like a form in a spreadsheet. A row here is a label cell and a control cell, the control being a
/// green ON, a red OFF, a value with a triangle, a track with a white knob, or the name of a key --
/// which is what the original's screen is made of.
/// </para>
/// <para>
/// It covers the screen rather than floating over it. The original's options are a page, not a
/// dialog: the world is hidden behind an opaque field so that nothing on it competes with the row
/// being read, and the rows are tall enough to be hit without aiming.
/// </para>
/// <para>
/// Every row here does something. That is a rule rather than an observation: a control that does
/// nothing is worse than a missing one, because the player spends a click and some trust finding
/// out. Settings the original shows and this fork has no feature for are left out rather than
/// faked.
/// </para>
/// <para>
/// Changes apply as they are made. A volume you cannot hear, or a shadow you cannot see, until the
/// panel closes is a setting you cannot judge.
/// </para>
/// </remarks>
public partial class OptionsView : Control
{
    // ─── the page, in the reference resolution's pixels ────────────────────────────────────────
    //
    // Measured off references/Menu/*.png at 1920x1080. The side margins are a share of the width so
    // the page keeps its proportions on a different window; everything vertical is fixed, because
    // the rows have a size a finger and an eye want rather than a size the screen implies.

    /// <summary>The page's margin from each side, as a share of the screen's width.</summary>
    private const float SideMargin = 0.075f;

    /// <summary>The top of the body plate, and the bottom's distance from the foot of the screen.</summary>
    private const float BodyTop = 184f;

    private const float BodyBottomMargin = 86f;

    private const float TabTop = 126f;
    private const float TabBottom = 184f;

    /// <summary>The band along the bottom of a tab, which is darker than its face.</summary>
    private const float TabFoot = 11f;

    private const float TabBevel = 5f;
    private const float TabGap = 7.25f;

    private const float CloseTop = 43f;
    private const float CloseHeight = 57f;
    private const float CloseWidth = 180f;

    private const float TitleBaseline = 85f;
    private const float SmallBaseline = 79f;

    /// <summary>Where "reset to defaults" starts, measured from the page's left edge.</summary>
    private const float ResetInset = 246f;

    // ─── the body ─────────────────────────────────────────────────────────────────────────────

    private const float RowHeight = 72f;
    private const float RowGap = 3.33f;
    private const float HeadingHeight = 72f;

    /// <summary>The cursor picker, which is two rows tall.</summary>
    private const float CursorsHeight = 144f;

    private const float BodyPad = 18f;

    /// <summary>The gutter between the rows and the scrollbar, and the scrollbar's own width.</summary>
    private const float ScrollGutter = 9f;

    private const float ScrollWidth = 29f;

    /// <summary>Where the control cell starts, as a share of the width the rows have.</summary>
    private const float SplitAt = 0.5705f;

    /// <summary>The gap between the label cell and the control cell.</summary>
    private const float CellGap = 7f;

    private const float ControlInset = 29f;

    private const float SliderTrackHeight = 36f;
    private const float SliderKnob = 36f;

    /// <summary>Where the slider's track begins and ends inside the row.</summary>
    private const float SliderLeft = 15f;

    /// <summary>
    /// The room kept clear to the right of a track for the number, which the track stops short of
    /// and the number is centred in.
    /// </summary>
    /// <remarks>
    /// Two widths, because two kinds of reading go there: a percentage carries a sign the plain
    /// counts on the Sound page do not, and giving both the wider gutter would leave the volumes
    /// with a track that stops well short of the page.
    /// </remarks>
    private const float SliderGutter = 97f;

    private const float SliderGutterNarrow = 72f;

    private const float SwitchWidth = 206f;

    /// <summary>The lit plate inside the switch, which is a little over half its trough.</summary>
    private const float SwitchPlate = 102f;
    private const float SwitchHeight = 54f;
    private const float SwitchBevel = 5f;

    private const float TriangleWidth = 17f;
    private const float TriangleHeight = 22f;

    /// <summary>One cursor's cell in the picker, and how large the artwork is drawn inside it.</summary>
    private const float CursorCell = 86.4f;

    private const float CursorArt = 64f;

    /// <summary>One entry in an open list of choices.</summary>
    private const float ChoiceHeight = 48f;

    // ─── type ─────────────────────────────────────────────────────────────────────────────────
    //
    // Sized by cap height against the reference rather than by name: the page is set much larger
    // than the HUD is, so the scale in Style has nothing on it that fits.

    private const int FontTitle = 66;
    private const int FontHeading = 46;
    private const int FontRow = 38;
    private const int FontTab = 28;
    private const int FontSmall = 30;
    private const int FontSwitch = 28;

    /// <summary>What fraction of its nominal size a capital in the interface's face stands.</summary>
    private const float CapRatio = 0.545f;

    // ─── the page's own colours ───────────────────────────────────────────────────────────────
    //
    // Everything with a token in Style takes it. What is left is the handful this screen is the
    // only user of: the two cell greys, the unlit half of a switch, and the scrollbar's plates.

    /// <summary>The left half of a row, which carries the label.</summary>
    private static readonly Color LabelCell = new("373737");

    /// <summary>The right half of a row, which carries the control.</summary>
    private static readonly Color ControlCell = new("565656");

    private static readonly Color LabelCellHover = new("414141");
    private static readonly Color ControlCellHover = new("606060");

    /// <summary>An unselected tab's face. Its border is <see cref="Style.TabIdle"/>.</summary>
    private static readonly Color TabFace = new("252525");

    private static readonly Color TabFaceFoot = new("212121");
    private static readonly Color TabActiveBevel = new("696969");
    private static readonly Color TabActiveFoot = new("3e3e3e");

    /// <summary>The dark line along the bottom of the red Close button.</summary>
    private static readonly Color CloseLow = new("a81326");

    /// <summary>A switch in its off position, which is unlit rather than alarming.</summary>
    private static readonly Color ToggleOff = new("8c2525");

    private static readonly Color ToggleOffHigh = new("bd4848");
    private static readonly Color ToggleOffLow = new("ab4141");
    private static readonly Color ToggleOffText = new("521515");

    /// <summary>The lower edge of a lit switch, and of the filled part of a slider.</summary>
    private static readonly Color ToggleOnLow = new("76ab41");

    /// <summary>The trough a switch slides in.</summary>
    private static readonly Color SwitchTrack = new("373737");

    private static readonly Color SwitchTrackEdge = new("313131");

    private static readonly Color ScrollTrack = new("323232");
    private static readonly Color ScrollThumb = new("484848");
    private static readonly Color ScrollButton = new("4f4f4f");
    private static readonly Color ScrollButtonHigh = new("5e5e5e");
    private static readonly Color ScrollButtonLow = new("3d3d3d");

    /// <summary>The plate under the cursor the player has picked.</summary>
    private static readonly Color CursorPicked = new("242424");

    private static readonly string[] TabNames = { "Controls", "Gameplay", "Video", "Sound" };

    private Settings _settings;
    private Body _body;
    private Page _page;
    private Chrome _chrome;
    private OptionsBackdrop _backdrop;

    private int _tab;

    /// <summary>
    /// What the pointer is over in the page's furniture.
    /// </summary>
    /// <remarks>
    /// Held here rather than on either of the two controls that care, because they are separate
    /// nodes for the sake of draw order -- <see cref="Page"/> takes the clicks from underneath
    /// everything and <see cref="Chrome"/> paints over the top of it -- and the highlight belongs
    /// to neither on its own.
    /// </remarks>
    private int _hoveredTab = -1;

    private bool _overClose;
    private bool _overReset;

    /// <summary>Raised whenever a value changes, so the caller can apply and save it.</summary>
    public event Action Changed;

    public bool IsOpen => _page is { Visible: true };

    public void Configure(Settings settings)
    {
        _settings = settings;
        _body?.Rebuild();
        Cursors.Apply(settings);
    }

    public override void _Ready()
    {
        // No FillScreen: the HUD canvas sizes its children to the reference-pixel space, which is
        // what puts this page's text on the same footing as the rest of the interface.
        MouseFilter = MouseFilterEnum.Ignore;

        // Children paint over their parent, so the order below is the order up the screen: the
        // field, then the rows, then the title and the tabs over the top of both.
        _page = new Page(this) { Visible = false, MouseFilter = MouseFilterEnum.Stop };
        AddChild(_page);

        _backdrop = new OptionsBackdrop();
        _page.AddChild(_backdrop);

        _body = new Body(this) { MouseFilter = MouseFilterEnum.Stop };
        _page.AddChild(_body);

        _chrome = new Chrome(this) { MouseFilter = MouseFilterEnum.Ignore };
        _page.AddChild(_chrome);

        Resized += Fit;
        if (GetParent() is HudLayer layer)
            layer.Reflowed += Fit;

        Fit();
    }

    /// <summary>The page's plate, in this control's own coordinates.</summary>
    private Rect2 PageBox()
    {
        float margin = Mathf.Round(Size.X * SideMargin);
        return new Rect2(margin, 0f, Mathf.Max(0f, Size.X - margin * 2f), Size.Y);
    }

    private Rect2 TabAt(int index)
    {
        var page = PageBox();
        // Both edges are rounded rather than the width, so four tabs and three gaps land on the
        // page's own edges instead of accumulating a pixel of drift across the strip.
        float width = (page.Size.X - TabGap * (TabNames.Length - 1)) / TabNames.Length;
        float left = Mathf.Round(page.Position.X + index * (width + TabGap));
        float right = Mathf.Round(page.Position.X + index * (width + TabGap) + width);

        return new Rect2(left, TabTop, right - left, TabBottom - TabTop);
    }

    private Rect2 CloseAt()
    {
        var page = PageBox();
        return new Rect2(page.End.X - CloseWidth, CloseTop, CloseWidth, CloseHeight);
    }

    private Rect2 ResetAt()
    {
        var page = PageBox();
        return new Rect2(
            page.Position.X + ResetInset, SmallBaseline - Mathf.Round(FontSmall * CapRatio) - 6f,
            Style.Measure("reset to defaults", FontSmall) + 8f, 32f);
    }

    private void Fit()
    {
        if (_page == null)
            return;

        _page.Position = Vector2.Zero;
        _page.Size = Size;

        _backdrop.Position = Vector2.Zero;
        _backdrop.Size = Size;

        var page = PageBox();
        _body.Position = new Vector2(page.Position.X, BodyTop);
        _body.Size = new Vector2(page.Size.X, Mathf.Max(0f, Size.Y - BodyTop - BodyBottomMargin));

        // Deliberately not a rebuild: a resize -- which changing the window mode causes -- would
        // otherwise throw the rows away and put the player back at the top of the list.
        if (_body.IsEmpty)
            _body.Rebuild();

        _chrome.Position = Vector2.Zero;
        _chrome.Size = Size;

        _body.QueueRedraw();
        _chrome.QueueRedraw();
    }

    /// <summary>
    /// Which page the panel opens on, by name.
    /// </summary>
    /// <remarks>
    /// For the command line, which is the only caller: an unattended run that wants a picture of
    /// the video settings has no hand on the keyboard to click the tab with.
    /// </remarks>
    public void ShowTab(string name)
    {
        int at = Array.FindIndex(TabNames, t => string.Equals(t, name, StringComparison.OrdinalIgnoreCase));
        if (at >= 0)
            _tab = at;
    }

    /// <summary>Opens the page, or closes it if it is already open.</summary>
    public void Toggle()
    {
        if (_page == null)
            return;

        // A rebinding left half-finished would otherwise swallow the next key pressed in the world,
        // and a list left open would be showing over the page the next time it is opened.
        _body?.StopListening();
        _body?.CloseList();

        _page.Visible = !_page.Visible;

        if (_page.Visible)
            _body?.Rebuild();
    }

    private void Apply()
    {
        Changed?.Invoke();
        _body?.QueueRedraw();
    }

    /// <summary>The baseline that puts a line of this size on the middle of a box.</summary>
    private static float Baseline(float centre, int size) =>
        Mathf.Round(centre + Mathf.Round(size * CapRatio) / 2f);

    // ==========================================================================================
    // Page: the clicks. Chrome: the title, the tabs and the close button
    // ==========================================================================================

    /// <summary>
    /// The whole screen, as far as the mouse is concerned.
    /// </summary>
    /// <remarks>
    /// It draws nothing. It sits under the field, the rows and the furniture, swallows every click
    /// that lands on none of them -- which is what stops the world being played through the page --
    /// and owns the hit tests for the tabs, the close button and the reset affordance, because
    /// those are painted by a sibling drawn above the rows.
    /// </remarks>
    private sealed partial class Page : Control
    {
        private readonly OptionsView _owner;

        public Page(OptionsView owner) => _owner = owner;

        public override void _Ready() => MouseExited += () =>
        {
            _owner._hoveredTab = -1;
            _owner._overClose = false;
            _owner._overReset = false;
            _owner._chrome?.QueueRedraw();
        };

        public override void _GuiInput(InputEvent @event)
        {
            switch (@event)
            {
                case InputEventMouseMotion motion:
                {
                    int over = -1;
                    for (int i = 0; i < TabNames.Length; i++)
                    {
                        if (_owner.TabAt(i).HasPoint(motion.Position))
                            over = i;
                    }

                    bool close = _owner.CloseAt().HasPoint(motion.Position);
                    bool reset = _owner.ResetAt().HasPoint(motion.Position);

                    if (over != _owner._hoveredTab || close != _owner._overClose
                        || reset != _owner._overReset)
                    {
                        _owner._hoveredTab = over;
                        _owner._overClose = close;
                        _owner._overReset = reset;
                        _owner._chrome?.QueueRedraw();
                    }

                    return;
                }

                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left } click:
                {
                    AcceptEvent();

                    if (_owner.CloseAt().HasPoint(click.Position))
                    {
                        _owner.Toggle();
                        return;
                    }

                    if (_owner.ResetAt().HasPoint(click.Position))
                    {
                        KeyBindings.ResetAll(_owner._settings);
                        _owner._body?.Rebuild();
                        return;
                    }

                    for (int i = 0; i < TabNames.Length; i++)
                    {
                        if (!_owner.TabAt(i).HasPoint(click.Position))
                            continue;

                        _owner._tab = i;
                        _owner._body?.StopListening();
                        _owner._body?.CloseList();
                        _owner._body?.Rebuild();
                        _owner._chrome?.QueueRedraw();
                        return;
                    }

                    return;
                }
            }
        }
    }

    /// <summary>The page's furniture: its name, its tabs, and the way out.</summary>
    private sealed partial class Chrome : Control
    {
        private readonly OptionsView _owner;

        public Chrome(OptionsView owner) => _owner = owner;

        public override void _Draw()
        {
            var page = _owner.PageBox();

            this.DrawText(new Vector2(page.Position.X, TitleBaseline), "Options", FontTitle, Style.Text);

            this.DrawText(new Vector2(page.Position.X + ResetInset, SmallBaseline), "reset to defaults",
                FontSmall, _owner._overReset ? Style.Text : Style.TextDim);

            DrawClose();

            for (int i = 0; i < TabNames.Length; i++)
                DrawTab(i);
        }

        /// <summary>The way out, in the original's red: a flat face with a bright lip and a dark foot.</summary>
        private void DrawClose()
        {
            var box = _owner.CloseAt();

            DrawRect(box, _owner._overClose ? Style.ButtonDangerHigh : Style.ButtonDanger);
            DrawRect(new Rect2(box.Position, new Vector2(box.Size.X, TabBevel)), Style.ButtonDangerHigh);
            DrawRect(new Rect2(box.Position, new Vector2(TabBevel, box.Size.Y)), Style.ButtonDangerHigh);
            DrawRect(new Rect2(box.End.X - TabBevel, box.Position.Y, TabBevel, box.Size.Y),
                Style.ButtonDangerHigh);
            DrawRect(new Rect2(box.Position.X, box.End.Y - TabBevel, box.Size.X, TabBevel), CloseLow);

            this.DrawText(
                new Vector2(box.GetCenter().X - Style.Measure("Close", FontSmall) / 2f, SmallBaseline),
                "Close", FontSmall, Style.Text);
        }

        /// <summary>
        /// One tab. The selected one is a step lighter than the page and keeps its lit edge; the
        /// rest sink almost into the field behind them.
        /// </summary>
        private void DrawTab(int index)
        {
            var box = _owner.TabAt(index);
            bool active = index == _owner._tab;
            bool hover = _owner._hoveredTab == index && !active;

            var faceColour = active ? Style.TabActive : hover ? Style.Panel : TabFace;
            var bevel = active ? TabActiveBevel : Style.TabIdle;
            var foot = active ? TabActiveFoot : TabFaceFoot;

            DrawRect(box, faceColour);
            DrawRect(new Rect2(box.Position.X, box.End.Y - TabFoot, box.Size.X, TabFoot), foot);

            DrawRect(new Rect2(box.Position, new Vector2(box.Size.X, TabBevel)), bevel);
            DrawRect(new Rect2(box.Position, new Vector2(TabBevel, box.Size.Y - TabFoot)), bevel);
            DrawRect(new Rect2(box.End.X - TabBevel, box.Position.Y, TabBevel, box.Size.Y - TabFoot), bevel);

            float centre = box.Position.Y + (box.Size.Y - TabFoot) / 2f;
            this.DrawText(
                new Vector2(box.GetCenter().X - Style.Measure(TabNames[index], FontTab) / 2f,
                    Baseline(centre, FontTab)),
                TabNames[index], FontTab, active ? Style.TabActiveText : Style.TextDim);
        }
    }

    // ==========================================================================================
    // Rows
    // ==========================================================================================

    private enum RowKind
    {
        Heading,
        Toggle,
        Choice,
        Slider,
        Key,
        Cursors,
    }

    private sealed class Row
    {
        public RowKind Kind;
        public string Label;

        public Func<bool> GetBool;
        public Action<bool> SetBool;

        public string[] Choices;
        public Func<int> GetChoice;
        public Action<int> SetChoice;

        public Func<float> GetValue;
        public Action<float> SetValue;

        /// <summary>How the number beside a slider reads. Percentages, volumes and plain steps differ.</summary>
        public Func<float, string> Format;

        /// <summary>Whether the travelled part of a slider's track is filled in.</summary>
        public bool Filled;

        /// <summary>How much room the number beside this slider is given.</summary>
        public float Gutter;

        public string Action;

        public float Height => Kind switch
        {
            RowKind.Heading => HeadingHeight,
            RowKind.Cursors => CursorsHeight,
            _ => RowHeight,
        };
    }

    /// <summary>Where a value sits in its range, as nought to one.</summary>
    private static float Fraction(float value, float low, float high) =>
        Mathf.Clamp((value - low) / (high - low), 0f, 1f);

    /// <summary>The value at that point in the range, rounded to something a slider can land on.</summary>
    private static float Lerp(float fraction, float low, float high) =>
        Mathf.Round((low + (high - low) * Mathf.Clamp(fraction, 0f, 1f)) * 100f) / 100f;

    /// <summary>Frame rate ceilings offered, with zero meaning none.</summary>
    private static readonly List<int> FpsChoices = new() { 0, 30, 60, 120, 144, 240 };

    private static readonly string[] FpsLabels = { "Unlimited", "30", "60", "120", "144", "240" };

    /// <summary>The chat sizes the original offers, in pixels.</summary>
    private static readonly List<int> ChatSizes = new() { 9, 11, 12, 14, 16 };

    // ==========================================================================================
    // Cursors
    // ==========================================================================================

    /// <summary>
    /// The pointer the game draws, chosen from the original's own sheet.
    /// </summary>
    /// <remarks>
    /// The artwork is <c>cursorsEmbed</c>, which is the sheet the original picks its crosshairs out
    /// of. Nothing is drawn by hand here and nothing is invented: the picker offers exactly the
    /// sprites the sheet holds, which is ten of the twelve the live game shows.
    /// </remarks>
    private static class Cursors
    {
        public const string Sheet = "cursorsEmbed";

        /// <summary>How many of the sheet's tiles carry artwork.</summary>
        public const int Count = 10;

        /// <summary>The smallest and largest the pointer is drawn, in pixels.</summary>
        private const int Smallest = 16;

        private const int Step = 6;

        public static Assets.Sprite Art(int index) =>
            ServiceLocator.Assets?.GetSprite(Sheet, Mathf.Clamp(index, 0, Count - 1)) ?? default;

        /// <summary>Pixels across, for a size setting of one to ten.</summary>
        public static int Pixels(int size) => Smallest + (Mathf.Clamp(size, 1, 10) - 1) * Step;

        /// <summary>
        /// Puts the chosen sprite on the mouse, at the chosen size.
        /// </summary>
        /// <remarks>
        /// The sprite is scaled with nearest-neighbour rather than handed to the engine at its own
        /// eight-pixel size and stretched, because the engine's cursor path filters what it is given
        /// and a filtered pixel crosshair is a smudge. Failing to build one leaves the system arrow,
        /// which is the right thing to fall back to.
        /// </remarks>
        public static void Apply(Settings settings)
        {
            if (settings == null)
                return;

            var sprite = Art(settings.CursorStyle);
            if (!sprite.IsValid)
                return;

            var source = sprite.Sheet.GetImage();
            if (source == null)
                return;

            var tile = Image.CreateEmpty(sprite.Region.Size.X, sprite.Region.Size.Y, false, Image.Format.Rgba8);
            tile.BlitRect(source, sprite.Region, Vector2I.Zero);

            int pixels = Pixels(settings.CursorSize);
            tile.Resize(pixels, pixels, Image.Interpolation.Nearest);

            Input.SetCustomMouseCursor(
                ImageTexture.CreateFromImage(tile), Input.CursorShape.Arrow,
                new Vector2(pixels, pixels) / 2f);
        }
    }

    // ==========================================================================================
    // Body: the scrolling list of rows
    // ==========================================================================================

    private sealed partial class Body : Control
    {
        private readonly OptionsView _owner;
        private readonly List<Row> _rows = new();

        private float _scroll;
        private int _hovered = -1;
        private bool _dragging;
        private int _draggingSlider = -1;

        /// <summary>The row waiting for a key press, or -1 when nothing is being rebound.</summary>
        private int _listening = -1;

        /// <summary>The row whose list of choices is open, or -1 when none is.</summary>
        private int _open = -1;

        /// <summary>Which entry of the open list the pointer is over, or -1.</summary>
        private int _openAt = -1;

        public Body(OptionsView owner) => _owner = owner;

        public override void _Ready()
        {
            ClipContents = true;
            MouseExited += () => { _hovered = -1; QueueRedraw(); };
        }

        private Settings Options => _owner._settings;

        /// <summary>Whether the rows have yet to be built, so a relayout knows to build them.</summary>
        public bool IsEmpty => _rows.Count == 0;

        /// <summary>Moves the list, clamped to what there is to see.</summary>
        public void Scroll(float by)
        {
            // An open list is pinned to a row, and the row is about to move out from under it.
            CloseList();

            float was = _scroll;
            _scroll = Mathf.Clamp(_scroll + by, 0f, MaxScroll());

            if (!Mathf.IsEqualApprox(was, _scroll))
                QueueRedraw();
        }

        public void StopListening()
        {
            if (_listening < 0)
                return;

            _listening = -1;
            QueueRedraw();
        }

        /// <summary>Puts away the list of choices, if one is showing.</summary>
        public void CloseList()
        {
            if (_open < 0)
                return;

            _open = -1;
            _openAt = -1;
            QueueRedraw();
        }

        public void Rebuild()
        {
            _rows.Clear();
            _scroll = 0f;
            _listening = -1;
            _open = -1;
            _openAt = -1;

            if (Options != null)
            {
                switch (_owner._tab)
                {
                    case 0: BuildControls(); break;
                    case 1: BuildGameplay(); break;
                    case 2: BuildVideo(); break;
                    default: BuildSound(); break;
                }
            }

            QueueRedraw();
        }

        private void Heading(string text) => _rows.Add(new Row { Kind = RowKind.Heading, Label = text });

        private void Toggle(string label, Func<bool> get, Action<bool> set) =>
            _rows.Add(new Row { Kind = RowKind.Toggle, Label = label, GetBool = get, SetBool = set });

        private void Choice(string label, string[] choices, Func<int> get, Action<int> set) =>
            _rows.Add(new Row
            {
                Kind = RowKind.Choice, Label = label, Choices = choices, GetChoice = get, SetChoice = set,
            });

        private void Slider(string label, Func<float> get, Action<float> set, Func<float, string> format = null) =>
            _rows.Add(new Row
            {
                Kind = RowKind.Slider, Label = label, GetValue = get, SetValue = set,
                Format = format ?? Percent, Gutter = SliderGutter,
            });

        /// <summary>
        /// A level, which the original draws differently from an ordinary slider.
        /// </summary>
        /// <remarks>
        /// The travelled part of the track is filled in green, the reading is a plain count out of
        /// a hundred rather than a percentage, and the gutter that reading sits in is narrower by
        /// the width of the sign it does not carry.
        /// </remarks>
        private void Volume(string label, Func<float> get, Action<float> set) =>
            _rows.Add(new Row
            {
                Kind = RowKind.Slider, Label = label, GetValue = get, SetValue = set,
                Format = Hundred, Filled = true, Gutter = SliderGutterNarrow,
            });

        private static string Percent(float value) => $"{Mathf.RoundToInt(value * 100f)}%";

        private static string Hundred(float value) => Mathf.RoundToInt(value * 100f).ToString();

        private void BuildControls()
        {
            string group = null;
            foreach (var binding in KeyBindings.All)
            {
                if (binding.Group != group)
                {
                    group = binding.Group;
                    Heading(group);
                }

                _rows.Add(new Row { Kind = RowKind.Key, Label = binding.Label, Action = binding.Action });
            }
        }

        private void BuildGameplay()
        {
            Heading("General");
            Toggle("Allow Camera Rotation", () => Options.AllowCameraRotation, on => Options.AllowCameraRotation = on);
            Toggle("Keep the player centred", () => Options.CenterOnPlayer, on => Options.CenterOnPlayer = on);
            Choice("Camera Rotation Speed", new[] { "Slow", "Normal", "Fast" },
                () => Options.CameraRotationSpeed, value => Options.CameraRotationSpeed = value);

            Heading("Opacity");
            Slider("Other players", () => Options.Opacity, value => Options.Opacity = Mathf.Max(0.1f, value));
            Toggle("Player on top", () => Options.PlayerOnTop, on => Options.PlayerOnTop = on);
            Toggle("Guild members", () => Options.FadeGuildMembers, on => Options.FadeGuildMembers = on);
            Toggle("Players", () => Options.FadePlayers, on => Options.FadePlayers = on);
            Toggle("Projectiles", () => Options.FadeProjectiles, on => Options.FadeProjectiles = on);

            Heading("Interface");
            Choice("Chat Font Size", new[] { "Tiny", "Small", "Default", "Large", "Big" },
                () => ChatSizes.IndexOf(Options.ChatFontSize) is var at && at >= 0 ? at : 2,
                value => Options.ChatFontSize = ChatSizes[value]);
            Toggle("Small Condition Icons", () => Options.SmallConditionIcons, on => Options.SmallConditionIcons = on);
            Toggle("Show Tier Level", () => Options.ShowTierLevel, on => Options.ShowTierLevel = on);
            Choice("Bar Numbers", new[] { "Off", "Fame", "HP/MP", "All" },
                () => Options.BarText, value => Options.BarText = value);
            Choice("HP Bars", new[] { "Off", "Enemy", "Ally", "All" },
                () => Options.HealthBars, value => Options.HealthBars = value);

            Heading("Social");
            Toggle("Hide Chat Window", () => Options.HideChat, on => Options.HideChat = on);
            Toggle("Player Chat", () => Options.PlayerChat, on => Options.PlayerChat = on);
            Toggle("Whisper Chat", () => Options.WhisperChat, on => Options.WhisperChat = on);
            Toggle("Guild Chat", () => Options.GuildChatShown, on => Options.GuildChatShown = on);
        }

        private void BuildVideo()
        {
            Heading("Cursor");
            _rows.Add(new Row { Kind = RowKind.Cursors });
            Slider("Cursor size",
                () => Fraction(Options.CursorSize, 1f, 10f),
                value =>
                {
                    Options.CursorSize = Mathf.RoundToInt(Lerp(value, 1f, 10f));
                    Cursors.Apply(Options);
                },
                value => Mathf.RoundToInt(1f + value * 9f).ToString());

            Heading("Display");
            Choice("Window Mode", new[] { "Fullscreen", "Windowed" },
                () => Options.Windowed ? 1 : 0, value => Options.Windowed = value == 1);

            Choice("Resolution", ResolutionLabels,
                () => NearestResolution(),
                value => DisplayServer.WindowSetSize(Resolutions[value]));

            Choice("Default Camera Angle", new[] { "0°", "45°" },
                () => Options.DefaultCameraAngle == 45 ? 1 : 0,
                value => Options.DefaultCameraAngle = value == 1 ? 45 : 0);

            // Shown as a share of the range rather than as a raw multiplier, so the slider reads
            // the way every other slider on this screen does.
            Slider("View Distance", () => 1f - Fraction(Options.CameraZoom, 0.5f, 2f),
                value => Options.CameraZoom = Lerp(1f - value, 0.5f, 2f));
            Slider("Bag Size", () => Fraction(Options.BagSize, 0.5f, 2.5f),
                value => Options.BagSize = Lerp(value, 0.5f, 2.5f));

            Heading("Rendering");

            // The preset writes the two below it. It is not stored anywhere -- it is read back
            // from what they say, so moving either one on its own is not overridden the next time
            // this page is opened, and lands on "Custom" if the pair matches no preset.
            Choice("Quality", QualityNames, () => QualityOf(Options), value => ApplyQuality(Options, value));

            Choice("Render Scale", ScaleLabels,
                () => Nearest(ScaleChoices, Options.RenderScale),
                value => Options.RenderScale = ScaleChoices[value]);

            Choice("Anti-Aliasing", new[] { "Off", "FXAA", "MSAA 2x", "MSAA 4x", "MSAA 8x + FXAA" },
                () => Options.AntiAliasing, value => Options.AntiAliasing = value);

            Choice("Interface Scale", HudScaleLabels,
                () => Nearest(HudScaleChoices, Options.HudScale),
                value => Options.HudScale = HudScaleChoices[value]);

            Choice("Frame Rate Limit", FpsLabels,
                () => FpsChoices.IndexOf(Options.MaxFps) is var at && at >= 0 ? at : 0,
                value => Options.MaxFps = FpsChoices[value]);

            Choice("V-Sync", new[] { "Off", "On", "Adaptive" },
                () => Options.VSync, value => Options.VSync = value);

            Heading("Effects");
            Toggle("Particles Master", () => Options.Particles, on => Options.Particles = on);
            Choice("Particle Effect", new[] { "Low", "Medium", "High" },
                () => Options.ParticleDetail, value => Options.ParticleDetail = value);
            Choice("Draw Gameview Shadows", new[] { "Off", "Low", "High" },
                () => Options.Shadows, value => Options.Shadows = value);
            Toggle("Enemy Particles", () => Options.EnemyParticles, on => Options.EnemyParticles = on);
            Toggle("Player Hit Particles", () => Options.PlayerHitParticles, on => Options.PlayerHitParticles = on);
            Toggle("AOE Particles", () => Options.AoeParticles, on => Options.AoeParticles = on);
            Toggle("Draw Text Bubbles", () => Options.TextBubbles, on => Options.TextBubbles = on);
            Toggle("Enemy Damage Text", () => Options.EnemyDamageText, on => Options.EnemyDamageText = on);
            Toggle("Ally Damage Text", () => Options.AllyDamageText, on => Options.AllyDamageText = on);
            Toggle("Curse Indication", () => Options.CurseIndication, on => Options.CurseIndication = on);
            Choice("Ally Shoot", new[] { "Show All", "Hide Projectiles", "Hide All" },
                () => Options.AllyShoot, value => Options.AllyShoot = value);
        }

        /// <summary>
        /// The presets, as the pair of values each one stands for.
        /// </summary>
        /// <remarks>
        /// Supersampling is where the quality is, so the ladder is mostly render scale: half again
        /// at High and double at Ultra. Low turns everything off for a machine that is struggling,
        /// which on this game is a real case -- a full realm is a few thousand sprites.
        /// </remarks>
        private static readonly (string Name, int Scale, int Aa)[] Presets =
        {
            ("Low", 100, 0),
            ("Medium", 100, 1),
            ("High", 150, 1),
            ("Ultra", 200, 4),
        };

        private static readonly string[] QualityNames =
            Presets.Select(p => p.Name).Append("Custom").ToArray();

        private static readonly int[] ScaleChoices = { 75, 100, 125, 150, 175, 200 };

        /// <summary>Zero is "fit to the window", which is what the interface has always done.</summary>
        private static readonly int[] HudScaleChoices = { 0, 100, 125, 150, 175, 200, 250 };

        private static readonly string[] HudScaleLabels =
            { "Fit to Window", "100%", "125%", "150%", "175%", "200%", "250%" };

        private static readonly string[] ScaleLabels =
            ScaleChoices.Select(v => $"{v}%").ToArray();

        /// <summary>The window sizes offered, which are the common sixteen-by-nine ones.</summary>
        private static readonly Vector2I[] Resolutions =
        {
            new(1280, 720), new(1600, 900), new(1920, 1080), new(2560, 1440), new(3840, 2160),
        };

        private static readonly string[] ResolutionLabels =
            Resolutions.Select(v => $"{v.X} x {v.Y}").ToArray();

        /// <summary>Whichever offered size the window is nearest, so the row says what is on screen.</summary>
        private static int NearestResolution()
        {
            var size = DisplayServer.WindowGetSize();

            int best = 0;
            for (int i = 1; i < Resolutions.Length; i++)
                if (Mathf.Abs(Resolutions[i].X - size.X) < Mathf.Abs(Resolutions[best].X - size.X))
                    best = i;

            return best;
        }

        /// <summary>Which preset the current pair of values is, or Custom.</summary>
        private static int QualityOf(App.Settings options)
        {
            for (int i = 0; i < Presets.Length; i++)
                if (Presets[i].Scale == options.RenderScale && Presets[i].Aa == options.AntiAliasing)
                    return i;

            return Presets.Length;
        }

        private static void ApplyQuality(App.Settings options, int preset)
        {
            if (preset < 0 || preset >= Presets.Length)
                return;

            options.RenderScale = Presets[preset].Scale;
            options.AntiAliasing = Presets[preset].Aa;
        }

        /// <summary>The index of the nearest offered value, so a hand-edited file still shows.</summary>
        private static int Nearest(int[] choices, int value)
        {
            int best = 0;
            for (int i = 1; i < choices.Length; i++)
                if (Mathf.Abs(choices[i] - value) < Mathf.Abs(choices[best] - value))
                    best = i;

            return best;
        }

        private void BuildSound()
        {
            Volume("Master Volume", () => Options.MasterVolume, value => Options.MasterVolume = value);
            Volume("Music Volume", () => Options.MusicVolume, value => Options.MusicVolume = value);
            Volume("Sound Effects Volume", () => Options.EffectVolume, value =>
            {
                Options.EffectVolume = value;

                // Played as it moves so the slider demonstrates the level it has just been set to.
                ServiceLocator.Audio?.PlayEffect("button_click");
            });
            Toggle("Play Weapon Sounds", () => Options.WeaponSounds, on => Options.WeaponSounds = on);
        }

        // ---- geometry ----

        private float Content()
        {
            float total = BodyPad * 2f;
            foreach (var row in _rows)
                total += row.Height + RowGap;
            return total;
        }

        private float MaxScroll() => Mathf.Max(0f, Content() - Size.Y);

        /// <summary>The left and right edges the rows are laid between.</summary>
        private float ContentLeft => BodyPad;

        private float ContentRight => Size.X - ScrollGutter * 2f - ScrollWidth;

        private Rect2 RowAt(int index)
        {
            float y = BodyPad - _scroll;
            for (int i = 0; i < index; i++)
                y += _rows[i].Height + RowGap;

            return new Rect2(ContentLeft, Mathf.Round(y), ContentRight - ContentLeft, _rows[index].Height);
        }

        private int RowUnder(Vector2 at)
        {
            for (int i = 0; i < _rows.Count; i++)
            {
                if (_rows[i].Kind != RowKind.Heading && RowAt(i).HasPoint(at))
                    return i;
            }

            return -1;
        }

        /// <summary>The label cell on the left of a row.</summary>
        private Rect2 LabelAt(int index)
        {
            var row = RowAt(index);
            return new Rect2(row.Position, new Vector2(Mathf.Round(row.Size.X * SplitAt), row.Size.Y));
        }

        /// <summary>The control cell on the right of a row.</summary>
        private Rect2 ControlAt(int index)
        {
            var row = RowAt(index);
            float left = Mathf.Round(row.Size.X * SplitAt) + CellGap;
            return new Rect2(row.Position.X + left, row.Position.Y, row.Size.X - left, row.Size.Y);
        }

        private Rect2 SliderTrack(int index)
        {
            var control = ControlAt(index);
            return new Rect2(
                control.Position.X + SliderLeft,
                Mathf.Round(control.GetCenter().Y - SliderTrackHeight / 2f),
                ContentRight - _rows[index].Gutter - control.Position.X - SliderLeft, SliderTrackHeight);
        }

        private Rect2 SwitchTrackAt(int index)
        {
            var control = ControlAt(index);
            return new Rect2(
                Mathf.Round(control.GetCenter().X - SwitchWidth / 2f),
                Mathf.Round(control.GetCenter().Y - SwitchHeight / 2f), SwitchWidth, SwitchHeight);
        }

        /// <summary>
        /// Where an open list of choices is drawn.
        /// </summary>
        /// <remarks>
        /// Under the control it belongs to, or over it when there is no room under -- the body clips
        /// its own contents, so a list that runs off the bottom is a list with its last entries
        /// missing, which is the half of it the player is most likely to be reaching for. Clamped
        /// after that, for the rare list taller than the page.
        /// </remarks>
        private Rect2 ChoiceList(int index)
        {
            var control = ControlAt(index);
            float height = _rows[index].Choices.Length * ChoiceHeight + 8f;

            float top = control.End.Y + 3f;
            if (top + height > Size.Y)
                top = control.Position.Y - height - 3f;

            top = Mathf.Clamp(top, 0f, Mathf.Max(0f, Size.Y - height));
            return new Rect2(control.Position.X, Mathf.Round(top), control.Size.X, height);
        }

        /// <summary>Which entry of the open list a point falls on, or -1 for none of them.</summary>
        private int ChoiceUnder(Vector2 at)
        {
            if (_open < 0)
                return -1;

            var list = ChoiceList(_open);
            if (!list.HasPoint(at))
                return -1;

            int index = (int)((at.Y - list.Position.Y - 4f) / ChoiceHeight);
            return index >= 0 && index < _rows[_open].Choices.Length ? index : -1;
        }

        // ---- the scrollbar, which is this page's own rather than the HUD's ----

        private Rect2 ScrollBox() =>
            new(Size.X - ScrollGutter - ScrollWidth, 0f, ScrollWidth, Size.Y);

        private Rect2 ScrollButtonAt(bool up)
        {
            var bar = ScrollBox();
            return up
                ? new Rect2(bar.Position.X, 10f, ScrollWidth, ScrollWidth)
                : new Rect2(bar.Position.X, bar.End.Y - 11f - ScrollWidth, ScrollWidth, ScrollWidth);
        }

        private Rect2 ScrollTrackAt()
        {
            var top = ScrollButtonAt(up: true);
            var foot = ScrollButtonAt(up: false);
            return new Rect2(top.Position.X, top.End.Y + 4f, ScrollWidth,
                Mathf.Max(0f, foot.Position.Y - 4f - (top.End.Y + 4f)));
        }

        private Rect2 ScrollThumbAt()
        {
            var track = ScrollTrackAt();
            float content = Mathf.Max(Content(), Size.Y);
            float height = Mathf.Max(40f, track.Size.Y * Size.Y / content);
            float travel = track.Size.Y - height;
            float max = MaxScroll();
            float fraction = max <= 0f ? 0f : Mathf.Clamp(_scroll / max, 0f, 1f);

            return new Rect2(track.Position.X, Mathf.Round(track.Position.Y + travel * fraction),
                ScrollWidth, Mathf.Round(height));
        }

        // ---- input ----

        public override void _GuiInput(InputEvent @event)
        {
            switch (@event)
            {
                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelDown }:
                    Scroll(RowHeight * 2f);
                    AcceptEvent();
                    return;

                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelUp }:
                    Scroll(-RowHeight * 2f);
                    AcceptEvent();
                    return;

                case InputEventMouseButton { Pressed: false, ButtonIndex: MouseButton.Left }:
                    _dragging = false;
                    _draggingSlider = -1;
                    return;

                case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left } click:
                    AcceptEvent();
                    Press(click.Position);
                    return;

                case InputEventMouseMotion motion:
                    if (_dragging)
                    {
                        DragThumb(motion.Position.Y);
                        return;
                    }

                    if (_draggingSlider >= 0)
                    {
                        DragSlider(_draggingSlider, motion.Position);
                        return;
                    }

                    // While a list is open it is the only thing the pointer can be over: the rows
                    // behind it are not reachable until it closes.
                    if (_open >= 0)
                    {
                        int entry = ChoiceUnder(motion.Position);
                        if (entry != _openAt)
                        {
                            _openAt = entry;
                            QueueRedraw();
                        }

                        return;
                    }

                    int over = RowUnder(motion.Position);
                    if (over != _hovered)
                    {
                        _hovered = over;
                        QueueRedraw();
                    }

                    return;
            }
        }

        private void DragThumb(float y)
        {
            var track = ScrollTrackAt();
            float travel = track.Size.Y - ScrollThumbAt().Size.Y;
            if (travel <= 0f)
                return;

            _scroll = Mathf.Clamp((y - track.Position.Y) / travel, 0f, 1f) * MaxScroll();
            QueueRedraw();
        }

        private void Press(Vector2 at)
        {
            // An open list takes the whole press, wherever it lands: on an entry it picks that
            // entry, anywhere else it just puts the list away. Falling through to the row beneath
            // would act on something the list was covering.
            if (_open >= 0)
            {
                int entry = ChoiceUnder(at);
                var opened = _rows[_open];
                CloseList();

                if (entry >= 0)
                {
                    opened.SetChoice(entry);
                    _owner.Apply();
                }

                QueueRedraw();
                return;
            }

            // The scrollbar first: it sits beside the rows but takes the whole of its own column.
            if (ScrollBox().HasPoint(at))
            {
                PressScrollbar(at);
                return;
            }

            int index = RowUnder(at);
            if (index < 0)
                return;

            var row = _rows[index];

            switch (row.Kind)
            {
                case RowKind.Toggle:
                    row.SetBool(!row.GetBool());
                    _owner.Apply();
                    break;

                case RowKind.Choice:
                    // Opens the list rather than stepping to the next value. Stepping was quiet
                    // enough on a two-value row, but the triangle drawn on the right of every one of
                    // these promises a list, and "Interface Scale" has seven settings you cannot
                    // see and can only walk forwards through -- overshoot it and you go round again.
                    _open = index;
                    _openAt = row.GetChoice();
                    break;

                case RowKind.Slider:
                    _draggingSlider = index;
                    DragSlider(index, at);
                    break;

                case RowKind.Cursors:
                    PressCursor(index, at);
                    break;

                case RowKind.Key:
                    // A mouse-bound action is shown but not caught here; see KeyBindings.BoundTo.
                    if (!KeyBindings.IsMouse(row.Action))
                        _listening = index;
                    break;
            }

            QueueRedraw();
        }

        private void PressScrollbar(Vector2 at)
        {
            if (ScrollButtonAt(up: true).HasPoint(at))
                _scroll = Mathf.Max(0f, _scroll - RowHeight);
            else if (ScrollButtonAt(up: false).HasPoint(at))
                _scroll = Mathf.Min(MaxScroll(), _scroll + RowHeight);
            else if (ScrollThumbAt().HasPoint(at))
                _dragging = true;
            else if (at.Y < ScrollThumbAt().Position.Y)
                _scroll = Mathf.Max(0f, _scroll - Size.Y);
            else
                _scroll = Mathf.Min(MaxScroll(), _scroll + Size.Y);

            QueueRedraw();
        }

        private void PressCursor(int index, Vector2 at)
        {
            var row = RowAt(index);
            float first = row.GetCenter().X - Cursors.Count * CursorCell / 2f;

            int picked = Mathf.FloorToInt((at.X - first) / CursorCell);
            if (picked < 0 || picked >= Cursors.Count)
                return;

            Options.CursorStyle = picked;
            Cursors.Apply(Options);
            _owner.Apply();
        }

        private void DragSlider(int index, Vector2 at)
        {
            var track = SliderTrack(index);
            float value = Mathf.Clamp(
                (at.X - track.Position.X - SliderKnob / 2f) / Mathf.Max(track.Size.X - SliderKnob, 1f), 0f, 1f);

            _rows[index].SetValue(Mathf.Round(value * 100f) / 100f);
            _owner.Apply();
        }

        /// <summary>
        /// Catches the next key for whichever binding is listening.
        /// </summary>
        /// <remarks>
        /// Taken as unhandled input rather than through the row, because the point is to catch keys
        /// the page has no interest in -- including the ones that would otherwise close it.
        /// </remarks>
        public override void _Input(InputEvent @event)
        {
            if (@event is not InputEventKey { Pressed: true, Echo: false } key)
                return;

            // Escape closes the open list and stops there. It is the innermost thing on the screen,
            // so it is the one the player means -- letting this through would shut the whole page
            // and lose the row they were part-way through setting.
            if (_open >= 0 && key.Keycode == Key.Escape)
            {
                CloseList();
                GetViewport().SetInputAsHandled();
                return;
            }

            // The keyboard moves the list too, which matters on the Controls tab: it is long enough
            // that reaching the bottom of it by wheel is a chore.
            if (_listening < 0)
            {
                switch (key.Keycode)
                {
                    case Key.Pagedown: Scroll(Size.Y - RowHeight); break;
                    case Key.Pageup: Scroll(-(Size.Y - RowHeight)); break;
                    case Key.Down: Scroll(RowHeight); break;
                    case Key.Up: Scroll(-RowHeight); break;
                    case Key.Home: Scroll(-MaxScroll()); break;
                    case Key.End: Scroll(MaxScroll()); break;
                    default: return;
                }

                GetViewport().SetInputAsHandled();
                return;
            }

            GetViewport().SetInputAsHandled();

            // Escape abandons the rebinding rather than binding Escape, which is the one key a
            // player cannot afford to lose and the one they will press to get out of this.
            if (key.Keycode != Key.Escape)
                KeyBindings.Bind(_rows[_listening].Action, key.Keycode, Options);

            _listening = -1;
            QueueRedraw();
        }

        // ---- drawing ----

        public override void _Draw()
        {
            DrawRect(new Rect2(Vector2.Zero, Size), Style.PanelInset);

            for (int i = 0; i < _rows.Count; i++)
            {
                var row = _rows[i];
                var box = RowAt(i);

                // Off the top or the bottom: nothing to draw, and the geometry above is cheap.
                if (box.End.Y < 0f || box.Position.Y > Size.Y)
                    continue;

                switch (row.Kind)
                {
                    case RowKind.Heading:
                        this.DrawText(
                            new Vector2(box.Position.X + 1f, Baseline(box.GetCenter().Y, FontHeading)),
                            row.Label, FontHeading, Style.Text);
                        continue;

                    case RowKind.Cursors:
                        DrawCursors(i, box);
                        continue;
                }

                DrawRow(i, row, box);
            }

            DrawScrollbar();

            // Last, so it covers the rows and the scrollbar both.
            if (_open >= 0)
                DrawChoiceList(_open);
        }

        /// <summary>A label cell, a control cell, and whichever control the row carries.</summary>
        private void DrawRow(int index, Row row, Rect2 box)
        {
            var label = LabelAt(index);
            var control = ControlAt(index);
            bool hover = _hovered == index;

            DrawRect(label, hover ? LabelCellHover : LabelCell);

            // A slider has no plate under it: the track is the control, laid straight on the page.
            if (row.Kind != RowKind.Slider)
                DrawRect(control, hover ? ControlCellHover : ControlCell);

            this.DrawText(
                new Vector2(label.Position.X + ControlInset, Baseline(label.GetCenter().Y, FontRow)),
                row.Label, FontRow, Style.TextDim);

            switch (row.Kind)
            {
                case RowKind.Toggle: DrawSwitch(index, row.GetBool()); break;
                case RowKind.Choice:
                    DrawChoice(control,
                        row.Choices[Mathf.Clamp(row.GetChoice(), 0, row.Choices.Length - 1)], index == _open);
                    break;
                case RowKind.Slider: DrawSlider(index, row); break;
                case RowKind.Key: DrawKey(control, row, index == _listening); break;
            }
        }

        /// <summary>The row of pointers, on a plate of their own, the chosen one on a darker square.</summary>
        private void DrawCursors(int index, Rect2 box)
        {
            DrawRect(box, LabelCell);

            float first = box.GetCenter().X - Cursors.Count * CursorCell / 2f;
            int chosen = Mathf.Clamp(Options?.CursorStyle ?? 0, 0, Cursors.Count - 1);

            for (int i = 0; i < Cursors.Count; i++)
            {
                var slot = new Rect2(
                    Mathf.Round(first + i * CursorCell), Mathf.Round(box.GetCenter().Y - CursorCell / 2f),
                    Mathf.Round(CursorCell), Mathf.Round(CursorCell));

                if (i == chosen)
                    DrawRect(slot, CursorPicked);

                var art = Cursors.Art(i);
                if (!art.IsValid)
                    continue;

                this.DrawSprite(art,
                    new Rect2(slot.GetCenter() - new Vector2(CursorArt, CursorArt) / 2f,
                        new Vector2(CursorArt, CursorArt)), outline: 0f);
            }
        }

        /// <summary>The open list of choices, drawn over whatever it lands on.</summary>
        private void DrawChoiceList(int index)
        {
            var row = _rows[index];
            var list = ChoiceList(index);
            int chosen = Mathf.Clamp(row.GetChoice(), 0, row.Choices.Length - 1);

            DrawRect(list, Style.PanelInset);
            DrawRect(list, ControlCell, filled: false, width: 4f);

            for (int i = 0; i < row.Choices.Length; i++)
            {
                var entry = new Rect2(list.Position.X + 4f, list.Position.Y + 4f + i * ChoiceHeight,
                    list.Size.X - 8f, ChoiceHeight);

                if (i == _openAt)
                    DrawRect(entry, ControlCellHover);
                else if (i == chosen)
                    DrawRect(entry, ControlCell);

                this.DrawText(
                    new Vector2(entry.Position.X + ControlInset, Baseline(entry.GetCenter().Y, FontRow)),
                    row.Choices[i], FontRow, i == chosen ? Style.Text : Style.TextDim);
            }
        }

        /// <summary>
        /// A switch: a dark trough with a lit plate at one end of it.
        /// </summary>
        /// <remarks>
        /// Both halves are the same plate in two colours. On is green with white lettering; off is a
        /// dark red with lettering barely a shade off the plate, which is what reads as unlit.
        /// </remarks>
        private void DrawSwitch(int index, bool on)
        {
            var track = SwitchTrackAt(index);

            DrawRect(track, SwitchTrack);
            DrawRect(new Rect2(track.Position, new Vector2(track.Size.X, 4f)), SwitchTrackEdge);
            DrawRect(new Rect2(track.Position, new Vector2(SwitchBevel, track.Size.Y)), SwitchTrackEdge);
            DrawRect(new Rect2(track.End.X - SwitchBevel, track.Position.Y, SwitchBevel, track.Size.Y),
                SwitchTrackEdge);

            var plate = new Rect2(
                on ? track.End.X - SwitchBevel - SwitchPlate : track.Position.X + SwitchBevel,
                track.Position.Y + 4f, SwitchPlate, track.Size.Y - 8f);

            var face = on ? Style.ToggleOn : ToggleOff;
            var high = on ? Style.ToggleOnHigh : ToggleOffHigh;
            var low = on ? ToggleOnLow : ToggleOffLow;

            DrawRect(plate, face);
            DrawRect(new Rect2(plate.Position, new Vector2(plate.Size.X, SwitchBevel)), high);
            DrawRect(new Rect2(plate.Position, new Vector2(SwitchBevel, plate.Size.Y)), high);
            DrawRect(new Rect2(plate.End.X - SwitchBevel, plate.Position.Y, SwitchBevel, plate.Size.Y), high);
            DrawRect(new Rect2(plate.Position.X, plate.End.Y - SwitchBevel, plate.Size.X, SwitchBevel), low);

            string text = on ? "ON" : "OFF";
            this.DrawText(
                new Vector2(plate.GetCenter().X - Style.Measure(text, FontSwitch) / 2f,
                    Baseline(plate.GetCenter().Y, FontSwitch)),
                text, FontSwitch, on ? Style.Text : ToggleOffText);
        }

        private void DrawChoice(Rect2 control, string value, bool open)
        {
            this.DrawText(
                new Vector2(control.Position.X + ControlInset, Baseline(control.GetCenter().Y, FontRow)),
                value, FontRow, Style.Text);

            // The triangle at the right edge, which is what says there is a list behind this. It
            // brightens while that list is showing, so the row you opened stays findable under it.
            float right = ContentRight - ControlInset;
            float top = Mathf.Round(control.GetCenter().Y - TriangleHeight / 2f);

            DrawColoredPolygon(
                new[]
                {
                    new Vector2(right - TriangleWidth, top),
                    new Vector2(right, top),
                    new Vector2(right - TriangleWidth / 2f, top + TriangleHeight),
                },
                open ? Style.TabActiveText : Style.Text);
        }

        /// <summary>A track, a round white knob on it, and the value in the gutter beside the row.</summary>
        private void DrawSlider(int index, Row row)
        {
            var track = SliderTrack(index);
            float value = Mathf.Clamp(row.GetValue(), 0f, 1f);
            float travel = Mathf.Max(track.Size.X - SliderKnob, 1f);
            float knobLeft = Mathf.Round(track.Position.X + travel * value);

            DrawRect(track, LabelCell);

            if (row.Filled && value > 0f)
            {
                var filled = new Rect2(track.Position, new Vector2(knobLeft + SliderKnob / 2f - track.Position.X,
                    track.Size.Y));

                DrawRect(filled, Style.ToggleOn);
                DrawRect(new Rect2(filled.Position, new Vector2(filled.Size.X, 4f)), Style.ToggleOnHigh);
            }

            // Two overlapping rectangles rather than one: the corners come off, which is as round as
            // a knob gets on a grid this size and is what the original draws.
            DrawRect(new Rect2(knobLeft + 5f, track.Position.Y, SliderKnob - 10f, SliderKnob), Style.Text);
            DrawRect(new Rect2(knobLeft, track.Position.Y + 5f, SliderKnob, SliderKnob - 10f), Style.Text);

            string text = row.Format(value);
            this.DrawText(
                new Vector2(ContentRight - row.Gutter / 2f - Style.Measure(text, FontRow) / 2f,
                    Baseline(track.GetCenter().Y, FontRow)),
                text, FontRow, Style.Text);
        }

        private void DrawKey(Rect2 control, Row row, bool listening)
        {
            string text = listening ? "press a key" : KeyBindings.BoundTo(row.Action);

            var colour = listening
                ? Style.FameFill
                : KeyBindings.IsMouse(row.Action) ? Style.TextDim
                : KeyBindings.IsChanged(row.Action) ? Style.TierSpecial
                : Style.Text;

            this.DrawText(
                new Vector2(control.GetCenter().X - Style.Measure(text, FontRow) / 2f,
                    Baseline(control.GetCenter().Y, FontRow)),
                text, FontRow, colour);
        }

        /// <summary>
        /// The bar down the right of the page: a square button at each end and a thumb between.
        /// </summary>
        /// <remarks>
        /// Always drawn, even on a tab short enough to need no scrolling -- the original does the
        /// same, and a bar that comes and goes with the tab moves the rows sideways under the
        /// pointer every time the page is switched.
        /// </remarks>
        private void DrawScrollbar()
        {
            DrawRect(ScrollTrackAt(), ScrollTrack);
            DrawRect(ScrollThumbAt(), _dragging ? Style.SlotBorderHi : ScrollThumb);

            DrawScrollButton(ScrollButtonAt(up: true), up: true);
            DrawScrollButton(ScrollButtonAt(up: false), up: false);
        }

        private void DrawScrollButton(Rect2 box, bool up)
        {
            DrawRect(box, ScrollButton);
            DrawRect(new Rect2(box.Position, new Vector2(box.Size.X, 4f)), ScrollButtonHigh);
            DrawRect(new Rect2(box.Position.X, box.End.Y - 3f, box.Size.X, 3f), ScrollButtonLow);

            var centre = box.GetCenter();
            float half = 6f;

            DrawColoredPolygon(
                up
                    ? new[]
                    {
                        centre + new Vector2(0f, -half),
                        centre + new Vector2(half + 1f, half),
                        centre + new Vector2(-half - 1f, half),
                    }
                    : new[]
                    {
                        centre + new Vector2(-half - 1f, -half),
                        centre + new Vector2(half + 1f, -half),
                        centre + new Vector2(0f, half),
                    },
                Style.Text);
        }
    }
}
