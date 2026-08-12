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
    /// <summary>The darkest ground, behind everything.</summary>
    public static readonly Color Void = new("101016");

    /// <summary>Panel fills, lighter at the top than the bottom.</summary>
    public static readonly Color PanelTop = new("2c2a31");

    public static readonly Color PanelBottom = new("1c1a20");

    /// <summary>A control at rest, and the same control under the pointer.</summary>
    public static readonly Color ControlTop = new("3a3742");

    public static readonly Color ControlBottom = new("26242c");
    public static readonly Color ControlHoverTop = new("4b4757");
    public static readonly Color ControlHoverBottom = new("332f3c");

    /// <summary>The accent. The original's player-name gold, which is the colour it means "you" with.</summary>
    public static readonly Color Gold = new("fcdf00");

    public static readonly Color GoldDim = new("8a7a1e");

    /// <summary>A cooler second accent, for anything that is not the main action.</summary>
    public static readonly Color Steel = new("7f9bb5");

    public static readonly Color Text = new("efece6");

    /// <summary>The original's tab grey, used for anything that only frames the words that matter.</summary>
    public static readonly Color Muted = new("9b9898");

    public static readonly Color Faint = new("6b6a6a");

    public static readonly Color Danger = new("e0574f");

    public static readonly Color Good = new("6fdc6f");

    /// <summary>Borders, which are the accent held well back.</summary>
    public static readonly Color Edge = new("55505f");

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
