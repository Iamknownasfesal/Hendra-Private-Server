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
    /// <summary>The bar under a sprite: forty-four by six, as the brief measures it.</summary>
    private const float BarHalfWidth = 22f;

    private const float BarHeight = 6f;
    private const float BarOffsetY = 4f;
    private const float NameOffsetY = -6f;

    /// <summary>The gap between the top of a sprite and the row of status icons over it.</summary>
    private const float ConditionGap = 6f;

    private static readonly Color BarBackground = new(0.33f, 0.33f, 0.33f);

    /// <summary>The brief's entity-health green, and the red it turns as the bar empties.</summary>
    private static readonly Color BarFill = new("4cd137");

    private static readonly Color BarLowFill = new("d02020");

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
    /// Anchored to a place in the world rather than to a place on the screen, and re-projected each
    /// frame, so a number stays over the thing it belongs to while the camera moves under it.
    /// </remarks>
    public void AddFloatingText(float x, float y, float z, string text, Color colour)
    {
        // A cap, because a boss taking a stream of hits can produce these faster than they expire
        // and the oldest are the least interesting.
        if (_texts.Count >= MostTexts)
            _texts.RemoveAt(0);

        _texts.Add(new FloatingText
        {
            X = x,
            Y = y,
            Z = z,
            Text = text,
            Colour = colour,
            BornMs = Time.GetTicksMsec(),
        });
    }

    /// <summary>How the overlay turns a world position into a screen one. Set by the world.</summary>
    public System.Func<float, float, float, Vector2> Project { get; set; }

    private const int MostTexts = 64;

    /// <summary>How long a number lives, and how far it rises in that time.</summary>
    private const float TextLifeMs = 900f;

    private const float TextRisePixels = 34f;

    private readonly List<FloatingText> _texts = new(MostTexts);

    private struct FloatingText
    {
        public float X;
        public float Y;
        public float Z;
        public string Text;
        public Color Colour;
        public ulong BornMs;
    }

    /// <summary>
    /// Draws the rising numbers and drops the ones that have finished.
    /// </summary>
    /// <remarks>
    /// They are deliberately not in <see cref="_items"/>: that list is cleared and refilled every
    /// frame from the entities in view, and a number has to outlive both the frame it was made in
    /// and, often, the monster it was made over.
    /// </remarks>
    private void DrawFloatingTexts()
    {
        if (_texts.Count == 0 || Project == null)
            return;

        ulong now = Time.GetTicksMsec();

        for (int i = _texts.Count - 1; i >= 0; i--)
        {
            var text = _texts[i];
            float age = (now - text.BornMs) / TextLifeMs;

            if (age >= 1f)
            {
                _texts.RemoveAt(i);
                continue;
            }

            var at = Project(text.X, text.Y, text.Z);

            // Quick at first and slowing, which reads as thrown off rather than floated up.
            at.Y -= TextRisePixels * Mathf.Sqrt(age);

            // Held at full strength for the first half, so it is legible before it starts to go.
            float alpha = age < 0.5f ? 1f : 1f - (age - 0.5f) * 2f;

            var size = _font.GetStringSize(text.Text, HorizontalAlignment.Left, -1, _fontSize);
            var origin = new Vector2(at.X - size.X / 2f, at.Y);

            DrawString(_font, origin + new Vector2(1f, 1f), text.Text, HorizontalAlignment.Left, -1,
                _fontSize, new Color(0f, 0f, 0f, 0.75f * alpha));

            DrawString(_font, origin, text.Text, HorizontalAlignment.Left, -1, _fontSize,
                text.Colour with { A = alpha });
        }
    }

    /// <summary>Call once per frame, after the items for that frame have been added.</summary>
    public void Commit() => QueueRedraw();

    public override void _Draw()
    {
        var bounds = GetViewportRect().Size;

        foreach (var item in _items)
        {
            // A generous margin, since a name can be much wider than its anchor point.
            if (item.Anchor.X < -200f || item.Anchor.X > bounds.X + 200f ||
                item.Anchor.Y < -100f || item.Anchor.Y > bounds.Y + 100f)
                continue;

            if (_healthBars && item.ShowHealthBar && item.MaxHp > 0)
                DrawHealthBar(item);

            if (!string.IsNullOrEmpty(item.Name))
                DrawName(item);

            if (item.Conditions is { Count: > 0 })
                DrawConditions(item);

            if (!string.IsNullOrEmpty(item.Bubble))
                DrawBubble(item);
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

        var measured = _font.GetStringSize(item.Bubble, HorizontalAlignment.Left, -1, _fontSize);

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

    private void DrawHealthBar(in OverlayItem item)
    {
        float top = item.Anchor.Y + BarOffsetY;
        var background = new Rect2(item.Anchor.X - BarHalfWidth, top, BarHalfWidth * 2f, BarHeight);
        DrawRect(background, BarBackground);

        float fraction = Mathf.Clamp(item.Hp / (float)item.MaxHp, 0f, 1f);

        if (fraction > 0f)
        {
            var fill = new Rect2(background.Position, new Vector2(background.Size.X * fraction, BarHeight));

            // Turning red as it empties makes a dangerous health level readable at a glance,
            // without having to read the number.
            DrawRect(fill, fraction < 0.25f ? BarLowFill : BarFill);
        }

        // A hairline of black around the whole thing, which is what keeps a green bar legible over
        // grass and a grey one legible over stone.
        DrawRect(background, Colors.Black, filled: false, width: 1f);
    }

    private void DrawName(in OverlayItem item)
    {
        var size = _font.GetStringSize(item.Name, HorizontalAlignment.Left, -1, _fontSize);
        var position = new Vector2(item.Anchor.X - size.X / 2f, item.Anchor.Y + NameOffsetY);

        // A cheap outline: the same text offset in four directions. Names sit over arbitrary
        // terrain, and without it a light name on light ground is unreadable.
        var outline = new Color(0f, 0f, 0f, 0.85f);
        DrawString(_font, position + new Vector2(1, 0), item.Name, HorizontalAlignment.Left, -1, _fontSize, outline);
        DrawString(_font, position + new Vector2(-1, 0), item.Name, HorizontalAlignment.Left, -1, _fontSize, outline);
        DrawString(_font, position + new Vector2(0, 1), item.Name, HorizontalAlignment.Left, -1, _fontSize, outline);
        DrawString(_font, position + new Vector2(0, -1), item.Name, HorizontalAlignment.Left, -1, _fontSize, outline);

        DrawString(_font, position, item.Name, HorizontalAlignment.Left, -1, _fontSize, item.NameColor);
    }
}
