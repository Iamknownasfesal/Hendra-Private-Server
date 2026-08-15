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
    /// How much of the world and the clusters over it survives the wash.
    /// </summary>
    /// <remarks>
    /// Measured rather than chosen: white interface text under the reference's wash reads 81, and
    /// its muted grey reads 57, which is the same factor twice.
    /// </remarks>
    private const float Survives = 0.318f;

    /// <summary>
    /// The colour the column behind the menu is flattened towards, and how far.
    /// </summary>
    /// <remarks>
    /// The column is not washed, it is greyed: white in its icon row comes out at 57 and the plate
    /// under those icons at 30, which no single multiplier produces — the two are 255 and 54
    /// converging on one value. Solving the pair gives this grey at this strength, and it is what
    /// makes a live interface read as an unavailable one rather than as a dark one.
    /// </remarks>
    private static readonly Color ColumnGrey = new("1a1a1a");

    private const float ColumnGreyStrength = 0.866f;

    /// <summary>
    /// How far the grey reaches at the very top of the column.
    /// </summary>
    /// <remarks>
    /// Less than the rest of it. The map's band comes out lighter at its head than at its foot in
    /// the reference — 55 against 41 along the frame, which is one colour under two strengths — so
    /// the map stays a map under the mark instead of going flat.
    /// </remarks>
    private const float ColumnGreyAtHead = 0.707f;

    /// <summary>
    /// How much of the column is left at the foot of the screen.
    /// </summary>
    /// <remarks>
    /// The grey fades to black down the column, from nothing at the fame bar to this at the bottom
    /// edge — which is what buries the worn slots, the inventory and the party list under the
    /// button stack while leaving the bars above it legible.
    /// </remarks>
    private const float ColumnFade = 0.76f;

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

        var layout = new HudLayout(Size);
        var column = layout.Column;

        float width = Mathf.Round(column.Size.X - ColumnInset * 2f);
        float left = Mathf.Round(column.Position.X + ColumnInset);

        // The mark stands in the map's band, which is the top of the column.
        var band = layout.Minimap;
        _logo.LogoWidth = Mathf.Round(band.Size.X * LogoShare);
        _logo.Position = new Vector2(
            Mathf.Round(band.Position.X + (band.Size.X - _logo.Size.X) / 2f),
            Mathf.Round(band.Position.Y + (band.Size.Y - _logo.Size.Y) / 2f));

        _continue.Position = new Vector2(left, Mathf.Round(Size.Y - BottomMargin - PlateHeight));
        _continue.Size = new Vector2(width, PlateHeight);

        float quit = _continue.Position.Y - CommitGap - PlateHeight;
        _quit.Position = new Vector2(left, Mathf.Round(quit));
        _quit.Size = new Vector2(width, PlateHeight);

        // The stack climbs to the mana bar and no further. At the reference height that ceiling is
        // never reached -- the pitch lands the top plate a pixel under it -- but a shorter screen
        // moves the whole stack up with the foot it hangs from, and without this the top of it
        // would end up over the bars it is supposed to leave readable.
        float room = quit - layout.ManaBar.Position.Y;
        float pitch = Mathf.Clamp(room / _stack.Length, PlateHeight + Style.ButtonShadowHeight, ButtonPitch);

        for (int i = 0; i < _stack.Length; i++)
        {
            _stack[i].Position = new Vector2(
                left, Mathf.Round(quit - (_stack.Length - i) * pitch));

            _stack[i].Size = new Vector2(width, PlateHeight);
        }

        // The version lines are centred over the world rather than over the screen, so the column
        // the menu is standing in does not push them off centre.
        _footer.Position = Vector2.Zero;
        _footer.Size = new Vector2(column.Position.X, Size.Y);
    }

    /// <summary>
    /// The two treatments the menu puts over what is already on the screen.
    /// </summary>
    /// <remarks>
    /// The world and everything drawn over it go under a flat wash. The column gets its own,
    /// because it is not being pushed into the background — it is the page the menu is printed on,
    /// and the reference greys it and then fades it out towards the foot rather than dimming it
    /// evenly. Both are painted before the children, which is what keeps the mark, the plates and
    /// the build lines at full strength on top.
    /// </remarks>
    public override void _Draw()
    {
        if (Size.X <= 0f || Size.Y <= 0f)
            return;

        var layout = new HudLayout(Size);
        var column = layout.Column;

        DrawRect(
            new Rect2(0f, 0f, column.Position.X, Size.Y),
            new Color(0f, 0f, 0f, 1f - Survives));

        var map = layout.Minimap;
        Band(column.Position.X, column.End.X, map.Position.Y, map.End.Y,
            ColumnGrey with { A = ColumnGreyAtHead }, ColumnGrey with { A = ColumnGreyStrength });

        DrawRect(
            new Rect2(column.Position.X, map.End.Y, column.Size.X, Size.Y - map.End.Y),
            ColumnGrey with { A = ColumnGreyStrength });

        float top = layout.FameBar.Position.Y;
        if (Size.Y > top)
            Band(column.Position.X, column.End.X, top, Size.Y,
                new Color(0f, 0f, 0f, 0f), new Color(0f, 0f, 0f, ColumnFade));
    }

    /// <summary>
    /// A rectangle whose colour runs from one value at its top edge to another at its bottom.
    /// </summary>
    /// <remarks>
    /// A four-cornered polygon rather than a texture: Godot interpolates a colour per vertex across
    /// the quad, which is a gradient with nothing to build, cache or resize.
    /// </remarks>
    private void Band(float left, float right, float top, float bottom, Color head, Color foot)
    {
        if (bottom <= top || right <= left)
            return;

        DrawPolygon(
            new[]
            {
                new Vector2(left, top), new Vector2(right, top),
                new Vector2(right, bottom), new Vector2(left, bottom),
            },
            new[] { head, head, foot, foot });
    }

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
