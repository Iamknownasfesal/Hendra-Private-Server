using Godot;

namespace Hendra.UI;

/// <summary>
/// The interface's palette and its theme.
/// </summary>
/// <remarks>
/// <para>
/// One place that decides what the game looks like, rather than a colour invented at each call
/// site. The theme is applied once at the window, so every control the port creates — the ones in
/// the menus, the ones in the panels, the ones that have not been written yet — picks it up without
/// being told about it.
/// </para>
/// <para>
/// The palette is drawn out of the original's own: its panel grey <c>0x242222</c> is the ink these
/// darks are mixed from, its tab grey <c>0x6B6A6A</c> is the muted text, and the gold is the yellow
/// it puts on player names. What is new is the depth — the original is flat fills and hairlines
/// because Flash's display list made anything else expensive, and that constraint is gone.
/// </para>
/// </remarks>
public static class Style
{
    // The brief's tokens, verbatim. These are the contract every cluster is measured against, so
    // they are transcribed rather than interpreted -- a colour invented here is a colour that has
    // to be found again later when something does not match the reference.

    /// <summary>Player card and chat: black at 55 percent.</summary>
    public static readonly Color Panel = new(0f, 0f, 0f, 0.55f);

    /// <summary>The minimap, which is solid.</summary>
    public static readonly Color PanelSolid = Colors.Black;

    /// <summary>Hotbar and equipment slots, which are near-white in the reference.</summary>
    public static readonly Color Slot = new("f2f2f2");

    public static readonly Color SlotBorder = new("b9b9b9");
    public static readonly Color SlotEmpty = new("ffffff");

    public static readonly Color BlueButton = new("2b7fd4");
    public static readonly Color BlueButtonHover = new("3a92e8");
    public static readonly Color BlueButtonActive = new("1f66ad");

    public static readonly Color XpFill = new("5fbb2e");
    public static readonly Color XpTrack = new("4a4a4a");
    public static readonly Color HpFill = new("d02020");
    public static readonly Color HpTrack = new("5a1414");
    public static readonly Color MpFill = new("5b86bd");
    public static readonly Color MpTrack = new("26364d");

    /// <summary>The bar under a sprite in the world.</summary>
    public static readonly Color EntityHp = new("4cd137");

    public static readonly Color Gem = new("f0912b");
    public static readonly Color Coin = new("ffd84a");
    public static readonly Color Star = new("ffd54a");

    /// <summary>The star beside the account rating, which is blue rather than gold.</summary>
    public static readonly Color StarPremium = new("4aa8e8");

    public static readonly Color MinimapBlip = new("ffc83d");
    public static readonly Color ChatName = new("62dd52");
    public static readonly Color Text = new("ffffff");
    public static readonly Color TextDim = new("cfcfcf");

    /// <summary>
    /// The shadow under every piece of text.
    /// </summary>
    /// <remarks>
    /// One pixel down-right at eight tenths black. The world under the overlay is any colour at
    /// all, and this is what keeps a white label legible over a sunlit floor.
    /// </remarks>
    public static readonly Color TextShadow = new(0f, 0f, 0f, 0.8f);

    /// <summary>Party dot states, which the brief asks to be tokens rather than hardcoded.</summary>
    public static readonly Color StatusOk = new("4cd137");

    public static readonly Color StatusLow = new("d02020");
    public static readonly Color StatusDead = new("6b6a6a");

    // The type scale, at the reference resolution.
    public const int FontName = 26;
    public const int FontBody = 17;
    public const int FontSmall = 15;
    public const int FontSlotNumber = 13;

    /// <summary>Every cluster's margin from the edge of the viewport.</summary>
    public const int EdgeMargin = 20;

    // Names kept from the previous palette so the screens that are not part of this brief -- the
    // title, the character select, the tooltip -- keep working while the HUD is rebuilt against the
    // tokens above.
    public static readonly Color Void = new("101016");
    public static readonly Color PanelTop = new("2c2a31");
    public static readonly Color PanelBottom = new("1c1a20");
    public static readonly Color ControlTop = new("3a3742");
    public static readonly Color ControlBottom = new("26242c");
    public static readonly Color ControlHoverTop = new("4b4757");
    public static readonly Color ControlHoverBottom = new("332f3c");
    public static readonly Color Gold = new("fcdf00");
    public static readonly Color GoldDim = new("8a7a1e");
    public static readonly Color Steel = new("7f9bb5");
    public static readonly Color Muted = new("9b9898");
    public static readonly Color Faint = new("6b6a6a");
    public static readonly Color Danger = new("e0574f");
    public static readonly Color Good = new("6fdc6f");
    public static readonly Color Edge = new("55505f");

    /// <summary>The size the interface is measured against.</summary>
    private static readonly Vector2 ReferenceSize = new(1920f, 1080f);

    /// <summary>
    /// Scales the interface for the window it is in.
    /// </summary>
    /// <remarks>
    /// Every measurement in the layout is taken at 1920 by 1080. Rather than making each of them
    /// resolution-aware, the whole interface is scaled by the smaller of the two ratios, so it
    /// keeps its proportions on any shape of screen -- including an ultrawide, where scaling by
    /// width alone would make everything enormous.
    /// </remarks>
    public static void ApplyScale(Window window)
    {
        if (window == null)
            return;

        var size = window.Size;
        if (size.X <= 0 || size.Y <= 0)
            return;

        float scale = Mathf.Min(size.X / ReferenceSize.X, size.Y / ReferenceSize.Y);
        window.ContentScaleFactor = Mathf.Clamp(scale, 0.75f, 1.5f);
    }

    /// <summary>
    /// Builds the theme for the whole interface.
    /// </summary>
    /// <remarks>
    /// Godot's defaults are a developer tool's defaults — flat blue-grey boxes sized for an editor.
    /// Restyling them centrally is what stops every screen looking like a settings dialog, and it
    /// reaches the controls this port does not draw itself: the dropdown, the text fields, the
    /// scrollbars, the checkbox.
    /// </remarks>
    public static Theme Build()
    {
        var theme = new Theme();

        StyleLineEdit(theme);
        StyleOptionButton(theme);
        StyleCheckBox(theme);
        StyleScrollbars(theme);
        StyleSeparators(theme);
        StyleTooltips(theme);
        StyleLabels(theme);
        StyleSliders(theme);

        return theme;
    }

    /// <summary>A filled box with a border, graded from top to bottom.</summary>
    public static StyleBoxFlat Box(Color top, Color bottom, Color border, int radius = 3, int borderWidth = 1)
    {
        var box = new StyleBoxFlat
        {
            BorderColor = border,
            ContentMarginLeft = 10,
            ContentMarginRight = 10,
            ContentMarginTop = 5,
            ContentMarginBottom = 5,
        };

        box.SetBorderWidthAll(borderWidth);
        box.SetCornerRadiusAll(radius);

        // A StyleBox has one fill, so the grade is faked by mixing the pair. It is enough to keep a
        // control from reading as a flat rectangle against a flat panel.
        box.ShadowColor = new Color(0f, 0f, 0f, 0.35f);
        box.ShadowSize = 3;
        box.ShadowOffset = new Vector2(0f, 2f);
        box.BgColor = bottom.Lerp(top, 0.5f);
        return box;
    }

    private static void StyleLineEdit(Theme theme)
    {
        var rest = Box(new Color("15141a"), new Color("101015"), Edge);
        rest.ContentMarginLeft = 9;
        rest.ContentMarginRight = 9;
        rest.ContentMarginTop = 7;
        rest.ContentMarginBottom = 7;

        var focused = (StyleBoxFlat)rest.Duplicate();
        focused.BorderColor = Gold with { A = 0.75f };
        focused.SetBorderWidthAll(1);

        theme.SetStylebox("normal", "LineEdit", rest);
        theme.SetStylebox("focus", "LineEdit", focused);
        theme.SetStylebox("read_only", "LineEdit", rest);
        theme.SetColor("font_color", "LineEdit", Text);
        theme.SetColor("font_placeholder_color", "LineEdit", Faint);
        theme.SetColor("caret_color", "LineEdit", Gold);
        theme.SetColor("selection_color", "LineEdit", Gold with { A = 0.25f });
        theme.SetFontSize("font_size", "LineEdit", 15);
    }

    private static void StyleOptionButton(Theme theme)
    {
        var rest = Box(ControlTop, ControlBottom, Edge);
        var hover = Box(ControlHoverTop, ControlHoverBottom, Gold with { A = 0.5f });
        var pressed = Box(ControlBottom, ControlBottom, Gold with { A = 0.6f });

        foreach (string type in new[] { "OptionButton", "MenuButton", "PopupMenu" })
        {
            theme.SetStylebox("normal", type, rest);
            theme.SetStylebox("hover", type, hover);
            theme.SetStylebox("pressed", type, pressed);
            theme.SetStylebox("focus", type, new StyleBoxEmpty());
            theme.SetColor("font_color", type, Text);
            theme.SetColor("font_hover_color", type, Colors.White);
            theme.SetFontSize("font_size", type, 15);
        }

        theme.SetStylebox("panel", "PopupMenu", Box(PanelTop, PanelBottom, Edge));
        theme.SetColor("font_color", "PopupMenu", Text);
        theme.SetColor("font_hover_color", "PopupMenu", Gold);
    }

    private static void StyleCheckBox(Theme theme)
    {
        theme.SetStylebox("normal", "CheckBox", new StyleBoxEmpty());
        theme.SetStylebox("hover", "CheckBox", new StyleBoxEmpty());
        theme.SetStylebox("pressed", "CheckBox", new StyleBoxEmpty());
        theme.SetStylebox("focus", "CheckBox", new StyleBoxEmpty());
        theme.SetColor("font_color", "CheckBox", Muted);
        theme.SetColor("font_hover_color", "CheckBox", Text);
        theme.SetFontSize("font_size", "CheckBox", 14);
    }

    private static void StyleScrollbars(Theme theme)
    {
        foreach (string type in new[] { "VScrollBar", "HScrollBar" })
        {
            theme.SetStylebox("scroll", type, Box(new Color("14131a"), new Color("14131a"), new Color(0, 0, 0, 0), 2, 0));
            theme.SetStylebox("grabber", type, Box(ControlTop, ControlBottom, Edge, 2));
            theme.SetStylebox("grabber_highlight", type, Box(ControlHoverTop, ControlHoverBottom, GoldDim, 2));
            theme.SetStylebox("grabber_pressed", type, Box(GoldDim, GoldDim, Gold, 2));
        }
    }

    private static void StyleSeparators(Theme theme)
    {
        var line = new StyleBoxLine { Color = Edge with { A = 0.6f }, Thickness = 1 };
        theme.SetStylebox("separator", "HSeparator", line);

        var upright = new StyleBoxLine { Color = Edge with { A = 0.6f }, Thickness = 1, Vertical = true };
        theme.SetStylebox("separator", "VSeparator", upright);
    }

    private static void StyleTooltips(Theme theme)
    {
        // The item tooltip draws its own panel; this is for the plain ones, which should at least
        // belong to the same game.
        theme.SetStylebox("panel", "TooltipPanel", Box(PanelTop, PanelBottom, Edge, 4));
        theme.SetColor("font_color", "TooltipLabel", Text);
        theme.SetFontSize("font_size", "TooltipLabel", 13);
    }

    private static void StyleLabels(Theme theme)
    {
        theme.SetColor("font_color", "Label", Text);
        theme.SetFontSize("font_size", "Label", 14);

        // A shadow under every label, so text stays readable over the world as well as over a panel.
        theme.SetColor("font_shadow_color", "Label", new Color(0f, 0f, 0f, 0.7f));
        theme.SetConstant("shadow_offset_x", "Label", 1);
        theme.SetConstant("shadow_offset_y", "Label", 1);
        theme.SetConstant("shadow_outline_size", "Label", 1);
    }

    private static void StyleSliders(Theme theme)
    {
        foreach (string type in new[] { "HSlider", "VSlider" })
        {
            theme.SetStylebox("slider", type, Box(new Color("15141a"), new Color("15141a"), Edge, 3, 1));
            theme.SetStylebox("grabber_area", type, Box(GoldDim, GoldDim, GoldDim, 3, 0));
            theme.SetStylebox("grabber_area_highlight", type, Box(Gold, Gold, Gold, 3, 0));
        }
    }
}
