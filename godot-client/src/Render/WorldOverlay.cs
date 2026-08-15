using System;
using System.Collections.Generic;
using Godot;
using Hendra.UI;

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

    /// <summary>
    /// The star rating drawn to the left of the name, or -1 for a name that carries no star.
    /// </summary>
    /// <remarks>
    /// Every player's name plate carries one and nothing else does, so this doubles as the flag
    /// that says which of the two plates to lay out.
    /// </remarks>
    public int Stars;

    /// <summary>Whether the star is drawn in the administrator's colour rather than a rating's.</summary>
    public bool Admin;

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
/// Health bars, name plates and speech balloons, drawn as screen-space UI over the 3D world.
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
    // The bar, measured off a 1920x1080 capture: fifty by seven overall, a one-pixel black edge on
    // every side, and four pixels of air between the entity's feet and the top of it.
    private const float BarWidth = 50f;

    private const float BarHeight = 7f;
    private const float BarEdge = 1f;
    private const float BarOffsetY = 4f;

    /// <summary>How far below the feet a name plate's cap line sits when there is no bar above it.</summary>
    private const float NameOffsetY = 4f;

    /// <summary>The air between a health bar and the name plate under it.</summary>
    private const float NameGap = 2f;

    /// <summary>The gap between the top of a sprite and the row of status icons over it.</summary>
    private const float ConditionGap = 6f;

    /// <summary>The empty end of a health bar. The original's grey, and the red it pulses towards.</summary>
    private static readonly Color BarTrack = new("545454");

    private static readonly Color BarTrackAlarm = new("ff0000");

    /// <summary>The filled end. Brighter than any green in the interface, which is the point of it.</summary>
    private static readonly Color BarFill = new("12db00");

    /// <summary>How fast the empty end of a nearly-dead bar pulses, in milliseconds per radian.</summary>
    private const float AlarmPeriodMs = 300f;

    /// <summary>The marker pointing at whatever is worth walking towards, off the edge of the view.</summary>
    private static readonly Color MarkerColour = new("d02020");

    private readonly List<OverlayItem> _items = new(128);

    /// <summary>The sheet status icons are cut from, or null before the assets are configured.</summary>
    private IConditionSheet _conditionSheet;

    /// <summary>Supplies the icon sheet, so the overlay does not have to know how assets are stored.</summary>
    public void Configure(IConditionSheet conditionSheet) => _conditionSheet = conditionSheet;

    /// <summary>
    /// How large a status icon is drawn, in pixels.
    /// </summary>
    /// <remarks>
    /// Pushed in by the world rather than read from the settings here, because this class draws and
    /// does not know the game has preferences. Sixteen is the ordinary size; the small setting drops
    /// it to eleven, which is where the row of them stops being wider than the thing it belongs to
    /// on an enemy carrying six effects at once.
    /// </remarks>
    public float ConditionIconSize { get; set; } = 16f;

    /// <summary>Where the quest objective is on screen, or null when there is none.</summary>
    private Vector2? _questTarget;

    public override void _Ready()
    {
        // Ignore the mouse entirely: this sits over the world and must not eat clicks meant for it.
        MouseFilter = MouseFilterEnum.Ignore;
        UI.ScreenFit.FillScreen(this);
    }

    /// <summary>Whether health bars are drawn. The original let players turn them off; so does this.</summary>
    private bool _healthBars = true;

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
    /// Numbers that rise off something and fade: damage dealt, damage taken, experience gained.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Tied to the object it is about rather than to the ground under it. The original's status
    /// text reads the object's screen position every frame and ends the moment that object leaves
    /// the map, so a number tracks a monster that is still walking and stops the instant it is
    /// gone. Keeping a position taken at arrival instead leaves numbers hanging in mid-air over
    /// wherever the thing used to be.
    /// </para>
    /// <para>
    /// The world position is what a text with no object to belong to is drawn at, and what every
    /// text falls back to before the world has handed over a way to find its object.
    /// </para>
    /// </remarks>
    /// <param name="anchorId">The object the text belongs to, or zero for a fixed place.</param>
    public void AddFloatingText(int anchorId, float x, float y, float z, string text, Color colour)
    {
        ulong start = Time.GetTicksMsec();

        // Two numbers born on one object in the same breath sit exactly on top of each other and
        // neither can be read. The original queues them and plays them one at a time; this holds
        // each newcomer back far enough that the one before it has already climbed clear.
        if (anchorId != 0)
        {
            ulong latest = start;

            for (int i = 0; i < _texts.Count; i++)
                if (_texts[i].AnchorId == anchorId && _texts[i].StartMs + StaggerMs > latest)
                    latest = _texts[i].StartMs + StaggerMs;

            // Past a whole lifetime of backlog the queue is no longer telling the player anything
            // about now, so the newcomer is dropped rather than shown a second late.
            if (latest > start + (ulong)TextLifeMs)
                return;

            start = latest;
        }

        // A cap, because a boss taking a stream of hits can produce these faster than they expire
        // and the oldest are the least interesting.
        if (_texts.Count >= MostTexts)
            _texts.RemoveAt(0);

        _texts.Add(new FloatingText
        {
            AnchorId = anchorId,
            X = x,
            Y = y,
            Z = z,
            Text = text,
            Colour = colour,
            StartMs = start,
        });
    }

    /// <summary>How the overlay turns a world position into a screen one. Set by the world.</summary>
    public System.Func<float, float, float, Vector2> Project { get; set; }

    /// <summary>
    /// Where the top of an object's artwork is on screen now, or null once it has left the map.
    /// </summary>
    /// <remarks>
    /// Set by the world, which is the only thing that knows both the map and how tall a sprite
    /// draws. Returning null is how a text learns its subject is gone.
    /// </remarks>
    public System.Func<int, Vector2?> HeadOf { get; set; }

    private const int MostTexts = 64;

    /// <summary>How long a number lives, and how far it rises in that time. The original's numbers.</summary>
    private const float TextLifeMs = 1000f;

    private const float TextRisePixels = 40f;

    /// <summary>The air between the top of a sprite and a number rising off it.</summary>
    private const float TextClearance = 20f;

    /// <summary>How far apart in time two numbers on one object are held. Enough to clear each other.</summary>
    private const ulong StaggerMs = 350;

    private readonly List<FloatingText> _texts = new(MostTexts);

    private struct FloatingText
    {
        /// <summary>The object this is about, or zero for a text pinned to a place in the world.</summary>
        public int AnchorId;

        public float X;
        public float Y;
        public float Z;
        public string Text;
        public Color Colour;

        /// <summary>When this begins to rise, which is later than its arrival when it was queued.</summary>
        public ulong StartMs;
    }

    /// <summary>
    /// Draws the rising numbers and drops the ones that have finished.
    /// </summary>
    /// <remarks>
    /// They are deliberately not in <see cref="_items"/>: that list is cleared and refilled every
    /// frame from the entities in view, and a number has to outlive the frame it was made in.
    /// </remarks>
    private void DrawFloatingTexts(float stretch)
    {
        if (_texts.Count == 0 || Project == null)
            return;

        ulong now = Time.GetTicksMsec();

        for (int i = _texts.Count - 1; i >= 0; i--)
        {
            var text = _texts[i];

            // Queued behind an earlier one on the same object: alive, but not yet begun.
            if (now < text.StartMs)
                continue;

            float age = (now - text.StartMs) / TextLifeMs;

            if (age >= 1f)
            {
                _texts.RemoveAt(i);
                continue;
            }

            bool tracked = text.AnchorId != 0 && HeadOf != null;
            var head = tracked ? HeadOf(text.AnchorId) : null;

            // The subject has left the map, so the text has nothing left to be about.
            if (tracked && !head.HasValue)
            {
                _texts.RemoveAt(i);
                continue;
            }

            var at = (head ?? Project(text.X, text.Y, text.Z)) * stretch;

            // Over the artwork rather than across the feet, where the name plate already is.
            at.Y -= TextClearance + TextRisePixels * age;

            // Held at full strength for the first half, so it is legible before it starts to go.
            float alpha = age < 0.5f ? 1f : 1f - (age - 0.5f) * 2f;

            float width = Measure(text.Text, WorldFontSize);

            DrawLine(
                new Vector2(at.X - width / 2f, at.Y), text.Text, WorldFontSize,
                text.Colour with { A = alpha }, GlyphEdge);
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

    /// <summary>The items carrying a name this frame, ordered nearest the camera first.</summary>
    private readonly List<int> _names = new(256);

    /// <summary>The rectangles the name plates claimed this frame occupy, nearest first.</summary>
    private readonly List<Rect2> _plates = new(256);

    /// <summary>The items those rectangles belong to, in the same order.</summary>
    private readonly List<int> _drawn = new(256);

    /// <summary>
    /// Nearest the camera first: greater screen Y is further down the world and so in front.
    /// </summary>
    /// <remarks>
    /// Held in a field rather than written at the call site, so sorting a busy frame does not
    /// allocate a closure per frame. The tie-break on X keeps the order of two things on one row
    /// the same from frame to frame, which is what stops a suppressed plate from flickering.
    /// </remarks>
    private readonly System.Comparison<int> _frontToBack;

    public WorldOverlay() =>
        _frontToBack = (a, b) =>
        {
            int byDepth = _items[b].Anchor.Y.CompareTo(_items[a].Anchor.Y);
            return byDepth != 0 ? byDepth : _items[a].Anchor.X.CompareTo(_items[b].Anchor.X);
        };

    /// <summary>
    /// The name plates, laid out nearest first and drawn back to front.
    /// </summary>
    /// <remarks>
    /// <para>
    /// A pile of gravestones is a real thing a Nexus produces -- every death leaves one where the
    /// player fell, named after them -- and a plate is several times wider than the tile it stands
    /// on, so a row of graves puts a dozen names on one line. Drawn in whatever order the map
    /// happens to hold them, they cross each other into a smear with no winner.
    /// </para>
    /// <para>
    /// Depth decides it instead. Whichever name is nearest the camera claims its space first and is
    /// drawn last, so it lands whole on top of the ones behind it -- which is what the reference
    /// shows where a player stands on a portal and their name crosses the portal's.
    /// </para>
    /// </remarks>
    private void DrawNames(float stretch)
    {
        _names.Clear();

        foreach (var (_, index) in _ranked)
        {
            if (_names.Count >= MostNames)
                break;

            if (!string.IsNullOrEmpty(_items[index].Name))
                _names.Add(index);
        }

        if (_names.Count > 1)
            _names.Sort(_frontToBack);

        _plates.Clear();
        _drawn.Clear();

        foreach (int index in _names)
        {
            var plate = NamePlate(InReferencePixels(_items[index], stretch));

            if (Buried(plate))
                continue;

            _plates.Add(plate);
            _drawn.Add(index);
        }

        // Back to front, so a name that is partly crossed is crossed by the one in front of it.
        for (int i = _drawn.Count - 1; i >= 0; i--)
            DrawName(InReferencePixels(_items[_drawn[i]], stretch), _plates[i]);
    }

    /// <summary>
    /// Whether a plate is buried under the ones already claimed in front of it.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Not "does it touch one". <c>Interact fs.png</c> has a player standing on a portal with their
    /// name laid straight across the portal's -- sixty-five per cent of it covered, measured -- and
    /// the game draws both. One name crossing one name is how the game looks and is allowed here.
    /// </para>
    /// <para>
    /// Two bounds, and the reference sets both. A name may be crossed by one other and no more, and
    /// no more than seven tenths of it may go under. Either is enough to keep the front name whole
    /// and leave the one behind it recognisable; past them a plate is buried, adds nothing but the
    /// smear, and stands down until it is clear.
    /// </para>
    /// </remarks>
    private bool Buried(in Rect2 plate)
    {
        float covered = 0f;
        int crossings = 0;

        for (int i = 0; i < _plates.Count; i++)
        {
            var over = _plates[i].Intersection(plate);

            if (over.Size.X <= 0f || over.Size.Y <= 0f)
                continue;

            covered += over.Size.X;
            crossings++;
        }

        return crossings > MostCrossings || covered > plate.Size.X * MostCovered;
    }

    /// <summary>How many other names may lie across one. The reference shows one, never two.</summary>
    private const int MostCrossings = 1;

    /// <summary>How much of a name may go under before it stands down. Measured off the reference.</summary>
    private const float MostCovered = 0.7f;

    /// <summary>
    /// How many screen pixels one canvas unit of this layer covers.
    /// </summary>
    /// <remarks>
    /// The project's base resolution is 1280 by 720 and its stretch mode is <c>canvas_items</c>, so
    /// every unit this layer is handed -- including the projected position of an entity -- is
    /// already being scaled before it reaches the glass. Everything here is measured off a 1920 by
    /// 1080 reference, which is screen pixels, so the whole pass is drawn under the inverse and
    /// then written in those. Without it a three-pixel border comes out four and a half, and the
    /// type is rasterised at one size and displayed at another.
    /// </remarks>
    private float Stretch
    {
        get
        {
            var window = (Vector2)GetWindow().Size;
            var logical = GetViewport().GetVisibleRect().Size;

            return logical.X > 0f && window.X > 0f ? window.X / logical.X : 1f;
        }
    }

    public override void _Draw()
    {
        float stretch = Stretch;
        var bounds = GetViewportRect().Size;
        var middle = bounds * stretch / 2f;
        _bounds = bounds * stretch;

        _ranked.Clear();

        // Everything below is in reference pixels, which is what the measurements are in.
        DrawSetTransform(Vector2.Zero, 0f, Vector2.One / stretch);

        for (int i = 0; i < _items.Count; i++)
        {
            var item = _items[i];

            // A generous margin, since a name can be much wider than its anchor point.
            if (item.Anchor.X < -200f || item.Anchor.X > bounds.X + 200f ||
                item.Anchor.Y < -100f || item.Anchor.Y > bounds.Y + 100f)
                continue;

            var placed = InReferencePixels(item, stretch);

            if (_healthBars && placed.ShowHealthBar && placed.MaxHp > 0)
                DrawHealthBar(placed);

            if (placed.Conditions is { Count: > 0 })
                DrawConditions(placed);

            if (!string.IsNullOrEmpty(placed.Name) || !string.IsNullOrEmpty(placed.Bubble))
                _ranked.Add((placed.Anchor.DistanceSquaredTo(middle), i));
        }

        // Only sorted when there is more text than will be drawn; the usual screenful skips it.
        if (_ranked.Count > MostNames)
            _ranked.Sort(static (a, b) => a.Distance.CompareTo(b.Distance));

        DrawNames(stretch);

        // Balloons in a second pass, so one entity's name never lands on top of another's balloon.
        int bubbles = 0;

        foreach (var (_, index) in _ranked)
        {
            if (bubbles >= MostBubbles)
                break;

            var item = InReferencePixels(_items[index], stretch);

            if (!string.IsNullOrEmpty(item.Bubble))
            {
                DrawBubble(item);
                bubbles++;
            }
        }

        if (_questTarget.HasValue)
            DrawQuestMarker(_questTarget.Value * stretch, _bounds);

        DrawFloatingTexts(stretch);

        DrawSetTransform(Vector2.Zero, 0f, Vector2.One);
    }

    /// <summary>The viewport, in reference pixels. Set once a frame by <see cref="_Draw"/>.</summary>
    private Vector2 _bounds = new(1920f, 1080f);

    /// <summary>
    /// A copy of an item with its placement moved from canvas units into reference pixels.
    /// </summary>
    /// <remarks>
    /// A copy, and never written back into the list. Godot redraws a canvas item whenever it feels
    /// the need to and not only when the world has refilled the list, so scaling in place turns one
    /// extra redraw into an entity drawn half a screen away from itself.
    /// </remarks>
    private static OverlayItem InReferencePixels(OverlayItem item, float stretch)
    {
        item.Anchor *= stretch;
        item.SpriteHeight *= stretch;
        return item;
    }

    /// <summary>
    /// A line of text in the interface's face, at its natural size on this layer.
    /// </summary>
    /// <remarks>
    /// Deliberately not <see cref="UI.Style.DrawOverWorld"/>. That one rasterises against
    /// <see cref="UI.Style.Sharpness"/>, which the HUD's own canvas sets from the player's interface
    /// scale -- and this layer is not on that canvas. Its own transform already puts one unit on one
    /// screen pixel, so the size asked for is the size drawn and a second correction would undo it.
    /// </remarks>
    /// <param name="edge">How thick a black ring to lay under the glyphs. Zero for none.</param>
    private void DrawLine(Vector2 at, string text, int size, Color colour, int edge)
    {
        if (edge > 0)
            DrawStringOutline(
                UI.Style.Sans, at, text, HorizontalAlignment.Left, -1, size, edge, UI.Style.TextOutline);

        DrawString(UI.Style.Sans, at, text, HorizontalAlignment.Left, -1, size, colour);
    }

    /// <summary>How wide a line of that text is on this layer.</summary>
    private static float Measure(string text, int size) =>
        UI.Style.Sans.GetStringSize(text, HorizontalAlignment.Left, -1, size).X;

    /// <summary>The black ring every glyph drawn over open ground carries.</summary>
    private const int GlyphEdge = 2;

    /// <summary>
    /// The size names, balloons and rising numbers are all set at over the world.
    /// </summary>
    /// <remarks>
    /// Measured against the reference rather than taken from the original's own point size, because
    /// the face is not the same one. A name in <c>Stats fs.png</c> is fourteen pixels of ink tall
    /// and a line inside a balloon in the same shot is thirteen; this face at eighteen drew eleven,
    /// and its glyphs came out fifteen per cent narrow for the same string. Twenty-one puts both
    /// the height and the width of a line within a few per cent of the reference.
    /// </remarks>
    private const int WorldFontSize = 21;

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

        float size = ConditionIconSize;

        int count = item.Conditions.Count;
        float left = item.Anchor.X - size * count / 2f;

        // Above the artwork rather than across it: the anchor is where the entity's feet are, so
        // the icons have to clear its own height before they are over its head.
        float top = item.Anchor.Y - Mathf.Max(item.SpriteHeight, 16f) - size - ConditionGap;

        // The inset scales with the icon, or a small one is mostly disc.
        float inset = Mathf.Max(1f, Mathf.Round(size / 8f));

        for (int i = 0; i < count; i++)
        {
            var region = _conditionSheet.Region(item.Conditions[i]);
            if (!region.HasValue)
                continue;

            var box = new Rect2(left + i * size, top, size, size);

            // A disc behind each one, so a pale icon still reads over a pale floor.
            DrawCircle(box.Position + box.Size / 2f, size * 0.42f, new Color(0f, 0f, 0f, 0.45f));
            DrawTextureRectRegion(_conditionSheet.Texture, box.Grow(-inset), region.Value);
        }
    }

    // The speech balloon, measured off the balloon at (876,198)-(1012,237) in Stats fs.png. Three
    // pixels of border, the same three taken off each corner, ten of air either side of the line
    // and six above and below the line's own box, and a tail fifteen wide and fifteen deep off the
    // bottom edge. The tail is not symmetrical: its right edge drops straight down and its left
    // edge runs back up at forty-five degrees -- which is what the equal width and depth are for --
    // so the point of it is the bottom right corner rather than the middle.
    private const float BubbleBorder = 3f;

    private const float BubbleCut = 3f;
    private const float BubblePadX = 10f;
    private const float BubblePadY = 6f;
    private const float TailWidth = 15f;
    private const float TailHeight = 15f;

    /// <summary>The balloon's dark maroon body.</summary>
    private static readonly Color BubbleFill = new("5b2626");

    /// <summary>Its border, which is also the colour the line inside it is set in.</summary>
    private static readonly Color BubbleEdge = new("fc8642");

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
        const int size = WorldFontSize;

        float textWidth = Measure(item.Bubble, size);
        float lineHeight = UI.Style.Sans.GetHeight(size);

        float width = Mathf.Round(textWidth + (BubblePadX + BubbleBorder) * 2f);
        float height = Mathf.Round(lineHeight + (BubblePadY + BubbleBorder) * 2f);

        // Above everything else the entity carries: its own artwork, and the status icons over that.
        float above = Mathf.Max(item.SpriteHeight, 16f)
                      + (item.Conditions is { Count: > 0 } ? ConditionIconSize + 8f : 0f);

        // The tail points at the speaker, and the body hangs centred over it -- so the tail's own
        // corner, not the middle of the balloon, is what lands on the entity.
        float tailRight = Mathf.Round(item.Anchor.X + TailWidth / 2f);
        float left = Mathf.Round(item.Anchor.X - width / 2f);
        float bottom = Mathf.Round(item.Anchor.Y - above - TailHeight - 2f);

        // Kept on screen, and the tail kept inside the body it hangs off.
        left = Mathf.Clamp(left, 4f, Mathf.Max(4f, _bounds.X - width - 4f));
        tailRight = Mathf.Clamp(
            tailRight, left + BubbleCut + TailWidth + BubbleBorder, left + width - BubbleCut - BubbleBorder);

        var box = new Rect2(left, bottom - height, width, height);

        DrawBubbleShadow(box, tailRight);

        DrawColoredPolygon(BalloonOutline(box, tailRight, 0f, TailWidth, TailHeight), BubbleEdge);

        DrawColoredPolygon(
            BalloonOutline(box.Grow(-BubbleBorder), tailRight - BubbleBorder, 0f,
                TailWidth - BubbleBorder, TailHeight - BubbleBorder * 2f),
            BubbleFill);

        // No black ring on this one: the line is on its own opaque plate, and the reference sets it
        // in the border's colour with nothing behind it.
        DrawLine(
            new Vector2(
                box.Position.X + BubbleBorder + BubblePadX,
                box.End.Y - BubbleBorder - BubblePadY - UI.Style.Sans.GetDescent(size)),
            item.Bubble, size, BubbleEdge, edge: 0);
    }

    /// <summary>
    /// The balloon's silhouette: a rectangle with its corners cut off and a tail on the bottom edge.
    /// </summary>
    /// <param name="tailRight">Where the tail's straight edge falls, in screen pixels.</param>
    /// <param name="cutBias">Taken off the corner cut, so an inset copy keeps the same slope.</param>
    private static Vector2[] BalloonOutline(
        in Rect2 box, float tailRight, float cutBias, float tailWidth, float tailHeight)
    {
        float cut = Mathf.Max(0f, BubbleCut - cutBias);
        float x0 = box.Position.X;
        float y0 = box.Position.Y;
        float x1 = box.End.X;
        float y1 = box.End.Y;

        return new[]
        {
            new Vector2(x0 + cut, y0),
            new Vector2(x1 - cut, y0),
            new Vector2(x1, y0 + cut),
            new Vector2(x1, y1 - cut),
            new Vector2(x1 - cut, y1),
            new Vector2(tailRight, y1),
            new Vector2(tailRight, y1 + tailHeight),
            new Vector2(tailRight - tailWidth, y1),
            new Vector2(x0 + cut, y1),
            new Vector2(x0, y1 - cut),
            new Vector2(x0, y0 + cut),
        };
    }

    /// <summary>
    /// The soft black halo under a balloon.
    /// </summary>
    /// <remarks>
    /// The original hands the balloon a sixteen-pixel drop shadow filter and lets Flash blur it.
    /// A blur at draw time would cost a render pass, so this is a stack of translucent rectangles
    /// stepping inwards instead. The numbers come off the reference: the ground five pixels under a
    /// balloon keeps about six tenths of its brightness and has recovered by fifteen, which is a
    /// squared falloff over twenty pixels at four and a half tenths in the middle.
    /// </remarks>
    private void DrawBubbleShadow(in Rect2 box, float tailRight)
    {
        var full = box.Merge(new Rect2(tailRight - TailWidth, box.End.Y, TailWidth, TailHeight));

        for (int i = 0; i < ShadowSteps.Length; i++)
            DrawRect(full.Grow(ShadowReach * (ShadowSteps.Length - 1 - i) / (float)ShadowSteps.Length),
                new Color(0f, 0f, 0f, ShadowSteps[i]));
    }

    /// <summary>How far the halo reaches past the balloon.</summary>
    private const float ShadowReach = 20f;

    /// <summary>How dark the halo is directly under the balloon.</summary>
    private const float ShadowStrength = 0.45f;

    /// <summary>
    /// What each ring of the halo contributes, outermost first.
    /// </summary>
    /// <remarks>
    /// Rectangles composite rather than replace, so a ring's own alpha is not the darkness wanted
    /// at that distance -- it is whatever turns the darkness already laid down into it. That is the
    /// arithmetic here, worked out once so the draw is eight rectangles and no per-frame maths.
    /// </remarks>
    private static readonly float[] ShadowSteps = BuildShadowSteps(8);

    private static float[] BuildShadowSteps(int rings)
    {
        var steps = new float[rings];
        float sofar = 0f;

        for (int i = 0; i < rings; i++)
        {
            // How far out this ring's edge is, as a fraction of the reach: the first is the whole
            // way out and the last sits on the balloon.
            float outward = (rings - 1 - i) / (float)rings;
            float wanted = ShadowStrength * (1f - outward) * (1f - outward);

            steps[i] = Mathf.Clamp((wanted - sofar) / Mathf.Max(0.001f, 1f - sofar), 0f, 1f);
            sofar = wanted;
        }

        return steps;
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
    /// The bar under a sprite.
    /// </summary>
    /// <remarks>
    /// The fill is one colour at every level. The original never turns it red -- what it does is
    /// pulse the *empty* end from grey towards red as the missing fraction grows, so a bar that is
    /// nearly gone flashes and a bar that has lost a sliver does not. Reading danger off the empty
    /// end rather than the full one is also what keeps the green readable at a glance.
    /// </remarks>
    private void DrawHealthBar(in OverlayItem item)
    {
        float top = Mathf.Round(item.Anchor.Y + BarOffsetY);
        float left = Mathf.Round(item.Anchor.X - BarWidth / 2f);

        DrawRect(new Rect2(left, top, BarWidth, BarHeight), Colors.Black);

        var inside = new Rect2(
            left + BarEdge, top + BarEdge, BarWidth - BarEdge * 2f, BarHeight - BarEdge * 2f);

        float fraction = Mathf.Clamp(item.Hp / (float)item.MaxHp, 0f, 1f);

        // The alarm is driven by the shared clock rather than anything per-entity, so a room full
        // of hurt monsters pulses in step.
        float alarm = Mathf.Abs(Mathf.Sin(Time.GetTicksMsec() / AlarmPeriodMs)) * (1f - fraction);
        DrawRect(inside, BarTrack.Lerp(BarTrackAlarm, alarm));

        if (fraction > 0f)
            DrawRect(inside with { Size = new Vector2(inside.Size.X * fraction, inside.Size.Y) }, BarFill);
    }

    /// <summary>The gap between a star and the name it belongs to.</summary>
    private const float StarGap = 5f;

    private const float StarSize = 11f;

    /// <summary>
    /// The rectangle an entity's name plate covers, its star included.
    /// </summary>
    /// <remarks>
    /// Under the feet rather than over the head, which is where the original puts it and why a
    /// crowd of players reads as a list you can scan: every name is on roughly the same line as the
    /// feet that own it. The star and the name are centred together, so the name itself sits a
    /// little right of the entity -- that offset is in the reference too.
    /// </remarks>
    private Rect2 NamePlate(in OverlayItem item)
    {
        const int size = WorldFontSize;

        float width = Measure(item.Name, size);
        float total = item.Stars >= 0 ? width + StarSize + StarGap : width;

        // Below the bar when there is one, or the two land on top of each other.
        float top = item.ShowHealthBar && _healthBars && item.MaxHp > 0
            ? Mathf.Round(item.Anchor.Y + BarOffsetY + BarHeight + NameGap)
            : Mathf.Round(item.Anchor.Y + NameOffsetY);

        // The star's row plus whatever hangs under the baseline: between them that is the ink. The
        // font's own line box reaches well above the cap line, and measuring by that would have a
        // name stand down for a neighbour it never touches.
        float height = StarSize + UI.Style.Sans.GetDescent(size);

        return new Rect2(Mathf.Round(item.Anchor.X - total / 2f), top, total, height);
    }

    /// <summary>The name itself, laid into the rectangle <see cref="NamePlate"/> measured for it.</summary>
    private void DrawName(in OverlayItem item, in Rect2 plate)
    {
        const int size = WorldFontSize;

        float left = plate.Position.X;
        float top = plate.Position.Y;

        if (item.Stars >= 0)
        {
            DrawStar(
                new Vector2(left + StarSize / 2f, top + StarSize / 2f), StarSize / 2f,
                UI.Fame.Colour(item.Stars, ClassCount, item.Admin));

            left += StarSize + StarGap;
        }

        // Baseline set so the cap line of the name and the top of the star share a row.
        float baseline = top + Mathf.Round(
            (StarSize + UI.Style.Sans.GetAscent(size) - UI.Style.Sans.GetDescent(size)) / 2f);

        DrawLine(new Vector2(left, baseline), item.Name, size, item.NameColor, GlyphEdge);
    }

    /// <summary>How many classes the game has, which is the width of each star colour's band.</summary>
    private const int ClassCount = 14;

    /// <summary>
    /// A five-pointed star with a black edge, the way one sits beside a name in the world.
    /// </summary>
    /// <remarks>
    /// The original composites a compiled star graphic out of its own assets; that artwork lives
    /// inside a SWF and cannot be pulled out, so the shape is built here from ten vertices. The
    /// edge is the same one every other glyph over the world gets, and for the same reason -- a
    /// pale star on pale ground is otherwise a smudge.
    /// </remarks>
    private void DrawStar(Vector2 centre, float radius, Color colour)
    {
        var points = new Vector2[10];

        for (int i = 0; i < points.Length; i++)
        {
            float reach = i % 2 == 0 ? radius : radius * 0.42f;
            float angle = -Mathf.Pi / 2f + i * Mathf.Pi / 5f;
            points[i] = centre + new Vector2(Mathf.Cos(angle), Mathf.Sin(angle)) * reach;
        }

        var edge = new Vector2[10];

        for (int i = 0; i < edge.Length; i++)
            edge[i] = centre + (points[i] - centre) * ((radius + 1.4f) / radius);

        DrawColoredPolygon(edge, Colors.Black);
        DrawColoredPolygon(points, colour);
    }
}
