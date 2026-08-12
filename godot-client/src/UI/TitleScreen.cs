using System;
using Godot;

namespace Hendra.UI;

/// <summary>
/// The first screen: the title art, and three ways off it.
/// </summary>
/// <remarks>
/// <para>
/// The original's <c>TitleView</c> stacks four layers — a live map behind, a dark wash over it, the
/// title graphic, and a bar of three buttons: Play in the centre, Servers to the left, Account to
/// the right — with version text on a band near the bottom of the eight-hundred-by-six-hundred
/// stage.
/// </para>
/// <para>
/// The live map behind is not reproduced. It exists to show the game moving before you have signed
/// in, which needs a whole second world running against no server; the title art is opaque over
/// almost all of it anyway.
/// </para>
/// <para>
/// The art is 800 by 600 and is scaled to fit whatever the window is, keeping its proportions. It
/// was authored for that size and stretching it to a wide monitor is worse than letterboxing it.
/// </para>
/// </remarks>
public partial class TitleScreen : Control
{
    private TextureRect _art;
    private Control _buttons;

    /// <summary>Raised when the player wants to sign in and pick a character.</summary>
    public event Action PlayPressed;

    /// <summary>Raised when the player wants to leave.</summary>
    public event Action QuitPressed;

    /// <summary>Raised when the player wants the account panel.</summary>
    public event Action AccountPressed;

    public override void _Ready()
    {
        this.FillScreen();

        var backdrop = new ColorRect { Color = new Color(0.04f, 0.04f, 0.05f) };
        backdrop.SetAnchorsPreset(LayoutPreset.FullRect);
        backdrop.MouseFilter = MouseFilterEnum.Ignore;
        AddChild(backdrop);

        _art = new TextureRect
        {
            Texture = App.ServiceLocator.Assets?.GetImage("TitleScreen"),
            ExpandMode = TextureRect.ExpandModeEnum.IgnoreSize,
            // Covers the window rather than letterboxing inside it: the art is a wash and a
            // wordmark, so cropping its edges costs nothing and black bars down both sides of a
            // wide monitor look like a fault.
            StretchMode = TextureRect.StretchModeEnum.KeepAspectCovered,
            MouseFilter = MouseFilterEnum.Ignore,
        };
        _art.SetAnchorsPreset(LayoutPreset.FullRect);
        AddChild(_art);

        // The buttons sit on the art's own band rather than the window's, so they stay where the
        // artwork expects them however the window is shaped.
        _buttons = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_buttons);

        // A centre container rather than a centre anchor: the anchor puts the bar's corner on the
        // middle of the screen, which reads as centred only until you look at it.
        var centre = new CenterContainer();
        centre.SetAnchorsPreset(LayoutPreset.FullRect);
        centre.MouseFilter = MouseFilterEnum.Ignore;
        _buttons.AddChild(centre);

        var bar = new HBoxContainer();
        bar.AddThemeConstantOverride("separation", 18);
        centre.AddChild(bar);

        // Account left, Play centre, Quit right. The original puts Servers here too, but its
        // Servers screen picks between named worlds the app server hands back -- and this client
        // already shows that list once you have signed in, which is the only point at which it
        // knows what the worlds are.
        bar.AddChild(MenuButton("Account", () => AccountPressed?.Invoke()));
        bar.AddChild(MenuButton("Play", () => PlayPressed?.Invoke(), primary: true));
        bar.AddChild(MenuButton("Quit", () => QuitPressed?.Invoke()));

        GetViewport().SizeChanged += PlaceButtons;
        PlaceButtons();
    }

    /// <summary>
    /// Puts the button bar along the bottom of the window.
    /// </summary>
    /// <remarks>
    /// The original pins its bar to a band on an 800 by 600 stage, which works because the stage is
    /// the window. Here the art covers the window and overflows it, so a position measured from the
    /// artwork lands off the bottom edge on anything wider than four by three. The window is the
    /// thing the player can see, so the window is what it is measured from.
    /// </remarks>
    private void PlaceButtons()
    {
        const float BarHeight = 60f;
        const float BottomMargin = 46f;

        var window = GetViewportRect().Size;
        if (window.X <= 0f || window.Y <= 0f)
            return;

        _buttons.Position = new Vector2(0f, window.Y - BarHeight - BottomMargin);
        _buttons.Size = new Vector2(window.X, BarHeight);
    }

    private static Button MenuButton(string text, Action pressed, bool primary = false)
    {
        var button = new Button
        {
            Text = text,
            CustomMinimumSize = new Vector2(primary ? 170 : 130, primary ? 44 : 38),
        };

        button.AddThemeFontSizeOverride("font_size", primary ? 22 : 17);
        button.Pressed += pressed;
        return button;
    }
}
