using System.Collections.Generic;
using Godot;

namespace Hendra.Render;

/// <summary>One entity's on-screen furniture: a health bar, a name, or both.</summary>
/// <summary>
/// Where status icons come from.
/// </summary>
/// <remarks>
/// An interface so the overlay stays a drawing concern: it needs a texture and a rectangle, not the
/// asset library's view of the world.
/// </remarks>
public interface IConditionSheet
{
    Godot.Texture2D Texture { get; }

    /// <summary>The rectangle for a sprite index, or null if the sheet has no such sprite.</summary>
    Godot.Rect2? Region(int index);
}

/// <summary>
/// Where a body is this frame, for the text hanging over it.
/// </summary>
/// <remarks>
/// The three things <c>CharacterStatusText.draw</c> asks its <c>go_</c> for every frame: whether it
/// is still in the world, whether it is being drawn, and where it is. Answered by the world rather
/// than remembered by the overlay, which is what makes the text follow a body that moves.
/// </remarks>
public struct FloatingAnchor
{
    /// <summary>The body has left the world. Its texts go with it.</summary>
    public bool Gone;

    /// <summary>Whether the body is on screen. A hidden body's texts wait rather than die.</summary>
    public bool Drawn;

    /// <summary>Where its feet land on screen, in pixels.</summary>
    public Vector2 Screen;

    /// <summary>How tall its artwork draws above that, in pixels.</summary>
    public float SpriteHeight;
}

public struct OverlayItem
{
    /// <summary>Where the entity's feet land on screen, in pixels.</summary>
    public Vector2 Anchor;

    /// <summary>
    /// How far above the anchor the sprite's own top is, in pixels.
    /// </summary>
    /// <remarks>
    /// Supplied by the caller, which is the only thing that knows how tall the artwork is once its
    /// size stat and the projection have been applied. Without it the status icons sit across the
    /// middle of whatever they belong to.
    /// </remarks>
    public float SpriteHeight;

    /// <summary>What this entity is saying, or null. Shown for as long as the server asked.</summary>
    public string Bubble;

    public string Name;
    public Color NameColor;

    /// <summary>The guild written under the name, or null for a player in none.</summary>
    public string Guild;

    /// <summary>Their rank in it, which is what the line is coloured by.</summary>
    public int GuildRank;

    /// <summary>Whether a star belongs beside this name. Players only.</summary>
    public bool ShowStar;

    /// <summary>The account's star rating, which the star is coloured by.</summary>
    public int Stars;

    /// <summary>Whether the account is an administrator, which is a colour of its own.</summary>
    public bool Admin;

    /// <summary>The halo colour, as a packed <c>0xRRGGBB</c>. Zero for no halo.</summary>
    public int Glow;

    public int Hp;
    public int MaxHp;
    public bool ShowHealthBar;

    /// <summary>
    /// Sprite indices into the condition sheet, one per effect currently showing.
    /// </summary>
    /// <remarks>
    /// Resolved by the caller rather than here, because which frame a multi-frame icon is on
    /// depends on the clock and the overlay does not have one.
    /// </remarks>
    public System.Collections.Generic.List<int> Conditions;
}

/// <summary>
/// Health bars and name plates, drawn as screen-space UI over the 3D world.
/// </summary>
/// <remarks>
/// <para>
/// Screen space rather than world geometry, so text stays crisp at any zoom and the bars keep a
/// constant size regardless of how large a monster is. The original rasterised every name into its
/// own bitmap, redrew it whenever the text changed, and rebuilt the health bar's geometry into the
/// display list each frame.
/// </para>
/// <para>
/// One <c>_Draw</c> pass over a flat list, which is cheap enough that culling beyond the viewport
/// bounds is the only optimisation worth having.
/// </para>
/// </remarks>
public partial class WorldOverlay : Control
{
    /// <summary>
    /// The bar under a sprite: forty wide by six high, four below the feet.
    /// </summary>
    /// <remarks>
    /// <c>GameObject.as:1047-1049</c>, whose <c>20 / 4 / 6</c> are in a space scaled fifty to the
    /// tile (<c>Camera.as:57</c>) -- the same fifty this port projects at, so they carry over as
    /// pixels unchanged.
    /// </remarks>
    private const float BarHalfWidth = 20f;

    private const float BarHeight = 6f;
    private const float BarOffsetY = 4f;
    private const float NameOffsetY = -6f;

    /// <summary>How far under the name the guild sits, in pixels.</summary>
    private const float GuildOffsetY = 11f;

    /// <summary>The gap between the top of a sprite and the row of status icons over it.</summary>
    private const float ConditionGap = 6f;

    /// <summary>The empty part of a bar, and the red it is pulsed towards. <c>GameObject.as:1042</c>.</summary>
    private static readonly Color BarBackground = new("545454");

    private static readonly Color BarEmptyWarning = new("ff0000");

    /// <summary>The one colour a bar's fill is ever drawn in. <c>GameObject.as:1031</c>.</summary>
    private static readonly Color BarFill = new("10ff00");

    /// <summary>The marker pointing at whatever is worth walking towards, off the edge of the view.</summary>
    private static readonly Color MarkerColour = new("d02020");

    private readonly List<OverlayItem> _items = new(128);

    /// <summary>The sheet status icons are cut from, or null before the assets are configured.</summary>
    private IConditionSheet _conditionSheet;

    /// <summary>Supplies the icon sheet, so the overlay does not have to know how assets are stored.</summary>
    public void Configure(IConditionSheet conditionSheet) => _conditionSheet = conditionSheet;

    /// <summary>Where the quest objective is on screen, or null when there is none.</summary>
    private Vector2? _questTarget;
    private Font _font;
    private int _fontSize = 13;

    public override void _Ready()
    {
        // Ignore the mouse entirely: this sits over the world and must not eat clicks meant for it.
        MouseFilter = MouseFilterEnum.Ignore;
        UI.ScreenFit.FillScreen(this);
        _font = ThemeDB.FallbackFont;
    }

    /// <summary>
    /// Whether health bars are drawn at all.
    /// </summary>
    /// <remarks>
    /// Off until asked for, which is the original's default -- <c>setDefault("HPBar", false)</c> at
    /// <c>Parameters.as:239</c>, turned on with H (<c>Parameters.as:164</c>,
    /// <c>MapUserInput.as:379-381</c>). It matters more than a default usually does because the
    /// original draws a bar for every enemy and player in sight whether hurt or not
    /// (<c>GameObject.as:1162-1174</c> has no <c>hp &lt; maxHp</c> test), so on a tile carrying a
    /// dozen bodies the bars stack into a wall that hides the bodies they belong to. Leaving the
    /// switch where the original leaves it is what keeps that wall a thing the player asked for.
    /// </remarks>
    private bool _healthBars;

    public void ToggleHealthBars() => _healthBars = !_healthBars;

    /// <summary>Clears the per-frame items. The rising numbers are not among them; see the note there.</summary>
    public void Clear()
    {
        _items.Clear();
        _questTarget = null;
    }

    /// <summary>
    /// Marks the quest objective. Pass null when there is none.
    /// </summary>
    /// <param name="screenPosition">Where the objective is, in pixels. May be off screen.</param>
    public void SetQuestMarker(Vector2? screenPosition)
    {
        // The age is what the fade is driven from, and it restarts whenever there is no marker, so
        // the next one to appear fades in as well.
        if (!screenPosition.HasValue)
            _markerAgeMs = 0f;

        _questTarget = screenPosition;
    }

    /// <summary>How long the current marker has been on screen, for the fade.</summary>
    private float _markerAgeMs;

    public override void _Process(double delta)
    {
        if (_questTarget.HasValue)
            _markerAgeMs += (float)delta * 1000f;

        // The numbers move every frame whether or not the world sent anything.
        if (_texts.Count > 0)
            QueueRedraw();
    }

    public void Add(in OverlayItem item) => _items.Add(item);

    /// <summary>
    /// Text thrown off a body: damage dealt, damage taken, experience gained, what the server has
    /// to say about somebody.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Held by object id rather than by position, because the original re-reads <c>go_.posS_</c>
    /// on every frame it draws (<c>CharacterStatusText.draw</c>, <c>:56-58</c>) and so its text
    /// tracks the body as it runs. A position taken once at arrival leaves the text standing where
    /// the body was, which over a fleeing monster is a number pointing at empty floor.
    /// </para>
    /// <para>
    /// Nothing caps how many of these there can be, as nothing caps the original: they are display
    /// objects on the overlay until their lifetime runs out. A boss under fire from a full party
    /// makes a wall of numbers, and that wall is the game.
    /// </para>
    /// </remarks>
    /// <param name="objectId">The body it hangs over, and follows.</param>
    /// <param name="lifetimeMs">
    /// How long the text lives, and so how long it takes to rise. Zero takes the thousand
    /// milliseconds every <c>makeNotification</c>, damage number and experience number uses; the
    /// three seconds of a condition effect and the two of a level are passed in.
    /// </param>
    /// <param name="delayMs">
    /// How long the text waits before it appears, which is the original's <c>offsetTime_</c>: a hit
    /// that lands several conditions at once staggers their names half a second apart rather than
    /// stacking them on one another.
    /// </param>
    public void AddFloatingText(int objectId, string text, Color colour, float lifetimeMs = 0f,
        float delayMs = 0f)
    {
        _texts.Add(new FloatingText
        {
            ObjectId = objectId,
            Text = text,
            Colour = colour,
            BornMs = Time.GetTicksMsec(),
            LifeMs = lifetimeMs > 0f ? lifetimeMs : TextLifeMs,
            DelayMs = delayMs,
        });
    }

    /// <summary>
    /// Text that waits its turn: one line at a time over a body, in the order they were asked for.
    /// </summary>
    /// <remarks>
    /// <para>
    /// <c>QueuedStatusTextList</c>. The original keeps one list per object id and adds only its
    /// <em>head</em> to the overlay (<c>append</c> calls <c>addChild</c> only when the list was
    /// empty), so only the head is drawn and only the head is ticked. Its clock starts on the first
    /// frame it is drawn — <c>CharacterStatusText.draw</c> sets <c>startTime_</c> then — which is
    /// why a text that spends a second waiting still gets its full life once it reaches the front.
    /// </para>
    /// <para>
    /// That is the whole reason <c>handleLevelUp(true)</c> is readable: it asks for "New Class
    /// Unlocked!" and "Level Up!" in the same frame, and they are shown in sequence rather than
    /// stacked on one another. A third arriving while two are queued simply goes on the end and
    /// waits for both.
    /// </para>
    /// </remarks>
    public void AddQueuedText(int objectId, string text, Color colour, float lifetimeMs = 0f,
        float delayMs = 0f)
    {
        var queued = new FloatingText
        {
            ObjectId = objectId,
            Text = text,
            Colour = colour,
            BornMs = Time.GetTicksMsec(),
            LifeMs = lifetimeMs > 0f ? lifetimeMs : TextLifeMs,
            DelayMs = delayMs,
            Queued = true,
        };

        // A list with a head already showing takes the new line on the tail; an empty one shows it.
        if (_waiting.TryGetValue(objectId, out var line) && line.Count > 0)
        {
            line.Add(queued);
            return;
        }

        if (_texts.Exists(shown => shown.Queued && shown.ObjectId == objectId))
        {
            if (line == null)
                _waiting[objectId] = line = new List<FloatingText>(2);

            line.Add(queued);
            return;
        }

        _texts.Add(queued);
    }

    /// <summary>
    /// The queued texts that are not at the front, keyed by the body they hang over.
    /// </summary>
    /// <remarks>
    /// The head of each list lives in <see cref="_texts"/>, which is what makes it the one that is
    /// drawn and aged; these are the ones the original leaves out of the display list entirely.
    /// </remarks>
    private readonly Dictionary<int, List<FloatingText>> _waiting = new();

    /// <summary>
    /// Moves a body's next queued line to the front, its clock starting now.
    /// </summary>
    /// <remarks>
    /// <c>QueuedStatusTextList.shift</c>, called from the head's own <c>dispose</c> — so a queue
    /// advances both when its head's life runs out and when the body it hangs over leaves the
    /// world, the two ways <c>draw</c> returns false.
    /// </remarks>
    private void PromoteNext(int objectId)
    {
        if (!_waiting.TryGetValue(objectId, out var line) || line.Count == 0)
        {
            _waiting.Remove(objectId);
            return;
        }

        var next = line[0];
        line.RemoveAt(0);

        if (line.Count == 0)
            _waiting.Remove(objectId);

        next.BornMs = Time.GetTicksMsec();
        _texts.Add(next);
    }

    /// <summary>Where a body is right now. Set by the world; asked again every frame.</summary>
    public System.Func<int, FloatingAnchor> AnchorOf { get; set; }

    /// <summary>
    /// How long a text lives, and how far it rises in that time.
    /// </summary>
    /// <remarks>
    /// A thousand milliseconds and forty pixels, from <c>makeNotification(...,1000)</c> and
    /// <c>CharacterStatusText.MAX_DRIFT</c>. The rise is linear in age -- <c>(age / lifetime) *
    /// MAX_DRIFT</c> -- and there is no fade at all: the original's text is at full strength for
    /// its whole life and then gone.
    /// </remarks>
    private const float TextLifeMs = 1000f;

    private const float MaxDrift = 40f;

    /// <summary>
    /// The gap between the top of a body's artwork and its text.
    /// </summary>
    /// <remarks>
    /// The constant term of the original's offset, <c>-(texture.height * size/100) * 5 - 20</c>:
    /// the first term is the drawn height of the sprite, which the anchor supplies, and this is the
    /// twenty pixels of air above it.
    /// </remarks>
    private const float TextGap = 20f;

    /// <summary>
    /// How large the text is drawn.
    /// </summary>
    /// <remarks>
    /// The original sets 24 against a name plate's 16 (<c>CharacterStatusText:33</c>,
    /// <c>GameObject.makeNameBitmapData</c>), so this is that same half-again over the name size
    /// used here.
    /// </remarks>
    private const int FloatingTextSize = 20;

    private readonly List<FloatingText> _texts = new(64);

    private struct FloatingText
    {
        /// <summary>The body this hangs over. Its position is read afresh every frame.</summary>
        public int ObjectId;

        public string Text;
        public Color Colour;
        public ulong BornMs;

        /// <summary>How long this one lives, in milliseconds.</summary>
        public float LifeMs;

        /// <summary>How long it waits before appearing, in milliseconds.</summary>
        public float DelayMs;

        /// <summary>
        /// Whether this line is the head of a queue, and so lets the next one through when it goes.
        /// </summary>
        public bool Queued;
    }

    /// <summary>
    /// Draws the rising text and drops what has finished.
    /// </summary>
    /// <remarks>
    /// Deliberately not in <see cref="_items"/>: that list is cleared and refilled every frame from
    /// the entities in view, and a text has to outlive the frame it was made in. It does not
    /// outlive its body -- a text whose body has left the world dies with it, as the original's
    /// <c>go_.map_ == null</c> test kills it -- and a body that is not being drawn hides its text
    /// without stopping its clock.
    /// </remarks>
    private void DrawFloatingTexts()
    {
        if (_texts.Count == 0 || AnchorOf == null)
            return;

        ulong now = Time.GetTicksMsec();

        for (int i = _texts.Count - 1; i >= 0; i--)
        {
            var text = _texts[i];
            float age = (now - text.BornMs) - text.DelayMs;

            if (age > text.LifeMs)
            {
                _texts.RemoveAt(i);

                if (text.Queued)
                    PromoteNext(text.ObjectId);

                continue;
            }

            var anchor = AnchorOf(text.ObjectId);

            if (anchor.Gone)
            {
                _texts.RemoveAt(i);

                if (text.Queued)
                    PromoteNext(text.ObjectId);

                continue;
            }

            // Still waiting its turn, or over something that is not on screen: it ages either way.
            if (age < 0f || !anchor.Drawn)
                continue;

            float rise = age / text.LifeMs * MaxDrift;
            var at = new Vector2(
                anchor.Screen.X,
                anchor.Screen.Y - anchor.SpriteHeight - TextGap - rise);

            var size = _font.GetStringSize(text.Text, HorizontalAlignment.Left, -1, FloatingTextSize);

            // Centred on the point in both directions, as the original centres the rasterised line
            // on its sprite's origin (`CharacterStatusText.onTextChanged`).
            var origin = new Vector2(
                at.X - size.X / 2f,
                at.Y - size.Y / 2f + _font.GetAscent(FloatingTextSize));

            // The black outline stands in for the original's GlowFilter, which is what keeps a red
            // number legible over a dark floor.
            var outline = new Color(0f, 0f, 0f, 0.85f);
            DrawString(_font, origin + new Vector2(1f, 0f), text.Text, HorizontalAlignment.Left, -1, FloatingTextSize, outline);
            DrawString(_font, origin + new Vector2(-1f, 0f), text.Text, HorizontalAlignment.Left, -1, FloatingTextSize, outline);
            DrawString(_font, origin + new Vector2(0f, 1f), text.Text, HorizontalAlignment.Left, -1, FloatingTextSize, outline);
            DrawString(_font, origin + new Vector2(0f, -1f), text.Text, HorizontalAlignment.Left, -1, FloatingTextSize, outline);

            DrawString(_font, origin, text.Text, HorizontalAlignment.Left, -1, FloatingTextSize, text.Colour);
        }
    }

    /// <summary>Call once per frame, after the items for that frame have been added.</summary>
    public void Commit() => QueueRedraw();

    /// <summary>
    /// How many of each kind of text is drawn in a frame, nearest the middle of the screen first.
    /// </summary>
    /// <remarks>
    /// Laying out a line of text is the most expensive thing this overlay does, and the number of
    /// things asking for one is set by the world rather than by anything the client controls: a
    /// room with ten thousand monsters in it wants ten thousand names and, if they are all shouting,
    /// as many speech balloons. Measuring is cached, so the remaining cost is the drawing itself,
    /// and these are set high enough to be a backstop against a pathological screen rather than a
    /// limit anybody meets in play -- past this many overlapping lines the screen is an unreadable
    /// wall regardless. Health bars and status icons have no cap: they are cheap, and they are what
    /// the player is actually reading.
    /// </remarks>
    private const int MostBubbles = 64;

    private const int MostNames = 400;

    /// <summary>Reused across frames so a busy screen does not allocate one of these per frame.</summary>
    private readonly List<(float Distance, int Index)> _ranked = new(256);

    public override void _Draw()
    {
        var bounds = GetViewportRect().Size;
        var middle = bounds / 2f;

        _ranked.Clear();

        for (int i = 0; i < _items.Count; i++)
        {
            var item = _items[i];

            // A generous margin, since a name can be much wider than its anchor point.
            if (item.Anchor.X < -200f || item.Anchor.X > bounds.X + 200f ||
                item.Anchor.Y < -100f || item.Anchor.Y > bounds.Y + 100f)
                continue;

            if (item.Glow != 0)
                DrawGlow(item);

            if (_healthBars && item.ShowHealthBar && item.MaxHp > 0)
                DrawHealthBar(item);

            if (item.Conditions is { Count: > 0 })
                DrawConditions(item);

            if (!string.IsNullOrEmpty(item.Name) || !string.IsNullOrEmpty(item.Bubble))
                _ranked.Add((item.Anchor.DistanceSquaredTo(middle), i));
        }

        // Only sorted when there is more text than will be drawn; the usual screenful skips it.
        if (_ranked.Count > MostNames)
            _ranked.Sort(static (a, b) => a.Distance.CompareTo(b.Distance));

        int names = 0;
        int bubbles = 0;

        foreach (var (_, index) in _ranked)
        {
            if (names >= MostNames && bubbles >= MostBubbles)
                break;

            var item = _items[index];

            if (names < MostNames && !string.IsNullOrEmpty(item.Name))
            {
                DrawName(item);
                names++;
            }

            if (bubbles < MostBubbles && !string.IsNullOrEmpty(item.Bubble))
            {
                DrawBubble(item);
                bubbles++;
            }
        }

        if (_questTarget.HasValue)
            DrawQuestMarker(_questTarget.Value, bounds);

        DrawFloatingTexts();
    }

    /// <summary>
    /// The status effects on an entity, in a row above it.
    /// </summary>
    /// <remarks>
    /// Above the health bar and centred on the entity, as the original places them, so a stack of
    /// effects grows outward from the middle rather than off to one side. Sixteen pixels each: the
    /// original composites an eight-pixel sprite into a sixteen-pixel square with a white outline
    /// glow, which is what makes them legible against a lit floor.
    /// </remarks>
    private void DrawConditions(in OverlayItem item)
    {
        if (_conditionSheet == null)
            return;

        const float Size = 16f;

        int count = item.Conditions.Count;
        float left = item.Anchor.X - Size * count / 2f;

        // Above the artwork rather than across it: the anchor is where the entity's feet are, so
        // the icons have to clear its own height before they are over its head.
        float top = item.Anchor.Y - Mathf.Max(item.SpriteHeight, 16f) - Size - ConditionGap;

        for (int i = 0; i < count; i++)
        {
            var region = _conditionSheet.Region(item.Conditions[i]);
            if (!region.HasValue)
                continue;

            var box = new Rect2(left + i * Size, top, Size, Size);

            // A disc behind each one, so a pale icon still reads over a pale floor.
            DrawCircle(box.Position + box.Size / 2f, Size * 0.42f, new Color(0f, 0f, 0f, 0.45f));
            DrawTextureRectRegion(_conditionSheet.Texture, box.Grow(-2f), region.Value);
        }
    }

    /// <summary>
    /// What somebody just said, over their head.
    /// </summary>
    /// <remarks>
    /// The original puts every line of chat over its speaker for a few seconds as well as in the
    /// log -- the Text packet carries the duration -- and without it a crowded Nexus is a wall of
    /// text in the corner with no way to tell who is talking to you.
    /// </remarks>
    private void DrawBubble(in OverlayItem item)
    {
        const float PaddingX = 6f;
        const float PaddingY = 3f;
        const float Tail = 5f;

        // Width through the shared cache; the height of a single line never varies with content.
        var measured = new Vector2(
            UI.Style.Measure(item.Bubble, _fontSize), _font.GetHeight(_fontSize));

        // Above everything else the entity carries: its own artwork, and the status icons over that.
        float above = Mathf.Max(item.SpriteHeight, 16f)
                      + (item.Conditions is { Count: > 0 } ? 24f : 0f);

        var box = new Rect2(
            item.Anchor.X - measured.X / 2f - PaddingX,
            item.Anchor.Y - above - measured.Y - PaddingY * 2f - Tail,
            measured.X + PaddingX * 2f,
            measured.Y + PaddingY * 2f);

        DrawRect(box, new Color(0f, 0f, 0f, 0.72f));
        DrawRect(box, new Color(1f, 1f, 1f, 0.25f), filled: false, width: 1f);

        // The tail, pointing back down at whoever said it.
        DrawColoredPolygon(
            new[]
            {
                new Vector2(item.Anchor.X - 4f, box.End.Y),
                new Vector2(item.Anchor.X + 4f, box.End.Y),
                new Vector2(item.Anchor.X, box.End.Y + Tail),
            },
            new Color(0f, 0f, 0f, 0.72f));

        DrawString(_font, new Vector2(box.Position.X + PaddingX, box.End.Y - PaddingY - _font.GetDescent(_fontSize)),
            item.Bubble, HorizontalAlignment.Left, -1, _fontSize, Colors.White);
    }

    /// <summary>
    /// Points at the quest objective: a bobbing arrow over it while it is visible, and one pinned
    /// to the edge of the screen pointing the way while it is not.
    /// </summary>
    /// <remarks>
    /// The two cases are the same drawing at different places, which is why the arrow points down
    /// when the target is on screen: it is an arrow aimed at the target either way.
    /// </remarks>
    private void DrawQuestMarker(Vector2 target, Vector2 bounds)
    {
        const float Margin = 40f;
        const float Size = 11f;
        const float BobPixels = 4f;
        const float BobPeriodMs = 900f;

        // Two hundred milliseconds to fade in, so a marker that appears because something walked
        // out of view arrives rather than blinks into existence.
        const float FadeMs = 200f;

        var colour = MarkerColour with { A = Mathf.Min(1f, _markerAgeMs / FadeMs) };
        var centre = bounds / 2f;

        bool visible = target.X > Margin && target.X < bounds.X - Margin &&
                       target.Y > Margin && target.Y < bounds.Y - Margin;

        Vector2 tip;
        float angle;

        if (visible)
        {
            // Above the target, pointing down at it, rising and falling so it catches the eye.
            float bob = Mathf.Sin(Time.GetTicksMsec() / BobPeriodMs * Mathf.Tau) * BobPixels;
            tip = target + new Vector2(0f, -28f + bob);
            angle = Mathf.Pi / 2f;
        }
        else
        {
            var direction = (target - centre).Normalized();
            if (direction == Vector2.Zero)
                return;

            // Pushed out to whichever edge it reaches first, so the arrow sits on the rim of the
            // view rather than in a corner.
            var half = bounds / 2f - new Vector2(Margin, Margin);
            float scale = Mathf.Min(
                Mathf.Abs(direction.X) < 0.0001f ? float.MaxValue : half.X / Mathf.Abs(direction.X),
                Mathf.Abs(direction.Y) < 0.0001f ? float.MaxValue : half.Y / Mathf.Abs(direction.Y));

            tip = centre + direction * scale;
            angle = direction.Angle();
        }

        // A simple triangle, built from the heading so both cases share one shape.
        var forward = Vector2.FromAngle(angle);
        var side = new Vector2(-forward.Y, forward.X);

        DrawColoredPolygon(
            new[]
            {
                tip,
                tip - forward * Size * 1.6f + side * Size * 0.7f,
                tip - forward * Size * 1.6f - side * Size * 0.7f,
            },
            colour);
    }

    /// <summary>
    /// The bar under one entity.
    /// </summary>
    /// <remarks>
    /// Two flat quads and no outline, as <c>GameObject.drawHpBar</c> draws them
    /// (<c>GameObject.as:1025-1065</c>). The fill never changes colour; what reports danger is the
    /// empty part, which is pulsed from grey towards red by the fraction of health missing, so a
    /// bar that is nearly gone throbs and a full one sits still.
    /// </remarks>
    private void DrawHealthBar(in OverlayItem item)
    {
        float top = item.Anchor.Y + BarOffsetY;
        var background = new Rect2(item.Anchor.X - BarHalfWidth, top, BarHalfWidth * 2f, BarHeight);

        float fraction = Mathf.Clamp(item.Hp / (float)item.MaxHp, 0f, 1f);

        // `lerpColor(0x545454, 0xFF0000, abs(sin(time / 300)) * missing)` -- GameObject.as:1041-1042.
        float missing = 1f - fraction;
        float pulse = Mathf.Abs(Mathf.Sin(Time.GetTicksMsec() / 300f)) * missing;
        DrawRect(background, BarBackground.Lerp(BarEmptyWarning, pulse));

        if (fraction > 0f)
            DrawRect(new Rect2(background.Position, background.Size.X * fraction, BarHeight), BarFill);
    }

    /// <summary>
    /// The halo <c>/glow</c> puts on somebody.
    /// </summary>
    /// <remarks>
    /// The original applies a <c>GlowFilter</c> of the colour to the sprite's own bitmap
    /// (<c>GlowRedrawer.as:19-46</c>), which needs the artwork. Drawn here as a soft ring around
    /// where the artwork stands, which reads as the same thing over a crowd and costs one circle.
    /// </remarks>
    private void DrawGlow(in OverlayItem item)
    {
        float height = Mathf.Max(item.SpriteHeight, 16f);
        var centre = new Vector2(item.Anchor.X, item.Anchor.Y - height / 2f);

        // Packed 0xRRGGBB, as the stat carries it.
        var colour = new Color(
            ((item.Glow >> 16) & 0xff) / 255f,
            ((item.Glow >> 8) & 0xff) / 255f,
            (item.Glow & 0xff) / 255f);

        // Two rings rather than one: the filter the original applies falls off, and a single flat
        // disc over the sprite would hide it instead of surrounding it.
        float radius = Mathf.Max(height, 20f) * 0.62f;
        DrawCircle(centre, radius * 1.18f, colour with { A = 0.16f });
        DrawArc(centre, radius, 0f, Mathf.Tau, 24, colour with { A = 0.75f }, 2f);
    }

    /// <summary>
    /// The star beside a player's name.
    /// </summary>
    /// <remarks>
    /// <c>Player.makeNameBitmapData</c> composites <c>FameUtil.numStarsToIcon(numStars_, admin_)</c>
    /// into the name plate for every player (<c>Player.as:750-756</c>). The rating is an account's
    /// total across its classes, and the colour is what says it at a glance -- with one colour of
    /// its own for an administrator, ahead of every rating.
    /// </remarks>
    private void DrawStar(in OverlayItem item, in Vector2 namePosition)
    {
        const float Size = 11f;

        var box = new Rect2(
            namePosition.X - Size - 2f,
            namePosition.Y + (_font.GetHeight(_fontSize) - Size) / 2f,
            Size,
            Size);

        UI.HudIcons.Star(this, box, UI.Fame.Colour(item.Stars, Classes, item.Admin));
    }

    /// <summary>How many classes the game has, which is the width of each star colour band.</summary>
    private const int Classes = 14;

    private void DrawName(in OverlayItem item)
    {
        var position = Outlined(item.Name, item.Anchor.Y + NameOffsetY, item.Anchor.X, item.NameColor);

        if (item.ShowStar)
            DrawStar(item, position);

        // The guild under the name, in the colour its rank earns. The original draws the same two
        // lines together (`GuildText.as:19-48`), and matching a guild against one's own is how a
        // player tells at a glance who is standing with them.
        if (!string.IsNullOrEmpty(item.Guild))
            Outlined(item.Guild, position.Y + GuildOffsetY, item.Anchor.X, GuildColour(item.GuildRank));
    }

    /// <summary>
    /// Draws one line centred over an anchor, with the cheap four-way outline, and says where it
    /// went.
    /// </summary>
    /// <remarks>
    /// Names sit over arbitrary terrain, and without the outline a light name on light ground is
    /// unreadable.
    /// </remarks>
    private Vector2 Outlined(string text, float top, float centreX, Color colour)
    {
        var size = _font.GetStringSize(text, HorizontalAlignment.Left, -1, _fontSize);
        var position = new Vector2(centreX - size.X / 2f, top);

        var outline = new Color(0f, 0f, 0f, 0.85f);
        DrawString(_font, position + new Vector2(1, 0), text, HorizontalAlignment.Left, -1, _fontSize, outline);
        DrawString(_font, position + new Vector2(-1, 0), text, HorizontalAlignment.Left, -1, _fontSize, outline);
        DrawString(_font, position + new Vector2(0, 1), text, HorizontalAlignment.Left, -1, _fontSize, outline);
        DrawString(_font, position + new Vector2(0, -1), text, HorizontalAlignment.Left, -1, _fontSize, outline);

        DrawString(_font, position, text, HorizontalAlignment.Left, -1, _fontSize, colour);
        return position;
    }

    /// <summary>
    /// What colour a guild name is drawn in, by the rank of whoever is wearing it.
    /// </summary>
    /// <remarks>
    /// The ranks the original numbers: 0 initiate, 10 member, 20 officer, 30 leader, 40 founder.
    /// The client colours the line by rank rather than writing the rank out, which is what keeps a
    /// second line over a head to one word.
    /// </remarks>
    private static Color GuildColour(int rank) => rank switch
    {
        >= 40 => new Color("ffdf00"),
        >= 30 => new Color("ff9c00"),
        >= 20 => new Color("9ce5ff"),
        >= 10 => new Color("d0d0d0"),
        _ => new Color("9b9898"),
    };
}
