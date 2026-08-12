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
    /// <summary>The band the original puts its version text on, as a fraction of the art's height.</summary>
    private const float BottomBand = 589.45f / 600f;

    private TextureRect _art;
    private Control _buttons;

    /// <summary>Raised when the player wants to sign in and pick a character.</summary>
    public event Action PlayPressed;

    /// <summary>Raised when the player wants the server list.</summary>
    public event Action ServersPressed;

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
            StretchMode = TextureRect.StretchModeEnum.KeepAspectCentered,
            MouseFilter = MouseFilterEnum.Ignore,
        };
        _art.SetAnchorsPreset(LayoutPreset.FullRect);
        AddChild(_art);

        // The buttons sit on the art's own band rather than the window's, so they stay where the
        // artwork expects them however the window is shaped.
        _buttons = new Control { MouseFilter = MouseFilterEnum.Ignore };
        AddChild(_buttons);

        var bar = new HBoxContainer();
        bar.AddThemeConstantOverride("separation", 18);
        bar.SetAnchorsPreset(LayoutPreset.Center);
        _buttons.AddChild(bar);

        // Servers left, Play centre, Account right -- the original's arrangement.
        bar.AddChild(MenuButton("Servers", () => ServersPressed?.Invoke()));
        bar.AddChild(MenuButton("Play", () => PlayPressed?.Invoke(), primary: true));
        bar.AddChild(MenuButton("Account", () => AccountPressed?.Invoke()));

        GetViewport().SizeChanged += PlaceButtons;
        PlaceButtons();
    }

    /// <summary>
    /// Puts the button bar on the artwork's bottom band, wherever the artwork ended up.
    /// </summary>
    private void PlaceButtons()
    {
        var window = GetViewportRect().Size;
        if (window.X <= 0f || window.Y <= 0f)
            return;

        // Where the 800x600 art landed once it was fitted to the window.
        float scale = Mathf.Min(window.X / 800f, window.Y / 600f);
        var art = new Vector2(800f * scale, 600f * scale);
        var origin = (window - art) / 2f;

        // The bar sits *above* the band rather than on it: the band is where the original puts its
        // version text, and a bar centred on it hangs off the bottom of the artwork.
        const float BarHeight = 88f;
        _buttons.Position = new Vector2(origin.X, origin.Y + art.Y * BottomBand - BarHeight);
        _buttons.Size = new Vector2(art.X, BarHeight);
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
