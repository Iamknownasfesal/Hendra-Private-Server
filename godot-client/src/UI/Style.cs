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

    /// <summary>
    /// Panel chrome: the grey the HUD column is filled with, behind the bars and the slot grids.
    /// </summary>
    public static readonly Color Panel = new("363636");

    /// <summary>
    /// The near-black a slot grid is bedded into, and the body colour of every floating panel.
    /// It is the darkest of the three surfaces and reads as the hole the plates sit in.
    /// </summary>
    public static readonly Color PanelInset = new("242222");

    /// <summary>The one-pixel outer border every panel carries.</summary>
    public static readonly Color PanelEdge = new("1e1e1e");

    public static readonly Color Divider = new("5c5c5c");

    /// <summary>The minimap, which is drawn on rather than filled.</summary>
    public static readonly Color PanelSolid = Colors.Black;

    // ─── the value ladder ─────────────────────────────────────────────────────────────────────
    //
    // Revision two had this backwards twice over: dark plates on a lighter board, every surface
    // within a few points of every other. Desaturated, that grid had no structure at all -- one flat
    // field with some texture in it. Revision five sets three plateaus instead, and the rule behind
    // them matters more than the numbers: any two touching surfaces differ by at least twenty points
    // of luminance. Near-black board, mid-grey plates, bright borders. If a later colour change
    // closes one of those gaps, the grid stops carrying the layout and that is a bug, not a taste.

    /// <summary>A slot's plate. Full and empty slots carry the same grey; only the border differs.</summary>
    public static readonly Color SlotEmpty = new("575757");

    public static readonly Color SlotEmptyEdge = new("616161");

    /// <summary>A slot with something in it. The same plate as an empty one.</summary>
    public static readonly Color Slot = new("575757");

    /// <summary>Four pixels, one step lighter than the plate. This is what draws the grid.</summary>
    public static readonly Color SlotBorder = new("616161");

    /// <summary>The border under the pointer, and for the hundred milliseconds after a key press.</summary>
    public static readonly Color SlotBorderHi = new("e2e2e2");

    /// <summary>A row nobody has bought yet.</summary>
    public static readonly Color SlotLocked = new("2a2a2a");

    /// <summary>The padlock on it, which has to clear its own plate.</summary>
    public static readonly Color SlotLockedIcon = new("6a6a6a");

    /// <summary>The large number an empty slot carries in the middle of itself.</summary>
    public static readonly Color SlotEmptyNumber = new("8f8f8f");

    /// <summary>
    /// The plate under an item this class cannot equip.
    /// </summary>
    /// <remarks>
    /// The original's restricted-use indicator, in its colour: a dark red behind the item rather
    /// than a mark beside it, so a bag full of loot for somebody else reads at a glance. It is the
    /// red of <see cref="SlotHighlight.Red"/> and resolves through the same map -- one mechanism
    /// with one meaning, rather than two that happen to look alike.
    /// </remarks>
    public static readonly Color SlotRestricted = SlotHighlights.RedFill;

    // Bars. All four read the same way: a flat fill, a lighter band along the top few pixels of it,
    // and a dark line along the bottom. Health is green and mana is blue; the red is nowhere in
    // this interface, and a red health bar is the single fastest way to be told apart from it.
    public static readonly Color FameFill = new("df7522");
    public static readonly Color FameFillHigh = new("fb8426");

    public static readonly Color HpFill = new("78c237");
    public static readonly Color HpFillHigh = new("87da3e");
    public static readonly Color MpFill = new("5d81da");
    public static readonly Color MpFillHigh = new("6991f5");
    public static readonly Color XpFill = new("78c237");
    public static readonly Color XpFillHigh = new("87da3e");
    public static readonly Color BarTrack = new("242222");
    public static readonly Color BarEdge = new("201105");
    public static readonly Color BarHighlight = new(1f, 1f, 1f, 0.28f);

    // Buttons. There are four plates and each one means something, so the colour is the label:
    // steel is the ordinary way on, red closes or quits, olive commits, orange spends currency.
    // Every one is a flat face with a lighter band along its top edge and a darker one along the
    // bottom, inverted while held.

    /// <summary>The ordinary button: desaturated blue-grey. Options, Journal, Servers, Enter.</summary>
    public static readonly Color ButtonFace = new("7a9595");

    public static readonly Color ButtonBevelHigh = new("98bfbf");
    public static readonly Color ButtonBevelLow = new("597676");
    public static readonly Color ButtonHover = new("8fabab");

    /// <summary>
    /// The frame around a button plate: four pixels top and bottom, five down each side.
    /// </summary>
    /// <remarks>
    /// Measured off the Options plate in <c>Menu/Esc menu.png</c>, x 26..327 by y 497..548.
    /// Reading only the horizontal runs is what makes this look like a pair of caps, and it is
    /// not: scanning a column at x=28 shows the light colour running the plate's whole height,
    /// so the sides are banded too. Three sides take the light colour and the bottom takes the
    /// dark one, and all four corners are notched out to the background — the same cut-corner
    /// habit the panels have. The same figures hold on the red and olive plates.
    /// </remarks>
    public const int ButtonFrameTop = 4;

    public const int ButtonFrameSide = 5;

    /// <summary>
    /// The hard shadow a plate casts, five rows of it, with no blur.
    /// </summary>
    /// <remarks>
    /// Measured under the Play plate in <c>Menu/New character UI.png</c>. Without it the plates
    /// float on the page instead of sitting on it.
    /// </remarks>
    public static readonly Color ButtonShadow = new("1b1b1b");

    public const int ButtonShadowHeight = 5;

    /// <summary>The way out: Quit, and every panel's Close.</summary>
    public static readonly Color ButtonDanger = new("de2d41");

    public static readonly Color ButtonDangerHigh = new("ff4b67");
    public static readonly Color ButtonDangerLow = new("a81326");

    /// <summary>The action a screen exists for: Continue, Play, Deposit All.</summary>
    public static readonly Color ButtonCommit = new("769300");

    public static readonly Color ButtonCommitHigh = new("a6c037");
    public static readonly Color ButtonCommitLow = new("5c7200");

    /// <summary>The one that costs currency: Upgrade, Buy Character Slot.</summary>
    public static readonly Color ButtonPromo = new("e69b07");

    /// <summary>A switch in its on position. Its off position is <see cref="ButtonDanger"/>.</summary>
    public static readonly Color ToggleOn = new("5a8c25");

    public static readonly Color ToggleOnHigh = new("82bd47");

    /// <summary>
    /// One button plate: the face, and the two capping bands drawn over it.
    /// </summary>
    /// <remarks>
    /// The three always travel together, so a caller picks a plate rather than three colours and
    /// cannot pair a red face with a steel cap. Geometry is in <see cref="ButtonCapHeight"/> and
    /// <see cref="ButtonCapInset"/> and is the same for all of them.
    /// </remarks>
    public readonly record struct ButtonPlate(Color Face, Color High, Color Low);

    /// <summary>The ordinary way on.</summary>
    public static ButtonPlate PlateSteel => new(ButtonFace, ButtonBevelHigh, ButtonBevelLow);

    /// <summary>The action a screen exists for.</summary>
    public static ButtonPlate PlateCommit => new(ButtonCommit, ButtonCommitHigh, ButtonCommitLow);

    /// <summary>Closing and quitting.</summary>
    public static ButtonPlate PlateDanger => new(ButtonDanger, ButtonDangerHigh, ButtonDangerLow);

    public static readonly Color TierNormal = new("ffffff");

    /// <summary>Untiered and unique grades, which are the ones worth stopping on.</summary>
    public static readonly Color TierSpecial = new("ff8c1a");

    /// <summary>
    /// The tag in a slot's bottom right corner, coloured by grade.
    /// </summary>
    /// <remarks>
    /// A lookup and not a conditional, so that a grade the data introduces later needs a line here
    /// and no change at all to anything that draws. A numeric tier -- T0, T13 -- is white and is the
    /// unremarkable case; the untiered grades are the ones worth stopping on and get the accent.
    /// </remarks>
    public static Color TierColour(string tag) => tag switch
    {
        "UT" => new Color("a855f7"),
        "ST" => new Color("ff8c1a"),
        _ => TierNormal,
    };

    public static readonly Color PotionCount = new("4ce04c");
    public static readonly Color Guild = new("5cd05c");
    public static readonly Color ChatName = new("5cd05c");
    public static readonly Color IconFame = new("e8622a");
    public static readonly Color IconGold = new("d6dc3f");
    /// <summary>The plate under the selected tab, which is the lighter of the pair.</summary>
    public static readonly Color TabActive = new("505050");

    /// <summary>An unselected tab, sunk almost into the page behind it.</summary>
    public static readonly Color TabIdle = new("2c2c2c");

    /// <summary>A selected tab's label. Unselected ones take <see cref="TextDim"/>.</summary>
    public static readonly Color TabActiveText = new("ffffff");

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

    // Secondary interface: the panels that open over the world. There is no gold anywhere in it.
    // A panel is the same near-black body as the rest of the interface, lifted off the world by a
    // thin light-grey edge and marked at the corners by small bracket ticks. What says "this
    // opened" is the edge and the shadow, not a frame in a second colour.
    public static readonly Color ModalFrame = new("8d8d8d");

    public static readonly Color ModalFrameDark = new("4a4a4a");

    /// <summary>The board the plates sit on. Near-black, and the same colour as the gutters.</summary>
    public static readonly Color ModalBody = new("242222");

    /// <summary>The title band, a hair cooler than the body it sits on.</summary>
    public static readonly Color ModalHeader = new("242426");

    /// <summary>A full-width band across the grid: the gift and locked dividers, and the sort bar.</summary>
    public static readonly Color ModalBand = new("333333");

    /// <summary>The trough a group of controls sits in, a step above the board and below a plate.</summary>
    public static readonly Color ModalTrough = new("313030");
    public static readonly Color ModalStripe = new(1f, 1f, 1f, 0.04f);

    /// <summary>The caption over an attribute, which is quieter than the number under it.</summary>
    public static readonly Color StatLabel = new("999999");

    /// <summary>An attribute's number: yellow, on its own darker plate.</summary>
    public static readonly Color StatValue = new("fcdf00");

    /// <summary>
    /// An attribute carried past its class ceiling. Blue rather than yellow, which is the one
    /// difference the grid draws: in the reference every attribute at or under its maximum is
    /// yellow, and only the one standing above it turns.
    /// </summary>
    public static readonly Color StatValueMax = new("0095ff");

    /// <summary>The plate an attribute's number sits on, and the border around it.</summary>
    public static readonly Color StatCell = new("333333");

    public static readonly Color StatCellEdge = new("404040");

    public static readonly Color StatBonus = new("0095ff");
    public static readonly Color StatPenalty = new("e05050");

    /// <summary>A tally's value in the statistics list. Saturated green, brighter than the guild green.</summary>
    public static readonly Color StatNumber = new("14eb00");

    /// <summary>
    /// Black, for the outline under a token and the edge around a sprite.
    /// </summary>
    /// <remarks>
    /// It is no longer under every string. See <see cref="DrawText"/> for why: an outline rescues
    /// text from a background that moves, and most of this interface's text sits on an opaque plate
    /// where the outline only closes up the counters and costs legibility.
    /// </remarks>
    public static readonly Color TextOutline = Colors.Black;

    /// <summary>The floor. Nothing in the interface renders below this, except a Tier 3 token.</summary>
    public const int SmallestReadable = 22;

    // ─── the type scale ───────────────────────────────────────────────────────────────────────
    //
    // At 1080p, before the canvas scale, and set from the reference rather than by eye. Every size
    // below was chosen by measuring a cap height in an Exalt screenshot and then finding the
    // nominal size that lands Jersey10's cap on it -- this face draws a cap at roughly half its
    // nominal size, which is the whole reason the old numbers looked reasonable and rendered at
    // half the reference.
    //
    // The measured caps, for anyone re-deriving these:
    //   full-screen page title (Options)          44
    //   page section heading (Cursor, Display)    25
    //   panel title (Vault, Attributes)           22
    //   row value, button label                   21-22
    //   row label                                 20
    //   name, bar label, bar value, stat value    16
    //   list row label                            15
    //   sub-line under a name                     12
    //
    // Four builders independently declared local constants in this range after measuring the same
    // thing, which is what finally made it obvious the shared scale was wrong rather than the face.

    /// <summary>A full-screen page's title. The largest thing in the interface.</summary>
    public const int FontPage = 80;

    /// <summary>A heading standing on a page between groups of rows.</summary>
    public const int FontPageHeading = 46;

    /// <summary>A panel's own name, and the only place a display treatment is allowed.</summary>
    public const int FontTitle = 42;

    /// <summary>A button's label, and the value in a settings row.</summary>
    public const int FontControl = 40;

    /// <summary>A heading inside a panel, and a player's name over the world.</summary>
    public const int FontName = 30;

    public const int FontHeader = 36;

    /// <summary>Reading text: stat rows, item names, chat, everything with words in it.</summary>
    public const int FontBody = 30;

    /// <summary>Secondary text, and the smallest size in the interface.</summary>
    public const int FontSmall = 22;

    /// <summary>
    /// Tier 3 tokens: tier tags, slot numbers, key hints.
    /// </summary>
    /// <remarks>
    /// Below the floor on purpose, and the one exception to it. These are two to four characters
    /// sitting directly on artwork -- <c>T12</c>, <c>UT</c>, <c>3</c> -- and they are matched by
    /// shape rather than read, so the rule that produced the floor does not apply to them. Anything
    /// with a word in it goes at <see cref="FontSmall"/> or above.
    /// </remarks>
    public const int FontTag = 22;

    /// <summary>The number an empty slot carries in the middle of itself.</summary>
    public const int FontEmptySlot = 44;

    /// <summary>Every cluster's margin from the edge of the viewport.</summary>
    public const int EdgeMargin = 20;

    private static Font _face;

    /// <summary>
    /// The face the whole interface is set in.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Jersey 10, a pixel font. This is the fourth face this interface has worn: a bitmap
    /// fallback, then Inter, then Source Sans, and the argument in revision six against a pixel
    /// font -- that it cannot tell an <c>a</c> from an <c>o</c> at fourteen pixels -- turned out to
    /// be an argument against a bad one. Jersey is proportional and rounded rather than blocky, so
    /// it stays a word at small sizes while every stem still lands on a whole pixel.
    /// </para>
    /// <para>
    /// Hinting and subpixel positioning stay off: both would drag stems off the grid the face was
    /// drawn on, and neither buys anything back. Antialiasing is on, which is a reversal. The
    /// argument against it -- that grey where the face means black or white is a smear -- only
    /// holds when one font pixel lands on a whole number of screen pixels. It does at scale 1 and
    /// 2, and it does not at the 0.77 a 1280 by 720 window produces, which is the size the game
    /// opens at. Hard edges there are not crisp, they are simply the wrong pixels; the grey is what
    /// makes the shape survive being resampled.
    /// </para>
    /// <para>
    /// One weight. There is no bold cut, and a pixel face does not want a synthesised one --
    /// emboldening it fattens stems by fractions of a pixel and undoes the grid. What used to be
    /// carried by weight is carried by size, colour, and the outline that everything drawn over
    /// the world gets.
    /// </para>
    /// </remarks>
    public static Font Sans
    {
        get
        {
            if (_face != null)
                return _face;

            if (ResourceLoader.Load("res://assets/fonts/Jersey10.ttf") is not FontFile file)
                return _face = ThemeDB.FallbackFont;

            file.Antialiasing = TextServer.FontAntialiasing.Gray;
            file.SubpixelPositioning = TextServer.SubpixelPositioning.Disabled;
            file.Hinting = TextServer.Hinting.None;

            return _face = file;
        }
    }

    /// <summary>The same face. Kept as a name so call sites do not all have to change again.</summary>
    public static Font Bold => Sans;

    /// <summary>Whichever a caller asked for, which is the same one.</summary>
    public static Font Face(bool bold) => Sans;

    /// <summary>
    /// How many screen pixels one interface pixel currently covers.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Set by <see cref="HudLayer"/>, which is the thing that scales. Everything in this interface
    /// is laid out in reference pixels and the canvas is scaled to fit the window, which is fine
    /// for a rectangle and ruinous for a glyph: at one and a half, every font pixel lands on one
    /// and a half screen pixels and a face drawn on a whole-pixel grid comes out mush. That is what
    /// "the text is blurry and the icons look pixelated" is.
    /// </para>
    /// <para>
    /// So text is not drawn at its nominal size and then scaled. It is drawn at the size it will
    /// actually occupy, under a transform that undoes the canvas scale -- the glyph is rasterised
    /// once, at screen resolution, on whole pixels. Callers still work in reference pixels and
    /// never see this; <see cref="Measure"/> and <see cref="BaselineIn"/> answer in the same units
    /// they always did.
    /// </para>
    /// </remarks>
    public static float Sharpness
    {
        get => _sharpness;
        set
        {
            value = Mathf.Clamp(value, 0.25f, 8f);
            if (Mathf.IsEqualApprox(_sharpness, value))
                return;

            _sharpness = value;
            _measured.Clear();
        }
    }

    private static float _sharpness = 1f;

    /// <summary>Whether the canvas is at one to one, in which case none of this is needed.</summary>
    private static bool Native => Mathf.IsEqualApprox(_sharpness, 1f);

    /// <summary>The size to rasterise at, so that what lands on screen is whole pixels.</summary>
    private static int Snap(int size) => Mathf.Max(1, Mathf.RoundToInt(size * _sharpness));

    /// <summary>
    /// Tier 1: reading text, on an opaque plate. No outline, no shadow.
    /// </summary>
    /// <remarks>
    /// The default, and by count almost everything. An outline exists to rescue a string from a
    /// background that could be any colour; over a panel there is no such background, and all the
    /// outline does at fourteen pixels is fill in the counters of <c>a</c>, <c>e</c> and <c>g</c>
    /// with black.
    /// </remarks>
    public static T Typeset<T>(this T label, int size, Color colour, bool bold = false)
        where T : Label
    {
        label.AddThemeFontOverride("font", Face(bold));
        label.AddThemeFontSizeOverride("font_size", size);
        label.AddThemeColorOverride("font_color", colour);

        label.AddThemeConstantOverride("outline_size", 0);
        label.AddThemeConstantOverride("shadow_offset_x", 0);
        label.AddThemeConstantOverride("shadow_offset_y", 0);

        label.MouseFilter = Control.MouseFilterEnum.Ignore;
        return label;
    }

    /// <summary>Tier 2 as a label: over the world, so one shadow down and right.</summary>
    public static T TypesetOverWorld<T>(this T label, int size, Color colour)
        where T : Label
    {
        label.Typeset(size, colour, bold: true);
        label.AddThemeColorOverride("font_outline_color", TextOutline);
        label.AddThemeConstantOverride("outline_size", 2);
        return label;
    }

    /// <summary>The shadow under Tier 2 text: one pixel, down and right, not a stroke.</summary>
    public static readonly Color HudShadow = new(0f, 0f, 0f, 0.9f);

    /// <summary>
    /// Tier 1, drawn: reading text on an opaque plate.
    /// </summary>
    /// <param name="at">The text's baseline, at its left edge unless an alignment says otherwise.</param>
    public static void DrawText(
        this CanvasItem into, Vector2 at, string text, int size, Color colour, bool bold = false,
        HorizontalAlignment alignment = HorizontalAlignment.Left, float width = -1f)
    {
        if (Native)
        {
            into.DrawString(Face(bold), at, text, alignment, width, size, colour);
            return;
        }

        Sharpen(into);
        into.DrawString(Face(bold), Up(at), text, alignment, Up(width), Snap(size), colour);
        Restore(into);
    }

    /// <summary>Undoes the canvas scale for the draws that follow, so glyphs land on whole pixels.</summary>
    private static void Sharpen(CanvasItem into) =>
        into.DrawSetTransform(Vector2.Zero, 0f, Vector2.One / _sharpness);

    private static void Restore(CanvasItem into) =>
        into.DrawSetTransform(Vector2.Zero, 0f, Vector2.One);

    private static Vector2 Up(Vector2 at) => (at * _sharpness).Round();

    private static float Up(float width) => width < 0f ? width : width * _sharpness;

    /// <summary>
    /// Tier 2, drawn: over the world, where the background is whatever the player walked onto.
    /// </summary>
    /// <remarks>
    /// A one-pixel outline on all four sides, and bold. Revision six asked for a single shadow
    /// down and right and was wrong about it: the original outlines everything it draws over the
    /// world, and at these sizes the outline is most of what gives that text its weight -- it is
    /// the difference between a bar label that belongs to the game and one that belongs to a web
    /// page. Reading text on a panel still takes neither; see <see cref="DrawText"/>.
    /// </remarks>
    public static void DrawOverWorld(
        this CanvasItem into, Vector2 at, string text, int size, Color colour,
        HorizontalAlignment alignment = HorizontalAlignment.Left, float width = -1f)
    {
        Outlined(into, at, text, size, colour, alignment, width);
    }

    /// <summary>
    /// Tier 3, drawn: two to four characters sitting directly on a sprite.
    /// </summary>
    /// <remarks>
    /// The one place a hard outline survives. A tier tag sits on artwork of no fixed colour, is too
    /// short to be read as a word, and is looked up rather than read -- so the outline's cost is a
    /// cost it does not pay.
    /// </remarks>
    public static void DrawToken(
        this CanvasItem into, Vector2 at, string text, int size, Color colour,
        HorizontalAlignment alignment = HorizontalAlignment.Left, float width = -1f)
    {
        Outlined(into, at, text, size, colour, alignment, width);
    }

    /// <summary>A string with a one-pixel black edge, rasterised at screen resolution.</summary>
    private static void Outlined(
        CanvasItem into, Vector2 at, string text, int size, Color colour,
        HorizontalAlignment alignment, float width)
    {
        if (Native)
        {
            into.DrawStringOutline(Bold, at, text, alignment, width, size, 1, TextOutline);
            into.DrawString(Bold, at, text, alignment, width, size, colour);
            return;
        }

        // The outline grows with everything else, or it disappears at high scales and swallows the
        // glyph at low ones.
        int edge = Mathf.Max(1, Mathf.RoundToInt(_sharpness));

        Sharpen(into);
        into.DrawStringOutline(Bold, Up(at), text, alignment, Up(width), Snap(size), edge, TextOutline);
        into.DrawString(Bold, Up(at), text, alignment, Up(width), Snap(size), colour);
        Restore(into);
    }

    /// <summary>
    /// How thick a black edge to put around artwork drawn this big, in screen pixels.
    /// </summary>
    /// <remarks>
    /// The world's outline is a fixed two screen pixels at any zoom -- see
    /// <c>shaders/sprite.gdshader</c> -- and two is right for the hotbar, where the artwork is about
    /// forty pixels across. The vault draws the same sprites at nearly twice that, and two pixels
    /// there is a hairline you have to look for. So it grows with the artwork, and stops at three:
    /// past that the edge starts closing up the gaps the art means to have in it, and a ring stops
    /// being a ring.
    /// </remarks>
    public static float SpriteOutline(in Rect2 box) =>
        Mathf.Clamp(Mathf.Round(Mathf.Min(box.Size.X, box.Size.Y) / 26f), 1f, 3f) / _sharpness
        * Mathf.Max(1f, Mathf.Round(_sharpness));

    /// <summary>The eight directions an outline is dilated in, as unit offsets.</summary>
    /// <remarks>
    /// Eight rather than four for the same reason the shader samples eight: a diagonal edge
    /// outlined from four sides comes out with a stair-step of bare pixels along it.
    /// </remarks>
    private static readonly Vector2[] Around =
    {
        new(-1f, 0f), new(1f, 0f), new(0f, -1f), new(0f, 1f),
        new(-1f, -1f), new(1f, -1f), new(-1f, 1f), new(1f, 1f),
    };

    /// <summary>
    /// Draws a sprite with a black edge around it, the way the world draws one.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Item art is drawn in its own palette and nothing else, and a lot of it -- iron greys, dark
    /// leathers, anything black-hafted -- is the colour of the plate it sits on. Without an edge
    /// those items do not end; they merge into the square, and a bag of them reads as one texture
    /// rather than as sixteen things.
    /// </para>
    /// <para>
    /// Eight offset copies of the sprite in black under the sprite itself. The world does this in a
    /// fragment shader because it has thousands of sprites a frame and can afford neither the draw
    /// calls nor a material switch; the interface has a few dozen, redraws them only when their
    /// contents change, and would have to give the whole canvas item a material to do it the other
    /// way -- which would outline the plate and the text too.
    /// </para>
    /// <para>
    /// The source rectangle never moves, only the destination, so a copy pushed a pixel left cannot
    /// drag in whatever sits next to it on the sheet.
    /// </para>
    /// </remarks>
    /// <param name="outline">
    /// Thickness in pixels, or negative to take it from the size the artwork is drawn at.
    /// </param>
    public static void DrawSprite(
        this CanvasItem into, in Assets.Sprite sprite, in Rect2 box, float outline = -1f)
    {
        if (!sprite.IsValid)
            return;

        if (outline < 0f)
            outline = SpriteOutline(box);

        if (outline > 0f)
            foreach (var direction in Around)
                into.DrawTextureRectRegion(
                    sprite.Sheet, new Rect2(box.Position + direction * outline, box.Size),
                    sprite.Region, TextOutline);

        into.DrawTextureRectRegion(sprite.Sheet, box, sprite.Region);
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
    public static float Measure(string text, int size, bool bold = false)
    {
        if (string.IsNullOrEmpty(text))
            return 0f;

        var key = (text, bold ? -size : size);
        if (_measured.TryGetValue(key, out float width))
            return width;

        // Measured at the size it will be rasterised at and brought back into reference pixels, so
        // that what is measured is what is drawn however the canvas is scaled.
        width = Face(bold).GetStringSize(text, HorizontalAlignment.Left, -1, Snap(size)).X / _sharpness;

        // Cleared wholesale rather than evicted one at a time: it only grows when the game starts
        // showing text it has never shown, which is not something that happens in a steady state.
        if (_measured.Count >= MostMeasured)
            _measured.Clear();

        _measured[key] = width;
        return width;
    }

    /// <summary>
    /// The baseline that centres a line of this size in a box of this height.
    /// </summary>
    /// <remarks>
    /// Written out once because it was written out at nine call sites, each of them reaching into
    /// the font for its ascent and descent, and each of them a place a font change had to be
    /// followed to.
    /// </remarks>
    public static float BaselineIn(float height, int size, bool bold = false)
    {
        var face = Face(bold);
        int at = Snap(size);

        return Mathf.Round(
            (height + (face.GetAscent(at) - face.GetDescent(at)) / _sharpness) / 2f);
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

        StyleFont(theme);
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

    /// <summary>
    /// A flat plate with a one-pixel edge, which is the shape everything in this interface is.
    /// </summary>
    /// <remarks>
    /// This replaced a rounded, graded, drop-shadowed box. That box was the menus' own language and
    /// nothing else in the game spoke it: the HUD is opaque plates with hard corners and a
    /// one-pixel border, and a text field with soft corners and a shadow under it sitting on the
    /// same screen read as a control borrowed from a different program.
    /// </remarks>
    public static StyleBoxFlat Plate(Color fill, Color border, int borderWidth = 1)
    {
        var box = new StyleBoxFlat
        {
            BgColor = fill,
            BorderColor = border,
            ContentMarginLeft = 10,
            ContentMarginRight = 10,
            ContentMarginTop = 5,
            ContentMarginBottom = 5,
        };

        box.SetBorderWidthAll(borderWidth);
        box.SetCornerRadiusAll(0);
        return box;
    }

    private static void StyleLineEdit(Theme theme)
    {
        // A slot's plate, because that is what a field is: a dark inset you put something into.
        var rest = Plate(Slot, SlotBorder);
        rest.ContentMarginLeft = 9;
        rest.ContentMarginRight = 9;
        rest.ContentMarginTop = 7;
        rest.ContentMarginBottom = 7;

        var focused = (StyleBoxFlat)rest.Duplicate();
        focused.BorderColor = SlotBorderHi;
        focused.SetBorderWidthAll(2);

        theme.SetStylebox("normal", "LineEdit", rest);
        theme.SetStylebox("focus", "LineEdit", focused);
        theme.SetStylebox("read_only", "LineEdit", rest);
        theme.SetColor("font_color", "LineEdit", Text);
        theme.SetColor("font_placeholder_color", "LineEdit", TextDim);
        theme.SetColor("caret_color", "LineEdit", ButtonPromo);
        theme.SetColor("selection_color", "LineEdit", ButtonPromo with { A = 0.3f });
        theme.SetFontSize("font_size", "LineEdit", FontBody);
    }

    private static void StyleOptionButton(Theme theme)
    {
        var rest = Plate(ButtonFace, PanelEdge);
        var hover = Plate(ButtonHover, SlotBorder);
        var pressed = Plate(ButtonBevelLow, ButtonPromo);

        foreach (string type in new[] { "OptionButton", "MenuButton", "PopupMenu" })
        {
            theme.SetStylebox("normal", type, rest);
            theme.SetStylebox("hover", type, hover);
            theme.SetStylebox("pressed", type, pressed);
            theme.SetStylebox("focus", type, new StyleBoxEmpty());
            theme.SetColor("font_color", type, Text);
            theme.SetColor("font_hover_color", type, Colors.White);
            theme.SetFontSize("font_size", type, FontBody);
        }

        theme.SetStylebox("panel", "PopupMenu", Plate(Panel, PanelEdge));
        theme.SetColor("font_color", "PopupMenu", Text);
        theme.SetColor("font_hover_color", "PopupMenu", ButtonPromo);
    }

    private static void StyleCheckBox(Theme theme)
    {
        theme.SetStylebox("normal", "CheckBox", new StyleBoxEmpty());
        theme.SetStylebox("hover", "CheckBox", new StyleBoxEmpty());
        theme.SetStylebox("pressed", "CheckBox", new StyleBoxEmpty());
        theme.SetStylebox("focus", "CheckBox", new StyleBoxEmpty());
        theme.SetColor("font_color", "CheckBox", TextDim);
        theme.SetColor("font_hover_color", "CheckBox", Text);
        theme.SetFontSize("font_size", "CheckBox", FontSmall);
    }

    /// <summary>
    /// The engine's scrollbars, flattened to match the one the panels draw themselves.
    /// </summary>
    /// <remarks>
    /// A flat track and a flat thumb: no gradient, no rounding, no drop shadow. What was here was
    /// <see cref="Box"/>, which is the shape a dialog control wants and is exactly what makes a
    /// scrollbar read as the editor's rather than as the game's.
    /// </remarks>
    private static void StyleScrollbars(Theme theme)
    {
        static StyleBoxFlat Flat(Color fill) => new() { BgColor = fill };

        foreach (string type in new[] { "VScrollBar", "HScrollBar" })
        {
            theme.SetStylebox("scroll", type, Flat(PanelInset));
            theme.SetStylebox("grabber", type, Flat(ButtonFace));
            theme.SetStylebox("grabber_highlight", type, Flat(ButtonHover));
            theme.SetStylebox("grabber_pressed", type, Flat(SlotBorderHi));
        }
    }

    private static void StyleSeparators(Theme theme)
    {
        var line = new StyleBoxLine { Color = Divider, Thickness = 1 };
        theme.SetStylebox("separator", "HSeparator", line);

        var upright = new StyleBoxLine { Color = Divider, Thickness = 1, Vertical = true };
        theme.SetStylebox("separator", "VSeparator", upright);
    }

    private static void StyleTooltips(Theme theme)
    {
        // The item tooltip draws its own panel; this is for the plain ones, which should at least
        // belong to the same game.
        theme.SetStylebox("panel", "TooltipPanel", Plate(Panel, PanelEdge));
        theme.SetColor("font_color", "TooltipLabel", Text);
        theme.SetFontSize("font_size", "TooltipLabel", FontSmall);
    }

    private static void StyleLabels(Theme theme)
    {
        theme.SetColor("font_color", "Label", Text);
        theme.SetFontSize("font_size", "Label", FontBody);

        // No shadow by default. It used to be under every label so that text over the world stayed
        // legible, but almost no label is over the world -- they are on plates -- and the ones that
        // are ask for it by name. See TypesetOverWorld.
        theme.SetConstant("shadow_offset_x", "Label", 0);
        theme.SetConstant("shadow_offset_y", "Label", 0);
    }

    /// <summary>Puts the interface's face on every control the port does not draw itself.</summary>
    private static void StyleFont(Theme theme)
    {
        theme.DefaultFont = Sans;
        theme.DefaultFontSize = FontBody;
    }

    private static void StyleSliders(Theme theme)
    {
        // The same track and fill the vitals bars use, so a slider is recognisably one of them.
        foreach (string type in new[] { "HSlider", "VSlider" })
        {
            theme.SetStylebox("slider", type, Plate(BarTrack, BarEdge));
            theme.SetStylebox("grabber_area", type, Plate(ButtonFace, ButtonFace, 0));
            theme.SetStylebox("grabber_area_highlight", type, Plate(ButtonPromo, ButtonPromo, 0));
        }
    }
}
