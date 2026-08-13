using System;
using System.Collections.Generic;
using System.Globalization;
using Godot;

namespace Hendra.UI;

/// <summary>One entry in the quick-action tray.</summary>
public readonly struct QuickAction
{
    public readonly string Id;
    public readonly string Tooltip;

    /// <summary>The glyph, drawn the same way every other icon in the interface is.</summary>
    public readonly Action<CanvasItem, Rect2, Color> Icon;

    /// <summary>The number in the corner, or zero for no badge.</summary>
    public readonly int Badge;

    public QuickAction(string id, string tooltip, Action<CanvasItem, Rect2, Color> icon, int badge = 0)
    {
        Id = id;
        Tooltip = tooltip;
        Icon = icon;
        Badge = badge;
    }
}

/// <summary>
/// The row of quick actions under the player card: event entries, gifts, quick consumables.
/// </summary>
/// <remarks>
/// Hidden entirely when there is nothing in it. The row collapses rather than reserving space,
/// which is what lets the seasonal pass under it sit tight against the menu buttons on an account
/// that has no entries -- and this fork has no protocol for any of them, so that is the usual case.
/// </remarks>
public partial class QuickTray : Control
{
    public const float Height = 24f;

    private const float Gap = 4f;
    private const int Most = 8;

    private readonly List<QuickAction> _actions = new();

    private int _hovered = -1;

    public QuickTray()
    {
        MouseFilter = MouseFilterEnum.Stop;
        FocusMode = FocusModeEnum.None;
        Visible = false;
    }

    /// <summary>Raised with the id of the pressed action.</summary>
    public event Action<string> Activated;

    public void Set(IReadOnlyList<QuickAction> actions)
    {
        _actions.Clear();

        if (actions != null)
        {
            for (int i = 0; i < actions.Count && i < Most; i++)
                _actions.Add(actions[i]);
        }

        Visible = _actions.Count > 0;
        QueueRedraw();
    }

    public override void _Ready() => MouseExited += () => { _hovered = -1; QueueRedraw(); };

    private int At(float x)
    {
        int index = (int)(x / (Height + Gap));
        return index >= 0 && index < _actions.Count ? index : -1;
    }

    public override void _GuiInput(InputEvent @event)
    {
        switch (@event)
        {
            case InputEventMouseMotion motion:
                int over = At(motion.Position.X);
                if (over != _hovered)
                {
                    _hovered = over;
                    TooltipText = over < 0 ? string.Empty : _actions[over].Tooltip;
                    QueueRedraw();
                }

                return;

            case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left } button:
                int picked = At(button.Position.X);
                AcceptEvent();

                if (picked >= 0)
                    Activated?.Invoke(_actions[picked].Id);

                return;
        }
    }

    public override void _Draw()
    {
        for (int i = 0; i < _actions.Count; i++)
        {
            var box = new Rect2(i * (Height + Gap), 0f, Height, Height);

            DrawRect(box, _hovered == i ? Style.ButtonHover : Style.ButtonFace);
            DrawRect(box, Style.PanelEdge, filled: false, width: 1f);

            _actions[i].Icon?.Invoke(this, box.Grow(-4f), Style.Text);

            if (_actions[i].Badge <= 0)
                continue;

            // The count at the top right, on its own plate so it reads over any glyph.
            string badge = _actions[i].Badge.ToString(CultureInfo.InvariantCulture);
            float width = Style.Measure(badge, Style.FontTag) + 4f;
            var plate = new Rect2(box.End.X - width, box.Position.Y - 2f, width, 12f);

            DrawRect(plate, Style.HpFill);
            this.DrawToken(new Vector2(plate.Position.X + 2f, plate.End.Y - 3f), badge, Style.FontTag, Style.Text);
        }
    }
}

/// <summary>
/// The seasonal pass, docked under the quick tray.
/// </summary>
/// <remarks>
/// <para>
/// Collapsible, and hidden outright until a season is running. Its body is server-written copy,
/// which is why it is drawn a line at a time out of a list this control split itself: there is no
/// markup parser between whatever the server sends and the screen, so there is nothing in it for a
/// server to smuggle. It is capped at five lines with the rest scrolled.
/// </para>
/// <para>
/// Nothing in this fork sends one. The control exists so that the day something does, the copy has
/// somewhere to land that is already safe.
/// </para>
/// </remarks>
public partial class SeasonPass : Control
{
    private const float TitleHeight = 24f;
    private const float BarHeight = 16f;
    private const float LineHeight = 15f;
    private const int MostLines = 5;

    private readonly List<string> _body = new();

    private string _title = string.Empty;
    private string _countdown = string.Empty;
    private int _tier;
    private float _progress;
    private bool _collapsed;
    private int _scroll;

    public SeasonPass()
    {
        MouseFilter = MouseFilterEnum.Stop;
        FocusMode = FocusModeEnum.None;
        Visible = false;
    }

    public void Set(string title, string countdown, int tier, float progress, string body)
    {
        _title = title ?? string.Empty;
        _countdown = countdown ?? string.Empty;
        _tier = tier;
        _progress = Mathf.Clamp(progress, 0f, 1f);

        _body.Clear();

        // Split on newlines only, and never interpreted. Whatever the server wrote is drawn as the
        // characters it wrote, tags and all.
        if (!string.IsNullOrEmpty(body))
        {
            foreach (string line in body.Split('\n'))
                _body.Add(line.Replace("\r", string.Empty));
        }

        Visible = _title.Length > 0;
        _scroll = 0;

        Size = new Vector2(Size.X, Visible ? TitleHeight + BarHeight + 6f + Lines() * LineHeight + 6f : 0f);
        QueueRedraw();
    }

    private int Lines() => _collapsed ? 0 : Mathf.Min(_body.Count, MostLines);

    public override void _GuiInput(InputEvent @event)
    {
        switch (@event)
        {
            case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left } button
                when button.Position.Y <= TitleHeight:
                _collapsed = !_collapsed;
                Size = new Vector2(Size.X, TitleHeight + BarHeight + 6f + Lines() * LineHeight + 6f);
                QueueRedraw();
                AcceptEvent();
                return;

            case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelDown }:
                _scroll = Mathf.Min(_scroll + 1, Mathf.Max(0, _body.Count - MostLines));
                QueueRedraw();
                return;

            case InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelUp }:
                _scroll = Mathf.Max(0, _scroll - 1);
                QueueRedraw();
                return;
        }
    }

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);

        DrawRect(full, Style.Panel);
        DrawRect(full, Style.PanelEdge, filled: false, width: 1f);

        float baseline = Style.BaselineIn(TitleHeight, Style.FontBody);

        this.DrawText(new Vector2(8f, baseline), _title, Style.FontBody, Style.Text);
        this.DrawText(
            new Vector2(Size.X - 8f - Style.Measure(_countdown, Style.FontSmall), baseline),
            _countdown, Style.FontSmall, Style.TextDim);

        if (_collapsed)
            return;

        // The tier bar, in the fame amber, with the tier it has reached beside it.
        string tier = _tier.ToString(CultureInfo.InvariantCulture);
        float tierWidth = Style.Measure(tier, Style.FontBody) + 10f;
        var bar = new Rect2(8f, TitleHeight, Size.X - 16f - tierWidth, BarHeight);

        DrawRect(bar, Style.BarTrack);

        if (_progress > 0f)
        {
            DrawRect(new Rect2(bar.Position.X + 1f, bar.Position.Y + 1f,
                (bar.Size.X - 2f) * _progress, bar.Size.Y - 2f), Style.FameFill);
        }

        DrawRect(bar, Style.BarEdge, filled: false, width: 1f);

        this.DrawText(
            new Vector2(bar.End.X + 6f, bar.End.Y - 4f), tier, Style.FontBody, Style.FameFill);

        float y = TitleHeight + BarHeight + 6f;
        for (int i = _scroll; i < _body.Count && i - _scroll < MostLines; i++)
        {
            this.DrawText(new Vector2(8f, y + LineHeight - 4f), _body[i], Style.FontSmall, Style.TextDim);
            y += LineHeight;
        }
    }
}
