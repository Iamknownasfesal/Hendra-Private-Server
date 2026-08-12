using Godot;

namespace Hendra.UI;

/// <summary>
/// A character's level, in a plate of its own.
/// </summary>
/// <remarks>
/// The level is one of the two things you compare between characters, and a number trailing a name
/// as more words is not something you compare — it is something you read. A badge is scannable down
/// a column, and it warms as the level climbs so the strongest character is the brightest thing on
/// the screen.
/// </remarks>
public partial class LevelBadge : Control
{
    /// <summary>The cap, where a character stops levelling and starts earning fame.</summary>
    private const int MaxLevel = 20;

    private readonly int _level;
    private readonly Color _colour;

    public LevelBadge(int level)
    {
        _level = level;

        float climb = Mathf.Clamp(level / (float)MaxLevel, 0f, 1f);
        _colour = Style.Steel.Lerp(Style.Gold, climb);

        CustomMinimumSize = new Vector2(52, 22);
    }

    public override void _Ready()
    {
        var label = new Label
        {
            Text = $"Lv {_level.ToString(System.Globalization.CultureInfo.InvariantCulture)}",
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
            MouseFilter = MouseFilterEnum.Ignore,
        };
        label.SetAnchorsPreset(LayoutPreset.FullRect);
        label.AddThemeFontSizeOverride("font_size", 14);
        label.AddThemeColorOverride("font_color", _colour);
        AddChild(label);
    }

    public override void _Draw()
    {
        var box = new Rect2(Vector2.Zero, Size);
        DrawRect(box, _colour with { A = 0.13f });
        DrawRect(box, _colour with { A = 0.5f }, filled: false, width: 1f);
    }
}
