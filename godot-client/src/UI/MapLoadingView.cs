using Godot;

namespace Hendra.UI;

/// <summary>
/// The screen shown while a world is being entered.
/// </summary>
/// <remarks>
/// <para>
/// The original's <c>MapLoadingView</c>: the world's display name across the middle, its difficulty
/// beneath as a row of markers, and a fade out once the map has arrived. It covers the gap between
/// leaving one world and standing in the next — a gap the port previously spent showing the old
/// world frozen, then the new one popping in mid-stream, which reads as a stutter rather than as
/// travel.
/// </para>
/// <para>
/// It is also doing real work beyond hiding a seam. A world arrives over several packets and the
/// ground fills in as they land; covering that means the first thing you see is a finished map
/// rather than one assembling itself around you.
/// </para>
/// </remarks>
public partial class MapLoadingView : Control
{
    /// <summary>How long the cover takes to fade once the world is ready.</summary>
    private const float FadeSeconds = 0.45f;

    /// <summary>
    /// How long it stays up at minimum.
    /// </summary>
    /// <remarks>
    /// A local server answers fast enough that the screen would otherwise be a single frame of
    /// flash, which is worse than not having one. Long enough to read the name is the point of it.
    /// </remarks>
    private const float MinimumSeconds = 0.8f;

    private Label _name;
    private DifficultyMarks _difficulty;
    private ColorRect _cover;

    private double _shownFor;
    private bool _ready;
    private float _fade = 1f;

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Stop;
        this.FillScreen();

        _cover = new ColorRect { Color = Style.Void };
        _cover.SetAnchorsPreset(LayoutPreset.FullRect);
        _cover.MouseFilter = MouseFilterEnum.Ignore;
        AddChild(_cover);

        AddChild(new Starfield());

        var centre = new CenterContainer();
        centre.SetAnchorsPreset(LayoutPreset.FullRect);
        centre.MouseFilter = MouseFilterEnum.Ignore;
        AddChild(centre);

        var column = new VBoxContainer { MouseFilter = MouseFilterEnum.Ignore };
        column.AddThemeConstantOverride("separation", 14);
        centre.AddChild(column);

        _name = new Label
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            MouseFilter = MouseFilterEnum.Ignore,
        };
        _name.AddThemeFontSizeOverride("font_size", 34);
        _name.AddThemeColorOverride("font_color", Style.Text);
        column.AddChild(_name);

        _difficulty = new DifficultyMarks { MouseFilter = MouseFilterEnum.Ignore };
        column.AddChild(_difficulty);
    }

    /// <summary>Names the world being entered. Called as soon as MapInfo lands.</summary>
    public void Show(string displayName, int difficulty)
    {
        _ready = false;
        _shownFor = 0;
        _fade = 1f;
        Modulate = Colors.White;
        Visible = true;

        if (_name != null)
            _name.Text = string.IsNullOrWhiteSpace(displayName) ? "Travelling" : displayName;

        _difficulty?.Set(difficulty);
    }

    /// <summary>The world is standing up. Starts the fade, once it has been up long enough to read.</summary>
    public void Finish() => _ready = true;

    public override void _Process(double delta)
    {
        if (!Visible)
            return;

        _shownFor += delta;

        if (!_ready || _shownFor < MinimumSeconds)
            return;

        _fade = Mathf.MoveToward(_fade, 0f, (float)delta / FadeSeconds);
        Modulate = new Color(1f, 1f, 1f, _fade);

        if (_fade <= 0f)
            Visible = false;
    }

    /// <summary>
    /// The world's difficulty, as a row of markers.
    /// </summary>
    /// <remarks>
    /// The original's loading screen has a difficulty indicator beside the name — the one thing
    /// besides the name worth knowing before you arrive somewhere.
    /// </remarks>
    private sealed partial class DifficultyMarks : Control
    {
        private const int Most = 5;

        private int _difficulty;

        public DifficultyMarks() => CustomMinimumSize = new Vector2(Most * 26, 18);

        public void Set(int difficulty)
        {
            _difficulty = Mathf.Clamp(difficulty, 0, Most);
            QueueRedraw();
        }

        public override void _Draw()
        {
            if (_difficulty <= 0)
                return;

            float spacing = Size.X / Most;
            float radius = 5f;

            for (int i = 0; i < Most; i++)
            {
                var at = new Vector2(spacing * (i + 0.5f), Size.Y / 2f);
                bool lit = i < _difficulty;

                DrawCircle(at, radius, lit
                    ? Style.Gold.Lerp(Style.Danger, i / (float)(Most - 1))
                    : new Color(1f, 1f, 1f, 0.12f));
            }
        }
    }
}
