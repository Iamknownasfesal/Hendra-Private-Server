using System;
using Godot;
using Hendra.App;

namespace Hendra.UI;

/// <summary>
/// The options panel: volumes, the camera, and a reminder of what the keys do.
/// </summary>
/// <remarks>
/// <para>
/// Deliberately small. The original's options screen ran to six hundred lines across a dozen
/// classes and a key-remapping widget per binding; almost all of it existed to let players rebind
/// keys, which this does not do yet. What it does do is the part players actually reach for — being
/// able to turn the music down.
/// </para>
/// <para>
/// Changes apply as they are made rather than on a confirm button. A volume slider you cannot hear
/// until you close the panel is useless.
/// </para>
/// </remarks>
public partial class OptionsView : Control
{
    private const int PanelWidth = 380;

    /// <summary>The keys the game uses, shown as a reminder. Fixed until rebinding exists.</summary>
    private static readonly (string Key, string Does)[] Bindings =
    {
        ("W A S D", "Move"),
        ("Q E", "Turn the camera"),
        ("R", "Reset the camera"),
        ("Left mouse", "Shoot"),
        ("Space", "Use ability"),
        ("Shift", "Hold fire"),
        ("F", "Health potion"),
        ("G", "Magic potion"),
        ("Enter", "Chat"),
        ("Insert", "Nexus"),
        ("Escape", "These options"),
    };

    private Settings _settings;
    private PanelContainer _panel;

    /// <summary>Raised whenever a value changes, so the caller can apply and save it.</summary>
    public event Action Changed;

    public bool IsOpen => _panel is { Visible: true };

    public void Configure(Settings settings) => _settings = settings;

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;
        this.FillScreen();

        _panel = new PanelContainer { Visible = false, MouseFilter = MouseFilterEnum.Stop };
        _panel.AddThemeStyleboxOverride("panel", new StyleBoxFlat
        {
            BgColor = new Color(0.07f, 0.07f, 0.08f, 0.97f),
            BorderColor = new Color(0.3f, 0.3f, 0.34f),
            BorderWidthTop = 1,
            BorderWidthBottom = 1,
            BorderWidthLeft = 1,
            BorderWidthRight = 1,
        });
        _panel.SetAnchorsPreset(LayoutPreset.Center);
        _panel.OffsetLeft = -PanelWidth / 2f;
        _panel.OffsetRight = PanelWidth / 2f;
        _panel.OffsetTop = -210;
        _panel.OffsetBottom = 210;
        AddChild(_panel);

        var margin = new MarginContainer();
        foreach (string side in new[] { "margin_left", "margin_top", "margin_right", "margin_bottom" })
            margin.AddThemeConstantOverride(side, 14);
        _panel.AddChild(margin);

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 8);
        margin.AddChild(column);

        var title = new Label { Text = "Options" };
        title.AddThemeFontSizeOverride("font_size", 20);
        column.AddChild(title);

        AddSlider(column, "Sound", () => _settings.EffectVolume, value =>
        {
            _settings.EffectVolume = value;

            // Played on release so the slider demonstrates the level it has just been set to.
            ServiceLocator.Audio?.PlayEffect("button_click");
        });

        AddSlider(column, "Music", () => _settings.MusicVolume, value => _settings.MusicVolume = value);

        var centred = new CheckBox { Text = "Keep the player centred" };
        centred.ButtonPressed = _settings.CenterOnPlayer;
        centred.Toggled += on =>
        {
            _settings.CenterOnPlayer = on;
            Changed?.Invoke();
        };
        column.AddChild(centred);

        column.AddChild(new HSeparator());

        var keys = new GridContainer { Columns = 2 };
        keys.AddThemeConstantOverride("h_separation", 16);
        keys.AddThemeConstantOverride("v_separation", 1);
        column.AddChild(keys);

        foreach (var (key, does) in Bindings)
        {
            var name = new Label { Text = key };
            name.AddThemeColorOverride("font_color", new Color(0.62f, 0.62f, 0.62f));
            keys.AddChild(name);
            keys.AddChild(new Label { Text = does });
        }

        var close = new Button { Text = "Close" };
        close.Pressed += Toggle;
        column.AddChild(close);
    }

    private void AddSlider(Control parent, string label, Func<float> get, Action<float> set)
    {
        var row = new HBoxContainer();
        row.AddThemeConstantOverride("separation", 10);
        parent.AddChild(row);

        row.AddChild(new Label { Text = label, CustomMinimumSize = new Vector2(60, 0) });

        var slider = new HSlider
        {
            MinValue = 0,
            MaxValue = 1,
            Step = 0.05,
            Value = get(),
            SizeFlagsHorizontal = SizeFlags.ExpandFill,
            CustomMinimumSize = new Vector2(200, 0),
        };

        var readout = new Label
        {
            Text = Percent(get()),
            CustomMinimumSize = new Vector2(48, 0),
            HorizontalAlignment = HorizontalAlignment.Right,
        };

        slider.ValueChanged += value =>
        {
            set((float)value);
            readout.Text = Percent((float)value);
            Changed?.Invoke();
        };

        row.AddChild(slider);
        row.AddChild(readout);
    }

    private static string Percent(float value) => $"{Mathf.RoundToInt(value * 100f)}%";

    /// <summary>Opens the panel, or closes it if it is already open.</summary>
    public void Toggle()
    {
        if (_panel == null)
            return;

        _panel.Visible = !_panel.Visible;
    }
}
