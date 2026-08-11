using System.Collections.Generic;
using Godot;

namespace Hendra.Render;

/// <summary>One entity's on-screen furniture: a health bar, a name, or both.</summary>
public struct OverlayItem
{
    /// <summary>Where the entity's feet land on screen, in pixels.</summary>
    public Vector2 Anchor;

    public string Name;
    public Color NameColor;

    public int Hp;
    public int MaxHp;
    public bool ShowHealthBar;
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
    private const float BarHalfWidth = 20f;
    private const float BarHeight = 6f;
    private const float BarOffsetY = 4f;
    private const float NameOffsetY = -6f;

    private static readonly Color BarBackground = new(0.33f, 0.33f, 0.33f);
    private static readonly Color BarFill = new(0.06f, 1.0f, 0.0f);
    private static readonly Color BarLowFill = new(1.0f, 0.15f, 0.0f);

    private readonly List<OverlayItem> _items = new(128);
    private Font _font;
    private int _fontSize = 13;

    public override void _Ready()
    {
        // Ignore the mouse entirely: this sits over the world and must not eat clicks meant for it.
        MouseFilter = MouseFilterEnum.Ignore;
        UI.ScreenFit.FillScreen(this);
        _font = ThemeDB.FallbackFont;
    }

    public void Clear() => _items.Clear();

    public void Add(in OverlayItem item) => _items.Add(item);

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

            if (item.ShowHealthBar && item.MaxHp > 0)
                DrawHealthBar(item);

            if (!string.IsNullOrEmpty(item.Name))
                DrawName(item);
        }
    }

    private void DrawHealthBar(in OverlayItem item)
    {
        float top = item.Anchor.Y + BarOffsetY;
        var background = new Rect2(item.Anchor.X - BarHalfWidth, top, BarHalfWidth * 2f, BarHeight);
        DrawRect(background, BarBackground);

        float fraction = Mathf.Clamp(item.Hp / (float)item.MaxHp, 0f, 1f);
        if (fraction <= 0f)
            return;

        var fill = new Rect2(background.Position, new Vector2(background.Size.X * fraction, BarHeight));

        // Turning red as it empties makes a dangerous health level readable at a glance, without
        // having to read the number.
        DrawRect(fill, fraction < 0.25f ? BarLowFill : BarFill);
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
