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
    //
    // Revision two replaced the whole palette: the translucent black chrome of the first pass is
    // gone and the interface is opaque grey plate with hard one-pixel edges, which is what the
    // pixel font and the bevelled buttons need behind them to read as one thing.

    /// <summary>Panel chrome: opaque grey, not translucent black.</summary>
    public static readonly Color Panel = new("474747");

    public static readonly Color PanelInset = new("3a3a3a");

    /// <summary>The one-pixel outer border every panel carries.</summary>
    public static readonly Color PanelEdge = new("1e1e1e");

    public static readonly Color Divider = new("5c5c5c");

    /// <summary>The minimap, which is drawn on rather than filled.</summary>
    public static readonly Color PanelSolid = Colors.Black;

    // Slots, which inverted in revision two: dark plates with a light border rather than the other
    // way round.
    public static readonly Color Slot = new("3a3a3a");

    public static readonly Color SlotBorder = new("9a9a9a");

    /// <summary>The border under the pointer, and for the hundred milliseconds after a key press.</summary>
    public static readonly Color SlotBorderHi = new("e2e2e2");

    /// <summary>The large number an empty slot carries in the middle of itself.</summary>
    public static readonly Color SlotEmptyNumber = new("8f8f8f");

    /// <summary>
    /// The plate under an item this class cannot equip.
    /// </summary>
    /// <remarks>
    /// The original's restricted-use indicator, in its colour: a dark red behind the item rather
    /// than a mark beside it, so a bag full of loot for somebody else reads at a glance.
    /// </remarks>
    public static readonly Color SlotRestricted = new("5c1d1d");

    // Bars. All four read the same way: an edge, a track, a flat fill, and a highlight along the
    // top of the fill that moves with it.
    public static readonly Color FameFill = new("f2a01c");

    public static readonly Color HpFill = new("e02b2b");
    public static readonly Color MpFill = new("3f7fd0");
    public static readonly Color XpFill = new("5fbb2e");
    public static readonly Color BarTrack = new("2b2b2b");
    public static readonly Color BarEdge = new("141414");
    public static readonly Color BarHighlight = new(1f, 1f, 1f, 0.28f);

    // Buttons: a flat face with a two-tone one-pixel bevel, inverted while held.
    public static readonly Color ButtonFace = new("5a5a5a");

    public static readonly Color ButtonBevelHigh = new("7d7d7d");
    public static readonly Color ButtonBevelLow = new("2e2e2e");
    public static readonly Color ButtonHover = new("6b6b6b");

    /// <summary>The one saturated thing in the top left corner, and the one asking for a click.</summary>
    public static readonly Color ButtonPromo = new("f2a01c");

    public static readonly Color TierNormal = new("ffffff");

    /// <summary>Untiered and unique grades, which are the ones worth stopping on.</summary>
    public static readonly Color TierSpecial = new("ff8c1a");

    public static readonly Color PotionCount = new("4ce04c");
    public static readonly Color Guild = new("5cd05c");
    public static readonly Color ChatName = new("5cd05c");
    public static readonly Color IconFame = new("e8622a");
    public static readonly Color IconGold = new("d6dc3f");
    public static readonly Color TabActive = new("cfcfcf");
    public static readonly Color TabIdle = new("3f3f3f");

    public static readonly Color Text = new("ffffff");
    public static readonly Color TextDim = new("b4b4b4");

    /// <summary>The bar under a sprite in the world.</summary>
    public static readonly Color EntityHp = new("4cd137");

    public static readonly Color Star = new("ffd54a");

    /// <summary>
    /// Minimap marks, which follow the game's own convention rather than a single token.
    /// </summary>
    /// <remarks>
    /// Yellow for other players, green for guildmates, red for anything hostile, blue for a way
    /// out, and white for whatever the quest is pointing at. Read at a glance and never legended,
    /// which only works because it is the same code every player already knows from the original.
    /// There is no purple for a party: this server has no party system to colour.
    /// </remarks>
    public static readonly Color BlipPlayer = new("ffc83d");

    public static readonly Color BlipGuild = new("5cd05c");
    public static readonly Color BlipEnemy = new("e02b2b");
    public static readonly Color BlipPortal = new("5b9bd5");
    public static readonly Color BlipQuest = new("ff4d4d");

    /// <summary>Gods, told apart from the rank and file by a warmer red.</summary>
    public static readonly Color BlipGod = new("ff8a3d");

    /// <summary>Heroes of Oryx and encounter bosses, in his own purple.</summary>
    public static readonly Color BlipBoss = new("b46ce0");

    /// <summary>Party portrait borders, which carry the member's state.</summary>
    public static readonly Color StatusOk = new("4cd137");

    public static readonly Color StatusLow = new("e02b2b");
    public static readonly Color StatusDead = new("6b6a6a");

    // Secondary interface: the panels that open over the world. A gold frame around a near-black
    // body, deliberately heavier than the flat grey chrome that is always on screen -- the contrast
    // is what says "this opened" rather than "this was always here".
    public static readonly Color ModalFrame = new("b4913f");

    public static readonly Color ModalFrameDark = new("6b5423");
    public static readonly Color ModalBody = new("262626");
    public static readonly Color ModalHeader = new("1b1b1b");
    public static readonly Color ModalBand = new("333333");
    public static readonly Color ModalStripe = new(1f, 1f, 1f, 0.04f);

    public static readonly Color StatLabel = new("c9b184");
    public static readonly Color StatValue = new("ffffff");

    /// <summary>An attribute that has reached its class ceiling, which is the point of the grid.</summary>
    public static readonly Color StatValueMax = new("ffd54a");

    public static readonly Color StatBonus = new("5cd05c");
    public static readonly Color StatPenalty = new("e05050");
    public static readonly Color StatNumber = new("5cd05c");

    /// <summary>
    /// The outline under every piece of text.
    /// </summary>
    /// <remarks>
    /// A hard one-pixel outline on all four sides, not a soft shadow offset down and right. The
    /// world under the overlay is any colour at all, and an outline is what keeps a pixel face
    /// legible over a sunlit floor without softening its edges.
    /// </remarks>
    public static readonly Color TextOutline = Colors.Black;

    // The type scale, at one times. Everything is drawn at these sizes and the whole canvas is
    // scaled by a whole or half step, so a glyph is never resampled.
    public const int FontName = 16;
    public const int FontBody = 12;
    public const int FontSmall = 12;
    public const int FontTag = 10;

    /// <summary>Every cluster's margin from the edge of the viewport.</summary>
    public const int EdgeMargin = 20;

    private static Font _pixel;

    /// <summary>
    /// The face the interface is set in.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The brief asks for a bitmap font. There is not one in the tree -- the extracted assets are
    /// all world artwork, and the original's interface type is a system face -- so this is the
    /// engine's fallback with everything that softens a glyph turned off: no antialiasing, no
    /// subpixel positioning, hinting on. At the sizes above, on a canvas that only ever scales by a
    /// whole or half step, that gives hard-edged text with no resampling in it.
    /// </para>
    /// <para>
    /// Dropping a real pixel face in is one line: load it here and everything follows, because
    /// nothing else in the interface names a font.
    /// </para>
    /// </remarks>
    public static Font Pixel
    {
        get
        {
            if (_pixel != null)
                return _pixel;

            _pixel = ThemeDB.FallbackFont;

            if (ThemeDB.FallbackFont?.Duplicate() is FontFile crisp)
            {
                crisp.Antialiasing = TextServer.FontAntialiasing.None;
                crisp.SubpixelPositioning = TextServer.SubpixelPositioning.Disabled;
                crisp.Hinting = TextServer.Hinting.Normal;
                crisp.ForceAutohinter = true;
                _pixel = crisp;
            }

            return _pixel;
        }
    }

    /// <summary>Sets a label's face, size, colour and outline in one call.</summary>
    public static T Typeset<T>(this T label, int size, Color colour)
        where T : Label
    {
        label.AddThemeFontOverride("font", Pixel);
        label.AddThemeFontSizeOverride("font_size", size);
        label.AddThemeColorOverride("font_color", colour);

        // A real outline rather than a shadow. Two pixels of outline size is what Godot needs to
        // put one solid pixel on each of the four sides.
        label.AddThemeColorOverride("font_outline_color", TextOutline);
        label.AddThemeConstantOverride("outline_size", 2);
        label.AddThemeConstantOverride("shadow_offset_x", 0);
        label.AddThemeConstantOverride("shadow_offset_y", 0);

        label.MouseFilter = Control.MouseFilterEnum.Ignore;
        return label;
    }

    /// <summary>
    /// Draws a string with the interface's outline, for the parts that draw rather than label.
    /// </summary>
    /// <param name="at">The text's baseline, at its left edge unless an alignment says otherwise.</param>
    public static void DrawOutlined(
        this CanvasItem into, Vector2 at, string text, int size, Color colour,
        HorizontalAlignment alignment = HorizontalAlignment.Left, float width = -1f)
    {
        into.DrawStringOutline(Pixel, at, text, alignment, width, size, 2, TextOutline);
        into.DrawString(Pixel, at, text, alignment, width, size, colour);
    }

    /// <summary>How wide a string is in the interface's face, for laying text out by hand.</summary>
    /// <summary>
    /// Width of a string, remembered rather than re-measured.
    /// </summary>
    /// <remarks>
    /// Measuring is shaping: the font server walks the string and lays out every glyph, and it
    /// costs the same whether the answer is new or the four hundredth identical copy this frame.
    /// The interface asks the same questions over and over -- the same names, the same numbers, a
    /// room full of monsters shouting the same line -- so the answers are kept. A few thousand of
    /// them is a few hundred kilobytes against a client already holding three hundred megabytes of
    /// texture, and it turns a per-frame cost that scales with what is on screen into one that
    /// scales with how many *different* things are on screen.
    /// </remarks>
    public static float Measure(string text, int size)
    {
        if (string.IsNullOrEmpty(text))
            return 0f;

        var key = (text, size);
        if (_measured.TryGetValue(key, out float width))
            return width;

        width = Pixel.GetStringSize(text, HorizontalAlignment.Left, -1, size).X;

        // Cleared wholesale rather than evicted one at a time: it only grows when the game starts
        // showing text it has never shown, which is not something that happens in a steady state.
        if (_measured.Count >= MostMeasured)
            _measured.Clear();

        _measured[key] = width;
        return width;
    }

    private const int MostMeasured = 4096;

    private static readonly System.Collections.Generic.Dictionary<(string, int), float> _measured = new();

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
