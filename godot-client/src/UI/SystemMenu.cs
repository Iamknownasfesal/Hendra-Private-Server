using System;
using Godot;

namespace Hendra.UI;

/// <summary>
/// The menu behind Escape: leave, change something, or stop playing.
/// </summary>
/// <remarks>
/// <para>
/// Not a dialog in the middle of the screen. It takes over the right-hand interface column — the
/// full-height strip the minimap and the vitals live in — putting the game's mark where the map was
/// and a stack of full-width plates down the rest of it, while the world and every other cluster go
/// dark behind a wash. That is what the reference does, and it is the better shape for it: the
/// column is already the part of the screen nothing is fought in, so the menu costs no view of the
/// world, and the mark at the top says which game you are looking at the way a pause screen should.
/// </para>
/// <para>
/// It does not pause. Nothing can: the world is on a server that keeps ticking, and a menu that
/// implied otherwise would get people killed while they read it. What it does instead is hold back
/// input — see <c>GameScene</c>, which folds this into the same predicate the options page uses —
/// so the character stands still rather than walking on under the wash.
/// </para>
/// <para>
/// Every button that opens a page closes this first. The pages are earlier children of the same
/// canvas, so one left open underneath would be painted over by the wash rather than shown.
/// </para>
/// </remarks>
public partial class SystemMenu : Control
{
    /// <summary>How far a plate is inset from each side of the column.</summary>
    private const float ColumnInset = 29f;

    private const float PlateHeight = 53f;

    /// <summary>
    /// One plate, its shadow, and the gap after it.
    /// </summary>
    /// <remarks>
    /// Not a round number because the stack is not built from one: the seven plates above the
    /// commit button span 387 pixels in the reference over six gaps, which is this.
    /// </remarks>
    private const float ButtonPitch = 64.6f;

    /// <summary>The gap under Quit, which sets Continue apart from the stack it ends.</summary>
    private const float CommitGap = 59f;

    /// <summary>Continue's distance from the foot of the screen.</summary>
    private const float BottomMargin = 30f;

    /// <summary>
    /// How much of the screen behind survives the wash.
    /// </summary>
    /// <remarks>
    /// Measured rather than chosen: white interface text under the reference's wash reads 81, and
    /// its muted grey reads 57, which is the same factor twice.
    /// </remarks>
    private const float Survives = 0.318f;

    /// <summary>The mark's width, as a fraction of the column's.</summary>
    private const float LogoShare = 0.825f;

    private GameLogo _logo;
    private TitleFooter _footer;
    private GameButton[] _stack;
    private GameButton _quit;
    private GameButton _continue;

    /// <summary>Raised for the options page, which this menu does not own.</summary>
    public event Action OptionsRequested;

    /// <summary>Raised to end the session and go back to the server and character list.</summary>
    public event Action ServersRequested;

    /// <summary>Raised for the account sheet.</summary>
    public event Action AccountRequested;

    /// <summary>Raised to close the game.</summary>
    public event Action QuitRequested;

    public bool IsOpen => Visible;

    public override void _Ready()
    {
        // The wash swallows the pointer, so a click on the darkened world neither walks the
        // character nor reaches the interface it is drawn over.
        MouseFilter = MouseFilterEnum.Stop;
        Visible = false;

        _logo = new GameLogo(1f);
        AddChild(_logo);

        _stack = new[]
        {
            Add("Options", () => { Close(); OptionsRequested?.Invoke(); }),

            // Nothing keeps a quest log, a server directory this client can switch between mid-
            // session, or a credits page, so these say so rather than pretending.
            Add("Journal", null),
            Add("Servers", () => { Close(); ServersRequested?.Invoke(); }),
            Add("Legends", null),
            Add("Account", () => { Close(); AccountRequested?.Invoke(); }),
            Add("Credits", null),
        };

        _quit = Add("Quit", () => QuitRequested?.Invoke(), Style.PlateDanger);
        _continue = Add("Continue", Close, Style.PlateCommit);

        // The two lines along the foot of the screen, in the same words the title screen sets them
        // in. Its own node so it is painted after the wash rather than under it.
        _footer = new TitleFooter(ClientBuild.VersionLine, ClientBuild.CopyrightLine);
        AddChild(_footer);

        Resized += Reflow;
        if (GetParent() is HudLayer layer)
            layer.Reflowed += Reflow;

        Reflow();
    }

    private GameButton Add(string label, Action pressed, Style.ButtonPlate? plate = null)
    {
        // Compact, which is the size the reference sets these labels at: a cap of sixteen pixels
        // rather than the twenty a menu button on the title screen wears.
        var button = new GameButton(label, compact: true, plate: plate);

        if (pressed == null)
            button.Disabled = true;
        else
            button.Pressed += pressed;

        AddChild(button);
        return button;
    }

    /// <summary>
    /// Fills the interface's right-hand column.
    /// </summary>
    /// <remarks>
    /// The column is read out of <see cref="HudLayout"/> rather than written here, so the menu
    /// keeps sitting over the same strip the map and the vitals do however wide that strip becomes.
    /// The stack is measured up from the foot of the screen for the same reason it is drawn that
    /// way: Continue is the button the hand goes to, and it should not move when a row above it
    /// does.
    /// </remarks>
    private void Reflow()
    {
        if (_logo == null || Size.X <= 0f || Size.Y <= 0f)
            return;

        var column = new HudLayout(Size).Minimap;

        float width = Mathf.Round(column.Size.X - ColumnInset * 2f);
        float left = Mathf.Round(column.Position.X + ColumnInset);

        _logo.LogoWidth = Mathf.Round(column.Size.X * LogoShare);
        _logo.Position = new Vector2(
            Mathf.Round(column.Position.X + (column.Size.X - _logo.Size.X) / 2f),
            Mathf.Round(column.Position.Y + (column.Size.Y - _logo.Size.Y) / 2f));

        _continue.Position = new Vector2(left, Mathf.Round(Size.Y - BottomMargin - PlateHeight));
        _continue.Size = new Vector2(width, PlateHeight);

        float quit = _continue.Position.Y - CommitGap - PlateHeight;
        _quit.Position = new Vector2(left, Mathf.Round(quit));
        _quit.Size = new Vector2(width, PlateHeight);

        for (int i = 0; i < _stack.Length; i++)
        {
            _stack[i].Position = new Vector2(
                left, Mathf.Round(quit - (_stack.Length - i) * ButtonPitch));

            _stack[i].Size = new Vector2(width, PlateHeight);
        }

        // The version lines are centred over the world rather than over the screen, so the column
        // the menu is standing in does not push them off centre.
        _footer.Position = Vector2.Zero;
        _footer.Size = new Vector2(column.Position.X, Size.Y);
    }

    /// <summary>The wash, which is the only thing this node draws itself.</summary>
    public override void _Draw() =>
        DrawRect(new Rect2(Vector2.Zero, Size), new Color(0f, 0f, 0f, 1f - Survives));

    public void Toggle()
    {
        if (_logo == null)
            return;

        if (Visible)
        {
            Close();
            return;
        }

        Reflow();
        Visible = true;
    }

    public void Close() => Visible = false;
}
