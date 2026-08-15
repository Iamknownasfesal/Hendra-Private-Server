using System;
using Godot;

namespace Hendra.UI;

/// <summary>
/// The first screen: the game's mark, and three ways off it.
/// </summary>
/// <remarks>
/// <para>
/// The original's <c>TitleView</c> stacks a live map, a dark wash over it, the title graphic and a
/// bar of buttons, with version text near the bottom of an eight-hundred-by-six-hundred stage. The
/// live map is not reproduced — it exists to show the game moving before you have signed in, which
/// needs a whole second world running against no server, and the art covers almost all of it
/// anyway.
/// </para>
/// <para>
/// What replaces it is the page every other full-screen menu in this game is printed on: the
/// near-black wash, the drifting scatter of diamonds, the bracket ornament in the corners. A title
/// screen that shares its page with the options and character screens belongs to the same game,
/// which the stock nebula it used to show did not.
/// </para>
/// <para>
/// Everything is laid out against 1920 by 1080 and scaled by the window's height, because that is
/// the resolution the references were measured at.
/// </para>
/// </remarks>
public partial class TitleScreen : Control
{
    /// <summary>The resolution the layout below is measured in.</summary>
    private const float ReferenceHeight = 1080f;

    private const float LogoWidth = 880f;
    private const float LogoTop = 150f;

    private const float ButtonWidth = 280f;
    private const float ButtonHeight = 52f;
    private const float ButtonGap = 22f;
    private const float ButtonTop = 800f;

    /// <summary>
    /// What this client calls itself.
    /// </summary>
    /// <remarks>
    /// Five parts, the way the original prints its own. The protocol carries a separate build string
    /// that the server checks — this one is for the player, and the two are allowed to differ.
    /// </remarks>
    private const string Version = "0.1.0.0.0";

    private MenuBackdrop _page;
    private GameLogo _logo;
    private TitleFooter _footer;
    private TitlePlate _account;
    private TitlePlate _play;
    private TitlePlate _quit;

    /// <summary>Raised when the player wants to sign in and pick a character.</summary>
    public event Action PlayPressed;

    /// <summary>Raised when the player wants to leave.</summary>
    public event Action QuitPressed;

    /// <summary>Raised when the player wants the account panel.</summary>
    public event Action AccountPressed;

    public override void _Ready()
    {
        this.FillScreen();

        _page = new MenuBackdrop();
        AddChild(_page);

        _logo = new GameLogo(LogoWidth);
        AddChild(_logo);

        // Account left, Play in the middle, Quit right. The original puts Servers here too, but its
        // Servers screen picks between named worlds the app server hands back — and this client
        // already shows that list once you have signed in, which is the only point at which it
        // knows what the worlds are.
        _account = Add("Account", Style.ButtonFace, Style.ButtonBevelHigh, Style.ButtonBevelLow,
            () => AccountPressed?.Invoke());

        _play = Add("Play", Style.ButtonCommit, Style.ButtonCommitHigh, Style.ButtonCommitLow,
            () => PlayPressed?.Invoke());

        _quit = Add("Quit", Style.ButtonDanger, Style.ButtonDangerHigh, Style.ButtonDanger.Darkened(0.25f),
            () => QuitPressed?.Invoke());

        // Its own node, added last. A Control paints itself before its children, so version text
        // drawn by this screen would be painted over by the page it is standing on.
        _footer = new TitleFooter(
            $"v {Version}, build id: {BuildId}",
            $"Copyright © {DateTime.Now.Year} Hendra. All Rights reserved.");
        AddChild(_footer);

        Resized += Reflow;
        Reflow();
    }

    private TitlePlate Add(string label, Color face, Color high, Color low, Action pressed)
    {
        var button = new TitlePlate(label, face, high, low);
        button.Pressed += pressed;
        AddChild(button);
        return button;
    }

    /// <summary>Places the page, the mark, the button row and the version lines.</summary>
    private void Reflow()
    {
        var window = Size;
        if (window.X <= 0f || window.Y <= 0f || _logo == null)
            return;

        float s = window.Y / ReferenceHeight;

        _page.Size = window;
        _footer.Size = window;

        _logo.LogoWidth = Mathf.Round(LogoWidth * s);
        _logo.Position = new Vector2(Mathf.Round((window.X - _logo.Size.X) / 2f), Mathf.Round(LogoTop * s));

        var buttons = new[] { _account, _play, _quit };
        float width = Mathf.Round(ButtonWidth * s);
        float gap = Mathf.Round(ButtonGap * s);
        float row = width * buttons.Length + gap * (buttons.Length - 1);
        float x = Mathf.Round((window.X - row) / 2f);

        foreach (var button in buttons)
        {
            button.Position = new Vector2(x, Mathf.Round(ButtonTop * s));
            button.Size = new Vector2(width, Mathf.Round(ButtonHeight * s));
            x += width + gap;
        }

        QueueRedraw();
    }

    /// <summary>
    /// A short identifier for exactly this build.
    /// </summary>
    /// <remarks>
    /// Taken from the assembly's module identity, which the compiler regenerates on every build. A
    /// hand-written constant here would be a number that stops moving the first time somebody
    /// forgets to bump it, which makes the line worse than useless when a player quotes it.
    /// </remarks>
    private static string BuildId =>
        System.Reflection.Assembly.GetExecutingAssembly().ManifestModule.ModuleVersionId
            .ToString("N")[..9];
}

/// <summary>
/// The two lines along the foot of the title screen: what this build is, and whose it is.
/// </summary>
/// <remarks>
/// The references carry the same pair on the Escape menu, centred over the world and set in the
/// interface's own face at the largest size it uses. Here they are the only text on the page below
/// the buttons, and they are set dim rather than white: on a near-black page white would make the
/// small print the second-loudest thing on the screen.
/// </remarks>
public partial class TitleFooter : Control
{
    private const float ReferenceHeight = 1080f;
    private const float VersionBaseline = 996f;
    private const float CopyrightBaseline = 1042f;

    private readonly string _version;
    private readonly string _copyright;

    public TitleFooter(string version, string copyright)
    {
        _version = version;
        _copyright = copyright;
        MouseFilter = MouseFilterEnum.Ignore;
    }

    public override void _Notification(int what)
    {
        if (what == NotificationResized)
            QueueRedraw();
    }

    public override void _Draw()
    {
        if (Size.X <= 0f || Size.Y <= 0f)
            return;

        float s = Size.Y / ReferenceHeight;
        int size = Mathf.Max(Style.SmallestReadable, Mathf.RoundToInt(Style.FontTitle * s));

        Line(_version, VersionBaseline * s, size);
        Line(_copyright, CopyrightBaseline * s, size);
    }

    private void Line(string text, float baseline, int size) =>
        this.DrawText(
            new Vector2(Mathf.Round((Size.X - Style.Measure(text, size)) / 2f), Mathf.Round(baseline)),
            text, size, Style.Text);
}

/// <summary>
/// The game's button plate, at the size the menus use it.
/// </summary>
/// <remarks>
/// The frame the references show: light across the top and down both sides, dark along the bottom,
/// all four corners left as background, and a hard shadow underneath. Held swaps light for dark and
/// drops the label a pixel, so the plate visibly goes in. Figures come from <see cref="Style"/>.
/// </remarks>
public partial class TitlePlate : Button
{
    /// <summary>How tall the caps are, and how far they are inset — the reference's own figures.</summary>
    private const float CapHeight = 4f;

    private const float CapInset = 5f;

    /// <summary>The plate height those figures were measured at.</summary>
    private const float ReferenceHeight = 52f;

    private readonly string _label;
    private readonly Color _face;
    private readonly Color _high;
    private readonly Color _low;

    private bool _hovered;

    public TitlePlate(string label, Color face, Color high, Color low)
    {
        _label = label;
        _face = face;
        _high = high;
        _low = low;

        Text = string.Empty;
        FocusMode = FocusModeEnum.None;

        foreach (string state in new[] { "normal", "hover", "pressed", "disabled", "focus" })
            AddThemeStyleboxOverride(state, new StyleBoxEmpty());
    }

    public override void _Ready()
    {
        MouseEntered += () => { _hovered = true; QueueRedraw(); };
        MouseExited += () => { _hovered = false; QueueRedraw(); };
    }

    public override void _Draw()
    {
        bool held = ButtonPressed || IsPressed();

        var face = held ? _face.Darkened(0.2f) : _hovered ? _face.Lightened(0.12f) : _face;
        var top = held ? _low : _high;
        var bottom = held ? _high : _low;

        float scale = Mathf.Max(1f, Size.Y / ReferenceHeight);
        float cap = Mathf.Round(Style.ButtonFrameTop * scale);
        float side = Mathf.Round(Style.ButtonFrameSide * scale);
        float inner = Size.X - side * 2f;
        float tall = Size.Y - cap * 2f;

        // The shadow the plate casts on the page, and then the frame: light across the top and
        // down both sides, dark along the bottom, with the four corners left as background.
        DrawRect(
            new Rect2(side, Size.Y, inner, Mathf.Round(Style.ButtonShadowHeight * scale)),
            Style.ButtonShadow);

        DrawRect(new Rect2(side, cap, inner, tall), face);
        DrawRect(new Rect2(side, 0f, inner, cap), top);
        DrawRect(new Rect2(0f, cap, side, tall), top);
        DrawRect(new Rect2(Size.X - side, cap, side, tall), top);
        DrawRect(new Rect2(side, Size.Y - cap, inner, cap), bottom);

        int size = Mathf.Max(Style.SmallestReadable, Mathf.RoundToInt(Style.FontControl * scale));
        var at = new Vector2(
            Mathf.Round((Size.X - Style.Measure(_label, size)) / 2f),
            Style.BaselineIn(Size.Y, size)) + (held ? Vector2.One : Vector2.Zero);

        this.DrawText(at, _label, size, Style.Text);
    }
}
